use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use image::{Rgba, RgbaImage};
use rp_bedrock::schema::{BedrockManifest, BedrockManifestMetadata};
use rp_core::{
    Edition, normalize_bedrock_engine_triplet, parse_minecraft_json,
};
use rp_io::{IoLimits, PackContainer};
use rp_java::schema::{JavaPackMcmeta, JavaPackSection};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::state::{ActivePackState, PackType, PackWatcherHandle, ProjectSnapshot, StudioState};
use crate::texture::{
    TextureFormatKind, compute_dominant_resolution, decode_texture_bytes, encode_texture_bytes,
    resize_nearest_neighbor,
};

pub const YAZI_INSTALL_HINT: &str =
    "Install yazi: `pacman -S yazi` / `apt install yazi` / `cargo install yazi-fm`";

/// Template kind for the New Project Wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectTemplateKind {
    JavaResourcePack,
    BedrockResourcePack,
    BedrockAddonWorkspace,
}

const fn default_true() -> bool {
    true
}

/// Wizard payload sent from frontend `create_project`.
#[derive(Debug, Clone, Deserialize)]
pub struct NewProjectConfig {
    pub kind: ProjectTemplateKind,
    pub pack_name: String,
    pub description: String,
    pub authors: Vec<String>,
    pub license: String,
    pub url: String,
    pub version: [u32; 3],
    pub java_pack_format: u32,
    pub java_supported_formats: Option<[u32; 2]>,
    pub java_namespace: String,
    pub bedrock_min_engine_version: [u32; 3],
    pub icon_path: Option<String>,
    pub target_parent_dir: String,
    #[serde(default = "default_true")]
    pub include_pstdr_workspace: bool,
}

/// Binary file response returned to the Texture Viewer / 2D Paint Editor.
#[derive(Debug, Clone, Serialize)]
pub struct BinaryAssetData {
    pub rel_path: String,
    pub base64_png: String,
    pub raw_base64: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub format: String,
}

/// Result returned by `fs_rename`.
#[derive(Debug, Clone, Serialize)]
pub struct RenameResult {
    pub old_rel_path: String,
    pub new_rel_path: String,
    pub snapshot: ProjectSnapshot,
}

/// Result returned by `fs_import_assets`.
#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub imported_paths: Vec<String>,
    pub skipped_paths: Vec<String>,
    pub snapshot: ProjectSnapshot,
}

/// Detected OS file managers and `yazi` availability.
#[derive(Debug, Clone, Serialize)]
pub struct DetectedFileManagers {
    pub primary: String,
    pub available: Vec<String>,
    pub yazi_available: bool,
    pub yazi_path: Option<String>,
    pub terminal_emulator: Option<String>,
    pub yazi_install_hint: String,
}

/// Unified metadata DTO synced 2-way with `pack.mcmeta`, `manifest.json`, or linked `BP/manifest.json` + `RP/manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackMetadataDto {
    pub edition: Edition,
    #[serde(default = "default_pack_type")]
    pub pack_type: PackType,
    pub name: String,
    pub description: String,
    pub authors: Vec<String>,
    pub license: String,
    pub url: String,
    pub version: [u32; 3],
    pub java_pack_format: u32,
    pub java_supported_formats: Option<[u32; 2]>,
    pub bedrock_format_version: u32,
    pub bedrock_min_engine_version: [u32; 3],
    pub bedrock_header_uuid: String,
    pub bedrock_module_uuid: String,
    pub bedrock_capabilities: Vec<String>,
    #[serde(default)]
    pub bp_header_uuid: Option<String>,
    #[serde(default)]
    pub bp_module_uuid: Option<String>,
    #[serde(default)]
    pub bp_script_module_uuid: Option<String>,
    #[serde(default)]
    pub bp_script_entry: Option<String>,
    pub icon_base64_png: Option<String>,
}

const fn default_pack_type() -> PackType {
    PackType::BedrockResourcePack
}

/// Sanitizes an arbitrary string into a lowercase Minecraft slug (`[a-z0-9_-]`).
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
/// e.g. `"My Cool Block!.PNG"` -> `"my_cool_block.png"`.
#[must_use]
pub fn sanitize_minecraft_filename(raw_name: &str) -> String {
    let trimmed = raw_name.trim();
    let lower = trimmed.to_ascii_lowercase();
    let compound_suffix = [".texture_set.json", ".png.mcmeta", ".geo.json"]
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
            return format!("{}.{}", sanitize_stem_chars(s), clean_ext);
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

/// Generates a canonical Minecraft Resource ID from a pack-relative path.
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
                return format!("{ns}:{}", strip_known_ext(stripped));
            }
            strip_known_ext(norm).to_string()
        }
        Edition::Bedrock => strip_known_ext(norm).to_string(),
    }
}

fn strip_known_ext(path: &str) -> &str {
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

/// Locates the `BP/` and `RP/` sub-pack folder prefixes (ending with `/`) inside a Bedrock Add-on container.
#[must_use]
pub fn find_addon_subpack_prefixes(container: &PackContainer) -> (Option<String>, Option<String>) {
    let mut bp_prefix: Option<String> = None;
    let mut rp_prefix: Option<String> = None;

    for (path, bytes) in container.files() {
        let Some((dir, file)) = path.split_once('/') else {
            continue;
        };
        if file.contains('/') || !file.eq_ignore_ascii_case("manifest.json") {
            continue;
        }
        let dir_lower = dir.to_ascii_lowercase();
        let prefix = format!("{dir}/");
        if matches!(
            dir_lower.as_str(),
            "bp" | "behavior_pack" | "behaviour_pack" | "behavior" | "behaviour"
        ) {
            bp_prefix = Some(prefix);
            continue;
        }
        if matches!(dir_lower.as_str(), "rp" | "resource_pack" | "resources") {
            rp_prefix = Some(prefix);
            continue;
        }
        // Inspect manifest module types for non-standard folder names (e.g. `my_addon_bp/`).
        if let Ok(val) = parse_minecraft_json::<serde_json::Value>(bytes)
            && let Some(mods) = val.get("modules").and_then(serde_json::Value::as_array)
        {
            let has_resources = mods.iter().any(|m| {
                m.get("type")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|t| t.eq_ignore_ascii_case("resources"))
            });
            let has_bp_mod = mods.iter().any(|m| {
                m.get("type")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|t| {
                        t.eq_ignore_ascii_case("data") || t.eq_ignore_ascii_case("script")
                    })
            });
            if has_bp_mod && bp_prefix.is_none() {
                bp_prefix = Some(prefix);
            } else if has_resources && rp_prefix.is_none() {
                rp_prefix = Some(prefix);
            }
        }
    }

    (bp_prefix, rp_prefix)
}

/// Detects the first-class `PackType` from container structure.
pub fn detect_pack_type(container: &PackContainer) -> Result<PackType> {
    if container.contains("pack.mcmeta") {
        return Ok(PackType::JavaResourcePack);
    }
    if container.contains("manifest.json") {
        return Ok(PackType::BedrockResourcePack);
    }
    let (bp_prefix, rp_prefix) = find_addon_subpack_prefixes(container);
    let has_pstdr = container
        .files()
        .keys()
        .any(|k| !k.contains('/') && k.eq_ignore_ascii_case("pstdr.toml"));
    if (bp_prefix.is_some() && rp_prefix.is_some())
        || (has_pstdr && (bp_prefix.is_some() || rp_prefix.is_some()))
    {
        return Ok(PackType::BedrockAddon);
    }
    bail!(
        "Unable to detect pack type: missing 'pack.mcmeta', 'manifest.json', or Bedrock Add-on BP/RP manifests"
    )
}

/// Detects edition from container structure (`Java` vs `Bedrock`).
pub fn detect_pack_edition(container: &PackContainer) -> Result<Edition> {
    match detect_pack_type(container)? {
        PackType::JavaResourcePack => Ok(Edition::Java),
        PackType::BedrockResourcePack | PackType::BedrockAddon => Ok(Edition::Bedrock),
    }
}

/// Loads a resource pack or Bedrock Add-on workspace from disk (folder, `.zip`, `.mcpack`, or `.mcaddon`)
/// into an `ActivePackState`.
pub fn load_active_pack_from_path(path: &Path) -> Result<ActivePackState> {
    let container = PackContainer::read_from_path(path, IoLimits::default())
        .with_context(|| format!("failed reading pack '{}'", path.display()))?;
    let pack_type = detect_pack_type(&container)?;
    let edition = match pack_type {
        PackType::JavaResourcePack => Edition::Java,
        PackType::BedrockResourcePack | PackType::BedrockAddon => Edition::Bedrock,
    };
    let is_directory = path.is_dir();

    // Backward compatibility: if user opens an isolated `RP/` folder next to a sibling `../BP/`.
    let companion_bp_dir = if pack_type == PackType::BedrockResourcePack
        && is_directory
        && path.file_name().and_then(|n| n.to_str()) == Some("RP")
    {
        let sibling_bp = path.parent().map(|p| p.join("BP"));
        sibling_bp.filter(|bp| bp.join("manifest.json").is_file())
    } else {
        None
    };

    let display_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .map_or_else(|| path.display().to_string(), ToString::to_string);

    let (pack_name, edition_label) = extract_pack_identity(&display_name, &container, pack_type);

    Ok(ActivePackState {
        source_path: Some(path.to_path_buf()),
        is_directory,
        companion_bp_dir,
        container,
        edition,
        pack_type,
        pack_name,
        edition_label,
        extra_folders: Default::default(),
    })
}

fn extract_pack_identity(
    fallback_name: &str,
    container: &PackContainer,
    pack_type: PackType,
) -> (String, String) {
    match pack_type {
        PackType::JavaResourcePack => {
            let mut name = fallback_name.to_string();
            let mut label = String::from("Java Edition");
            if let Some(bytes) = container.get("pack.mcmeta") {
                if let Ok(mcmeta) = parse_minecraft_json::<JavaPackMcmeta>(bytes)
                    && let Ok((fmt, _, _)) = mcmeta.validate_and_extract()
                {
                    label = format!("Java (fmt {fmt})");
                }
                if let Ok(raw) = parse_minecraft_json::<serde_json::Value>(bytes)
                    && let Some(n) = raw.get("meta").and_then(|m| m.get("name")).and_then(|v| v.as_str())
                    && !n.trim().is_empty()
                {
                    name = n.to_string();
                }
            }
            (name, label)
        }
        PackType::BedrockResourcePack => {
            if let Some(bytes) = container.get("manifest.json")
                && let Ok(manifest) = parse_minecraft_json::<BedrockManifest>(bytes)
            {
                let v = manifest.header.min_engine_version;
                let name = if manifest.header.name.trim().is_empty() {
                    fallback_name.to_string()
                } else {
                    manifest.header.name
                };
                return (name, format!("Bedrock {}.{}.{}", v[0], v[1], v[2]));
            }
            (fallback_name.to_string(), String::from("Bedrock 1.x.x"))
        }
        PackType::BedrockAddon => {
            let (bp_prefix, rp_prefix) = find_addon_subpack_prefixes(container);
            let rp_bytes = rp_prefix
                .as_deref()
                .and_then(|p| container.get(&format!("{p}manifest.json")));
            let bp_bytes = bp_prefix
                .as_deref()
                .and_then(|p| container.get(&format!("{p}manifest.json")));
            if let Some(bytes) = rp_bytes.or(bp_bytes)
                && let Ok(manifest) = parse_minecraft_json::<BedrockManifest>(bytes)
            {
                let v = manifest.header.min_engine_version;
                let raw_name = manifest.header.name.trim();
                let cleaned = raw_name
                    .strip_suffix("(Behavior Pack)")
                    .map(str::trim)
                    .unwrap_or(raw_name);
                let name = if cleaned.is_empty() {
                    fallback_name.to_string()
                } else {
                    cleaned.to_string()
                };
                return (name, format!("Bedrock Add-on {}.{}.{}", v[0], v[1], v[2]));
            }
            (fallback_name.to_string(), String::from("Bedrock Add-on"))
        }
    }
}

/// Scaffolds a new project (Java RP, Bedrock RP, or Bedrock Add-On RP+BP) and returns the path to open.
pub fn scaffold_project_on_disk(cfg: &NewProjectConfig) -> Result<PathBuf> {
    let pack_name = cfg.pack_name.trim();
    if pack_name.is_empty() {
        bail!("Pack Name must not be empty");
    }
    let parent_str = cfg.target_parent_dir.trim();
    if parent_str.is_empty() {
        bail!("Target project folder must not be empty");
    }
    let parent = PathBuf::from(parent_str);
    let slug = sanitize_minecraft_slug(pack_name);
    let folder = if slug.is_empty() {
        String::from("new_resource_pack")
    } else {
        slug
    };
    let project_root = parent.join(&folder);
    fs::create_dir_all(&project_root)?;

    let icon_png = load_or_generate_icon(cfg.icon_path.as_deref(), cfg.kind)?;
    let stone_png = generate_starter_stone_png()?;
    let stick_png = generate_starter_item_png()?;

    let authors: Vec<String> = cfg
        .authors
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    match cfg.kind {
        ProjectTemplateKind::JavaResourcePack => {
            let ns_slug = sanitize_minecraft_slug(&cfg.java_namespace);
            let ns = if ns_slug.is_empty() {
                String::from("minecraft")
            } else {
                ns_slug
            };
            for d in [
                format!("assets/{ns}/textures/block"),
                format!("assets/{ns}/textures/item"),
                format!("assets/{ns}/textures/entity"),
                format!("assets/{ns}/textures/gui"),
                format!("assets/{ns}/textures/mob_effect"),
                format!("assets/{ns}/models/block"),
                format!("assets/{ns}/models/item"),
                format!("assets/{ns}/blockstates"),
                format!("assets/{ns}/lang"),
            ] {
                fs::create_dir_all(project_root.join(d))?;
            }

            let fmt = cfg.java_pack_format.clamp(1, 200);
            let mut pack_obj = serde_json::Map::new();
            pack_obj.insert(String::from("pack_format"), json!(fmt));
            pack_obj.insert(String::from("description"), json!(cfg.description));
            if let Some([lo, hi]) = cfg.java_supported_formats {
                pack_obj.insert(
                    String::from("supported_formats"),
                    json!([lo.min(hi).max(1), lo.max(hi).max(1)]),
                );
            }
            let mut root = serde_json::Map::new();
            root.insert(String::from("pack"), serde_json::Value::Object(pack_obj));
            root.insert(
                String::from("meta"),
                json!({
                    "name": pack_name,
                    "version": format!("{}.{}.{}", cfg.version[0], cfg.version[1], cfg.version[2]),
                    "authors": authors,
                    "license": cfg.license.trim(),
                    "url": cfg.url.trim(),
                }),
            );
            fs::write(
                project_root.join("pack.mcmeta"),
                serde_json::to_vec_pretty(&serde_json::Value::Object(root))?,
            )?;
            fs::write(project_root.join("pack.png"), icon_png)?;
            fs::write(
                project_root.join(format!("assets/{ns}/textures/block/stone.png")),
                stone_png,
            )?;
            fs::write(
                project_root.join(format!("assets/{ns}/lang/en_us.json")),
                serde_json::to_vec_pretty(&json!({}))?,
            )?;
            Ok(project_root)
        }
        ProjectTemplateKind::BedrockResourcePack => {
            scaffold_bedrock_rp_dir(
                &project_root,
                cfg,
                &authors,
                &icon_png,
                &stone_png,
                &stick_png,
                None,
            )?;
            Ok(project_root)
        }
        ProjectTemplateKind::BedrockAddonWorkspace => {
            let rp_dir = project_root.join("RP");
            let bp_dir = project_root.join("BP");
            fs::create_dir_all(&rp_dir)?;
            fs::create_dir_all(&bp_dir)?;

            let rp_uuid = Uuid::new_v4();
            let mut bp_uuid = Uuid::new_v4();
            while bp_uuid == rp_uuid {
                bp_uuid = Uuid::new_v4();
            }

            scaffold_bedrock_rp_dir(
                &rp_dir,
                cfg,
                &authors,
                &icon_png,
                &stone_png,
                &stick_png,
                Some((rp_uuid, bp_uuid)),
            )?;
            scaffold_bedrock_bp_dir(&bp_dir, cfg, &authors, &icon_png, bp_uuid, rp_uuid)?;
            if cfg.include_pstdr_workspace {
                scaffold_pstdr_workspace(&project_root, &folder, cfg)?;
            }
            Ok(project_root)
        }
    }
}

fn scaffold_bedrock_rp_dir(
    root: &Path,
    cfg: &NewProjectConfig,
    authors: &[String],
    icon_png: &[u8],
    stone_png: &[u8],
    stick_png: &[u8],
    linked: Option<(Uuid, Uuid)>,
) -> Result<()> {
    for d in [
        "textures/blocks",
        "textures/items",
        "textures/ui",
        "textures/entity",
        "models/entity",
        "texts",
    ] {
        fs::create_dir_all(root.join(d))?;
    }

    let mut min_ver = normalize_bedrock_engine_triplet(cfg.bedrock_min_engine_version);
    min_ver[0] = 1;

    let mut manifest = BedrockManifest::new(
        cfg.pack_name.trim(),
        cfg.description.clone(),
        cfg.version,
        min_ver,
    );
    if let Some((rp_uuid, _)) = linked {
        manifest.header.uuid = rp_uuid;
        if let Some(m) = manifest.modules.first_mut()
            && m.uuid == rp_uuid
        {
            m.uuid = Uuid::new_v4();
        }
    }
    let final_authors = if authors.is_empty() {
        vec![String::from("PackStudio Artist")]
    } else {
        authors.to_vec()
    };
    manifest.metadata = Some(BedrockManifestMetadata {
        authors: final_authors.clone(),
    });

    let mut manifest_val: serde_json::Value =
        serde_json::from_slice(&manifest.to_json_vec_versioned(2)?)?;
    if let Some(obj) = manifest_val.as_object_mut() {
        let mut meta = serde_json::Map::new();
        meta.insert(String::from("authors"), json!(final_authors));
        if !cfg.license.trim().is_empty() {
            meta.insert(String::from("license"), json!(cfg.license.trim()));
        }
        if !cfg.url.trim().is_empty() {
            meta.insert(String::from("url"), json!(cfg.url.trim()));
        }
        obj.insert(String::from("metadata"), serde_json::Value::Object(meta));
        if let Some((_, bp_uuid)) = linked {
            obj.insert(
                String::from("dependencies"),
                json!([{ "uuid": bp_uuid.to_string(), "version": cfg.version }]),
            );
        }
    }

    fs::write(root.join("manifest.json"), serde_json::to_vec_pretty(&manifest_val)?)?;
    fs::write(root.join("pack_icon.png"), icon_png)?;
    fs::write(root.join("textures/blocks/stone.png"), stone_png)?;
    fs::write(root.join("textures/items/stick.png"), stick_png)?;
    fs::write(
        root.join("textures/terrain_texture.json"),
        serde_json::to_vec_pretty(&json!({
            "resource_pack_name": cfg.pack_name.trim(),
            "texture_name": "atlas.terrain",
            "padding": 8,
            "num_mip_levels": 4,
            "texture_data": { "stone": { "textures": "textures/blocks/stone" } }
        }))?,
    )?;
    fs::write(
        root.join("textures/item_texture.json"),
        serde_json::to_vec_pretty(&json!({
            "resource_pack_name": cfg.pack_name.trim(),
            "texture_name": "atlas.items",
            "texture_data": { "stick": { "textures": "textures/items/stick" } }
        }))?,
    )?;
    fs::write(
        root.join("texts/languages.json"),
        serde_json::to_vec_pretty(&json!(["en_US"]))?,
    )?;
    fs::write(
        root.join("texts/en_US.lang"),
        format!(
            "pack.name={}\npack.description={}\n",
            cfg.pack_name.trim(),
            cfg.description.replace('\n', " ")
        ),
    )?;
    Ok(())
}

fn scaffold_bedrock_bp_dir(
    root: &Path,
    cfg: &NewProjectConfig,
    authors: &[String],
    icon_png: &[u8],
    bp_uuid: Uuid,
    rp_uuid: Uuid,
) -> Result<()> {
    for d in ["blocks", "items", "entities", "scripts", "texts"] {
        fs::create_dir_all(root.join(d))?;
    }
    let mut min_ver = normalize_bedrock_engine_triplet(cfg.bedrock_min_engine_version);
    min_ver[0] = 1;

    let mut bp_mod = Uuid::new_v4();
    while bp_mod == bp_uuid || bp_mod == rp_uuid {
        bp_mod = Uuid::new_v4();
    }
    let mut script_mod = Uuid::new_v4();
    while script_mod == bp_uuid || script_mod == rp_uuid || script_mod == bp_mod {
        script_mod = Uuid::new_v4();
    }
    let final_authors = if authors.is_empty() {
        vec![String::from("PackStudio Artist")]
    } else {
        authors.to_vec()
    };

    let bp_manifest = json!({
        "format_version": 2,
        "header": {
            "name": format!("{} (Behavior Pack)", cfg.pack_name.trim()),
            "description": cfg.description,
            "uuid": bp_uuid.to_string(),
            "version": cfg.version,
            "min_engine_version": min_ver
        },
        "modules": [
            {
                "description": cfg.description,
                "type": "data",
                "uuid": bp_mod.to_string(),
                "version": cfg.version
            },
            {
                "description": "Scripting API Module (@minecraft/server 2.11.0)",
                "type": "script",
                "language": "javascript",
                "uuid": script_mod.to_string(),
                "version": cfg.version,
                "entry": "scripts/main.js"
            }
        ],
        "dependencies": [
            {
                "uuid": rp_uuid.to_string(),
                "version": cfg.version
            },
            {
                "module_name": "@minecraft/server",
                "version": "2.11.0"
            }
        ],
        "metadata": {
            "authors": final_authors,
            "license": cfg.license.trim(),
            "url": cfg.url.trim()
        }
    });
    fs::write(root.join("manifest.json"), serde_json::to_vec_pretty(&bp_manifest)?)?;
    fs::write(root.join("pack_icon.png"), icon_png)?;
    fs::write(
        root.join("scripts/main.js"),
        format!(
            "import {{ world }} from \"@minecraft/server\";\n\nworld.afterEvents.worldLoad.subscribe(() => {{\n  console.warn(\"[{}] Behavior Pack script initialized.\");\n}});\n",
            cfg.pack_name.trim().replace('"', "\\\"")
        ),
    )?;
    Ok(())
}

fn scaffold_pstdr_workspace(
    project_root: &Path,
    slug: &str,
    cfg: &NewProjectConfig,
) -> Result<()> {
    let src_dir = project_root.join("src");
    fs::create_dir_all(&src_dir)?;
    let ver = format!("{}.{}.{}", cfg.version[0], cfg.version[1], cfg.version[2]);
    let desc_escaped = cfg.description.replace('"', "\\\"");
    let toml = format!(
        "[package]\nname = \"{slug}\"\nversion = \"{ver}\"\ndescription = \"{desc_escaped}\"\n\n[target]\nedition = \"bedrock\"\nminecraft_version = \"26.60\"\nscript_api = \"@minecraft/server\"\nscript_api_version = \"2.11.0\"\n\n[paths]\nsrc = \"src\"\nbehavior_pack = \"BP\"\nresource_pack = \"RP\"\n"
    );
    fs::write(project_root.join("pstdr.toml"), toml)?;

    let main_pstdr = format!(
        "// PackStudio Add-on DSL starter (Bedrock 26.60 / @minecraft/server 2.11.0)\naddon \"{slug}\" {{\n    version = \"{ver}\";\n    target = \"bedrock:26.60\";\n}}\n"
    );
    fs::write(src_dir.join("main.pstdr"), main_pstdr)?;
    Ok(())
}

fn load_or_generate_icon(icon_path: Option<&str>, kind: ProjectTemplateKind) -> Result<Vec<u8>> {
    if let Some(p_str) = icon_path.map(str::trim).filter(|s| !s.is_empty()) {
        let p = Path::new(p_str);
        if p.is_file() {
            let dyn_img = image::open(p)?;
            let rgba = dyn_img.into_rgba8();
            let final_img = if rgba.width() > 256 || rgba.height() > 256 {
                resize_nearest_neighbor(&rgba, 128, 128)?
            } else {
                rgba
            };
            return encode_texture_bytes(&final_img, TextureFormatKind::Png);
        }
    }

    let mut icon = RgbaImage::new(64, 64);
    let accent = match kind {
        ProjectTemplateKind::JavaResourcePack => Rgba([0x4C, 0xAF, 0x50, 255]),
        ProjectTemplateKind::BedrockResourcePack => Rgba([0x00, 0x7A, 0xCC, 255]),
        ProjectTemplateKind::BedrockAddonWorkspace => Rgba([0xD9, 0x77, 0x06, 255]),
    };
    for y in 0..64u32 {
        for x in 0..64u32 {
            let px = if y < 20 {
                accent
            } else if (x + y) % 8 == 0 {
                Rgba([0x66, 0x47, 0x2F, 255])
            } else {
                Rgba([0x7A, 0x55, 0x38, 255])
            };
            icon.put_pixel(x, y, px);
        }
    }
    encode_texture_bytes(&icon, TextureFormatKind::Png)
}

fn generate_starter_stone_png() -> Result<Vec<u8>> {
    let mut stone = RgbaImage::new(16, 16);
    for y in 0..16u32 {
        for x in 0..16u32 {
            let shade = 116u8.saturating_add(((x * 13 + y * 7) % 28) as u8);
            stone.put_pixel(x, y, Rgba([shade, shade, shade, 255]));
        }
    }
    encode_texture_bytes(&stone, TextureFormatKind::Png)
}

fn generate_starter_item_png() -> Result<Vec<u8>> {
    let mut stick = RgbaImage::new(16, 16);
    for i in 3..13u32 {
        stick.put_pixel(i, 15 - i, Rgba([0x89, 0x67, 0x27, 255]));
        if i + 1 < 16 {
            stick.put_pixel(i + 1, 15 - i, Rgba([0x68, 0x4E, 0x1E, 255]));
        }
    }
    encode_texture_bytes(&stick, TextureFormatKind::Png)
}

/// Checks whether a binary exists in `$PATH`.
#[must_use]
pub fn find_in_path(bin_name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(bin_name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(target_os = "windows")]
        {
            let exe = dir.join(format!("{bin_name}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

/// Parses output lines written by `yazi --chooser-file=<path>`, trimming CR/LF,
/// discarding blank lines, and preserving spaces inside file paths.
#[must_use]
pub fn parse_yazi_chooser_output(raw: &str) -> Vec<PathBuf> {
    raw.lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Detects available GUI terminal emulator for interactive `yazi --chooser-file` popup.
#[must_use]
pub fn detect_terminal_emulator() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        for candidate in [
            "x-terminal-emulator",
            "qterminal",
            "kitty",
            "alacritty",
            "wezterm",
            "foot",
            "xfce4-terminal",
            "konsole",
            "gnome-terminal",
            "lxterminal",
            "xterm",
        ] {
            if find_in_path(candidate).is_some() {
                return Some(String::from(candidate));
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Tauri Command: Open a resource pack folder, `.zip`, or `.mcpack`.
#[tauri::command]
pub fn open_project(
    app: AppHandle,
    state: tauri::State<'_, StudioState>,
    path: String,
) -> Result<ProjectSnapshot, String> {
    let target = PathBuf::from(path.trim());
    let pack = load_active_pack_from_path(&target).map_err(|e| format!("{e:#}"))?;
    let (tex_count, dom_res) = compute_dominant_resolution(&pack.container);
    let snapshot = pack.to_snapshot(dom_res, tex_count);

    let watcher_root = if pack.is_directory {
        pack.source_path.clone()
    } else {
        None
    };

    if let Ok(guard) = state.blockbench.lock()
        && let Some(host) = guard.as_ref()
    {
        host.server.set_project_root(watcher_root.clone());
    }

    if let Ok(mut wguard) = state.watcher.lock() {
        *wguard = None;
        if let Some(root) = watcher_root {
            let app_clone = app.clone();
            if let Ok(handle) = PackWatcherHandle::start(root, move |changed_paths| {
                let _ = app_clone.emit("fs://changed", changed_paths);
            }) {
                *wguard = Some(handle);
            }
        }
    }

    let mut guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    *guard = Some(pack);
    Ok(snapshot)
}

/// Tauri Command: Scaffold a new Java RP / Bedrock RP / Bedrock Add-On workspace and open it.
#[tauri::command]
pub fn create_project(
    app: AppHandle,
    state: tauri::State<'_, StudioState>,
    config: NewProjectConfig,
) -> Result<ProjectSnapshot, String> {
    let open_dir = scaffold_project_on_disk(&config).map_err(|e| format!("{e:#}"))?;
    open_project(app, state, open_dir.display().to_string())
}

/// Tauri Command: Create a file inside the active project.
#[tauri::command]
pub fn fs_create_file(
    state: tauri::State<'_, StudioState>,
    rel_path: String,
    initial_content: Option<String>,
) -> Result<ProjectSnapshot, String> {
    let bytes = initial_content.unwrap_or_default().into_bytes();
    let mut guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_mut()
        .ok_or_else(|| String::from("No resource pack currently open"))?;
    pack.write_asset(&rel_path, bytes)
        .map_err(|e| format!("{e:#}"))?;
    let (tex_count, dom_res) = compute_dominant_resolution(&pack.container);
    Ok(pack.to_snapshot(dom_res, tex_count))
}

/// Tauri Command: Create an empty folder inside the active project.
#[tauri::command]
pub fn fs_create_dir(
    state: tauri::State<'_, StudioState>,
    rel_path: String,
) -> Result<ProjectSnapshot, String> {
    let clean = rp_io::sanitize_archive_path(&rel_path).map_err(|e| format!("{e:#}"))?;
    let mut guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_mut()
        .ok_or_else(|| String::from("No resource pack currently open"))?;
    if pack.is_directory
        && let Some(root) = &pack.source_path
    {
        fs::create_dir_all(root.join(&clean)).map_err(|e| format!("{e:#}"))?;
    }
    pack.extra_folders.insert(clean);
    let (tex_count, dom_res) = compute_dominant_resolution(&pack.container);
    Ok(pack.to_snapshot(dom_res, tex_count))
}

/// Tauri Command: Rename a file or folder inside the active project.
#[tauri::command]
pub fn fs_rename(
    state: tauri::State<'_, StudioState>,
    old_rel_path: String,
    new_name: String,
    is_dir: bool,
) -> Result<RenameResult, String> {
    let clean_name = new_name.trim().trim_matches('/');
    if clean_name.is_empty() || clean_name.contains("..") || clean_name.contains('/') {
        return Err(format!("Invalid target name '{new_name}'"));
    }
    let new_rel = match old_rel_path.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{clean_name}"),
        None => clean_name.to_string(),
    };

    let mut guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_mut()
        .ok_or_else(|| String::from("No resource pack currently open"))?;

    if pack.is_directory
        && let Some(root) = &pack.source_path
    {
        let src = root.join(&old_rel_path);
        let dst = root.join(&new_rel);
        if src.exists() {
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p).map_err(|e| format!("{e:#}"))?;
            }
            fs::rename(&src, &dst).map_err(|e| format!("{e:#}"))?;
        }
    }

    if is_dir {
        let old_prefix = format!("{old_rel_path}/");
        let keys: Vec<String> = pack
            .container
            .files()
            .keys()
            .filter(|k| k.starts_with(&old_prefix))
            .cloned()
            .collect();
        for k in keys {
            if let Some(bytes) = pack.container.remove(&k)
                && let Some(suffix) = k.strip_prefix(&old_prefix)
            {
                pack.container
                    .insert(format!("{new_rel}/{suffix}"), bytes)
                    .map_err(|e| format!("{e:#}"))?;
            }
        }
        pack.extra_folders.remove(&old_rel_path);
        pack.extra_folders.insert(new_rel.clone());
    } else if let Some(bytes) = pack.container.remove(&old_rel_path) {
        pack.container
            .insert(&new_rel, bytes)
            .map_err(|e| format!("{e:#}"))?;
    }

    let (tex_count, dom_res) = compute_dominant_resolution(&pack.container);
    let snapshot = pack.to_snapshot(dom_res, tex_count);
    Ok(RenameResult {
        old_rel_path,
        new_rel_path: new_rel,
        snapshot,
    })
}

/// Tauri Command: Delete a file or folder inside the active project.
#[tauri::command]
pub fn fs_delete(
    state: tauri::State<'_, StudioState>,
    rel_path: String,
) -> Result<ProjectSnapshot, String> {
    let mut guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_mut()
        .ok_or_else(|| String::from("No resource pack currently open"))?;

    if pack.is_directory
        && let Some(root) = &pack.source_path
    {
        let full = root.join(&rel_path);
        if full.is_file() {
            fs::remove_file(&full).map_err(|e| format!("{e:#}"))?;
        } else if full.is_dir() {
            fs::remove_dir_all(&full).map_err(|e| format!("{e:#}"))?;
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
    pack.extra_folders.remove(&rel_path);

    let (tex_count, dom_res) = compute_dominant_resolution(&pack.container);
    Ok(pack.to_snapshot(dom_res, tex_count))
}

/// Tauri Command: Import assets from OS disk paths into `target_dir`, optionally sanitizing
/// filenames for Minecraft and converting `.jpg`/`.webp` images into `.png`.
#[tauri::command]
pub fn fs_import_assets(
    state: tauri::State<'_, StudioState>,
    target_dir: String,
    disk_paths: Vec<String>,
    auto_sanitize: bool,
    convert_images_to_png: bool,
    overwrite: bool,
) -> Result<ImportResult, String> {
    let mut guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_mut()
        .ok_or_else(|| String::from("No resource pack currently open"))?;

    let mut imported_paths = Vec::new();
    let mut skipped_paths = Vec::new();
    let dir_clean = target_dir.trim().trim_matches('/');

    for src_str in disk_paths {
        let src = PathBuf::from(&src_str);
        if !src.is_file() {
            skipped_paths.push(src_str);
            continue;
        }
        let orig_name = src
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("asset.png")
            .to_string();
        let lower = orig_name.to_ascii_lowercase();
        let is_non_png_image =
            lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".webp");

        let mut target_name = if convert_images_to_png && is_non_png_image {
            if let Some((stem, _)) = orig_name.rsplit_once('.') {
                format!("{stem}.png")
            } else {
                format!("{orig_name}.png")
            }
        } else {
            orig_name
        };

        if auto_sanitize {
            target_name = sanitize_minecraft_filename(&target_name);
        }

        let mut dest_rel = if dir_clean.is_empty() {
            target_name
        } else {
            format!("{dir_clean}/{target_name}")
        };

        if pack.container.contains(&dest_rel) && !overwrite {
            dest_rel = make_unique_path(&pack.container, &dest_rel);
        }

        let bytes = if convert_images_to_png && is_non_png_image {
            let dyn_img = image::open(&src).map_err(|e| format!("{e:#}"))?;
            encode_texture_bytes(&dyn_img.into_rgba8(), TextureFormatKind::Png)
                .map_err(|e| format!("{e:#}"))?
        } else {
            fs::read(&src).map_err(|e| format!("{e:#}"))?
        };

        pack.write_asset(&dest_rel, bytes)
            .map_err(|e| format!("{e:#}"))?;
        imported_paths.push(dest_rel);
    }

    let (tex_count, dom_res) = compute_dominant_resolution(&pack.container);
    let snapshot = pack.to_snapshot(dom_res, tex_count);
    Ok(ImportResult {
        imported_paths,
        skipped_paths,
        snapshot,
    })
}

fn make_unique_path(container: &PackContainer, rel_path: &str) -> String {
    let (dir, file) = rel_path
        .rsplit_once('/')
        .map_or(("", rel_path), |(d, f)| (d, f));
    let (stem, ext) = file.rsplit_once('.').map_or((file, ""), |(s, e)| (s, e));
    for idx in 2..1000u32 {
        let name = if ext.is_empty() {
            format!("{stem}_{idx}")
        } else {
            format!("{stem}_{idx}.{ext}")
        };
        let candidate = if dir.is_empty() {
            name
        } else {
            format!("{dir}/{name}")
        };
        if !container.contains(&candidate) {
            return candidate;
        }
    }
    format!("{rel_path}_copy")
}

/// Tauri Command: Read a UTF-8/UTF-16/Latin-1 text file from the active pack.
#[tauri::command]
pub fn fs_read_text(state: tauri::State<'_, StudioState>, rel_path: String) -> Result<String, String> {
    let guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_ref()
        .ok_or_else(|| String::from("No resource pack currently open"))?;
    let bytes = pack
        .container
        .get(&rel_path)
        .ok_or_else(|| format!("File '{rel_path}' not found"))?;
    Ok(rp_core::decode_minecraft_text(bytes).into_owned())
}

/// Tauri Command: Write a text file back to the active pack (and disk if directory-backed).
#[tauri::command]
pub fn fs_write_text(
    state: tauri::State<'_, StudioState>,
    rel_path: String,
    content: String,
) -> Result<ProjectSnapshot, String> {
    let mut guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_mut()
        .ok_or_else(|| String::from("No resource pack currently open"))?;
    pack.write_asset(&rel_path, content.into_bytes())
        .map_err(|e| format!("{e:#}"))?;
    let (tex_count, dom_res) = compute_dominant_resolution(&pack.container);
    Ok(pack.to_snapshot(dom_res, tex_count))
}

/// Tauri Command: Read a binary image asset (`.png` / `.tga`) as base64 + PNG preview + dimensions.
#[tauri::command]
pub fn fs_read_binary_base64(
    state: tauri::State<'_, StudioState>,
    rel_path: String,
) -> Result<BinaryAssetData, String> {
    let guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_ref()
        .ok_or_else(|| String::from("No resource pack currently open"))?;
    let bytes = pack
        .container
        .get(&rel_path)
        .ok_or_else(|| format!("Binary asset '{rel_path}' not found"))?;

    let raw_base64 = BASE64_STANDARD.encode(bytes);
    if let Ok((img, fmt)) = decode_texture_bytes(bytes, &rel_path) {
        let png_bytes = if fmt == TextureFormatKind::Png {
            bytes.to_vec()
        } else {
            encode_texture_bytes(&img, TextureFormatKind::Png).map_err(|e| format!("{e:#}"))?
        };
        Ok(BinaryAssetData {
            rel_path,
            base64_png: BASE64_STANDARD.encode(&png_bytes),
            raw_base64,
            width: Some(img.width()),
            height: Some(img.height()),
            format: fmt.label().to_string(),
        })
    } else {
        Ok(BinaryAssetData {
            rel_path,
            base64_png: raw_base64.clone(),
            raw_base64,
            width: None,
            height: None,
            format: String::from("BIN"),
        })
    }
}

/// Tauri Command: Write base64-encoded binary PNG/asset bytes from the 2D Paint Canvas back to the pack.
#[tauri::command]
pub fn fs_write_binary_base64(
    state: tauri::State<'_, StudioState>,
    rel_path: String,
    base64_data: String,
) -> Result<ProjectSnapshot, String> {
    let clean_b64 = base64_data
        .split_once(',')
        .map_or(base64_data.as_str(), |(_, b)| b)
        .trim();
    let bytes = BASE64_STANDARD
        .decode(clean_b64)
        .map_err(|e| format!("invalid base64 binary payload: {e}"))?;

    let mut guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_mut()
        .ok_or_else(|| String::from("No resource pack currently open"))?;
    pack.write_asset(&rel_path, bytes)
        .map_err(|e| format!("{e:#}"))?;
    let (tex_count, dom_res) = compute_dominant_resolution(&pack.container);
    Ok(pack.to_snapshot(dom_res, tex_count))
}

/// Tauri Command: Auto-detect `yazi` and OS file managers (`pcmanfm-qt`, `thunar`, `dolphin`, etc.).
#[tauri::command]
pub fn detect_file_managers() -> DetectedFileManagers {
    let mut available = Vec::new();
    #[cfg(target_os = "linux")]
    let candidates = [
        "pcmanfm-qt",
        "thunar",
        "dolphin",
        "nautilus",
        "nemo",
        "pcmanfm",
        "xdg-open",
    ];
    #[cfg(target_os = "windows")]
    let candidates = ["explorer.exe"];
    #[cfg(target_os = "macos")]
    let candidates = ["open"];

    for c in candidates {
        if find_in_path(c).is_some() {
            available.push(c.to_string());
        }
    }
    let primary = available
        .first()
        .cloned()
        .unwrap_or_else(|| String::from(candidates[candidates.len() - 1]));
    let yazi_path = find_in_path("yazi");

    DetectedFileManagers {
        primary,
        available,
        yazi_available: yazi_path.is_some(),
        yazi_path: yazi_path.map(|p| p.display().to_string()),
        terminal_emulator: detect_terminal_emulator(),
        yazi_install_hint: String::from(YAZI_INSTALL_HINT),
    }
}

/// Tauri Command: Run `yazi --chooser-file=<tmp>` in an interactive terminal and return chosen paths.
#[tauri::command]
pub async fn pick_with_yazi(start_dir: Option<String>) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>> {
        if find_in_path("yazi").is_none() {
            bail!("yazi is not installed in $PATH. {YAZI_INSTALL_HINT}");
        }
        let dir = start_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let dir_arg = if dir.is_dir() {
            dir
        } else {
            dir.parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
        };

        let tmp_dir = env::temp_dir().join("pack-studio-yazi");
        fs::create_dir_all(&tmp_dir)?;
        let chooser_file = tmp_dir.join(format!("chooser-{}.txt", Uuid::new_v4().simple()));

        let status = if let Some(term) = detect_terminal_emulator() {
            let flag = match term.as_str() {
                "gnome-terminal" => "--",
                _ => "-e",
            };
            Command::new(&term)
                .arg(flag)
                .arg("yazi")
                .arg("--chooser-file")
                .arg(&chooser_file)
                .arg(&dir_arg)
                .status()
                .with_context(|| format!("failed launching '{term}' with yazi chooser"))?
        } else {
            Command::new("yazi")
                .arg("--chooser-file")
                .arg(&chooser_file)
                .arg(&dir_arg)
                .status()
                .context("failed launching yazi chooser")?
        };

        let picked = if chooser_file.is_file() {
            let raw = fs::read_to_string(&chooser_file).unwrap_or_default();
            let _ = fs::remove_file(&chooser_file);
            parse_yazi_chooser_output(&raw)
                .into_iter()
                .map(|p| p.display().to_string())
                .collect()
        } else {
            Vec::new()
        };

        if !status.success() && picked.is_empty() {
            bail!("yazi exited with status {status}");
        }
        Ok(picked)
    })
    .await
    .map_err(|e| format!("yazi task panicked: {e}"))?
    .map_err(|e| format!("{e:#}"))
}

/// Tauri Command: Reveal a file or folder in the detected/overridden OS file manager.
#[tauri::command]
pub fn reveal_in_file_manager(
    state: tauri::State<'_, StudioState>,
    path: String,
    manager_override: Option<String>,
) -> Result<(), String> {
    let target = {
        let p = PathBuf::from(path.trim());
        if p.exists() {
            p
        } else if let Ok(guard) = state.active_pack.lock()
            && let Some(pack) = guard.as_ref()
            && let Some(root) = &pack.source_path
        {
            let joined = root.join(path.trim());
            if joined.exists() { joined } else { root.clone() }
        } else {
            p
        }
    };

    let dir = if target.is_dir() {
        target.as_path()
    } else {
        target.parent().unwrap_or(target.as_path())
    };
    let detected = detect_file_managers().primary;
    let bin = manager_override
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&detected);

    Command::new(bin)
        .arg(dir)
        .spawn()
        .map_err(|e| format!("failed launching '{bin}' on '{}': {e}", dir.display()))?;
    Ok(())
}

/// Tauri Command: Read structured `pack.mcmeta`, `manifest.json`, or linked `BP/manifest.json` + `RP/manifest.json` metadata + icon from active pack.
#[tauri::command]
pub fn get_pack_metadata(state: tauri::State<'_, StudioState>) -> Result<PackMetadataDto, String> {
    let guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_ref()
        .ok_or_else(|| String::from("No resource pack currently open"))?;

    let (bp_prefix, rp_prefix) = if pack.pack_type == PackType::BedrockAddon {
        find_addon_subpack_prefixes(&pack.container)
    } else {
        (None, None)
    };

    let icon_bytes = match pack.pack_type {
        PackType::JavaResourcePack => pack
            .container
            .get("pack.png")
            .or_else(|| pack.container.get("pack_icon.png")),
        PackType::BedrockResourcePack => pack
            .container
            .get("pack_icon.png")
            .or_else(|| pack.container.get("pack.png")),
        PackType::BedrockAddon => rp_prefix
            .as_deref()
            .and_then(|p| pack.container.get(&format!("{p}pack_icon.png")))
            .or_else(|| {
                bp_prefix
                    .as_deref()
                    .and_then(|p| pack.container.get(&format!("{p}pack_icon.png")))
            })
            .or_else(|| pack.container.get("pack_icon.png")),
    };

    let mut dto = PackMetadataDto {
        edition: pack.edition,
        pack_type: pack.pack_type,
        name: pack.pack_name.clone(),
        description: String::new(),
        authors: vec![String::from("PackStudio Artist")],
        license: String::from("All Rights Reserved"),
        url: String::new(),
        version: [1, 0, 0],
        java_pack_format: 34,
        java_supported_formats: Some([15, 64]),
        bedrock_format_version: 2,
        bedrock_min_engine_version: [1, 21, 50],
        bedrock_header_uuid: Uuid::new_v4().to_string(),
        bedrock_module_uuid: Uuid::new_v4().to_string(),
        bedrock_capabilities: Vec::new(),
        bp_header_uuid: None,
        bp_module_uuid: None,
        bp_script_module_uuid: None,
        bp_script_entry: None,
        icon_base64_png: icon_bytes.map(|b| BASE64_STANDARD.encode(b)),
    };

    match pack.pack_type {
        PackType::JavaResourcePack => {
            if let Some(bytes) = pack.container.get("pack.mcmeta") {
                if let Ok(mcmeta) = parse_minecraft_json::<JavaPackMcmeta>(bytes)
                    && let Ok((fmt, range, desc)) = mcmeta.validate_and_extract()
                {
                    dto.java_pack_format = fmt;
                    dto.java_supported_formats = range;
                    dto.description = desc;
                }
                if let Ok(raw) = parse_minecraft_json::<serde_json::Value>(bytes)
                    && let Some(meta) = raw.get("meta").and_then(|v| v.as_object())
                {
                    if let Some(n) = meta.get("name").and_then(|v| v.as_str()) {
                        dto.name = n.to_string();
                    }
                    if let Some(arr) = meta.get("authors").and_then(|v| v.as_array()) {
                        let parsed: Vec<String> = arr
                            .iter()
                            .filter_map(|v| v.as_str().map(ToString::to_string))
                            .collect();
                        if !parsed.is_empty() {
                            dto.authors = parsed;
                        }
                    }
                    if let Some(l) = meta.get("license").and_then(|v| v.as_str()) {
                        dto.license = l.to_string();
                    }
                    if let Some(u) = meta.get("url").and_then(|v| v.as_str()) {
                        dto.url = u.to_string();
                    }
                }
            }
        }
        PackType::BedrockResourcePack => {
            if let Some(bytes) = pack.container.get("manifest.json") {
                populate_bedrock_manifest_into_dto(bytes, &mut dto, false);
            }
        }
        PackType::BedrockAddon => {
            if let Some(bp_bytes) = bp_prefix
                .as_deref()
                .and_then(|p| pack.container.get(&format!("{p}manifest.json")))
            {
                populate_bedrock_manifest_into_dto(bp_bytes, &mut dto, true);
            }
            if let Some(rp_bytes) = rp_prefix
                .as_deref()
                .and_then(|p| pack.container.get(&format!("{p}manifest.json")))
            {
                populate_bedrock_manifest_into_dto(rp_bytes, &mut dto, false);
            }
        }
    }

    Ok(dto)
}

fn populate_bedrock_manifest_into_dto(bytes: &[u8], dto: &mut PackMetadataDto, is_bp: bool) {
    if let Ok(manifest) = parse_minecraft_json::<BedrockManifest>(bytes) {
        dto.bedrock_format_version = manifest.format_version.clamp(1, 3);
        let cleaned_name = if is_bp {
            manifest
                .header
                .name
                .trim()
                .strip_suffix("(Behavior Pack)")
                .map(str::trim)
                .unwrap_or(manifest.header.name.trim())
                .to_string()
        } else {
            manifest.header.name
        };
        if !cleaned_name.is_empty() {
            dto.name = cleaned_name;
        }
        dto.description = manifest.header.description;
        dto.version = manifest.header.version;
        dto.bedrock_min_engine_version =
            normalize_bedrock_engine_triplet(manifest.header.min_engine_version);
        if dto.bedrock_min_engine_version[0] != 1 {
            dto.bedrock_min_engine_version[0] = 1;
        }
        if is_bp {
            dto.bp_header_uuid = Some(manifest.header.uuid.to_string());
        } else {
            dto.bedrock_header_uuid = manifest.header.uuid.to_string();
            if let Some(first_mod) = manifest.modules.first() {
                dto.bedrock_module_uuid = first_mod.uuid.to_string();
            }
            dto.bedrock_capabilities = manifest.capabilities;
        }
        if let Some(m) = manifest.metadata
            && !m.authors.is_empty()
        {
            dto.authors = m.authors;
        }
    }
    if let Ok(raw) = parse_minecraft_json::<serde_json::Value>(bytes) {
        if is_bp && let Some(mods) = raw.get("modules").and_then(serde_json::Value::as_array) {
            for m in mods {
                let mtype = m
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let muuid = m
                    .get("uuid")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string);
                if mtype.eq_ignore_ascii_case("data") && dto.bp_module_uuid.is_none() {
                    dto.bp_module_uuid = muuid;
                } else if mtype.eq_ignore_ascii_case("script") {
                    dto.bp_script_module_uuid = muuid;
                    dto.bp_script_entry = m
                        .get("entry")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string);
                }
            }
        }
        if let Some(meta) = raw.get("metadata").and_then(|v| v.as_object()) {
            if let Some(l) = meta.get("license").and_then(|v| v.as_str()) {
                dto.license = l.to_string();
            }
            if let Some(u) = meta.get("url").and_then(|v| v.as_str()) {
                dto.url = u.to_string();
            }
        }
    }
}

/// Tauri Command: Persist metadata edits back to `pack.mcmeta`, `manifest.json`, or linked `BP/manifest.json` + `RP/manifest.json`.
#[tauri::command]
pub fn save_pack_metadata(
    state: tauri::State<'_, StudioState>,
    mut metadata: PackMetadataDto,
) -> Result<ProjectSnapshot, String> {
    let mut guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_mut()
        .ok_or_else(|| String::from("No resource pack currently open"))?;

    match pack.pack_type {
        PackType::JavaResourcePack => {
            let fmt = metadata.java_pack_format.clamp(1, 200);
            let mut pack_obj = serde_json::Map::new();
            pack_obj.insert(String::from("pack_format"), json!(fmt));
            pack_obj.insert(String::from("description"), json!(metadata.description));
            if let Some([lo, hi]) = metadata.java_supported_formats {
                pack_obj.insert(
                    String::from("supported_formats"),
                    json!([lo.min(hi).max(1), lo.max(hi).max(1)]),
                );
            }
            let root = json!({
                "pack": serde_json::Value::Object(pack_obj),
                "meta": {
                    "name": metadata.name,
                    "version": format!("{}.{}.{}", metadata.version[0], metadata.version[1], metadata.version[2]),
                    "authors": metadata.authors,
                    "license": metadata.license,
                    "url": metadata.url,
                }
            });
            let bytes = serde_json::to_vec_pretty(&root).map_err(|e| format!("{e:#}"))?;
            pack.write_asset("pack.mcmeta", bytes)
                .map_err(|e| format!("{e:#}"))?;
            let _ = JavaPackMcmeta {
                pack: JavaPackSection {
                    pack_format: Some(fmt),
                    description: json!(metadata.description),
                    supported_formats: None,
                    min_format: None,
                    max_format: None,
                },
            };
            pack.pack_name = metadata.name;
            pack.edition_label = format!("Java (fmt {fmt})");
        }
        PackType::BedrockResourcePack => {
            metadata.bedrock_min_engine_version =
                normalize_bedrock_engine_triplet(metadata.bedrock_min_engine_version);
            if metadata.bedrock_min_engine_version[0] != 1 {
                metadata.bedrock_min_engine_version[0] = 1;
            }
            let h_uuid = Uuid::parse_str(metadata.bedrock_header_uuid.trim())
                .unwrap_or_else(|_| Uuid::new_v4());
            let mut m_uuid = Uuid::parse_str(metadata.bedrock_module_uuid.trim())
                .unwrap_or_else(|_| Uuid::new_v4());
            if m_uuid == h_uuid {
                m_uuid = Uuid::new_v4();
            }

            let mut manifest = BedrockManifest::new(
                metadata.name.clone(),
                metadata.description.clone(),
                metadata.version,
                metadata.bedrock_min_engine_version,
            );
            manifest.format_version = metadata.bedrock_format_version.clamp(1, 3);
            manifest.header.uuid = h_uuid;
            if let Some(m) = manifest.modules.first_mut() {
                m.uuid = m_uuid;
            }
            manifest.capabilities = metadata.bedrock_capabilities;
            let authors = if metadata.authors.is_empty() {
                vec![String::from("PackStudio Artist")]
            } else {
                metadata.authors
            };
            manifest.metadata = Some(BedrockManifestMetadata {
                authors: authors.clone(),
            });

            let base = manifest
                .to_json_vec_versioned(manifest.format_version)
                .map_err(|e| format!("{e:#}"))?;
            let mut val: serde_json::Value =
                serde_json::from_slice(&base).map_err(|e| format!("{e:#}"))?;
            if let Some(obj) = val.as_object_mut() {
                let mut meta = serde_json::Map::new();
                meta.insert(String::from("authors"), json!(authors));
                if !metadata.license.trim().is_empty() {
                    meta.insert(String::from("license"), json!(metadata.license.trim()));
                }
                if !metadata.url.trim().is_empty() {
                    meta.insert(String::from("url"), json!(metadata.url.trim()));
                }
                obj.insert(String::from("metadata"), serde_json::Value::Object(meta));
            }
            let bytes = serde_json::to_vec_pretty(&val).map_err(|e| format!("{e:#}"))?;
            pack.write_asset("manifest.json", bytes)
                .map_err(|e| format!("{e:#}"))?;
            let v = metadata.bedrock_min_engine_version;
            pack.pack_name = metadata.name;
            pack.edition_label = format!("Bedrock {}.{}.{}", v[0], v[1], v[2]);
        }
        PackType::BedrockAddon => {
            metadata.bedrock_min_engine_version =
                normalize_bedrock_engine_triplet(metadata.bedrock_min_engine_version);
            if metadata.bedrock_min_engine_version[0] != 1 {
                metadata.bedrock_min_engine_version[0] = 1;
            }
            let (bp_prefix, rp_prefix) = find_addon_subpack_prefixes(&pack.container);

            let rp_h_uuid = Uuid::parse_str(metadata.bedrock_header_uuid.trim())
                .unwrap_or_else(|_| Uuid::new_v4());
            let mut rp_m_uuid = Uuid::parse_str(metadata.bedrock_module_uuid.trim())
                .unwrap_or_else(|_| Uuid::new_v4());
            if rp_m_uuid == rp_h_uuid {
                rp_m_uuid = Uuid::new_v4();
            }

            let mut bp_h_uuid = metadata
                .bp_header_uuid
                .as_deref()
                .and_then(|s| Uuid::parse_str(s.trim()).ok())
                .unwrap_or_else(Uuid::new_v4);
            while bp_h_uuid == rp_h_uuid || bp_h_uuid == rp_m_uuid {
                bp_h_uuid = Uuid::new_v4();
            }
            let mut bp_m_uuid = metadata
                .bp_module_uuid
                .as_deref()
                .and_then(|s| Uuid::parse_str(s.trim()).ok())
                .unwrap_or_else(Uuid::new_v4);
            while bp_m_uuid == bp_h_uuid || bp_m_uuid == rp_h_uuid || bp_m_uuid == rp_m_uuid {
                bp_m_uuid = Uuid::new_v4();
            }

            let authors = if metadata.authors.is_empty() {
                vec![String::from("PackStudio Artist")]
            } else {
                metadata.authors.clone()
            };
            let mut meta_obj = serde_json::Map::new();
            meta_obj.insert(String::from("authors"), json!(authors));
            if !metadata.license.trim().is_empty() {
                meta_obj.insert(String::from("license"), json!(metadata.license.trim()));
            }
            if !metadata.url.trim().is_empty() {
                meta_obj.insert(String::from("url"), json!(metadata.url.trim()));
            }

            // 1. Update RP/manifest.json (if RP sub-pack exists)
            if let Some(rp_pre) = rp_prefix.as_deref() {
                let rp_manifest_path = format!("{rp_pre}manifest.json");
                let mut manifest = BedrockManifest::new(
                    metadata.name.clone(),
                    metadata.description.clone(),
                    metadata.version,
                    metadata.bedrock_min_engine_version,
                );
                manifest.format_version = metadata.bedrock_format_version.clamp(1, 3);
                manifest.header.uuid = rp_h_uuid;
                if let Some(m) = manifest.modules.first_mut() {
                    m.uuid = rp_m_uuid;
                }
                manifest.capabilities = metadata.bedrock_capabilities.clone();
                manifest.metadata = Some(BedrockManifestMetadata {
                    authors: authors.clone(),
                });

                let base = manifest
                    .to_json_vec_versioned(manifest.format_version)
                    .map_err(|e| format!("{e:#}"))?;
                let mut val: serde_json::Value =
                    serde_json::from_slice(&base).map_err(|e| format!("{e:#}"))?;
                if let Some(obj) = val.as_object_mut() {
                    obj.insert(
                        String::from("metadata"),
                        serde_json::Value::Object(meta_obj.clone()),
                    );
                    if bp_prefix.is_some() {
                        obj.insert(
                            String::from("dependencies"),
                            json!([{ "uuid": bp_h_uuid.to_string(), "version": metadata.version }]),
                        );
                    }
                }
                let bytes = serde_json::to_vec_pretty(&val).map_err(|e| format!("{e:#}"))?;
                pack.write_asset(&rp_manifest_path, bytes)
                    .map_err(|e| format!("{e:#}"))?;
            }

            // 2. Update BP/manifest.json (if BP sub-pack exists), preserving script modules & @minecraft/* deps
            if let Some(bp_pre) = bp_prefix.as_deref() {
                let bp_manifest_path = format!("{bp_pre}manifest.json");
                let mut bp_val: serde_json::Value = pack
                    .container
                    .get(&bp_manifest_path)
                    .and_then(|b| parse_minecraft_json::<serde_json::Value>(b).ok())
                    .unwrap_or_else(|| json!({ "format_version": 2 }));

                let Some(obj) = bp_val.as_object_mut() else {
                    return Err(String::from("BP/manifest.json root is not a JSON object"));
                };
                obj.insert(
                    String::from("format_version"),
                    json!(metadata.bedrock_format_version.clamp(1, 3)),
                );
                obj.insert(
                    String::from("header"),
                    json!({
                        "name": format!("{} (Behavior Pack)", metadata.name.trim()),
                        "description": metadata.description,
                        "uuid": bp_h_uuid.to_string(),
                        "version": metadata.version,
                        "min_engine_version": metadata.bedrock_min_engine_version,
                    }),
                );

                // Rebuild modules while preserving existing script module or metadata script fields
                let mut modules = vec![json!({
                    "description": metadata.description,
                    "type": "data",
                    "uuid": bp_m_uuid.to_string(),
                    "version": metadata.version,
                })];
                let existing_script = obj
                    .get("modules")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|arr| {
                        arr.iter().find(|m| {
                            m.get("type")
                                .and_then(serde_json::Value::as_str)
                                .is_some_and(|t| t.eq_ignore_ascii_case("script"))
                        })
                    })
                    .cloned();
                if let Some(mut script_mod) = existing_script {
                    if let Some(sobj) = script_mod.as_object_mut() {
                        if let Some(suuid) = metadata
                            .bp_script_module_uuid
                            .as_deref()
                            .and_then(|s| Uuid::parse_str(s.trim()).ok())
                        {
                            sobj.insert(String::from("uuid"), json!(suuid.to_string()));
                        }
                        if let Some(entry) = metadata
                            .bp_script_entry
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            sobj.insert(String::from("entry"), json!(entry));
                        }
                        sobj.insert(String::from("version"), json!(metadata.version));
                    }
                    modules.push(script_mod);
                }
                obj.insert(String::from("modules"), serde_json::Value::Array(modules));

                // Preserve module_name dependencies (e.g. @minecraft/server) and sync RP UUID dependency
                let mut deps: Vec<serde_json::Value> = obj
                    .get("dependencies")
                    .and_then(serde_json::Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter(|d| d.get("module_name").is_some())
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                if rp_prefix.is_some() {
                    deps.insert(
                        0,
                        json!({
                            "uuid": rp_h_uuid.to_string(),
                            "version": metadata.version,
                        }),
                    );
                }
                obj.insert(String::from("dependencies"), serde_json::Value::Array(deps));
                obj.insert(
                    String::from("metadata"),
                    serde_json::Value::Object(meta_obj),
                );

                let bytes = serde_json::to_vec_pretty(&bp_val).map_err(|e| format!("{e:#}"))?;
                pack.write_asset(&bp_manifest_path, bytes)
                    .map_err(|e| format!("{e:#}"))?;
            }

            let v = metadata.bedrock_min_engine_version;
            pack.pack_name = metadata.name;
            pack.edition_label = format!("Bedrock Add-on {}.{}.{}", v[0], v[1], v[2]);
        }
    }

    let (tex_count, dom_res) = compute_dominant_resolution(&pack.container);
    Ok(pack.to_snapshot(dom_res, tex_count))
}
