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

            // Filter OS metadata junk files at scan time so they never enter the pipeline.
            if is_os_junk(&clean_path) {
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
    ///
    /// Preserves Bedrock Add-on multi-pack workspaces (`BP/` + `RP/`, or `pstdr.toml` + `BP/`),
    /// unwrapping only an outer archive wrapper folder (e.g. `my-addon/BP/...`, `my-addon/RP/...`)
    /// without ever dropping sibling packs or workspace files.
    pub fn unwrap_single_root_folder(&mut self) {
        self.normalize_root_metadata_names();
        if self.files.contains_key("pack.mcmeta") || self.files.contains_key("manifest.json") {
            return;
        }
        if is_bedrock_addon_keys(self.files.keys().map(String::as_str)) {
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
        if candidates.is_empty() {
            return;
        }
        // Shallowest nesting depth first
        candidates.sort_by_key(|p| (p.matches('/').count(), p.len()));

        // 1. Check if all non-junk entries sit inside a single outer wrapper directory
        //    (e.g. `my-addon/BP/manifest.json` + `my-addon/RP/manifest.json` + `my-addon/pstdr.toml`,
        //    or `Downloads/MyPack/pack.mcmeta`).
        if let Some(common_prefix) = common_directory_prefix(self.files.keys().map(String::as_str))
        {
            let stripped_keys: Vec<&str> = self
                .files
                .keys()
                .filter_map(|k| k.strip_prefix(&common_prefix))
                .filter(|s| !s.is_empty())
                .collect();

            let exposes_single_pack = stripped_keys.iter().any(|k| {
                !k.contains('/')
                    && (k.eq_ignore_ascii_case("pack.mcmeta")
                        || k.eq_ignore_ascii_case("manifest.json"))
            });
            let exposes_addon = is_bedrock_addon_keys(stripped_keys.iter().copied());

            if exposes_single_pack || exposes_addon {
                self.strip_prefix_in_place(&common_prefix);
                return;
            }
        }

        // 2. Check if a single manifest candidate wraps all non-junk files.
        if let Some(prefix) = candidates
            .iter()
            .find(|prefix| {
                self.files
                    .keys()
                    .all(|k| k.starts_with(prefix.as_str()) || is_os_junk(k))
            })
            .cloned()
        {
            self.strip_prefix_in_place(&prefix);
            return;
        }

        // 3. Fallback for single-pack archives that include a stray root file (e.g. README.txt):
        //    ONLY unwrap when there is a single pack manifest and no `pstdr.toml` workspace marker.
        let has_pstdr_toml = self.files.keys().any(|k| {
            k.rsplit('/')
                .next()
                .is_some_and(|f| f.eq_ignore_ascii_case("pstdr.toml"))
        });
        if candidates.len() == 1 && !has_pstdr_toml {
            self.strip_prefix_in_place(&candidates[0]);
        }
    }

    fn strip_prefix_in_place(&mut self, prefix: &str) {
        let mut unwrapped = BTreeMap::new();
        for (path, bytes) in std::mem::take(&mut self.files) {
            if let Some(stripped) = path.strip_prefix(prefix)
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

    /// Writes the container to `path` as a `.zip`/`.mcpack`/`.mcaddon` archive or directory based on extension.
    pub fn write_to_path(&self, path: &Path) -> Result<(), IoError> {
        let is_archive = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("zip")
                    || ext.eq_ignore_ascii_case("mcpack")
                    || ext.eq_ignore_ascii_case("mcaddon")
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
            if is_os_junk(&clean) {
                continue;
            }
            out.push((clean, entry_path, sym_meta.len()));
        }
    }

    Ok(())
}

/// Returns `true` if `keys` represent a Bedrock Add-on root layout:
/// either at least two depth-1 `<folder>/manifest.json` sub-packs (e.g. `BP/` + `RP/`),
/// or a root `pstdr.toml` alongside at least one depth-1 `<folder>/manifest.json`.
fn is_bedrock_addon_keys<'a>(keys: impl Iterator<Item = &'a str>) -> bool {
    let mut subpack_manifest_dirs = std::collections::BTreeSet::new();
    let mut has_root_pstdr_toml = false;

    for key in keys {
        if is_os_junk(key) {
            continue;
        }
        if !key.contains('/') {
            if key.eq_ignore_ascii_case("pstdr.toml") {
                has_root_pstdr_toml = true;
            }
            continue;
        }
        if let Some((dir, file)) = key.split_once('/')
            && !file.contains('/')
            && file.eq_ignore_ascii_case("manifest.json")
        {
            subpack_manifest_dirs.insert(dir.to_ascii_lowercase());
        }
    }

    subpack_manifest_dirs.len() >= 2 || (has_root_pstdr_toml && !subpack_manifest_dirs.is_empty())
}

/// Computes the longest common directory prefix (ending with `/`) shared by all non-junk keys,
/// returning `None` if no common directory prefix exists.
fn common_directory_prefix<'a>(keys: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut shared: Option<Vec<&str>> = None;
    for key in keys {
        if is_os_junk(key) {
            continue;
        }
        let dir_segments: Vec<&str> = match key.rsplit_once('/') {
            Some((dir, _)) => dir.split('/').collect(),
            None => Vec::new(),
        };
        match &mut shared {
            None => shared = Some(dir_segments),
            Some(cur) => {
                let common_len = cur
                    .iter()
                    .zip(dir_segments.iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                cur.truncate(common_len);
            }
        }
    }
    let segs = shared?;
    if segs.is_empty() {
        None
    } else {
        Some(format!("{}/", segs.join("/")))
    }
}

/// Returns `true` if `path` is an OS metadata/junk entry (`desktop.ini`, `.DS_Store`,
/// `Thumbs.db`, `__MACOSX/`, or macOS AppleDouble `._*` resource fork).
#[must_use]
pub fn is_os_junk(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    let lower = norm.to_ascii_lowercase();
    if lower.starts_with("__macosx/") || lower.contains("/__macosx/") || lower == "__macosx" {
        return true;
    }
    let file_name = lower.rsplit('/').next().unwrap_or(lower.as_str());
    matches!(file_name, "desktop.ini" | ".ds_store" | "thumbs.db")
        || file_name.starts_with("._")
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

    #[test]
    fn filters_os_junk_files_at_scan_time() {
        let zip_bytes = build_raw_zip(&[
            ("pack.mcmeta", b"{}"),
            ("desktop.ini", b"[.ShellClassInfo]"),
            ("assets/minecraft/textures/Desktop.INI", b"junk"),
            (".DS_Store", b"mac"),
            ("Thumbs.db", b"win"),
            ("__MACOSX/._pack.mcmeta", b"fork"),
            ("assets/minecraft/textures/block/stone.png", b"png"),
        ]);
        let parsed =
            PackContainer::read_from_zip_bytes(&zip_bytes, IoLimits::default()).expect("read zip");
        assert!(parsed.contains("pack.mcmeta"));
        assert!(parsed.contains("assets/minecraft/textures/block/stone.png"));
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn preserves_flat_bedrock_addon_workspace_intact() {
        let zip_bytes = build_raw_zip(&[
            ("BP/manifest.json", b"{\"header\":{\"name\":\"Test BP\"}}"),
            ("BP/scripts/main.js", b"console.log('hi');"),
            ("RP/manifest.json", b"{\"header\":{\"name\":\"Test RP\"}}"),
            ("RP/textures/blocks/stone.png", b"png"),
            ("pstdr.toml", b"[package]\nname = \"test\"\n"),
            ("src/main.pstdr", b"addon \"test\" {}\n"),
        ]);
        let parsed =
            PackContainer::read_from_zip_bytes(&zip_bytes, IoLimits::default()).expect("read zip");
        assert!(parsed.contains("BP/manifest.json"));
        assert!(parsed.contains("BP/scripts/main.js"));
        assert!(parsed.contains("RP/manifest.json"));
        assert!(parsed.contains("RP/textures/blocks/stone.png"));
        assert!(parsed.contains("pstdr.toml"));
        assert!(parsed.contains("src/main.pstdr"));
        assert!(!parsed.contains("manifest.json"));
        assert_eq!(parsed.len(), 6);

        // Also verify reading from an on-disk directory preserves all files intact.
        let temp = TempDir::new().expect("tempdir");
        parsed
            .write_to_directory(temp.path())
            .expect("write addon dir");
        let from_dir =
            PackContainer::read_from_directory(temp.path(), IoLimits::default()).expect("read dir");
        assert_eq!(from_dir, parsed);
    }

    #[test]
    fn unwraps_only_outer_wrapper_for_wrapped_bedrock_addon() {
        let zip_bytes = build_raw_zip(&[
            ("my-addon/BP/manifest.json", b"{\"header\":{\"name\":\"BP\"}}"),
            ("my-addon/RP/manifest.json", b"{\"header\":{\"name\":\"RP\"}}"),
            ("my-addon/RP/textures/items/sword.png", b"png"),
            ("my-addon/pstdr.toml", b"[package]\nname = \"my-addon\"\n"),
            ("my-addon/src/main.pstdr", b"addon \"my-addon\" {}\n"),
            ("__MACOSX/my-addon/._pstdr.toml", b"junk"),
        ]);
        let parsed =
            PackContainer::read_from_zip_bytes(&zip_bytes, IoLimits::default()).expect("read zip");
        assert!(parsed.contains("BP/manifest.json"));
        assert!(parsed.contains("RP/manifest.json"));
        assert!(parsed.contains("RP/textures/items/sword.png"));
        assert!(parsed.contains("pstdr.toml"));
        assert!(parsed.contains("src/main.pstdr"));
        assert!(!parsed.contains("my-addon/BP/manifest.json"));
        assert!(!parsed.contains("manifest.json"));
        assert_eq!(parsed.len(), 5);

        // Also verify pstdr.toml + BP/ only (without RP/) does not promote BP/ to root.
        let bp_only_pstdr = build_raw_zip(&[
            ("wrapped/BP/manifest.json", b"{}"),
            ("wrapped/pstdr.toml", b"[package]\n"),
            ("wrapped/src/main.pstdr", b"addon \"bp\" {}\n"),
        ]);
        let parsed_bp = PackContainer::read_from_zip_bytes(&bp_only_pstdr, IoLimits::default())
            .expect("read bp-only pstdr");
        assert!(parsed_bp.contains("BP/manifest.json"));
        assert!(parsed_bp.contains("pstdr.toml"));
        assert!(parsed_bp.contains("src/main.pstdr"));
        assert!(!parsed_bp.contains("manifest.json"));
    }
}
