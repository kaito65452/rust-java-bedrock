use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use egui::{Color32, RichText, Ui};
use image::{Rgba, RgbaImage};
use rp_bedrock::schema::{BedrockManifest, BedrockManifestMetadata};
use rp_core::{Edition, normalize_bedrock_engine_triplet};
use serde_json::json;
use uuid::Uuid;

use crate::artist::{TextureFormatKind, encode_texture_bytes, resize_nearest_neighbor};
use crate::explorer::sanitize_minecraft_slug;

/// Supported project templates in the New Project Wizard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProjectTemplateKind {
    #[default]
    JavaResourcePack,
    BedrockResourcePack,
    BedrockAddonWorkspace,
}

impl ProjectTemplateKind {
    /// All selectable templates in display order.
    pub const ALL: [Self; 3] = [
        Self::JavaResourcePack,
        Self::BedrockResourcePack,
        Self::BedrockAddonWorkspace,
    ];

    /// Human-readable title shown in the Wizard selector.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::JavaResourcePack => "Minecraft: Java Edition (Resource Pack)",
            Self::BedrockResourcePack => "Minecraft: Bedrock Edition (Resource Pack)",
            Self::BedrockAddonWorkspace => "Minecraft: Bedrock Edition (Add-On: RP + BP)",
        }
    }

    /// Short description of the generated folder layout.
    #[must_use]
    pub const fn subtitle(self) -> &'static str {
        match self {
            Self::JavaResourcePack => "pack.mcmeta, pack.png, assets/<namespace>/{textures,models,blockstates,lang}",
            Self::BedrockResourcePack => "manifest.json (UUIDv4, [1,x,x]), pack_icon.png, textures/, models/entity/, texts/",
            Self::BedrockAddonWorkspace => "Linked RP/ + BP/ folders with matching UUID dependencies ready for .mcaddon",
        }
    }

    /// Corresponding Minecraft edition.
    #[must_use]
    pub const fn edition(self) -> Edition {
        match self {
            Self::JavaResourcePack => Edition::Java,
            Self::BedrockResourcePack | Self::BedrockAddonWorkspace => Edition::Bedrock,
        }
    }
}

/// Interactive configuration state for the New Project Wizard.
#[derive(Debug, Clone)]
pub struct NewProjectWizardState {
    pub open: bool,
    pub kind: ProjectTemplateKind,
    pub pack_name: String,
    pub description: String,
    pub authors: String,
    pub license: String,
    pub url: String,
    pub version: [u32; 3],
    pub java_pack_format: u32,
    pub java_use_supported_formats: bool,
    pub java_supported_min: u32,
    pub java_supported_max: u32,
    pub bedrock_min_engine_version: [u32; 3],
    pub java_namespace: String,
    pub icon_path: Option<PathBuf>,
    pub target_parent_dir: String,
    pub error_message: Option<String>,
}

impl Default for NewProjectWizardState {
    fn default() -> Self {
        let default_dir = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("PackStudioProjects");
        Self {
            open: false,
            kind: ProjectTemplateKind::JavaResourcePack,
            pack_name: String::from("My Resource Pack"),
            description: String::from("§bCustom textures §7created with §aPackStudio"),
            authors: String::from("Resource Pack Artist"),
            license: String::from("All Rights Reserved"),
            url: String::new(),
            version: [1, 0, 0],
            java_pack_format: 34,
            java_use_supported_formats: true,
            java_supported_min: 15,
            java_supported_max: 64,
            bedrock_min_engine_version: [1, 21, 0],
            java_namespace: String::from("minecraft"),
            icon_path: None,
            target_parent_dir: default_dir.display().to_string(),
            error_message: None,
        }
    }
}

impl NewProjectWizardState {
    /// Parses the comma-separated `authors` field into a clean non-empty vector.
    #[must_use]
    pub fn parsed_authors(&self) -> Vec<String> {
        self.authors
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect()
    }

    /// Returns the sanitized Java namespace (defaults to `"minecraft"`).
    #[must_use]
    pub fn sanitized_namespace(&self) -> String {
        let ns = sanitize_minecraft_slug(&self.java_namespace);
        if ns.is_empty() {
            String::from("minecraft")
        } else {
            ns
        }
    }
}

/// Result returned after successfully scaffolding a new project on disk.
#[derive(Debug, Clone)]
pub struct ScaffoldResult {
    /// Root directory of the created project (or `RP/` for Add-On workspace to open in PackStudio).
    pub open_dir: PathBuf,
    /// Workspace root directory (same as `open_dir` for single packs, parent of `RP/` & `BP/` for Add-On).
    pub workspace_root: PathBuf,
    /// Companion Behavior Pack directory when `ProjectTemplateKind::BedrockAddonWorkspace` was created.
    pub companion_bp_dir: Option<PathBuf>,
}

/// Scaffolds a new Java RP, Bedrock RP, or Bedrock Add-On (RP + BP) workspace on disk.
pub fn scaffold_new_project(cfg: &NewProjectWizardState) -> Result<ScaffoldResult> {
    let pack_name = cfg.pack_name.trim();
    if pack_name.is_empty() {
        bail!("Pack Name must not be empty");
    }
    let parent = PathBuf::from(cfg.target_parent_dir.trim());
    if cfg.target_parent_dir.trim().is_empty() {
        bail!("Project location folder must not be empty");
    }

    let folder_slug = sanitize_minecraft_slug(pack_name);
    let folder_name = if folder_slug.is_empty() {
        String::from("new_resource_pack")
    } else {
        folder_slug
    };
    let project_root = parent.join(&folder_name);
    fs::create_dir_all(&project_root).with_context(|| {
        format!(
            "failed to create project directory '{}'",
            project_root.display()
        )
    })?;

    let icon_png = load_or_generate_icon(cfg.icon_path.as_deref(), cfg.kind)?;
    let stone_png = generate_starter_stone_png()?;
    let stick_png = generate_starter_item_png()?;
    let authors = cfg.parsed_authors();

    match cfg.kind {
        ProjectTemplateKind::JavaResourcePack => {
            scaffold_java_rp(&project_root, cfg, &authors, &icon_png, &stone_png)?;
            Ok(ScaffoldResult {
                open_dir: project_root.clone(),
                workspace_root: project_root,
                companion_bp_dir: None,
            })
        }
        ProjectTemplateKind::BedrockResourcePack => {
            scaffold_bedrock_rp(
                &project_root,
                cfg,
                &authors,
                &icon_png,
                &stone_png,
                &stick_png,
                None,
            )?;
            Ok(ScaffoldResult {
                open_dir: project_root.clone(),
                workspace_root: project_root,
                companion_bp_dir: None,
            })
        }
        ProjectTemplateKind::BedrockAddonWorkspace => {
            let rp_dir = project_root.join("RP");
            let bp_dir = project_root.join("BP");
            fs::create_dir_all(&rp_dir)?;
            fs::create_dir_all(&bp_dir)?;

            let rp_header_uuid = Uuid::new_v4();
            let mut bp_header_uuid = Uuid::new_v4();
            while bp_header_uuid == rp_header_uuid {
                bp_header_uuid = Uuid::new_v4();
            }

            scaffold_bedrock_rp(
                &rp_dir,
                cfg,
                &authors,
                &icon_png,
                &stone_png,
                &stick_png,
                Some((rp_header_uuid, bp_header_uuid)),
            )?;
            scaffold_bedrock_bp(&bp_dir, cfg, &authors, &icon_png, bp_header_uuid, rp_header_uuid)?;

            Ok(ScaffoldResult {
                open_dir: rp_dir,
                workspace_root: project_root,
                companion_bp_dir: Some(bp_dir),
            })
        }
    }
}

fn scaffold_java_rp(
    root: &Path,
    cfg: &NewProjectWizardState,
    authors: &[String],
    icon_png: &[u8],
    stone_png: &[u8],
) -> Result<()> {
    let ns = cfg.sanitized_namespace();
    let dirs = [
        format!("assets/{ns}/textures/block"),
        format!("assets/{ns}/textures/item"),
        format!("assets/{ns}/textures/entity"),
        format!("assets/{ns}/textures/gui"),
        format!("assets/{ns}/textures/mob_effect"),
        format!("assets/{ns}/models/block"),
        format!("assets/{ns}/models/item"),
        format!("assets/{ns}/blockstates"),
        format!("assets/{ns}/lang"),
    ];
    for d in &dirs {
        fs::create_dir_all(root.join(d))?;
    }

    let fmt = cfg.java_pack_format.clamp(1, 200);
    let mut pack_obj = serde_json::Map::new();
    pack_obj.insert(String::from("pack_format"), json!(fmt));
    pack_obj.insert(String::from("description"), json!(cfg.description));
    if cfg.java_use_supported_formats {
        let lo = cfg.java_supported_min.min(cfg.java_supported_max).max(1);
        let hi = cfg.java_supported_min.max(cfg.java_supported_max).max(lo);
        pack_obj.insert(String::from("supported_formats"), json!([lo, hi]));
    }

    let mut mcmeta = serde_json::Map::new();
    mcmeta.insert(String::from("pack"), serde_json::Value::Object(pack_obj));
    if !authors.is_empty() || !cfg.license.trim().is_empty() || !cfg.url.trim().is_empty() {
        mcmeta.insert(
            String::from("meta"),
            json!({
                "name": cfg.pack_name.trim(),
                "version": format!("{}.{}.{}", cfg.version[0], cfg.version[1], cfg.version[2]),
                "authors": authors,
                "license": cfg.license.trim(),
                "url": cfg.url.trim(),
            }),
        );
    }

    fs::write(
        root.join("pack.mcmeta"),
        serde_json::to_vec_pretty(&serde_json::Value::Object(mcmeta))?,
    )?;
    fs::write(root.join("pack.png"), icon_png)?;
    fs::write(
        root.join(format!("assets/{ns}/textures/block/stone.png")),
        stone_png,
    )?;
    fs::write(
        root.join(format!("assets/{ns}/lang/en_us.json")),
        serde_json::to_vec_pretty(&json!({}))?,
    )?;
    Ok(())
}

fn scaffold_bedrock_rp(
    root: &Path,
    cfg: &NewProjectWizardState,
    authors: &[String],
    icon_png: &[u8],
    stone_png: &[u8],
    stick_png: &[u8],
    linked_uuids: Option<(Uuid, Uuid)>,
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
    if let Some((rp_uuid, _)) = linked_uuids {
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
        let mut meta_obj = serde_json::Map::new();
        meta_obj.insert(String::from("authors"), json!(final_authors));
        if !cfg.license.trim().is_empty() {
            meta_obj.insert(String::from("license"), json!(cfg.license.trim()));
        }
        if !cfg.url.trim().is_empty() {
            meta_obj.insert(String::from("url"), json!(cfg.url.trim()));
        }
        obj.insert(String::from("metadata"), serde_json::Value::Object(meta_obj));

        if let Some((_, bp_uuid)) = linked_uuids {
            obj.insert(
                String::from("dependencies"),
                json!([
                    {
                        "uuid": bp_uuid.to_string(),
                        "version": cfg.version
                    }
                ]),
            );
        }
    }

    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest_val)?,
    )?;
    fs::write(root.join("pack_icon.png"), icon_png)?;
    fs::write(root.join("textures/blocks/stone.png"), stone_png)?;
    fs::write(root.join("textures/items/stick.png"), stick_png)?;

    let terrain_json = json!({
        "resource_pack_name": cfg.pack_name.trim(),
        "texture_name": "atlas.terrain",
        "padding": 8,
        "num_mip_levels": 4,
        "texture_data": {
            "stone": {
                "textures": "textures/blocks/stone"
            }
        }
    });
    fs::write(
        root.join("textures/terrain_texture.json"),
        serde_json::to_vec_pretty(&terrain_json)?,
    )?;

    let item_json = json!({
        "resource_pack_name": cfg.pack_name.trim(),
        "texture_name": "atlas.items",
        "texture_data": {
            "stick": {
                "textures": "textures/items/stick"
            }
        }
    });
    fs::write(
        root.join("textures/item_texture.json"),
        serde_json::to_vec_pretty(&item_json)?,
    )?;

    fs::write(root.join("texts/languages.json"), serde_json::to_vec_pretty(&json!(["en_US"]))?)?;
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

fn scaffold_bedrock_bp(
    root: &Path,
    cfg: &NewProjectWizardState,
    authors: &[String],
    icon_png: &[u8],
    bp_header_uuid: Uuid,
    rp_header_uuid: Uuid,
) -> Result<()> {
    for d in ["blocks", "items", "entities", "texts"] {
        fs::create_dir_all(root.join(d))?;
    }

    let mut min_ver = normalize_bedrock_engine_triplet(cfg.bedrock_min_engine_version);
    min_ver[0] = 1;

    let mut bp_mod_uuid = Uuid::new_v4();
    while bp_mod_uuid == bp_header_uuid || bp_mod_uuid == rp_header_uuid {
        bp_mod_uuid = Uuid::new_v4();
    }

    let final_authors = if authors.is_empty() {
        vec![String::from("PackStudio Artist")]
    } else {
        authors.to_vec()
    };

    let mut meta_obj = serde_json::Map::new();
    meta_obj.insert(String::from("authors"), json!(final_authors));
    if !cfg.license.trim().is_empty() {
        meta_obj.insert(String::from("license"), json!(cfg.license.trim()));
    }
    if !cfg.url.trim().is_empty() {
        meta_obj.insert(String::from("url"), json!(cfg.url.trim()));
    }

    let bp_manifest = json!({
        "format_version": 2,
        "header": {
            "name": format!("{} (Behavior Pack)", cfg.pack_name.trim()),
            "description": cfg.description,
            "uuid": bp_header_uuid.to_string(),
            "version": cfg.version,
            "min_engine_version": min_ver
        },
        "modules": [
            {
                "description": cfg.description,
                "type": "data",
                "uuid": bp_mod_uuid.to_string(),
                "version": cfg.version
            }
        ],
        "dependencies": [
            {
                "uuid": rp_header_uuid.to_string(),
                "version": cfg.version
            }
        ],
        "metadata": meta_obj
    });

    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&bp_manifest)?,
    )?;
    fs::write(root.join("pack_icon.png"), icon_png)?;
    Ok(())
}

fn load_or_generate_icon(icon_path: Option<&Path>, kind: ProjectTemplateKind) -> Result<Vec<u8>> {
    if let Some(path) = icon_path
        && path.is_file()
    {
        let dyn_img = image::open(path)
            .with_context(|| format!("failed opening custom pack icon '{}'", path.display()))?;
        let rgba = dyn_img.into_rgba8();
        let final_img = if rgba.width() > 256 || rgba.height() > 256 {
            resize_nearest_neighbor(&rgba, 128, 128)?
        } else {
            rgba
        };
        return encode_texture_bytes(&final_img, TextureFormatKind::Png);
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

/// Styled span parsed from Minecraft `§`-formatted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McFormattedSpan {
    pub text: String,
    pub color: Color32,
    pub bold: bool,
    pub italic: bool,
}

/// Minecraft `§` color palette (`§0`..`§f`).
pub const MC_COLOR_BUTTONS: &[(&str, Color32, &str)] = &[
    ("§0", Color32::from_rgb(0x11, 0x11, 0x11), "Black"),
    ("§1", Color32::from_rgb(0x00, 0x00, 0xAA), "Dark Blue"),
    ("§2", Color32::from_rgb(0x00, 0xAA, 0x00), "Dark Green"),
    ("§3", Color32::from_rgb(0x00, 0xAA, 0xAA), "Dark Aqua"),
    ("§4", Color32::from_rgb(0xAA, 0x00, 0x00), "Dark Red"),
    ("§5", Color32::from_rgb(0xAA, 0x00, 0xAA), "Dark Purple"),
    ("§6", Color32::from_rgb(0xFF, 0xAA, 0x00), "Gold"),
    ("§7", Color32::from_rgb(0xAA, 0xAA, 0xAA), "Gray"),
    ("§8", Color32::from_rgb(0x55, 0x55, 0x55), "Dark Gray"),
    ("§9", Color32::from_rgb(0x55, 0x55, 0xFF), "Blue"),
    ("§a", Color32::from_rgb(0x55, 0xFF, 0x55), "Green"),
    ("§b", Color32::from_rgb(0x55, 0xFF, 0xFF), "Aqua"),
    ("§c", Color32::from_rgb(0xFF, 0x55, 0x55), "Red"),
    ("§d", Color32::from_rgb(0xFF, 0x55, 0xFF), "Light Purple"),
    ("§e", Color32::from_rgb(0xFF, 0xFF, 0x55), "Yellow"),
    ("§f", Color32::from_rgb(0xFF, 0xFF, 0xFF), "White"),
];

/// Parses Minecraft `§` formatting codes into colored/styled spans.
#[must_use]
pub fn parse_minecraft_formatted_text(input: &str) -> Vec<McFormattedSpan> {
    let default_col = Color32::from_rgb(0xD4, 0xD4, 0xD4);
    let mut color = default_col;
    let mut bold = false;
    let mut italic = false;
    let mut spans = Vec::new();
    let mut current = String::new();

    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '§'
            && let Some(&code) = chars.peek()
        {
            let lower = code.to_ascii_lowercase();
            let handled = match lower {
                '0' => {
                    color = Color32::from_rgb(0x33, 0x33, 0x33);
                    true
                }
                '1' => {
                    color = Color32::from_rgb(0x3B, 0x59, 0xD9);
                    true
                }
                '2' => {
                    color = Color32::from_rgb(0x00, 0xAA, 0x00);
                    true
                }
                '3' => {
                    color = Color32::from_rgb(0x00, 0xAA, 0xAA);
                    true
                }
                '4' => {
                    color = Color32::from_rgb(0xCC, 0x22, 0x22);
                    true
                }
                '5' => {
                    color = Color32::from_rgb(0xAA, 0x00, 0xAA);
                    true
                }
                '6' => {
                    color = Color32::from_rgb(0xFF, 0xAA, 0x00);
                    true
                }
                '7' => {
                    color = Color32::from_rgb(0xAA, 0xAA, 0xAA);
                    true
                }
                '8' => {
                    color = Color32::from_rgb(0x66, 0x66, 0x66);
                    true
                }
                '9' => {
                    color = Color32::from_rgb(0x55, 0x55, 0xFF);
                    true
                }
                'a' => {
                    color = Color32::from_rgb(0x55, 0xFF, 0x55);
                    true
                }
                'b' => {
                    color = Color32::from_rgb(0x55, 0xFF, 0xFF);
                    true
                }
                'c' => {
                    color = Color32::from_rgb(0xFF, 0x55, 0x55);
                    true
                }
                'd' => {
                    color = Color32::from_rgb(0xFF, 0x55, 0xFF);
                    true
                }
                'e' => {
                    color = Color32::from_rgb(0xFF, 0xFF, 0x55);
                    true
                }
                'f' => {
                    color = Color32::WHITE;
                    true
                }
                'l' => {
                    bold = true;
                    true
                }
                'o' => {
                    italic = true;
                    true
                }
                'r' => {
                    color = default_col;
                    bold = false;
                    italic = false;
                    true
                }
                _ => false,
            };

            if handled {
                if !current.is_empty() {
                    spans.push(McFormattedSpan {
                        text: std::mem::take(&mut current),
                        color,
                        bold,
                        italic,
                    });
                }
                let _ = chars.next();
                continue;
            }
        }
        current.push(ch);
    }

    if !current.is_empty() {
        spans.push(McFormattedSpan {
            text: current,
            color,
            bold,
            italic,
        });
    }
    spans
}

/// Renders a live Minecraft-styled preview of `text` (preserving `§` colors, bold, italic, newlines).
pub fn render_minecraft_formatted_preview(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(0x14, 0x14, 0x16))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(0x2D, 0x2D, 0x2D)))
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            for line in text.lines() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for span in parse_minecraft_formatted_text(line) {
                        let mut rt = RichText::new(span.text).color(span.color).size(12.0);
                        if span.bold {
                            rt = rt.strong();
                        }
                        if span.italic {
                            rt = rt.italics();
                        }
                        ui.label(rt);
                    }
                });
            }
        });
}

/// Renders a compact toolbar of Minecraft `§0`–`§f`, `§l`, `§o`, `§r` buttons that append to `target`.
pub fn render_mc_formatting_toolbar(ui: &mut Ui, target: &mut String) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for &(code, color, tip) in MC_COLOR_BUTTONS {
            let btn = egui::Button::new(RichText::new("■").color(color).size(11.0))
                .min_size(egui::vec2(16.0, 16.0));
            if ui.add(btn).on_hover_text(format!("{code} ({tip})")).clicked() {
                target.push_str(code);
            }
        }
        for (code, label, tip) in [
            ("§l", "B", "Bold (§l)"),
            ("§o", "I", "Italic (§o)"),
            ("§r", "Reset", "Reset (§r)"),
        ] {
            if ui.small_button(label).on_hover_text(tip).clicked() {
                target.push_str(code);
            }
        }
    });
}
