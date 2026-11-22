use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use image::{Rgba, RgbaImage};
use rp_bedrock::schema::BedrockManifest;
use rp_core::{Edition, ResolutionTier, WarningSeverity, parse_minecraft_json};
use rp_io::PackContainer;
use rp_java::schema::JavaPackMcmeta;

use crate::artist::{TextureFormatKind, compute_dominant_resolution, encode_texture_bytes};

/// Asset categories displayed in the VS Code-style Explorer sidebar tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssetGroup {
    ManifestMeta,
    Textures,
    Models,
    Blockstates,
    UiEffects,
    Sounds,
    Other,
}

impl AssetGroup {
    /// Display order matching the PackStudio workbench layout.
    pub const ALL: [Self; 7] = [
        Self::ManifestMeta,
        Self::Textures,
        Self::Models,
        Self::Blockstates,
        Self::UiEffects,
        Self::Sounds,
        Self::Other,
    ];

    /// Display label for the Explorer tree section.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ManifestMeta => "Manifest / Meta",
            Self::Textures => "Textures",
            Self::Models => "Models",
            Self::Blockstates => "Blockstates",
            Self::UiEffects => "UI / Effects",
            Self::Sounds => "Sounds",
            Self::Other => "Locales & Other",
        }
    }

    /// Icon prefix for Explorer tree headers.
    #[must_use]
    pub const fn icon(self) -> &'static str {
        match self {
            Self::ManifestMeta => "📋",
            Self::Textures => "🎨",
            Self::Models => "🧊",
            Self::Blockstates => "🧱",
            Self::UiEffects => "✨",
            Self::Sounds => "🔊",
            Self::Other => "📄",
        }
    }
}

/// Classifies an archive relative path into one of the Explorer asset groups.
#[must_use]
pub fn classify_asset_group(path: &str) -> AssetGroup {
    let lower = path.to_ascii_lowercase();

    if lower == "pack.mcmeta"
        || lower == "manifest.json"
        || lower == "pack.png"
        || lower == "pack_icon.png"
        || lower == "bug_pack_icon.png"
        || lower.ends_with(".mcmeta")
        || lower == "texts/languages.json"
        || lower == "splashes.json"
        || lower.ends_with("/texts/splashes.txt")
    {
        return AssetGroup::ManifestMeta;
    }

    if lower.contains("/blockstates/")
        || lower.starts_with("blockstates/")
        || lower == "blocks.json"
        || lower.contains("/items/") && lower.ends_with(".json") && !lower.contains("textures/")
    {
        return AssetGroup::Blockstates;
    }

    if lower.contains("textures/gui/")
        || lower.contains("textures/ui/")
        || lower.contains("textures/mob_effect/")
        || lower.contains("textures/particle/")
        || lower.contains("textures/particles/")
        || lower.starts_with("ui/")
        || lower.starts_with("particles/")
    {
        return AssetGroup::UiEffects;
    }

    if lower.contains("/models/")
        || lower.starts_with("models/")
        || lower.ends_with(".geo.json")
        || lower.starts_with("attachables/")
        || lower.starts_with("entity/")
    {
        return AssetGroup::Models;
    }

    if lower.contains("/sounds/")
        || lower.starts_with("sounds/")
        || lower == "sounds.json"
        || lower.ends_with("/sounds.json")
        || lower == "sounds/sound_definitions.json"
        || lower.ends_with(".ogg")
        || lower.ends_with(".fsb")
    {
        return AssetGroup::Sounds;
    }

    if lower.ends_with(".png")
        || lower.ends_with(".tga")
        || lower.ends_with(".texture_set.json")
        || lower == "textures/terrain_texture.json"
        || lower == "textures/item_texture.json"
        || lower == "textures/flipbook_textures.json"
    {
        return AssetGroup::Textures;
    }

    AssetGroup::Other
}

/// Origin of the currently open resource pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackSource {
    /// Opened directly from an unpacked filesystem directory (supports direct save + hot-reload).
    Directory(PathBuf),
    /// Opened from a `.zip` or `.mcpack` archive.
    Archive(PathBuf),
    /// Newly created in-memory Bedrock pack (not yet exported to disk).
    InMemory(String),
}

impl PackSource {
    /// Human-readable display title for the sidebar header / window title.
    #[must_use]
    pub fn display_name(&self) -> String {
        match self {
            Self::Directory(p) | Self::Archive(p) => p
                .file_name()
                .and_then(|s| s.to_str())
                .map_or_else(|| p.display().to_string(), ToString::to_string),
            Self::InMemory(name) => name.clone(),
        }
    }

    /// Returns the root folder path if opened as a directory.
    #[must_use]
    pub fn directory_path(&self) -> Option<&Path> {
        match self {
            Self::Directory(p) => Some(p.as_path()),
            Self::Archive(_) | Self::InMemory(_) => None,
        }
    }

    /// Returns the filesystem path if backed by disk (folder or archive).
    #[must_use]
    pub fn disk_path(&self) -> Option<&Path> {
        match self {
            Self::Directory(p) | Self::Archive(p) => Some(p.as_path()),
            Self::InMemory(_) => None,
        }
    }
}

/// In-memory loaded resource pack state for PackStudio.
#[derive(Debug, Clone)]
pub struct LoadedPack {
    pub source: PackSource,
    pub container: PackContainer,
    pub edition: Edition,
    pub pack_name: String,
    pub edition_label: String,
    pub texture_count: usize,
    pub dominant_resolution: Option<ResolutionTier>,
    pub grouped_assets: BTreeMap<AssetGroup, Vec<String>>,
    pub problem_severity_by_path: BTreeMap<String, WarningSeverity>,
}

impl LoadedPack {
    /// Builds a `LoadedPack` from a `PackSource` and parsed `PackContainer`.
    pub fn from_container(source: PackSource, container: PackContainer) -> Result<Self> {
        let edition = detect_pack_edition(&container)?;
        let (pack_name, edition_label) = extract_pack_identity(&source, &container, edition);
        let (texture_count, dominant_resolution) = compute_dominant_resolution(&container);
        let grouped_assets = build_grouped_assets(&container);

        Ok(Self {
            source,
            container,
            edition,
            pack_name,
            edition_label,
            texture_count,
            dominant_resolution,
            grouped_assets,
            problem_severity_by_path: BTreeMap::new(),
        })
    }

    /// Recomputes asset groups, texture counts, dominant resolution, and edition badge after edits.
    pub fn refresh_index(&mut self) {
        if let Ok(ed) = detect_pack_edition(&self.container) {
            self.edition = ed;
        }
        let (pack_name, edition_label) =
            extract_pack_identity(&self.source, &self.container, self.edition);
        self.pack_name = pack_name;
        self.edition_label = edition_label;
        let (texture_count, dominant_resolution) = compute_dominant_resolution(&self.container);
        self.texture_count = texture_count;
        self.dominant_resolution = dominant_resolution;
        self.grouped_assets = build_grouped_assets(&self.container);
    }

    /// Inserts or updates a file in the in-memory container and, if opened from a directory,
    /// writes it to disk immediately.
    pub fn write_asset(&mut self, rel_path: &str, bytes: Vec<u8>) -> Result<()> {
        if let Some(root) = self.source.directory_path() {
            let clean = rp_io::sanitize_archive_path(rel_path)
                .with_context(|| format!("invalid relative asset path '{rel_path}'"))?;
            let full = root.join(&clean);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&full, &bytes)
                .with_context(|| format!("failed to write '{}'", full.display()))?;
        }
        self.container
            .insert(rel_path, bytes)
            .with_context(|| format!("failed to insert '{rel_path}' into pack container"))?;
        self.refresh_index();
        Ok(())
    }

    /// Resolves the on-disk path of an asset if the pack is backed by a directory.
    #[must_use]
    pub fn resolve_disk_file(&self, rel_path: &str) -> Option<PathBuf> {
        self.source.directory_path().map(|root| root.join(rel_path))
    }

    /// Exports the current `PackContainer` to a `.zip`, `.mcpack`, or directory path on disk.
    pub fn export_to_path(&self, target: &Path) -> Result<()> {
        self.container
            .write_to_path(target)
            .with_context(|| format!("failed to export pack to '{}'", target.display()))
    }
}

/// Detects Java vs Bedrock edition from root metadata files.
pub fn detect_pack_edition(container: &PackContainer) -> Result<Edition> {
    if container.contains("pack.mcmeta") {
        Ok(Edition::Java)
    } else if container.contains("manifest.json") {
        Ok(Edition::Bedrock)
    } else {
        bail!("Unable to detect pack edition: missing both 'pack.mcmeta' and 'manifest.json'")
    }
}

fn build_grouped_assets(container: &PackContainer) -> BTreeMap<AssetGroup, Vec<String>> {
    let mut grouped: BTreeMap<AssetGroup, Vec<String>> = BTreeMap::new();
    for path in container.files().keys() {
        let group = classify_asset_group(path);
        grouped.entry(group).or_default().push(path.clone());
    }
    grouped
}

fn extract_pack_identity(
    source: &PackSource,
    container: &PackContainer,
    edition: Edition,
) -> (String, String) {
    match edition {
        Edition::Java => {
            let label = container
                .get("pack.mcmeta")
                .and_then(|b| parse_minecraft_json::<JavaPackMcmeta>(b).ok())
                .and_then(|m| m.validate_and_extract().ok())
                .map_or_else(
                    || String::from("Java Edition"),
                    |(fmt, _, _)| format_java_edition_label(fmt),
                );
            (source.display_name(), label)
        }
        Edition::Bedrock => {
            if let Some(bytes) = container.get("manifest.json")
                && let Ok(manifest) = parse_minecraft_json::<BedrockManifest>(bytes)
            {
                let v = manifest.header.min_engine_version;
                let name = if manifest.header.name.trim().is_empty() {
                    source.display_name()
                } else {
                    manifest.header.name
                };
                return (name, format!("Bedrock {}.{}.{}", v[0], v[1], v[2]));
            }
            (source.display_name(), String::from("Bedrock 1.x.x"))
        }
    }
}

fn format_java_edition_label(pack_format: u32) -> String {
    let ver = match pack_format {
        0..=8 => "1.18-",
        9..=12 => "1.19",
        13..=15 => "1.20.1",
        16..=32 => "1.20.x",
        33..=45 => "1.21",
        46..=63 => "1.21.4+",
        64..=99 => "1.21.9+ / 26.x",
        _ => "26.x+",
    };
    format!("Java {ver} (fmt {pack_format})")
}

/// Converts a pack archive path into a canonical Minecraft resource path string
/// for the "Copy Resource Path" context menu action.
#[must_use]
pub fn canonical_resource_path(archive_path: &str, edition: Edition) -> String {
    let norm = archive_path.replace('\\', "/");
    match edition {
        Edition::Java => {
            if let Some(rest) = norm.strip_prefix("assets/")
                && let Some((ns, tail)) = rest.split_once('/')
            {
                let stripped = tail
                    .strip_prefix("textures/")
                    .or_else(|| tail.strip_prefix("models/"))
                    .or_else(|| tail.strip_prefix("sounds/"))
                    .unwrap_or(tail);
                let no_ext = stripped
                    .strip_suffix(".png")
                    .or_else(|| stripped.strip_suffix(".json"))
                    .or_else(|| stripped.strip_suffix(".ogg"))
                    .unwrap_or(stripped);
                return format!("{ns}:{no_ext}");
            }
            norm
        }
        Edition::Bedrock => norm
            .strip_suffix(".png")
            .or_else(|| norm.strip_suffix(".tga"))
            .or_else(|| norm.strip_suffix(".json"))
            .unwrap_or(&norm)
            .to_string(),
    }
}

/// Returns `true` if the path is an image asset handled by the Texture Inspector.
#[must_use]
pub fn is_texture_asset(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".png") || lower.ends_with(".tga")
}

/// Returns `true` if the path is a text/JSON/lang asset handled by the Code Editor.
#[must_use]
pub fn is_text_asset(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".json")
        || lower.ends_with(".mcmeta")
        || lower.ends_with(".lang")
        || lower.ends_with(".txt")
        || lower.ends_with(".properties")
        || lower.ends_with(".md")
}

/// Creates a clean, audit-passing starter Bedrock Resource Pack in memory.
pub fn create_new_bedrock_pack(name: &str) -> Result<LoadedPack> {
    let pack_name = if name.trim().is_empty() {
        String::from("New Bedrock Pack")
    } else {
        name.trim().to_string()
    };

    let manifest = BedrockManifest::new(
        pack_name.clone(),
        "Created with PackStudio",
        [1, 0, 0],
        [1, 21, 0],
    );
    let manifest_bytes = manifest.to_json_vec_versioned(2)?;

    // Generate a clean 16x16 pixel-art stone texture and pack icon.
    let mut stone = RgbaImage::new(16, 16);
    for y in 0..16u32 {
        for x in 0..16u32 {
            let shade = 115u8.saturating_add(((x * 13 + y * 7) % 32) as u8);
            stone.put_pixel(x, y, Rgba([shade, shade, shade, 255]));
        }
    }
    let stone_png = encode_texture_bytes(&stone, TextureFormatKind::Png)?;

    let mut icon = RgbaImage::new(32, 32);
    for y in 0..32u32 {
        for x in 0..32u32 {
            let is_top = y < 10;
            let px = if is_top {
                Rgba([0x59, 0xB8, 0x4C, 255])
            } else {
                Rgba([0x7A, 0x55, 0x38, 255])
            };
            icon.put_pixel(x, y, px);
        }
    }
    let icon_png = encode_texture_bytes(&icon, TextureFormatKind::Png)?;

    let terrain_json = serde_json::json!({
        "resource_pack_name": pack_name,
        "texture_name": "atlas.terrain",
        "padding": 8,
        "num_mip_levels": 4,
        "texture_data": {
            "stone": {
                "textures": "textures/blocks/stone"
            }
        }
    });

    let mut container = PackContainer::new();
    container.insert("manifest.json", manifest_bytes)?;
    container.insert("pack_icon.png", icon_png)?;
    container.insert("textures/blocks/stone.png", stone_png)?;
    container.insert(
        "textures/terrain_texture.json",
        serde_json::to_vec_pretty(&terrain_json)?,
    )?;

    LoadedPack::from_container(PackSource::InMemory(pack_name), container)
}

/// Launches an external executable (e.g. Blockbench or an image editor) with `target_path`.
/// If `configured_exe` is empty, falls back to OS default handler (`xdg-open`/`open`/`cmd`).
pub fn open_in_external_app(target_path: &Path, configured_exe: &str) -> Result<()> {
    let trimmed = configured_exe.trim();
    if !trimmed.is_empty() {
        Command::new(trimmed)
            .arg(target_path)
            .spawn()
            .with_context(|| {
                format!(
                    "failed to launch '{}' for '{}'",
                    trimmed,
                    target_path.display()
                )
            })?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(target_path)
            .spawn()
            .with_context(|| format!("failed to run xdg-open on '{}'", target_path.display()))?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(target_path)
            .spawn()
            .with_context(|| format!("failed to run open on '{}'", target_path.display()))?;
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(target_path)
            .spawn()
            .with_context(|| format!("failed to run explorer on '{}'", target_path.display()))?;
    }
    Ok(())
}

/// Opens the parent directory of `target_path` in the OS file manager.
pub fn reveal_in_file_manager(target_path: &Path) -> Result<()> {
    let dir = if target_path.is_dir() {
        target_path
    } else {
        target_path.parent().unwrap_or(target_path)
    };
    open_in_external_app(dir, "")
}

/// Materializes an archive asset into a temporary file so external tools (like Blockbench or
/// GIMP/Aseprite) can inspect it even when the pack was opened from a `.zip`/`.mcpack`.
pub fn materialize_temp_asset(rel_path: &str, bytes: &[u8]) -> Result<PathBuf> {
    let base = std::env::temp_dir().join("pack-studio-preview");
    let clean = rp_io::sanitize_archive_path(rel_path)?;
    let out = base.join(clean);
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&out, bytes)?;
    Ok(out)
}
