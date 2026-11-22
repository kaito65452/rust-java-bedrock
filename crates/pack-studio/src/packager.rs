use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rp_core::{AuditStatus, Edition};
use rp_io::{IoLimits, PackContainer, is_os_junk, sanitize_archive_path};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::artist::{TextureFormatKind, encode_texture_bytes, resize_nearest_neighbor};
use crate::explorer::sanitize_minecraft_slug;
use crate::pipeline::run_semantic_audit_sync;

/// Output archive format selectable in the Flexible Packager / Exporter dialog.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PackageFormat {
    #[default]
    Zip,
    Mcpack,
    Mcaddon,
}

impl PackageFormat {
    /// File extension without leading dot.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Mcpack => "mcpack",
            Self::Mcaddon => "mcaddon",
        }
    }

    /// UI display label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Zip => ".zip (Java / Universal)",
            Self::Mcpack => ".mcpack (Bedrock RP)",
            Self::Mcaddon => ".mcaddon (Bedrock RP + BP)",
        }
    }

    /// Sensible default format for a given pack edition and optional companion BP presence.
    #[must_use]
    pub const fn default_for(edition: Edition, has_companion_bp: bool) -> Self {
        match edition {
            Edition::Java => Self::Zip,
            Edition::Bedrock if has_companion_bp => Self::Mcaddon,
            Edition::Bedrock => Self::Mcpack,
        }
    }
}

/// Deflate compression level preset for packaging.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CompressionPreset {
    Fast,
    #[default]
    Balanced,
    Maximum,
}

impl CompressionPreset {
    /// UI label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast (Level 1)",
            Self::Balanced => "Balanced (Level 6)",
            Self::Maximum => "Maximum (Level 9)",
        }
    }

    /// Deflate compression level passed to `zip::write::SimpleFileOptions`.
    #[must_use]
    pub const fn deflate_level(self) -> i64 {
        match self {
            Self::Fast => 1,
            Self::Balanced => 6,
            Self::Maximum => 9,
        }
    }
}

/// Configuration for the Flexible Packager / Exporter dialog.
#[derive(Debug, Clone)]
pub struct ExportPackageConfig {
    pub open: bool,
    pub format: PackageFormat,
    pub filename_template: String,
    pub output_dir: String,
    pub strip_junk_files: bool,
    pub compression: CompressionPreset,
    pub run_semantic_audit: bool,
    pub allow_export_with_errors: bool,
    pub last_summary: Option<ExportPackageSummary>,
    pub last_error: Option<String>,
}

impl Default for ExportPackageConfig {
    fn default() -> Self {
        let dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            open: false,
            format: PackageFormat::Zip,
            filename_template: String::from("{pack_name}-v{version}-{edition}"),
            output_dir: dir.display().to_string(),
            strip_junk_files: true,
            compression: CompressionPreset::Balanced,
            run_semantic_audit: true,
            allow_export_with_errors: false,
            last_summary: None,
            last_error: None,
        }
    }
}

/// Export completion summary displayed in the Packager dialog.
#[derive(Debug, Clone)]
pub struct ExportPackageSummary {
    pub output_path: PathBuf,
    pub size_bytes: u64,
    pub asset_count: usize,
    pub stripped_junk_count: usize,
    pub audit_status: Option<AuditStatus>,
    pub audit_summary: String,
}

/// Evaluates `{pack_name}`, `{version}`, and `{edition}` placeholders into a clean archive filename.
#[must_use]
pub fn render_output_filename(
    template: &str,
    pack_name: &str,
    version: [u32; 3],
    edition: Edition,
    format: PackageFormat,
) -> String {
    let safe_name = sanitize_minecraft_slug(pack_name);
    let safe_name = if safe_name.is_empty() {
        String::from("resource_pack")
    } else {
        safe_name
    };
    let ver_str = format!("{}.{}.{}", version[0], version[1], version[2]);
    let ed_str = match edition {
        Edition::Java => "java",
        Edition::Bedrock => "bedrock",
    };

    let stem = template
        .trim()
        .replace("{pack_name}", &safe_name)
        .replace("{version}", &ver_str)
        .replace("{edition}", ed_str);
    let stem = stem
        .trim_end_matches(".zip")
        .trim_end_matches(".mcpack")
        .trim_end_matches(".mcaddon");
    let stem = if stem.is_empty() { "pack_export" } else { stem };
    format!("{stem}.{}", format.extension())
}

/// Returns `true` if `path` is OS junk, VCS metadata (`.git/`), or temporary backup clutter
/// that should be stripped during packaging.
#[must_use]
pub fn is_packaging_junk(path: &str) -> bool {
    if is_os_junk(path) {
        return true;
    }
    let norm = path.replace('\\', "/");
    let lower = norm.to_ascii_lowercase();
    if lower == ".git"
        || lower.starts_with(".git/")
        || lower.contains("/.git/")
        || lower.starts_with(".vscode/")
        || lower.contains("/.vscode/")
        || lower.starts_with(".cache/")
    {
        return true;
    }
    let file_name = lower.rsplit('/').next().unwrap_or(&lower);
    file_name.ends_with(".bak")
        || file_name.ends_with(".tmp")
        || file_name.ends_with(".swp")
        || file_name.ends_with('~')
}

/// Packages `container` (and optional companion Behavior Pack directory for `.mcaddon`) into
/// `.zip`, `.mcpack`, or `.mcaddon` with clean root structure (never double-nested) and optional
/// pre-export Semantic Audit gate.
pub fn export_pack_archive(
    container: &PackContainer,
    companion_bp_dir: Option<&Path>,
    cfg: &ExportPackageConfig,
    edition: Edition,
    pack_name: &str,
    version: [u32; 3],
    uv_atlas_guard: bool,
) -> Result<ExportPackageSummary> {
    let mut cleaned_container = PackContainer::new();
    let mut stripped_junk_count = 0usize;
    for (path, bytes) in container.files() {
        if cfg.strip_junk_files && is_packaging_junk(path) {
            stripped_junk_count += 1;
            continue;
        }
        cleaned_container.insert(path, bytes.clone())?;
    }

    let mut audit_status = None;
    let mut audit_summary = String::from("Skipped");

    if cfg.run_semantic_audit {
        let audit = run_semantic_audit_sync(&cleaned_container, uv_atlas_guard)?;
        audit_status = Some(audit.status);
        audit_summary = audit.summary.clone();
        if audit.status == AuditStatus::Failed && !cfg.allow_export_with_errors {
            bail!(
                "Packaging blocked by Semantic Audit Gate (FAILED): {}",
                audit.summary
            );
        }
    }

    let out_dir = PathBuf::from(cfg.output_dir.trim());
    if cfg.output_dir.trim().is_empty() {
        bail!("Output directory must not be empty");
    }
    fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create export directory '{}'", out_dir.display()))?;

    let filename = render_output_filename(
        &cfg.filename_template,
        pack_name,
        version,
        edition,
        cfg.format,
    );
    let output_path = out_dir.join(filename);

    let file = File::create(&output_path)
        .with_context(|| format!("failed to create output archive '{}'", output_path.display()))?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(cfg.compression.deflate_level()));

    let mut asset_count = 0usize;

    if cfg.format == PackageFormat::Mcaddon {
        // .mcaddon bundles RP/ and optional BP/ cleanly at archive root
        for (path, bytes) in cleaned_container.files() {
            let clean = sanitize_archive_path(path)?;
            zip.start_file(format!("RP/{clean}"), options)?;
            zip.write_all(bytes)?;
            asset_count += 1;
        }
        if let Some(bp_dir) = companion_bp_dir
            && bp_dir.is_dir()
        {
            let bp_container = PackContainer::read_from_directory(bp_dir, IoLimits::default())?;
            for (path, bytes) in bp_container.files() {
                if cfg.strip_junk_files && is_packaging_junk(path) {
                    stripped_junk_count += 1;
                    continue;
                }
                let clean = sanitize_archive_path(path)?;
                zip.start_file(format!("BP/{clean}"), options)?;
                zip.write_all(bytes)?;
                asset_count += 1;
            }
        }
    } else {
        // Clean flat root for .zip / .mcpack (pack.mcmeta or manifest.json directly at archive root!)
        for (path, bytes) in cleaned_container.files() {
            let clean = sanitize_archive_path(path)?;
            zip.start_file(clean, options)?;
            zip.write_all(bytes)?;
            asset_count += 1;
        }
    }

    zip.finish()?;
    let size_bytes = fs::metadata(&output_path).map_or(0, |m| m.len());

    Ok(ExportPackageSummary {
        output_path,
        size_bytes,
        asset_count,
        stripped_junk_count,
        audit_status,
        audit_summary,
    })
}

/// Imports an image file (`.png`, `.tga`, `.jpg`, `.webp`) as a Pack Icon, optionally resizing
/// via Nearest-Neighbor to `target_size` (e.g., `128` or `256`), and encodes it as PNG bytes.
pub fn import_pack_icon_file(src_path: &Path, resize_target: Option<u32>) -> Result<Vec<u8>> {
    let dyn_img = image::open(src_path)
        .with_context(|| format!("failed to decode pack icon '{}'", src_path.display()))?;
    let rgba = dyn_img.into_rgba8();
    let final_img = match resize_target {
        Some(sz) if sz > 0 && (rgba.width() != sz || rgba.height() != sz) => {
            resize_nearest_neighbor(&rgba, sz, sz)?
        }
        _ => rgba,
    };
    encode_texture_bytes(&final_img, TextureFormatKind::Png)
}
