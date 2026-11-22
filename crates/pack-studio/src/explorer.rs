use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use image::{Rgba, RgbaImage};
use rp_core::Edition;
use rp_io::PackContainer;

use crate::artist::{TextureFormatKind, encode_texture_bytes};
use crate::command_palette::fuzzy_score;
use crate::workspace::LoadedPack;

/// Sanitizes an arbitrary human-entered name into a valid Minecraft lowercase slug (`[a-z0-9_-]`).
#[must_use]
pub fn sanitize_minecraft_slug(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_underscore = false;
    for ch in raw.trim().chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_lowercase() || lower.is_ascii_digit() || lower == '-' {
            out.push(lower);
            prev_underscore = false;
        } else if (lower.is_whitespace() || lower == '_' || lower == '.' || lower == '/')
            && !prev_underscore
            && !out.is_empty()
        {
            out.push('_');
            prev_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

/// Sanitizes an imported asset filename for Minecraft resource locations:
/// e.g. `"My Cool Block!.PNG"` -> `"my_cool_block.png"`, `"My Sword 01.png"` -> `"my_sword_01.png"`.
#[must_use]
pub fn sanitize_minecraft_filename(raw_name: &str) -> String {
    let trimmed = raw_name.trim();
    // Preserve compound `.geo.json`, `.png.mcmeta`, `.texture_set.json` suffixes cleanly.
    let lower = trimmed.to_ascii_lowercase();
    let compound_suffix = [
        ".texture_set.json",
        ".png.mcmeta",
        ".geo.json",
    ]
    .into_iter()
    .find(|s| lower.ends_with(s));

    let (raw_stem, ext) = if let Some(suf) = compound_suffix {
        let stem_len = trimmed.len().saturating_sub(suf.len());
        (&trimmed[..stem_len], suf)
    } else if let Some((s, e)) = trimmed.rsplit_once('.') {
        let clean_ext = e
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase();
        if clean_ext.is_empty() {
            (trimmed, "")
        } else {
            return format!(
                "{}.{}",
                sanitize_stem_chars(s),
                clean_ext
            );
        }
    } else {
        (trimmed, "")
    };

    let clean_stem = sanitize_stem_chars(raw_stem);
    if ext.is_empty() {
        clean_stem
    } else {
        format!("{clean_stem}{ext}")
    }
}

fn sanitize_stem_chars(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_underscore = false;
    for ch in raw.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            prev_underscore = false;
        } else if (c == '_' || c == '-' || c.is_whitespace()) && !prev_underscore && !out.is_empty()
        {
            out.push('_');
            prev_underscore = true;
        }
    }
    let clean = out.trim_matches('_').to_string();
    if clean.is_empty() {
        String::from("asset")
    } else {
        clean
    }
}

/// Checks whether a filename already conforms to strict Minecraft resource location naming
/// (lowercase ASCII letters, digits, `_`, `.`, `-`, no spaces or uppercase).
#[must_use]
pub fn is_valid_minecraft_filename(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-')
        })
}

/// Converts a relative pack path into a canonical Minecraft Resource ID:
/// - Java: `assets/minecraft/textures/block/stone.png` -> `minecraft:block/stone`
/// - Java custom namespace: `assets/my_mod/models/item/blade.json` -> `my_mod:item/blade`
/// - Bedrock: `textures/blocks/stone.png` -> `textures/blocks/stone`
#[must_use]
pub fn generate_minecraft_resource_id(rel_path: &str, edition: Edition) -> String {
    let norm = rel_path.replace('\\', "/");
    let norm = norm.trim_start_matches('/');
    match edition {
        Edition::Java => {
            if let Some(rest) = norm.strip_prefix("assets/")
                && let Some((ns, tail)) = rest.split_once('/')
            {
                let stripped = tail
                    .strip_prefix("textures/")
                    .or_else(|| tail.strip_prefix("models/"))
                    .or_else(|| tail.strip_prefix("blockstates/"))
                    .or_else(|| tail.strip_prefix("sounds/"))
                    .or_else(|| tail.strip_prefix("items/"))
                    .unwrap_or(tail);
                let no_ext = strip_known_asset_ext(stripped);
                return format!("{ns}:{no_ext}");
            }
            strip_known_asset_ext(norm).to_string()
        }
        Edition::Bedrock => strip_known_asset_ext(norm).to_string(),
    }
}

fn strip_known_asset_ext(path: &str) -> &str {
    for ext in [
        ".texture_set.json",
        ".png.mcmeta",
        ".geo.json",
        ".png",
        ".tga",
        ".json",
        ".ogg",
        ".lang",
    ] {
        if let Some(stripped) = path.strip_suffix(ext) {
            return stripped;
        }
    }
    path
}

/// Hierarchical VS Code-style file tree node built from the active `PackContainer` + empty folders.
#[derive(Debug, Clone, Default)]
pub struct FileTreeNode {
    pub name: String,
    /// Relative path from pack root (empty for root node).
    pub rel_path: String,
    pub dirs: BTreeMap<String, FileTreeNode>,
    pub files: Vec<(String, String)>, // (file_name, full_rel_path)
}

impl FileTreeNode {
    /// Builds a deterministic hierarchical directory tree matching `filter` (case-insensitive substring).
    #[must_use]
    pub fn build(
        container: &PackContainer,
        extra_folders: &BTreeSet<String>,
        filter: &str,
    ) -> Self {
        let mut root = Self::default();
        let needle = filter.trim().to_ascii_lowercase();

        for folder in extra_folders {
            if !needle.is_empty() && !folder.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            root.insert_folder(folder);
        }

        for path in container.files().keys() {
            if !needle.is_empty() && !path.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            root.insert_file(path);
        }

        root
    }

    fn insert_folder(&mut self, rel_folder: &str) {
        let mut cur = self;
        let mut acc = String::new();
        for seg in rel_folder.split('/').filter(|s| !s.is_empty()) {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            cur = cur.dirs.entry(seg.to_string()).or_insert_with(|| Self {
                name: seg.to_string(),
                rel_path: acc.clone(),
                ..Self::default()
            });
        }
    }

    fn insert_file(&mut self, rel_path: &str) {
        let mut parts: Vec<&str> = rel_path.split('/').filter(|s| !s.is_empty()).collect();
        let Some(file_name) = parts.pop() else {
            return;
        };
        let mut cur = self;
        let mut acc = String::new();
        for seg in parts {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            cur = cur.dirs.entry(seg.to_string()).or_insert_with(|| Self {
                name: seg.to_string(),
                rel_path: acc.clone(),
                ..Self::default()
            });
        }
        cur.files.push((file_name.to_string(), rel_path.to_string()));
    }
}

/// Active inline edit mode inside the Explorer tree (`New File`, `New Folder`, or `Rename`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplorerInlineEdit {
    NewFile { parent_dir: String, input: String },
    NewFolder { parent_dir: String, input: String },
    Rename { old_path: String, is_dir: bool, input: String },
}

/// Explorer clipboard state supporting Cut (`Ctrl+X`) and Copy (`Ctrl+C`) -> Paste (`Ctrl+V`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplorerClipboard {
    Copy(String),
    Cut(String),
}

/// Pending asset queued for Smart Minecraft Import (supports `.jpg`/`.webp` -> `.png`,
/// auto-sanitize filename, and conflict resolution Overwrite / Keep Both / Cancel).
#[derive(Debug, Clone)]
pub struct PendingImportItem {
    pub source_disk_path: PathBuf,
    pub target_dir: String,
    pub original_name: String,
    pub sanitized_name: String,
    pub needs_png_conversion: bool,
    pub needs_sanitization: bool,
    pub convert_to_png: bool,
    pub auto_sanitize: bool,
}

impl PendingImportItem {
    /// Inspects `src` to detect non-PNG image formats (`.jpg`/`.webp`) and invalid Minecraft names.
    #[must_use]
    pub fn from_path(src: PathBuf, target_dir: String) -> Self {
        let original_name = src
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("asset.png")
            .to_string();
        let lower = original_name.to_ascii_lowercase();
        let needs_png_conversion =
            lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".webp");

        let converted_name = if needs_png_conversion {
            if let Some((stem, _)) = original_name.rsplit_once('.') {
                format!("{stem}.png")
            } else {
                format!("{original_name}.png")
            }
        } else {
            original_name.clone()
        };
        let sanitized_name = sanitize_minecraft_filename(&converted_name);
        let needs_sanitization = !is_valid_minecraft_filename(&converted_name);

        Self {
            source_disk_path: src,
            target_dir,
            original_name,
            sanitized_name,
            needs_png_conversion,
            needs_sanitization,
            convert_to_png: needs_png_conversion,
            auto_sanitize: needs_sanitization,
        }
    }

    /// Computes the final target archive relative path based on user checkboxes.
    #[must_use]
    pub fn resolved_target_rel_path(&self) -> String {
        let name = if self.auto_sanitize {
            if self.convert_to_png {
                self.sanitized_name.clone()
            } else {
                sanitize_minecraft_filename(&self.original_name)
            }
        } else if self.convert_to_png && self.needs_png_conversion {
            if let Some((stem, _)) = self.original_name.rsplit_once('.') {
                format!("{stem}.png")
            } else {
                format!("{}.png", self.original_name)
            }
        } else {
            self.original_name.clone()
        };

        if self.target_dir.is_empty() {
            name
        } else {
            format!("{}/{name}", self.target_dir.trim_matches('/'))
        }
    }
}

/// Resolution choice when an imported file collides with an existing file in the pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolution {
    Overwrite,
    KeepBoth,
    Cancel,
}

/// Reads a source file from disk, optionally converting `.jpg`/`.webp` to `.png`, and returns
/// `(final_relative_path, raw_bytes)`.
pub fn prepare_import_bytes(
    item: &PendingImportItem,
    existing_container: &PackContainer,
    resolution: ConflictResolution,
) -> Result<Option<(String, Vec<u8>)>> {
    if resolution == ConflictResolution::Cancel {
        return Ok(None);
    }

    let mut rel_path = item.resolved_target_rel_path();
    if resolution == ConflictResolution::KeepBoth && existing_container.contains(&rel_path) {
        rel_path = make_unique_rel_path(existing_container, &rel_path);
    }

    let bytes = if item.convert_to_png && item.needs_png_conversion {
        let dyn_img = image::open(&item.source_disk_path).with_context(|| {
            format!(
                "failed to decode '{}' for PNG conversion",
                item.source_disk_path.display()
            )
        })?;
        encode_texture_bytes(&dyn_img.into_rgba8(), TextureFormatKind::Png)?
    } else {
        fs::read(&item.source_disk_path).with_context(|| {
            format!(
                "failed to read imported file '{}'",
                item.source_disk_path.display()
            )
        })?
    };

    Ok(Some((rel_path, bytes)))
}

/// Generates a non-colliding sibling path like `textures/blocks/stone_2.png`.
#[must_use]
pub fn make_unique_rel_path(container: &PackContainer, rel_path: &str) -> String {
    let (dir, file) = rel_path
        .rsplit_once('/')
        .map_or(("", rel_path), |(d, f)| (d, f));
    let (stem, ext) = file.rsplit_once('.').map_or((file, ""), |(s, e)| (s, e));
    for idx in 2..1000u32 {
        let candidate_name = if ext.is_empty() {
            format!("{stem}_{idx}")
        } else {
            format!("{stem}_{idx}.{ext}")
        };
        let candidate = if dir.is_empty() {
            candidate_name
        } else {
            format!("{dir}/{candidate_name}")
        };
        if !container.contains(&candidate) {
            return candidate;
        }
    }
    format!("{rel_path}_copy")
}

/// Generates a blank transparent 16x16 (or `size x size`) RGBA PNG buffer for `New Blank Texture`.
pub fn create_blank_png_bytes(size: u32) -> Result<Vec<u8>> {
    let dim = size.clamp(1, 2048);
    let img = RgbaImage::from_pixel(dim, dim, Rgba([0, 0, 0, 0]));
    encode_texture_bytes(&img, TextureFormatKind::Png)
}

/// Fuzzy-filters all file paths in the pack for the VS Code Quick Open (`Ctrl+P`) modal.
#[must_use]
pub fn filter_quick_open_files(container: &PackContainer, query: &str) -> Vec<String> {
    let mut scored: Vec<(i32, String)> = container
        .files()
        .keys()
        .filter_map(|path| fuzzy_score(query, path).map(|s| (s, path.clone())))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().take(60).map(|(_, p)| p).collect()
}

/// Performs file deletion in `LoadedPack` (both in-memory container and on-disk directory).
pub fn delete_pack_asset(pack: &mut LoadedPack, rel_path: &str) -> Result<()> {
    if let Some(root) = pack.source.directory_path() {
        let full = root.join(rel_path);
        if full.is_file() {
            fs::remove_file(&full)?;
        } else if full.is_dir() {
            fs::remove_dir_all(&full)?;
        }
    }
    let prefix = format!("{rel_path}/");
    let to_remove: Vec<String> = pack
        .container
        .files()
        .keys()
        .filter(|k| k.as_str() == rel_path || k.starts_with(&prefix))
        .cloned()
        .collect();
    for k in to_remove {
        pack.container.remove(&k);
    }
    pack.refresh_index();
    Ok(())
}

/// Renames a file or folder inside `LoadedPack` (updating both disk and container).
pub fn rename_pack_asset(
    pack: &mut LoadedPack,
    old_rel: &str,
    new_name_only: &str,
    is_dir: bool,
) -> Result<String> {
    let clean_new_name = new_name_only.trim().trim_matches('/');
    if clean_new_name.is_empty() || clean_new_name.contains("..") || clean_new_name.contains('/') {
        bail!("invalid target name '{new_name_only}'");
    }
    let new_rel = match old_rel.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{clean_new_name}"),
        None => clean_new_name.to_string(),
    };
    if new_rel == old_rel {
        return Ok(new_rel);
    }

    if let Some(root) = pack.source.directory_path() {
        let src = root.join(old_rel);
        let dst = root.join(&new_rel);
        if src.exists() {
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p)?;
            }
            fs::rename(&src, &dst)?;
        }
    }

    if is_dir {
        let old_prefix = format!("{old_rel}/");
        let moved: Vec<(String, Vec<u8>)> = pack
            .container
            .files()
            .keys()
            .filter(|k| k.starts_with(&old_prefix))
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|k| {
                let bytes = pack.container.remove(&k)?;
                let suffix = k.strip_prefix(&old_prefix)?;
                Some((format!("{new_rel}/{suffix}"), bytes))
            })
            .collect();
        for (k, b) in moved {
            pack.container.insert(&k, b)?;
        }
    } else if let Some(bytes) = pack.container.remove(old_rel) {
        pack.container.insert(&new_rel, bytes)?;
    }

    pack.refresh_index();
    Ok(new_rel)
}

/// Duplicates a single file inside `LoadedPack` into a unique sibling path.
pub fn duplicate_pack_asset(pack: &mut LoadedPack, rel_path: &str) -> Result<String> {
    let Some(bytes) = pack.container.get(rel_path).map(ToOwned::to_owned) else {
        bail!("asset '{rel_path}' not found");
    };
    let target = make_unique_rel_path(&pack.container, rel_path);
    pack.write_asset(&target, bytes)?;
    Ok(target)
}

/// Pastes a copied or cut file into `target_dir` inside `LoadedPack`.
pub fn paste_pack_asset(
    pack: &mut LoadedPack,
    clip: &ExplorerClipboard,
    target_dir: &str,
) -> Result<String> {
    let (src_path, is_cut) = match clip {
        ExplorerClipboard::Copy(p) => (p.as_str(), false),
        ExplorerClipboard::Cut(p) => (p.as_str(), true),
    };
    let Some(bytes) = pack.container.get(src_path).map(ToOwned::to_owned) else {
        bail!("clipboard source '{src_path}' no longer exists");
    };
    let fname = src_path.rsplit('/').next().unwrap_or(src_path);
    let base_target = if target_dir.is_empty() {
        fname.to_string()
    } else {
        format!("{}/{fname}", target_dir.trim_matches('/'))
    };
    let final_target = if pack.container.contains(&base_target) && base_target != src_path {
        make_unique_rel_path(&pack.container, &base_target)
    } else if base_target == src_path && !is_cut {
        make_unique_rel_path(&pack.container, &base_target)
    } else {
        base_target
    };

    pack.write_asset(&final_target, bytes)?;
    if is_cut && final_target != src_path {
        delete_pack_asset(pack, src_path)?;
    }
    Ok(final_target)
}
