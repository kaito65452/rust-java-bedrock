use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Cursor, Read, Seek, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::{IoError, sanitize_archive_path};

/// Default single-file uncompressed limit (64 MiB).
pub const DEFAULT_MAX_SINGLE_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Default total uncompressed pack limit (512 MiB).
pub const DEFAULT_MAX_TOTAL_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
/// Default maximum file entry count.
pub const DEFAULT_MAX_FILE_COUNT: usize = 50_000;

/// Security and stability limits enforced while reading packs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoLimits {
    /// Maximum allowed uncompressed bytes for any single entry.
    pub max_single_file_bytes: u64,
    /// Maximum allowed cumulative uncompressed bytes across the entire pack.
    pub max_total_uncompressed_bytes: u64,
    /// Maximum allowed number of entries in the archive or directory.
    pub max_file_count: usize,
}

impl Default for IoLimits {
    fn default() -> Self {
        Self {
            max_single_file_bytes: DEFAULT_MAX_SINGLE_FILE_BYTES,
            max_total_uncompressed_bytes: DEFAULT_MAX_TOTAL_UNCOMPRESSED_BYTES,
            max_file_count: DEFAULT_MAX_FILE_COUNT,
        }
    }
}

/// Deterministic, forward-slash-normalized in-memory resource pack container.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackContainer {
    files: BTreeMap<String, Vec<u8>>,
}

impl PackContainer {
    /// Creates an empty container.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or replaces a file after validating and normalizing its path.
    pub fn insert(&mut self, path: impl AsRef<str>, data: Vec<u8>) -> Result<(), IoError> {
        let clean = sanitize_archive_path(path.as_ref())?;
        self.files.insert(clean, data);
        Ok(())
    }

    /// Retrieves raw bytes for a normalized path.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    /// Returns `true` if `path` exists in the container.
    #[must_use]
    pub fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    /// Removes and returns a file if present.
    pub fn remove(&mut self, path: &str) -> Option<Vec<u8>> {
        self.files.remove(path)
    }

    /// Returns all files in deterministic lexicographical order.
    #[must_use]
    pub fn files(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.files
    }

    /// Consumes the container and returns the underlying deterministic file map.
    #[must_use]
    pub fn into_files(self) -> BTreeMap<String, Vec<u8>> {
        self.files
    }

    /// Returns the number of files in the container.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Returns `true` if the container holds no files.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Returns total byte size across all files in the container.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.files.values().map(|v| v.len() as u64).sum()
    }

    /// Processes all files in parallel via `rayon` while preserving deterministic key order.
    pub fn par_map_files<T, E, F>(&self, f: F) -> Result<Vec<T>, E>
    where
        F: Fn(&str, &[u8]) -> Result<T, E> + Sync + Send,
        T: Send,
        E: Send,
    {
        let ordered: Vec<(&String, &Vec<u8>)> = self.files.iter().collect();
        ordered
            .into_par_iter()
            .map(|(path, bytes)| f(path.as_str(), bytes.as_slice()))
            .collect()
    }

    /// Reads a resource pack from either a `.zip`/`.mcpack` file or a directory.
    pub fn read_from_path(path: &Path, limits: IoLimits) -> Result<Self, IoError> {
        let meta = fs::metadata(path)?;
        if meta.is_dir() {
            Self::read_from_directory(path, limits)
        } else {
            let file = File::open(path)?;
            Self::read_from_zip_reader(file, limits)
        }
    }

    /// Reads a resource pack from in-memory `.zip` archive bytes.
    pub fn read_from_zip_bytes(bytes: &[u8], limits: IoLimits) -> Result<Self, IoError> {
        Self::read_from_zip_reader(Cursor::new(bytes), limits)
    }

    /// Reads a `.zip` archive from any `Read + Seek` stream while enforcing security & size limits.
    pub fn read_from_zip_reader<R: Read + Seek>(
        reader: R,
        limits: IoLimits,
    ) -> Result<Self, IoError> {
        let mut archive = ZipArchive::new(reader)?;
        let entry_count = archive.len();
        if entry_count > limits.max_file_count {
            return Err(IoError::FileCountLimitExceeded {
                limit: limits.max_file_count,
            });
        }

        let mut container = Self::new();
        let mut total_bytes: u64 = 0;

        for idx in 0..entry_count {
            let mut entry = archive.by_index(idx)?;
            let raw_name = entry.name().to_string();

            // Allow harmless root directory markers such as "./"
            if entry.is_dir() && (raw_name == "." || raw_name == "./") {
                continue;
            }

            // Validate entry path BEFORE checking `is_dir()` so malicious directories are rejected.
            let clean_path = sanitize_archive_path(&raw_name)?;

            // Reject symbolic links stored in POSIX zip headers (`S_IFLNK = 0o120000`).
            if let Some(mode) = entry.unix_mode()
                && (mode & 0o17_0000) == 0o12_0000
            {
                return Err(IoError::ZipSlipDetected { raw_path: raw_name });
            }

            if entry.is_dir() {
                continue;
            }

            if entry.size() > limits.max_single_file_bytes {
                return Err(IoError::FileSizeLimitExceeded {
                    path: clean_path,
                    limit: limits.max_single_file_bytes,
                });
            }

            let mut buf = Vec::with_capacity(
                usize::try_from(entry.size().min(limits.max_single_file_bytes)).unwrap_or(0),
            );
            let mut limited = (&mut entry).take(limits.max_single_file_bytes.saturating_add(1));
            limited.read_to_end(&mut buf)?;

            if (buf.len() as u64) > limits.max_single_file_bytes {
                return Err(IoError::FileSizeLimitExceeded {
                    path: clean_path,
                    limit: limits.max_single_file_bytes,
                });
            }

            total_bytes = total_bytes.checked_add(buf.len() as u64).ok_or(
                IoError::TotalSizeLimitExceeded {
                    limit: limits.max_total_uncompressed_bytes,
                },
            )?;
            if total_bytes > limits.max_total_uncompressed_bytes {
                return Err(IoError::TotalSizeLimitExceeded {
                    limit: limits.max_total_uncompressed_bytes,
                });
            }

            container.files.insert(clean_path, buf);
        }

        container.unwrap_single_root_folder();
        Ok(container)
    }

    /// Reads a resource pack directory recursively in deterministic order using parallel file reads.
    pub fn read_from_directory(root: &Path, limits: IoLimits) -> Result<Self, IoError> {
        let mut discovered: Vec<(String, PathBuf, u64)> = Vec::new();
        collect_dir_entries(root, root, &mut discovered, limits.max_file_count)?;
        discovered.sort_by(|a, b| a.0.cmp(&b.0));

        let mut running_total: u64 = 0;
        for (rel_path, _, size) in &discovered {
            if *size > limits.max_single_file_bytes {
                return Err(IoError::FileSizeLimitExceeded {
                    path: rel_path.clone(),
                    limit: limits.max_single_file_bytes,
                });
            }
            running_total =
                running_total
                    .checked_add(*size)
                    .ok_or(IoError::TotalSizeLimitExceeded {
                        limit: limits.max_total_uncompressed_bytes,
                    })?;
            if running_total > limits.max_total_uncompressed_bytes {
                return Err(IoError::TotalSizeLimitExceeded {
                    limit: limits.max_total_uncompressed_bytes,
                });
            }
        }

        let loaded: Result<Vec<(String, Vec<u8>)>, IoError> = discovered
            .into_par_iter()
            .map(|(rel_path, full_path, _)| {
                let bytes = fs::read(&full_path)?;
                if (bytes.len() as u64) > limits.max_single_file_bytes {
                    return Err(IoError::FileSizeLimitExceeded {
                        path: rel_path,
                        limit: limits.max_single_file_bytes,
                    });
                }
                Ok((rel_path, bytes))
            })
            .collect();

        let entries = loaded?;
        let mut total_actual: u64 = 0;
        let mut files = BTreeMap::new();
        for (rel_path, bytes) in entries {
            total_actual = total_actual.saturating_add(bytes.len() as u64);
            if total_actual > limits.max_total_uncompressed_bytes {
                return Err(IoError::TotalSizeLimitExceeded {
                    limit: limits.max_total_uncompressed_bytes,
                });
            }
            files.insert(rel_path, bytes);
        }

        let mut container = Self { files };
        container.unwrap_single_root_folder();
        Ok(container)
    }

    /// Strips wrapping folder prefixes (e.g. `Downloads/MyPack/Pack.mcmeta` -> `pack.mcmeta`)
    /// when all non-junk entries sit inside a single folder containing `pack.mcmeta` or `manifest.json`,
    /// and normalizes root metadata filename casing.
    pub fn unwrap_single_root_folder(&mut self) {
        self.normalize_root_metadata_names();
        if self.files.contains_key("pack.mcmeta") || self.files.contains_key("manifest.json") {
            return;
        }

        let mut candidates: Vec<String> = self
            .files
            .keys()
            .filter_map(|key| {
                let (prefix, file_name) = key.rsplit_once('/')?;
                let lower = file_name.to_ascii_lowercase();
                if !prefix.is_empty() && (lower == "pack.mcmeta" || lower == "manifest.json") {
                    Some(format!("{prefix}/"))
                } else {
                    None
                }
            })
            .collect();
        // Shallowest nesting depth first
        candidates.sort_by_key(|p| (p.matches('/').count(), p.len()));

        let Some(prefix) = candidates
            .iter()
            .find(|prefix| {
                self.files
                    .keys()
                    .all(|k| k.starts_with(prefix.as_str()) || is_os_junk(k))
            })
            .cloned()
            .or_else(|| candidates.into_iter().next())
        else {
            return;
        };

        let mut unwrapped = BTreeMap::new();
        for (path, bytes) in std::mem::take(&mut self.files) {
            if let Some(stripped) = path.strip_prefix(&prefix)
                && !stripped.is_empty()
            {
                unwrapped.insert(stripped.to_string(), bytes);
            }
        }
        self.files = unwrapped;
        self.normalize_root_metadata_names();
    }

    fn normalize_root_metadata_names(&mut self) {
        let keys_to_fix: Vec<(String, &'static str)> = self
            .files
            .keys()
            .filter_map(|k| {
                if k.contains('/') {
                    return None;
                }
                match k.to_ascii_lowercase().as_str() {
                    "pack.mcmeta" if k != "pack.mcmeta" => Some((k.clone(), "pack.mcmeta")),
                    "pack.png" if k != "pack.png" => Some((k.clone(), "pack.png")),
                    "manifest.json" if k != "manifest.json" => Some((k.clone(), "manifest.json")),
                    "pack_icon.png" if k != "pack_icon.png" => Some((k.clone(), "pack_icon.png")),
                    _ => None,
                }
            })
            .collect();

        for (old_key, new_key) in keys_to_fix {
            if let Some(bytes) = self.files.remove(&old_key) {
                self.files.entry(new_key.to_string()).or_insert(bytes);
            }
        }
    }

    /// Writes all files into a `.zip` archive in deterministic lexicographical order.
    pub fn write_to_zip_writer<W: Write + Seek>(&self, writer: W) -> Result<(), IoError> {
        let mut zip = ZipWriter::new(writer);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        for (path, bytes) in &self.files {
            let clean = sanitize_archive_path(path)?;
            zip.start_file(clean, options)?;
            zip.write_all(bytes)?;
        }

        zip.finish()?;
        Ok(())
    }

    /// Serializes the container into in-memory `.zip` bytes.
    pub fn write_to_zip_bytes(&self) -> Result<Vec<u8>, IoError> {
        let mut cursor = Cursor::new(Vec::new());
        self.write_to_zip_writer(&mut cursor)?;
        Ok(cursor.into_inner())
    }

    /// Writes all files into `root` directory, creating subdirectories as needed.
    pub fn write_to_directory(&self, root: &Path) -> Result<(), IoError> {
        fs::create_dir_all(root)?;
        for (path, bytes) in &self.files {
            let clean = sanitize_archive_path(path)?;
            let target = root.join(&clean);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(target, bytes)?;
        }
        Ok(())
    }

    /// Writes the container to `path` as a `.zip`/`.mcpack` archive or directory based on extension.
    pub fn write_to_path(&self, path: &Path) -> Result<(), IoError> {
        let is_archive = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("zip") || ext.eq_ignore_ascii_case("mcpack")
            });

        if is_archive {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }
            let file = File::create(path)?;
            self.write_to_zip_writer(file)
        } else {
            self.write_to_directory(path)
        }
    }
}

fn collect_dir_entries(
    root: &Path,
    current: &Path,
    out: &mut Vec<(String, PathBuf, u64)>,
    max_file_count: usize,
) -> Result<(), IoError> {
    let mut entries: Vec<PathBuf> = fs::read_dir(current)?
        .map(|res| res.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();

    for entry_path in entries {
        let sym_meta = fs::symlink_metadata(&entry_path)?;
        if sym_meta.file_type().is_symlink() {
            return Err(IoError::ZipSlipDetected {
                raw_path: entry_path.display().to_string(),
            });
        }

        if sym_meta.is_dir() {
            collect_dir_entries(root, &entry_path, out, max_file_count)?;
        } else if sym_meta.is_file() {
            if out.len() >= max_file_count {
                return Err(IoError::FileCountLimitExceeded {
                    limit: max_file_count,
                });
            }
            let rel = entry_path.strip_prefix(root).map_err(|err| {
                IoError::InvalidArchive(format!(
                    "failed to relativize '{}': {err}",
                    entry_path.display()
                ))
            })?;
            let rel_str = rel.to_string_lossy();
            let clean = sanitize_archive_path(&rel_str)?;
            out.push((clean, entry_path, sym_meta.len()));
        }
    }

    Ok(())
}

fn is_os_junk(path: &str) -> bool {
    path.starts_with("__MACOSX/")
        || path == ".DS_Store"
        || path.ends_with("/.DS_Store")
        || path == "Thumbs.db"
        || path.ends_with("/Thumbs.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn build_raw_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut cursor);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for (name, data) in entries {
                writer.start_file(*name, opts).expect("start_file");
                writer.write_all(data).expect("write_all");
            }
            writer.finish().expect("finish");
        }
        cursor.into_inner()
    }

    #[test]
    fn rejects_zip_slip_entries() {
        for bad_name in [
            "../evil.png",
            "assets/../../evil.png",
            "/etc/passwd",
            r"C:\evil.png",
        ] {
            let zip_bytes = build_raw_zip(&[(bad_name, b"pwned")]);
            let result = PackContainer::read_from_zip_bytes(&zip_bytes, IoLimits::default());
            assert!(
                result.is_err(),
                "expected error for malicious path '{bad_name}'"
            );
        }
    }

    #[test]
    fn enforces_single_and_total_size_limits() {
        let big = vec![b'A'; 1024];
        let zip_bytes = build_raw_zip(&[("pack.mcmeta", &big)]);
        let strict_limits = IoLimits {
            max_single_file_bytes: 512,
            max_total_uncompressed_bytes: 4096,
            max_file_count: 100,
        };
        assert!(matches!(
            PackContainer::read_from_zip_bytes(&zip_bytes, strict_limits),
            Err(IoError::FileSizeLimitExceeded { .. })
        ));

        let zip_multi = build_raw_zip(&[("a.txt", &[b'x'; 400]), ("b.txt", &[b'y'; 400])]);
        let total_limits = IoLimits {
            max_single_file_bytes: 500,
            max_total_uncompressed_bytes: 600,
            max_file_count: 100,
        };
        assert!(matches!(
            PackContainer::read_from_zip_bytes(&zip_multi, total_limits),
            Err(IoError::TotalSizeLimitExceeded { .. })
        ));
    }

    #[test]
    fn deterministic_ordering_and_dir_zip_roundtrip() {
        let mut container = PackContainer::new();
        container
            .insert("z_file.txt", b"last".to_vec())
            .expect("insert z");
        container
            .insert(r"assets\minecraft\pack.txt", b"middle".to_vec())
            .expect("insert a");
        container
            .insert("a_file.txt", b"first".to_vec())
            .expect("insert first");

        let keys: Vec<&String> = container.files().keys().collect();
        assert_eq!(
            keys,
            vec!["a_file.txt", "assets/minecraft/pack.txt", "z_file.txt"]
        );

        let zip_bytes = container.write_to_zip_bytes().expect("write zip");
        let parsed =
            PackContainer::read_from_zip_bytes(&zip_bytes, IoLimits::default()).expect("read zip");
        assert_eq!(parsed, container);

        let temp = TempDir::new().expect("tempdir");
        parsed
            .write_to_directory(temp.path())
            .expect("write directory");
        let from_dir =
            PackContainer::read_from_directory(temp.path(), IoLimits::default()).expect("read dir");
        assert_eq!(from_dir, container);

        let lengths = from_dir
            .par_map_files(|path, data| Ok::<_, IoError>((path.to_string(), data.len())))
            .expect("parallel map");
        assert_eq!(
            lengths,
            vec![
                (String::from("a_file.txt"), 5),
                (String::from("assets/minecraft/pack.txt"), 6),
                (String::from("z_file.txt"), 4),
            ]
        );
    }

    #[test]
    fn unwraps_single_wrapped_root_folder() {
        let zip_bytes = build_raw_zip(&[
            ("KITSUNE/manifest.json", b"{}"),
            ("KITSUNE/textures/items/sword.png", b"png"),
            ("__MACOSX/KITSUNE/._manifest.json", b"junk"),
        ]);
        let parsed =
            PackContainer::read_from_zip_bytes(&zip_bytes, IoLimits::default()).expect("read zip");
        assert!(parsed.contains("manifest.json"));
        assert!(parsed.contains("textures/items/sword.png"));
        assert!(!parsed.contains("KITSUNE/manifest.json"));

        let nested_zip = build_raw_zip(&[
            ("Downloads/PvP_Pack_v2/Pack.mcmeta", b"{}"),
            ("Downloads/PvP_Pack_v2/Pack.png", b"icon"),
            (
                "Downloads/PvP_Pack_v2/assets/minecraft/textures/block/stone.png",
                b"stone",
            ),
            ("__MACOSX/Downloads/._PvP_Pack_v2", b"junk"),
        ]);
        let parsed_nested =
            PackContainer::read_from_zip_bytes(&nested_zip, IoLimits::default()).expect("read zip");
        assert!(parsed_nested.contains("pack.mcmeta"));
        assert!(parsed_nested.contains("pack.png"));
        assert!(parsed_nested.contains("assets/minecraft/textures/block/stone.png"));
    }
}
