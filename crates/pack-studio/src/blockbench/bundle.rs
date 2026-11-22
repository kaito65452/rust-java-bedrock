use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

#[cfg(feature = "embed-blockbench")]
const EMBEDDED_BLOCKBENCH_ARCHIVE: &[u8] =
    include_bytes!("../../../../assets/vendor/blockbench-web.tar.zst");

/// Result returned by [`ensure_extracted_from_bytes`] / [`ensure_extracted_from_path`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractOutcome {
    /// Directory containing extracted `index.html`, `dist/bundle.js`, `studio_bridge.js`, `LICENSE.MD`, etc.
    pub cache_dir: PathBuf,
    /// SHA-256 hex digest of the source `.tar.zst` archive.
    pub sha256: String,
    /// `true` if extraction was performed, `false` if existing `.version` cache matched (0 ms startup).
    pub extracted: bool,
}

/// Returns the default user-local cache directory (`<data_local_dir>/pack-studio/blockbench-web`).
#[must_use]
pub fn default_cache_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("pack-studio")
        .join("blockbench-web")
}

/// Locates `blockbench-web.tar.zst` from disk (next to binary, parent install dir, or workspace root).
#[must_use]
pub fn locate_archive_on_disk() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("blockbench-web.tar.zst"));
        candidates.push(dir.join("assets").join("vendor").join("blockbench-web.tar.zst"));
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join("assets").join("vendor").join("blockbench-web.tar.zst"));
        }
    }
    candidates.push(PathBuf::from("assets/vendor/blockbench-web.tar.zst"));
    candidates.push(PathBuf::from("../assets/vendor/blockbench-web.tar.zst"));
    candidates.push(PathBuf::from("../../assets/vendor/blockbench-web.tar.zst"));

    candidates.into_iter().find(|p| p.is_file())
}

/// Ensures the Blockbench web bundle is extracted into `cache_dir` from either the optional
/// embedded archive (`embed-blockbench` feature) or `assets/vendor/blockbench-web.tar.zst` on disk.
pub fn ensure_default_bundle_extracted(cache_dir: &Path) -> Result<ExtractOutcome> {
    #[cfg(feature = "embed-blockbench")]
    {
        ensure_extracted_from_bytes(EMBEDDED_BLOCKBENCH_ARCHIVE, cache_dir)
    }
    #[cfg(not(feature = "embed-blockbench"))]
    {
        let Some(archive_path) = locate_archive_on_disk() else {
            bail!(
                "Blockbench web bundle not found (expected assets/vendor/blockbench-web.tar.zst). Run scripts/build_blockbench.sh first."
            );
        };
        ensure_extracted_from_path(&archive_path, cache_dir)
    }
}

/// Computes SHA-256 hex digest for arbitrary bytes.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{b:02x}");
    }
    hex
}

/// Lazily extracts a `.tar.zst` archive from disk into `cache_dir`, skipping extraction completely
/// when `<cache_dir>/.version` matches the archive SHA-256 and `index.html` is present.
pub fn ensure_extracted_from_path(archive_path: &Path, cache_dir: &Path) -> Result<ExtractOutcome> {
    let bytes = fs::read(archive_path).with_context(|| {
        format!(
            "failed to read Blockbench archive '{}'",
            archive_path.display()
        )
    })?;
    ensure_extracted_from_bytes(&bytes, cache_dir)
}

/// Lazily extracts a `.tar.zst` byte slice into `cache_dir` with `.version` cache verification
/// and Tar-Slip traversal protection.
pub fn ensure_extracted_from_bytes(
    archive_bytes: &[u8],
    cache_dir: &Path,
) -> Result<ExtractOutcome> {
    let digest = sha256_hex(archive_bytes);
    let version_file = cache_dir.join(".version");
    let index_html = cache_dir.join("index.html");

    if version_file.is_file()
        && index_html.is_file()
        && let Ok(existing) = fs::read_to_string(&version_file)
        && existing.trim() == digest
    {
        return Ok(ExtractOutcome {
            cache_dir: cache_dir.to_path_buf(),
            sha256: digest,
            extracted: false,
        });
    }

    // Re-extract cleanly into cache_dir
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir).with_context(|| {
            format!(
                "failed to clean stale Blockbench cache '{}'",
                cache_dir.display()
            )
        })?;
    }
    fs::create_dir_all(cache_dir).with_context(|| {
        format!(
            "failed to create Blockbench cache directory '{}'",
            cache_dir.display()
        )
    })?;

    extract_tar_zst_stream(Cursor::new(archive_bytes), cache_dir)?;
    fs::write(&version_file, format!("{digest}\n")).with_context(|| {
        format!(
            "failed to write Blockbench cache version marker '{}'",
            version_file.display()
        )
    })?;

    Ok(ExtractOutcome {
        cache_dir: cache_dir.to_path_buf(),
        sha256: digest,
        extracted: true,
    })
}

fn extract_tar_zst_stream<R: Read>(reader: R, dest_root: &Path) -> Result<()> {
    let decoder = zstd::stream::read::Decoder::new(reader)
        .context("failed to initialize zstd streaming decoder")?;
    let mut archive = tar::Archive::new(decoder);

    for entry_res in archive
        .entries()
        .context("failed to iterate tar archive entries")?
    {
        let mut entry = entry_res.context("corrupt tar entry")?;
        let rel_path = entry
            .path()
            .context("invalid UTF-8 path in tar entry")?
            .into_owned();

        let sanitized = sanitize_tar_rel_path(&rel_path)?;
        if sanitized.as_os_str().is_empty() {
            continue;
        }

        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            bail!(
                "refusing to extract link entry '{}' from Blockbench archive",
                rel_path.display()
            );
        }

        let target = dest_root.join(&sanitized);
        if entry_type.is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry_type.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out = fs::File::create(&target).with_context(|| {
                format!("failed creating extracted file '{}'", target.display())
            })?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }
    Ok(())
}

fn sanitize_tar_rel_path(raw: &Path) -> Result<PathBuf> {
    let mut clean = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(seg) => clean.push(seg),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("unsafe path traversal in tar archive: '{}'", raw.display());
            }
        }
    }
    Ok(clean)
}
