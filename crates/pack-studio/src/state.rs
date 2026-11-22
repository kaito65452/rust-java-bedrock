use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Context, Result};
use egui_dock::{DockState, Node, NodeIndex, SurfaceIndex, TabIndex};
use glam::Vec2;
use image::RgbaImage;
use rp_bedrock::schema::{BedrockManifest, BedrockManifestMetadata};
use rp_core::{
    AuditStatus, ConversionWarning, Edition, WarningSeverity, decode_minecraft_text,
    normalize_bedrock_engine_triplet, parse_minecraft_json,
};
use rp_java::schema::JavaPackMcmeta;
use serde_json::json;
use uuid::Uuid;

use crate::artist::{
    BiomeTintPreset, TextureFormatKind, TextureStats, decode_texture_bytes,
};
use crate::explorer::{ExplorerClipboard, ExplorerInlineEdit, PendingImportItem};
use crate::packager::{ExportPackageConfig, PackageFormat};
use crate::pipeline::{ConversionConfig, ConversionDirection, ConversionResult, PreflightReport};
use crate::wizard::NewProjectWizardState;
use crate::workspace::{LoadedPack, is_texture_asset};

/// Navigation destinations on the far-left VS Code Activity Bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ActivityTab {
    #[default]
    Explorer,
    Search,
    Converter,
    Metadata,
    Settings,
}

impl ActivityTab {
    /// All Activity Bar items in top-to-bottom order.
    pub const ALL: [Self; 5] = [
        Self::Explorer,
        Self::Search,
        Self::Converter,
        Self::Metadata,
        Self::Settings,
    ];

    /// Compact glyph rendered in the 48px vertical Activity Bar.
    #[must_use]
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Explorer => "📁",
            Self::Search => "🔍",
            Self::Converter => "⇄",
            Self::Metadata => "📦",
            Self::Settings => "⚙",
        }
    }

    /// Tooltip & sidebar header title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Explorer => "EXPLORER",
            Self::Search => "SEARCH",
            Self::Converter => "CONVERTER & PREFLIGHT",
            Self::Metadata => "METADATA & PACKAGING",
            Self::Settings => "SETTINGS",
        }
    }

    /// Hover tooltip including keyboard shortcut if bound.
    #[must_use]
    pub const fn tooltip(self) -> &'static str {
        match self {
            Self::Explorer => "Explorer (Ctrl+Shift+E)",
            Self::Search => "Search Assets (Ctrl+Shift+F)",
            Self::Converter => "Converter & Preflight",
            Self::Metadata => "Metadata & Packaging",
            Self::Settings => "Settings",
        }
    }
}

/// Tabs in the VS Code-style collapsible Bottom Panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BottomTab {
    #[default]
    Problems,
    Output,
}

/// Per-tab state for the Texture Inspector & Artist Canvas (`.png` / `.tga`).
#[derive(Debug, Clone)]
pub struct TextureTabState {
    pub path: String,
    pub zoom: f32,
    pub pan: Vec2,
    pub fit_to_view: bool,
    pub show_pixel_grid: bool,
    pub show_alpha_mask: bool,
    pub show_biome_tint: bool,
    pub biome_preset: BiomeTintPreset,
    pub downscale_target: u32,
    pub dirty: bool,
    pub revision: u64,
    pub decoded: Option<RgbaImage>,
    pub stats: Option<TextureStats>,
    pub load_error: Option<String>,
}

impl TextureTabState {
    /// Creates a new texture inspector tab for `path` and decodes its initial bytes if available.
    #[must_use]
    pub fn new(path: String, bytes: Option<&[u8]>, default_downscale: u32) -> Self {
        let mut state = Self {
            path,
            zoom: 8.0,
            pan: Vec2::ZERO,
            fit_to_view: true,
            show_pixel_grid: true,
            show_alpha_mask: false,
            show_biome_tint: false,
            biome_preset: BiomeTintPreset::Plains,
            downscale_target: default_downscale,
            dirty: false,
            revision: 1,
            decoded: None,
            stats: None,
            load_error: None,
        };
        if let Some(raw) = bytes {
            state.reload_from_bytes(raw);
        }
        state
    }

    /// Re-decodes raw PNG/TGA bytes and bumps `revision` so the GPU texture handle refreshes.
    pub fn reload_from_bytes(&mut self, bytes: &[u8]) {
        match decode_texture_bytes(bytes, &self.path) {
            Ok((img, fmt)) => {
                let stats = TextureStats::from_image(&img, fmt, bytes.len());
                self.decoded = Some(img);
                self.stats = Some(stats);
                self.load_error = None;
                self.revision = self.revision.saturating_add(1);
            }
            Err(err) => {
                self.decoded = None;
                self.stats = None;
                self.load_error = Some(format!("{err:#}"));
            }
        }
    }

    /// Replaces the decoded image in-place after an artist transform (downscale, etc.).
    pub fn set_transformed_image(&mut self, img: RgbaImage, fmt: TextureFormatKind, byte_len: usize) {
        self.stats = Some(TextureStats::from_image(&img, fmt, byte_len));
        self.decoded = Some(img);
        self.load_error = None;
        self.dirty = false;
        self.revision = self.revision.saturating_add(1);
    }

    /// Cache key for the GPU `egui::TextureHandle` including active preview modes.
    #[must_use]
    pub fn render_cache_key(&self) -> String {
        format!(
            "{}:rev={}:alpha={}:tint={}:biome={:?}",
            self.path,
            self.revision,
            self.show_alpha_mask,
            self.show_biome_tint,
            self.biome_preset
        )
    }
}

/// Per-tab state for the JSON / Code Editor (`.json`, `.mcmeta`, `.lang`, `.txt`).
#[derive(Debug, Clone)]
pub struct CodeTabState {
    pub path: String,
    pub text: String,
    pub saved_text: String,
    pub dirty: bool,
    pub json_error: Option<String>,
    pub is_json_like: bool,
}

impl CodeTabState {
    /// Creates a code editor tab initialized from raw bytes.
    #[must_use]
    pub fn new(path: String, bytes: Option<&[u8]>) -> Self {
        let lower = path.to_ascii_lowercase();
        let is_json_like = lower.ends_with(".json") || lower.ends_with(".mcmeta");
        let text = bytes
            .map(|b| decode_minecraft_text(b).into_owned())
            .unwrap_or_default();
        let mut state = Self {
            path,
            saved_text: text.clone(),
            text,
            dirty: false,
            json_error: None,
            is_json_like,
        };
        state.validate_syntax();
        state
    }

    /// Reloads editor text from external disk changes if not dirty, or updates saved snapshot.
    pub fn reload_from_bytes(&mut self, bytes: &[u8]) {
        let text = decode_minecraft_text(bytes).into_owned();
        self.saved_text = text.clone();
        self.text = text;
        self.dirty = false;
        self.validate_syntax();
    }

    /// Validates JSON syntax in real time for `.json` and `.mcmeta` buffers.
    pub fn validate_syntax(&mut self) {
        self.dirty = self.text != self.saved_text;
        if !self.is_json_like {
            self.json_error = None;
            return;
        }
        match serde_json::from_str::<serde_json::Value>(&self.text) {
            Ok(_) => self.json_error = None,
            Err(strict_err) => {
                if parse_minecraft_json::<serde_json::Value>(self.text.as_bytes()).is_ok() {
                    self.json_error = Some(format!(
                        "JSONC / non-strict JSON (line {}, col {}): {}",
                        strict_err.line(),
                        strict_err.column(),
                        strict_err
                    ));
                } else {
                    self.json_error = Some(format!(
                        "Syntax error at line {}, col {}: {}",
                        strict_err.line(),
                        strict_err.column(),
                        strict_err
                    ));
                }
            }
        }
    }

    /// Pretty-formats JSON text in place.
    pub fn format_json(&mut self) -> Result<()> {
        let value: serde_json::Value = parse_minecraft_json(self.text.as_bytes())
            .context("cannot format invalid JSON")?;
        self.text = serde_json::to_string_pretty(&value)?;
        self.validate_syntax();
        Ok(())
    }

    /// Marks the current text as saved.
    pub fn mark_saved(&mut self) {
        self.saved_text = self.text.clone();
        self.dirty = false;
    }
}

/// Dockable editor tabs rendered inside the central `egui_dock::DockArea`.
#[derive(Debug, Clone)]
pub enum EditorTab {
    Welcome,
    Texture(TextureTabState),
    Code(CodeTabState),
    ConversionDashboard,
}

impl EditorTab {
    /// Unique stable key used for deduplicating and locating tabs.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Welcome => String::from("__welcome__"),
            Self::Texture(t) => format!("tex:{}", t.path),
            Self::Code(c) => format!("code:{}", c.path),
            Self::ConversionDashboard => String::from("__converter__"),
        }
    }

    /// Display title shown in the VS Code-style tab bar.
    #[must_use]
    pub fn title(&self) -> String {
        match self {
            Self::Welcome => String::from("Welcome"),
            Self::Texture(t) => {
                let name = t.path.rsplit('/').next().unwrap_or(&t.path);
                if t.dirty {
                    format!("🎨 {name} ●")
                } else {
                    format!("🎨 {name}")
                }
            }
            Self::Code(c) => {
                let name = c.path.rsplit('/').next().unwrap_or(&c.path);
                if c.dirty {
                    format!("📄 {name} ●")
                } else {
                    format!("📄 {name}")
                }
            }
            Self::ConversionDashboard => String::from("⇄ Conversion & Preflight"),
        }
    }

    /// Returns the underlying pack-relative asset path if this tab represents a file.
    #[must_use]
    pub fn asset_path(&self) -> Option<&str> {
        match self {
            Self::Texture(t) => Some(t.path.as_str()),
            Self::Code(c) => Some(c.path.as_str()),
            Self::Welcome | Self::ConversionDashboard => None,
        }
    }
}

/// Flexible visual form state synced 2-way with Java `pack.mcmeta` and Bedrock `manifest.json`.
#[derive(Debug, Clone)]
pub struct PackMetadataForm {
    // Shared artist metadata
    pub authors_list: Vec<String>,
    pub new_author_input: String,
    pub license: String,
    pub url: String,
    pub icon_resize_target: Option<u32>,
    pub icon_revision: u64,

    // Java pack.mcmeta fields
    pub java_pack_name: String,
    pub java_pack_format: u32,
    pub java_use_supported_formats: bool,
    pub java_supported_min: u32,
    pub java_supported_max: u32,
    pub java_version: [u32; 3],
    pub java_description: String,

    // Bedrock manifest.json fields
    pub bedrock_format_version: u32,
    pub bedrock_name: String,
    pub bedrock_description: String,
    pub bedrock_version: [u32; 3],
    pub bedrock_min_engine_version: [u32; 3],
    pub bedrock_header_uuid: String,
    pub bedrock_module_uuid: String,
    pub bedrock_cap_pbr: bool,
    pub bedrock_cap_raytraced: bool,

    pub status_message: Option<String>,
}

impl Default for PackMetadataForm {
    fn default() -> Self {
        let header_uuid = Uuid::new_v4();
        let mut module_uuid = Uuid::new_v4();
        while module_uuid == header_uuid {
            module_uuid = Uuid::new_v4();
        }
        Self {
            authors_list: vec![String::from("PackStudio Artist")],
            new_author_input: String::new(),
            license: String::from("All Rights Reserved"),
            url: String::new(),
            icon_resize_target: Some(128),
            icon_revision: 1,
            java_pack_name: String::from("PackStudio Resource Pack"),
            java_pack_format: 34,
            java_use_supported_formats: true,
            java_supported_min: 15,
            java_supported_max: 64,
            java_version: [1, 0, 0],
            java_description: String::from("§bPackStudio §7Resource Pack"),
            bedrock_format_version: 2,
            bedrock_name: String::from("PackStudio Bedrock Pack"),
            bedrock_description: String::from("§aCreated with PackStudio"),
            bedrock_version: [1, 0, 0],
            bedrock_min_engine_version: [1, 21, 0],
            bedrock_header_uuid: header_uuid.to_string(),
            bedrock_module_uuid: module_uuid.to_string(),
            bedrock_cap_pbr: false,
            bedrock_cap_raytraced: false,
            status_message: None,
        }
    }
}

impl PackMetadataForm {
    /// Generates fresh distinct RFC-4122 UUIDv4 strings for both header and module.
    pub fn regenerate_uuids(&mut self) {
        let header = Uuid::new_v4();
        let mut module = Uuid::new_v4();
        while module == header {
            module = Uuid::new_v4();
        }
        self.bedrock_header_uuid = header.to_string();
        self.bedrock_module_uuid = module.to_string();
    }

    /// Guarantees `bedrock_min_engine_version` always satisfies Bedrock's `[1, minor, patch]` rule.
    pub fn enforce_bedrock_engine_version_invariant(&mut self) {
        self.bedrock_min_engine_version =
            normalize_bedrock_engine_triplet(self.bedrock_min_engine_version);
        if self.bedrock_min_engine_version[0] != 1 {
            self.bedrock_min_engine_version[0] = 1;
        }
    }

    /// Returns the active pack name & version for packaging templates.
    #[must_use]
    pub fn active_name_and_version(&self, edition: Edition) -> (String, [u32; 3]) {
        match edition {
            Edition::Java => (self.java_pack_name.clone(), self.java_version),
            Edition::Bedrock => (self.bedrock_name.clone(), self.bedrock_version),
        }
    }

    /// Loads current metadata values from the open pack's `pack.mcmeta` or `manifest.json`.
    pub fn load_from_pack(&mut self, pack: &LoadedPack) {
        self.java_pack_name = pack.pack_name.clone();
        if let Some(bytes) = pack.container.get("pack.mcmeta") {
            if let Ok(mcmeta) = parse_minecraft_json::<JavaPackMcmeta>(bytes)
                && let Ok((fmt, range, desc)) = mcmeta.validate_and_extract()
            {
                self.java_pack_format = fmt;
                self.java_description = desc;
                if let Some([lo, hi]) = range {
                    self.java_use_supported_formats = true;
                    self.java_supported_min = lo;
                    self.java_supported_max = hi;
                } else {
                    self.java_use_supported_formats = false;
                }
            }
            if let Ok(raw) = parse_minecraft_json::<serde_json::Value>(bytes)
                && let Some(meta) = raw.get("meta").and_then(|v| v.as_object())
            {
                if let Some(n) = meta.get("name").and_then(|v| v.as_str())
                    && !n.trim().is_empty()
                {
                    self.java_pack_name = n.to_string();
                }
                if let Some(arr) = meta.get("authors").and_then(|v| v.as_array()) {
                    let parsed: Vec<String> = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect();
                    if !parsed.is_empty() {
                        self.authors_list = parsed;
                    }
                }
                if let Some(lic) = meta.get("license").and_then(|v| v.as_str()) {
                    self.license = lic.to_string();
                }
                if let Some(url) = meta.get("url").and_then(|v| v.as_str()) {
                    self.url = url.to_string();
                }
            }
        }

        if let Some(bytes) = pack.container.get("manifest.json") {
            if let Ok(manifest) = parse_minecraft_json::<BedrockManifest>(bytes) {
                self.bedrock_format_version = manifest.format_version.clamp(1, 3);
                self.bedrock_name = manifest.header.name;
                self.bedrock_description = manifest.header.description;
                self.bedrock_version = manifest.header.version;
                self.bedrock_min_engine_version =
                    normalize_bedrock_engine_triplet(manifest.header.min_engine_version);
                self.bedrock_header_uuid = manifest.header.uuid.to_string();
                if let Some(first_mod) = manifest.modules.first() {
                    self.bedrock_module_uuid = first_mod.uuid.to_string();
                }
                self.bedrock_cap_pbr = manifest.capabilities.iter().any(|c| c == "pbr");
                self.bedrock_cap_raytraced = manifest.capabilities.iter().any(|c| c == "raytraced");
                if let Some(meta) = manifest.metadata
                    && !meta.authors.is_empty()
                {
                    self.authors_list = meta.authors;
                }
            }
            if let Ok(raw) = parse_minecraft_json::<serde_json::Value>(bytes)
                && let Some(meta) = raw.get("metadata").and_then(|v| v.as_object())
            {
                if let Some(lic) = meta.get("license").and_then(|v| v.as_str()) {
                    self.license = lic.to_string();
                }
                if let Some(url) = meta.get("url").and_then(|v| v.as_str()) {
                    self.url = url.to_string();
                }
            }
        }
        self.enforce_bedrock_engine_version_invariant();
        self.icon_revision = self.icon_revision.saturating_add(1);
    }

    /// Serializes the Java `pack.mcmeta` form into pretty JSON bytes (including optional `meta`).
    pub fn build_java_mcmeta_bytes(&self) -> Result<Vec<u8>> {
        let fmt = self.java_pack_format.clamp(1, 200);
        let mut pack_obj = serde_json::Map::new();
        pack_obj.insert(String::from("pack_format"), json!(fmt));
        pack_obj.insert(String::from("description"), json!(self.java_description));
        if self.java_use_supported_formats {
            let lo = self.java_supported_min.min(self.java_supported_max).max(1);
            let hi = self.java_supported_min.max(self.java_supported_max).max(lo);
            pack_obj.insert(String::from("supported_formats"), json!([lo, hi]));
        }

        let mut root = serde_json::Map::new();
        root.insert(String::from("pack"), serde_json::Value::Object(pack_obj));

        let authors: Vec<String> = self
            .authors_list
            .iter()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();
        if !authors.is_empty() || !self.license.trim().is_empty() || !self.url.trim().is_empty() {
            root.insert(
                String::from("meta"),
                json!({
                    "name": self.java_pack_name.trim(),
                    "version": format!("{}.{}.{}", self.java_version[0], self.java_version[1], self.java_version[2]),
                    "authors": authors,
                    "license": self.license.trim(),
                    "url": self.url.trim(),
                }),
            );
        }
        Ok(serde_json::to_vec_pretty(&serde_json::Value::Object(root))?)
    }

    /// Serializes the Bedrock `manifest.json` form into pretty JSON bytes while strictly
    /// guaranteeing `min_engine_version[0] == 1` and distinct RFC-4122 UUIDv4 values.
    pub fn build_bedrock_manifest_bytes(&mut self) -> Result<Vec<u8>> {
        self.enforce_bedrock_engine_version_invariant();
        let header_uuid = Uuid::parse_str(self.bedrock_header_uuid.trim())
            .unwrap_or_else(|_| Uuid::new_v4());
        let mut module_uuid = Uuid::parse_str(self.bedrock_module_uuid.trim())
            .unwrap_or_else(|_| Uuid::new_v4());
        if module_uuid == header_uuid {
            module_uuid = Uuid::new_v4();
        }
        self.bedrock_header_uuid = header_uuid.to_string();
        self.bedrock_module_uuid = module_uuid.to_string();

        let mut manifest = BedrockManifest::new(
            self.bedrock_name.clone(),
            self.bedrock_description.clone(),
            self.bedrock_version,
            self.bedrock_min_engine_version,
        );
        manifest.format_version = self.bedrock_format_version.clamp(1, 3);
        manifest.header.uuid = header_uuid;
        if let Some(m) = manifest.modules.first_mut() {
            m.uuid = module_uuid;
        }
        if self.bedrock_cap_pbr {
            manifest.capabilities.push(String::from("pbr"));
        }
        if self.bedrock_cap_raytraced {
            manifest.capabilities.push(String::from("raytraced"));
        }
        let authors: Vec<String> = self
            .authors_list
            .iter()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();
        let final_authors = if authors.is_empty() {
            vec![String::from("PackStudio Artist")]
        } else {
            authors
        };
        manifest.metadata = Some(BedrockManifestMetadata {
            authors: final_authors.clone(),
        });

        let base = manifest.to_json_vec_versioned(manifest.format_version)?;
        let mut val: serde_json::Value = serde_json::from_slice(&base)?;
        if let Some(obj) = val.as_object_mut() {
            let mut meta_obj = serde_json::Map::new();
            meta_obj.insert(String::from("authors"), json!(final_authors));
            if !self.license.trim().is_empty() {
                meta_obj.insert(String::from("license"), json!(self.license.trim()));
            }
            if !self.url.trim().is_empty() {
                meta_obj.insert(String::from("url"), json!(self.url.trim()));
            }
            obj.insert(String::from("metadata"), serde_json::Value::Object(meta_obj));
        }
        Ok(serde_json::to_vec_pretty(&val)?)
    }
}

/// Persistent studio settings configurable in the Settings sidebar tab.
#[derive(Debug, Clone)]
pub struct StudioSettings {
    pub blockbench_path: String,
    pub external_image_editor: String,
    pub convert_pbr: bool,
    pub default_downscale_size: u32,
    pub uv_atlas_guard: bool,
}

impl Default for StudioSettings {
    fn default() -> Self {
        Self {
            blockbench_path: String::from("blockbench"),
            external_image_editor: String::new(),
            convert_pbr: false,
            default_downscale_size: 64,
            uv_atlas_guard: true,
        }
    }
}

/// Timestamped entry in the Bottom Panel OUTPUT console.
#[derive(Debug, Clone)]
pub struct OutputLogEntry {
    pub seq: usize,
    pub level: WarningSeverity,
    pub message: String,
}

/// Core pure workbench state for PackStudio (UI-independent so it can be thoroughly unit-tested).
pub struct WorkbenchState {
    pub active_activity: ActivityTab,
    pub sidebar_visible: bool,
    pub bottom_panel_visible: bool,
    pub active_bottom_tab: BottomTab,
    pub dock_state: DockState<EditorTab>,
    pub loaded_pack: Option<LoadedPack>,
    /// Optional companion Bedrock Behavior Pack folder when an Add-On workspace (RP + BP) is open.
    pub companion_bp_dir: Option<PathBuf>,
    pub recent_projects: Vec<PathBuf>,
    pub search_query: String,
    pub problems_filter: String,
    pub problems: Vec<ConversionWarning>,
    pub output_logs: Vec<OutputLogEntry>,
    pub last_audit_status: Option<AuditStatus>,
    pub last_audit_summary: String,
    pub conversion_config: ConversionConfig,
    pub preflight_report: Option<PreflightReport>,
    pub last_conversion: Option<ConversionResult>,
    pub metadata_form: PackMetadataForm,
    pub settings: StudioSettings,
    pub wizard: NewProjectWizardState,
    pub export_config: ExportPackageConfig,
    // VS Code Explorer state
    pub explorer_filter: String,
    pub explorer_selected_dir: String,
    pub explorer_selected_item: Option<(String, bool)>,
    pub explorer_inline_edit: Option<ExplorerInlineEdit>,
    pub explorer_clipboard: Option<ExplorerClipboard>,
    pub explorer_extra_folders: BTreeSet<String>,
    pub explorer_collapse_epoch: u64,
    pub pending_imports: Vec<PendingImportItem>,
    // Command Palette (Ctrl+Shift+P / F1) & Quick Open File (Ctrl+P)
    pub command_palette_open: bool,
    pub command_palette_query: String,
    pub command_palette_selected: usize,
    pub quick_open_visible: bool,
    pub quick_open_query: String,
    pub quick_open_selected: usize,
    pub busy_label: Option<String>,
    pub watcher_status: String,
    next_log_seq: usize,
}

impl Default for WorkbenchState {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkbenchState {
    /// Initializes a fresh VS Code-style workbench with the Welcome tab open.
    #[must_use]
    pub fn new() -> Self {
        let mut state = Self {
            active_activity: ActivityTab::Explorer,
            sidebar_visible: true,
            bottom_panel_visible: true,
            active_bottom_tab: BottomTab::Problems,
            dock_state: DockState::new(vec![EditorTab::Welcome]),
            loaded_pack: None,
            companion_bp_dir: None,
            recent_projects: Vec::new(),
            search_query: String::new(),
            problems_filter: String::new(),
            problems: Vec::new(),
            output_logs: Vec::new(),
            last_audit_status: None,
            last_audit_summary: String::from("Not audited"),
            conversion_config: ConversionConfig::default(),
            preflight_report: None,
            last_conversion: None,
            metadata_form: PackMetadataForm::default(),
            settings: StudioSettings::default(),
            wizard: NewProjectWizardState::default(),
            export_config: ExportPackageConfig::default(),
            explorer_filter: String::new(),
            explorer_selected_dir: String::new(),
            explorer_selected_item: None,
            explorer_inline_edit: None,
            explorer_clipboard: None,
            explorer_extra_folders: BTreeSet::new(),
            explorer_collapse_epoch: 0,
            pending_imports: Vec::new(),
            command_palette_open: false,
            command_palette_query: String::new(),
            command_palette_selected: 0,
            quick_open_visible: false,
            quick_open_query: String::new(),
            quick_open_selected: 0,
            busy_label: None,
            watcher_status: String::from("Idle"),
            next_log_seq: 1,
        };
        state.log_output(
            WarningSeverity::Info,
            "PackStudio initialized — press Ctrl+Shift+P (Command Palette) or Ctrl+P (Quick Open).",
        );
        state
    }

    /// Appends a message to the Bottom Panel OUTPUT log.
    pub fn log_output(&mut self, level: WarningSeverity, message: impl Into<String>) {
        let seq = self.next_log_seq;
        self.next_log_seq = self.next_log_seq.saturating_add(1);
        self.output_logs.push(OutputLogEntry {
            seq,
            level,
            message: message.into(),
        });
        if self.output_logs.len() > 500 {
            let excess = self.output_logs.len() - 500;
            self.output_logs.drain(0..excess);
        }
    }

    /// Switches Activity Bar tab (clicking the active icon toggles sidebar visibility like VS Code).
    pub fn select_activity(&mut self, tab: ActivityTab) {
        if self.active_activity == tab && self.sidebar_visible {
            self.sidebar_visible = false;
        } else {
            self.active_activity = tab;
            self.sidebar_visible = true;
        }
    }

    /// Explicitly focuses a specific Activity Bar view and ensures the sidebar is open.
    pub fn focus_activity(&mut self, tab: ActivityTab) {
        self.active_activity = tab;
        self.sidebar_visible = true;
    }

    /// Toggles the Primary Side Bar (`Ctrl+B`).
    pub fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
    }

    /// Toggles the Bottom Panel (`Ctrl+J`).
    pub fn toggle_bottom_panel(&mut self) {
        self.bottom_panel_visible = !self.bottom_panel_visible;
    }

    /// Opens the Command Palette modal and clears the query.
    pub fn open_command_palette(&mut self) {
        self.quick_open_visible = false;
        self.command_palette_open = true;
        self.command_palette_query.clear();
        self.command_palette_selected = 0;
    }

    /// Closes the Command Palette modal.
    pub fn close_command_palette(&mut self) {
        self.command_palette_open = false;
    }

    /// Opens the Quick Open File (`Ctrl+P`) fuzzy jumper modal.
    pub fn open_quick_open(&mut self) {
        self.command_palette_open = false;
        self.quick_open_visible = true;
        self.quick_open_query.clear();
        self.quick_open_selected = 0;
    }

    /// Closes the Quick Open File modal.
    pub fn close_quick_open(&mut self) {
        self.quick_open_visible = false;
    }

    /// Records a recently opened pack path (deduplicated, newest first, capped at 10).
    pub fn push_recent_project(&mut self, path: PathBuf) {
        self.recent_projects.retain(|p| p != &path);
        self.recent_projects.insert(0, path);
        self.recent_projects.truncate(10);
    }

    /// Installs a newly loaded pack, updates metadata form, export format defaults, and conversion direction.
    pub fn set_loaded_pack(&mut self, pack: LoadedPack) {
        if let Some(p) = pack.source.disk_path() {
            self.push_recent_project(p.to_path_buf());
        }
        // Auto-detect companion BP folder if opened RP is inside an Add-On workspace (RP/ + BP/)
        self.companion_bp_dir = pack.source.directory_path().and_then(|dir| {
            let sibling_bp = dir.parent()?.join("BP");
            if dir.file_name().and_then(|n| n.to_str()) == Some("RP")
                && sibling_bp.join("manifest.json").is_file()
            {
                Some(sibling_bp)
            } else {
                None
            }
        });

        self.conversion_config.direction = match pack.edition {
            Edition::Java => ConversionDirection::JavaToBedrock,
            Edition::Bedrock => ConversionDirection::BedrockToJava,
        };
        self.conversion_config.target_version = match self.conversion_config.direction {
            ConversionDirection::JavaToBedrock => String::from("1.21.0"),
            ConversionDirection::BedrockToJava => String::from("1.21.4"),
        };
        self.export_config.format =
            PackageFormat::default_for(pack.edition, self.companion_bp_dir.is_some());
        self.metadata_form.load_from_pack(&pack);
        self.preflight_report = None;
        self.last_conversion = None;
        self.problems.clear();
        self.explorer_selected_dir.clear();
        self.explorer_selected_item = None;
        self.explorer_inline_edit = None;
        self.explorer_extra_folders.clear();
        self.last_audit_status = None;
        self.last_audit_summary = String::from("Pending audit");
        self.loaded_pack = Some(pack);
    }

    /// Opens an asset file in the central DockArea (focusing existing tab if already open).
    pub fn open_asset_tab(&mut self, rel_path: &str) {
        let target_key = if is_texture_asset(rel_path) {
            format!("tex:{rel_path}")
        } else {
            format!("code:{rel_path}")
        };
        if self.focus_tab_by_key(&target_key) {
            return;
        }
        let bytes = self
            .loaded_pack
            .as_ref()
            .and_then(|p| p.container.get(rel_path));
        let tab = if is_texture_asset(rel_path) {
            EditorTab::Texture(TextureTabState::new(
                rel_path.to_string(),
                bytes,
                self.settings.default_downscale_size,
            ))
        } else {
            EditorTab::Code(CodeTabState::new(rel_path.to_string(), bytes))
        };
        self.dock_state.push_to_focused_leaf(tab);
    }

    /// Opens an asset file in a split view to the right of the current editor leaf.
    pub fn open_asset_to_side(&mut self, rel_path: &str) {
        let bytes = self
            .loaded_pack
            .as_ref()
            .and_then(|p| p.container.get(rel_path));
        let tab = if is_texture_asset(rel_path) {
            EditorTab::Texture(TextureTabState::new(
                rel_path.to_string(),
                bytes,
                self.settings.default_downscale_size,
            ))
        } else {
            EditorTab::Code(CodeTabState::new(rel_path.to_string(), bytes))
        };
        let root_node = NodeIndex::root();
        let tree = self.dock_state.main_surface_mut();
        let _ = tree.split_right(root_node, 0.5, vec![tab]);
    }

    /// Opens or focuses the Conversion & Preflight Dashboard tab.
    pub fn open_conversion_dashboard(&mut self) {
        if self.focus_tab_by_key("__converter__") {
            return;
        }
        self.dock_state
            .push_to_focused_leaf(EditorTab::ConversionDashboard);
    }

    /// Opens or focuses the Welcome tab.
    pub fn open_welcome_tab(&mut self) {
        if self.focus_tab_by_key("__welcome__") {
            return;
        }
        self.dock_state.push_to_focused_leaf(EditorTab::Welcome);
    }

    fn find_tab_location(&self, key: &str) -> Option<(SurfaceIndex, NodeIndex, TabIndex)> {
        for (surf_idx, surface) in self.dock_state.iter_surfaces().enumerate() {
            let Some(tree) = surface.node_tree() else {
                continue;
            };
            for (node_idx, node) in tree.iter().enumerate() {
                if let Node::Leaf { tabs, .. } = node {
                    for (tab_idx, tab) in tabs.iter().enumerate() {
                        if tab.key() == key {
                            return Some((
                                SurfaceIndex(surf_idx),
                                NodeIndex(node_idx),
                                TabIndex(tab_idx),
                            ));
                        }
                    }
                }
            }
        }
        None
    }

    /// Finds an open tab by key and activates/focuses it. Returns `true` if found.
    pub fn focus_tab_by_key(&mut self, key: &str) -> bool {
        if let Some(loc) = self.find_tab_location(key) {
            self.dock_state.set_active_tab(loc);
            return true;
        }
        false
    }

    /// Closes an open tab matching `key`. Returns `true` if a tab was closed.
    pub fn close_tab_by_key(&mut self, key: &str) -> bool {
        if let Some(loc) = self.find_tab_location(key) {
            self.dock_state.remove_tab(loc);
            return true;
        }
        false
    }

    /// Counts total currently open tabs across all dock surfaces.
    #[must_use]
    pub fn open_tab_count(&self) -> usize {
        self.dock_state.iter_all_tabs().count()
    }

    /// Returns the asset path of the currently focused editor tab, if it is a file tab.
    #[must_use]
    pub fn active_asset_path(&mut self) -> Option<String> {
        self.dock_state
            .find_active_focused()
            .and_then(|(_, tab)| tab.asset_path().map(ToString::to_string))
    }

    /// Returns `(error_count, warning_count)` across current Problems diagnostics.
    #[must_use]
    pub fn problem_counts(&self) -> (usize, usize) {
        let mut errors = 0usize;
        let mut warnings = 0usize;
        for p in &self.problems {
            match p.severity {
                WarningSeverity::Error => errors += 1,
                WarningSeverity::Warning => warnings += 1,
                WarningSeverity::Info => {}
            }
        }
        (errors, warnings)
    }

    /// Replaces the Problems list, updates the Status Bar audit verdict, and refreshes per-file
    /// sidebar status badges (red dot for Error, yellow dot for Warning).
    pub fn apply_diagnostics(
        &mut self,
        status: AuditStatus,
        summary: String,
        diagnostics: Vec<ConversionWarning>,
    ) {
        self.last_audit_status = Some(status);
        self.last_audit_summary = summary;
        self.problems = diagnostics;

        if let Some(pack) = &mut self.loaded_pack {
            let mut map: BTreeMap<String, WarningSeverity> = BTreeMap::new();
            for diag in &self.problems {
                let entry = map
                    .entry(diag.source_path.clone())
                    .or_insert(diag.severity);
                if diag.severity > *entry {
                    *entry = diag.severity;
                }
            }
            pack.problem_severity_by_path = map;
        }
    }

    /// Reloads any open editor tabs whose path matches `rel_path` after a hot-reload or bridge save.
    pub fn hot_reload_asset(&mut self, rel_path: &str, bytes: &[u8]) {
        for (_, tab) in self.dock_state.iter_all_tabs_mut() {
            match tab {
                EditorTab::Texture(tex) if tex.path == rel_path => {
                    tex.reload_from_bytes(bytes);
                }
                EditorTab::Code(code) if code.path == rel_path && !code.dirty => {
                    code.reload_from_bytes(bytes);
                }
                _ => {}
            }
        }
    }

    /// Saves the currently focused Code Editor tab back to the pack container (and disk if folder).
    pub fn save_active_code_tab(&mut self) -> Result<Option<String>> {
        let target: Option<(String, String)> = self
            .dock_state
            .find_active_focused()
            .and_then(|(_, tab)| match tab {
                EditorTab::Code(code) => Some((code.path.clone(), code.text.clone())),
                _ => None,
            });

        let Some((path, text)) = target else {
            return Ok(None);
        };

        if let Some(pack) = &mut self.loaded_pack {
            pack.write_asset(&path, text.into_bytes())?;
            if path == "pack.mcmeta" || path == "manifest.json" {
                self.metadata_form.load_from_pack(pack);
            }
        }

        if let Some((_, EditorTab::Code(code))) = self.dock_state.find_active_focused() {
            code.mark_saved();
        }
        Ok(Some(path))
    }
}
