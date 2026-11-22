use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;

use rp_cli::{ConvertOptions, VersionSpecificOptions, convert_pack};
use rp_core::{
    BedrockGeoVersion, ConversionReport, Edition, MappingTable, ResourcePackIR, RotationSnapMode,
};
use rp_io::IoLimits;

use crate::preview::{render_png_bytes_halfblock, render_texture_halfblock};
use crate::scanner::{
    AuditOutcome, DiscoveredPack, PackDiff, PackSortKey, audit_pack_roundtrip, diff_packs,
    export_audit_json, export_report_json, load_pack_ir, repack_dir_to_archive,
    scan_directory_for_packs, sort_discovered_packs, suggest_output_path, unpack_pack_to_dir,
};

/// Active top-level tab in `rp-tui`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    /// Tab 1: Directory pack manager & scanner.
    Packs,
    /// Tab 2: Deep asset explorer + terminal texture preview + interactive 3D Braille viewport.
    Explorer,
    /// Tab 3: Interactive single-pack converter & live report viewer.
    Convert,
    /// Tab 4: Multi-pack batch converter & round-trip benchmark/audit runner.
    BatchAudit,
    /// Tab 5: Side-by-side canonical IR Pack Diff / Compare.
    Compare,
    /// Tab 6: Supported Minecraft version matrix & session logs.
    VersionsLogs,
}

impl ActiveTab {
    /// All tabs in visual order.
    pub const ALL: [Self; 6] = [
        Self::Packs,
        Self::Explorer,
        Self::Convert,
        Self::BatchAudit,
        Self::Compare,
        Self::VersionsLogs,
    ];

    /// Display title for tab bar.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Packs => "1:Packs",
            Self::Explorer => "2:Explorer",
            Self::Convert => "3:Convert",
            Self::BatchAudit => "4:Batch",
            Self::Compare => "5:Compare",
            Self::VersionsLogs => "6:Versions",
        }
    }

    /// Zero-based index for `ratatui::widgets::Tabs`.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Packs => 0,
            Self::Explorer => 1,
            Self::Convert => 2,
            Self::BatchAudit => 3,
            Self::Compare => 4,
            Self::VersionsLogs => 5,
        }
    }

    /// Next tab cyclically.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Packs => Self::Explorer,
            Self::Explorer => Self::Convert,
            Self::Convert => Self::BatchAudit,
            Self::BatchAudit => Self::Compare,
            Self::Compare => Self::VersionsLogs,
            Self::VersionsLogs => Self::Packs,
        }
    }

    /// Previous tab cyclically.
    #[must_use]
    pub const fn prev(self) -> Self {
        match self {
            Self::Packs => Self::VersionsLogs,
            Self::Explorer => Self::Packs,
            Self::Convert => Self::Explorer,
            Self::BatchAudit => Self::Convert,
            Self::Compare => Self::BatchAudit,
            Self::VersionsLogs => Self::Compare,
        }
    }
}

/// Asset category filter inside the Explorer tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetCategoryFilter {
    All,
    Textures,
    Models,
    Attachables,
    Sounds,
    Locales,
    Passthrough,
}

impl AssetCategoryFilter {
    /// Display name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Textures => "Textures",
            Self::Models => "Models",
            Self::Attachables => "Attachables",
            Self::Sounds => "Sounds",
            Self::Locales => "Locales",
            Self::Passthrough => "Passthrough",
        }
    }

    /// Cycles to the next category filter.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::All => Self::Textures,
            Self::Textures => Self::Models,
            Self::Models => Self::Attachables,
            Self::Attachables => Self::Sounds,
            Self::Sounds => Self::Locales,
            Self::Locales => Self::Passthrough,
            Self::Passthrough => Self::All,
        }
    }
}

/// Flattened explorer list entry referencing an item in the loaded `ResourcePackIR`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplorerItem {
    Texture(String),
    Model(String),
    Attachable(String),
    SoundFile(String),
    SoundEvent(String),
    Locale(String),
    Passthrough(String),
}

impl ExplorerItem {
    /// Short category badge for list rendering.
    #[must_use]
    pub const fn badge(&self) -> &'static str {
        match self {
            Self::Texture(_) => "TEX",
            Self::Model(_) => "3D ",
            Self::Attachable(_) => "ATT",
            Self::SoundFile(_) => "OGG",
            Self::SoundEvent(_) => "SFX",
            Self::Locale(_) => "LNG",
            Self::Passthrough(_) => "RAW",
        }
    }

    /// Item identifier string.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Texture(s)
            | Self::Model(s)
            | Self::Attachable(s)
            | Self::SoundFile(s)
            | Self::SoundEvent(s)
            | Self::Locale(s)
            | Self::Passthrough(s) => s.as_str(),
        }
    }
}

/// Loaded deep inspection state for the Explorer tab.
#[derive(Debug, Clone)]
pub struct LoadedPack {
    /// Source pack path.
    pub path: PathBuf,
    /// Detected edition.
    pub edition: Edition,
    /// Full parsed IR.
    pub ir: ResourcePackIR,
    /// Parse diagnostics.
    pub report: ConversionReport,
}

/// Interactive conversion form state for the Convert tab.
#[derive(Debug, Clone)]
pub struct ConvertForm {
    /// Focused form field index (`0..=8`).
    pub selected_field: usize,
    /// Source pack path.
    pub input_path: String,
    /// Output pack path.
    pub output_path: String,
    /// Target edition (`None` = Auto Flip).
    pub target_edition: Option<Edition>,
    /// Index into `target_versions` list.
    pub target_version_idx: usize,
    /// Available version strings (newest first).
    pub target_versions: Vec<String>,
    /// Rotation mode for Bedrock -> Java.
    pub rotation_mode: RotationSnapMode,
    /// Bedrock geometry version override (`None` = Auto).
    pub bedrock_geo_version: Option<BedrockGeoVersion>,
    /// Bedrock manifest format_version override (`None` = Auto).
    pub bedrock_manifest_version: Option<u32>,
    /// Java `items/*.json` emission override (`None` = Auto).
    pub emit_java_items: Option<bool>,
    /// Strict mode toggle.
    pub strict: bool,
}

impl ConvertForm {
    /// Total configurable form rows.
    pub const FIELD_COUNT: usize = 9;

    /// Builds a form populated with all versions from `mappings` (newest first, defaulting to `26.2`).
    #[must_use]
    pub fn new(mappings: &MappingTable) -> Self {
        let mut versions: Vec<String> = mappings
            .pack_format_versions()
            .iter()
            .rev()
            .map(|v| v.minecraft_version.clone())
            .collect();
        if versions.is_empty() {
            versions.push(String::from("26.3"));
            versions.push(String::from("26.2"));
        }
        let default_idx = versions.iter().position(|v| v == "26.2").unwrap_or(0);
        Self {
            selected_field: 0,
            input_path: String::new(),
            output_path: String::new(),
            target_edition: None,
            target_version_idx: default_idx,
            target_versions: versions,
            rotation_mode: RotationSnapMode::Preserve,
            bedrock_geo_version: None,
            bedrock_manifest_version: None,
            emit_java_items: None,
            strict: false,
        }
    }

    /// Currently selected target version string.
    #[must_use]
    pub fn selected_version(&self) -> Option<&str> {
        self.target_versions
            .get(self.target_version_idx)
            .map(String::as_str)
    }

    /// Populates input and suggested output path from a discovered pack.
    pub fn load_from_pack(&mut self, pack: &DiscoveredPack) {
        self.input_path = pack.path.display().to_string();
        let target = match &pack.summary {
            Ok(s) => match s.edition {
                Edition::Java => Edition::Bedrock,
                Edition::Bedrock => Edition::Java,
            },
            Err(_) => Edition::Java,
        };
        self.target_edition = Some(target);
        self.output_path = suggest_output_path(&pack.path, target);
    }

    /// Builds `VersionSpecificOptions` from the form values.
    #[must_use]
    pub fn version_options(&self) -> VersionSpecificOptions {
        VersionSpecificOptions {
            supported_formats: None,
            rotation_mode: self.rotation_mode,
            bedrock_geo_version: self.bedrock_geo_version,
            bedrock_manifest_version: self.bedrock_manifest_version,
            emit_java_items_definitions: self.emit_java_items,
        }
    }

    /// Cycles the currently focused enum/bool field.
    pub fn cycle_current_field(&mut self, forward: bool) {
        match self.selected_field {
            2 => {
                self.target_edition = match (self.target_edition, forward) {
                    (None, true) => Some(Edition::Java),
                    (Some(Edition::Java), true) => Some(Edition::Bedrock),
                    (Some(Edition::Bedrock), true) => None,
                    (None, false) => Some(Edition::Bedrock),
                    (Some(Edition::Bedrock), false) => Some(Edition::Java),
                    (Some(Edition::Java), false) => None,
                };
                if !self.input_path.is_empty()
                    && let Some(ed) = self.target_edition
                {
                    self.output_path = suggest_output_path(Path::new(&self.input_path), ed);
                }
            }
            3 => {
                let len = self.target_versions.len().max(1);
                if forward {
                    self.target_version_idx = (self.target_version_idx + 1) % len;
                } else {
                    self.target_version_idx = (self.target_version_idx + len - 1) % len;
                }
            }
            4 => {
                self.rotation_mode = match (self.rotation_mode, forward) {
                    (RotationSnapMode::Preserve, true) => RotationSnapMode::Snap22_5,
                    (RotationSnapMode::Snap22_5, true) => RotationSnapMode::Clamp45,
                    (RotationSnapMode::Clamp45, true) => RotationSnapMode::Preserve,
                    (RotationSnapMode::Preserve, false) => RotationSnapMode::Clamp45,
                    (RotationSnapMode::Clamp45, false) => RotationSnapMode::Snap22_5,
                    (RotationSnapMode::Snap22_5, false) => RotationSnapMode::Preserve,
                };
            }
            5 => {
                let opts = [
                    None,
                    Some(BedrockGeoVersion::V1_21_0),
                    Some(BedrockGeoVersion::V1_16_0),
                    Some(BedrockGeoVersion::V1_12_0),
                    Some(BedrockGeoVersion::V1_10_0),
                ];
                let pos = opts
                    .iter()
                    .position(|o| *o == self.bedrock_geo_version)
                    .unwrap_or(0);
                let next = if forward {
                    (pos + 1) % opts.len()
                } else {
                    (pos + opts.len() - 1) % opts.len()
                };
                self.bedrock_geo_version = opts[next];
            }
            6 => {
                let opts = [None, Some(3), Some(2), Some(1)];
                let pos = opts
                    .iter()
                    .position(|o| *o == self.bedrock_manifest_version)
                    .unwrap_or(0);
                let next = if forward {
                    (pos + 1) % opts.len()
                } else {
                    (pos + opts.len() - 1) % opts.len()
                };
                self.bedrock_manifest_version = opts[next];
            }
            7 => {
                let opts = [None, Some(true), Some(false)];
                let pos = opts
                    .iter()
                    .position(|o| *o == self.emit_java_items)
                    .unwrap_or(0);
                let next = if forward {
                    (pos + 1) % opts.len()
                } else {
                    (pos + opts.len() - 1) % opts.len()
                };
                self.emit_java_items = opts[next];
            }
            8 => {
                self.strict = !self.strict;
            }
            _ => {}
        }
    }
}

/// Active modal / inline text prompt overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    /// Typing a new scan directory path.
    EditScanDir(String),
    /// Typing a pack list filter query.
    FilterPacks(String),
    /// Typing an Explorer asset search query.
    FilterAssets(String),
    /// Editing `ConvertForm::input_path`.
    EditConvertInput(String),
    /// Editing `ConvertForm::output_path`.
    EditConvertOutput(String),
    /// Confirming deletion of a resource pack file or directory (`y`/`n`).
    ConfirmDelete {
        path: PathBuf,
        is_dir: bool,
    },
}

/// Result of the last single-pack conversion.
#[derive(Debug, Clone)]
pub struct ConversionResultState {
    pub source: Edition,
    pub target: Edition,
    pub output_path: PathBuf,
    pub report: ConversionReport,
    pub elapsed_ms: u128,
}

/// Messages sent from background worker threads back to the TUI event loop.
#[derive(Debug)]
pub enum WorkerMsg {
    ScanComplete {
        dir: PathBuf,
        result: Result<Vec<DiscoveredPack>, String>,
    },
    PackLoaded {
        result: Result<Box<LoadedPack>, String>,
    },
    ConvertComplete {
        result: Result<ConversionResultState, String>,
    },
    DiffComplete {
        result: Result<Box<PackDiff>, String>,
    },
    ArchiveOpComplete {
        result: Result<String, String>,
        rescan: bool,
    },
    BatchProgress {
        completed: usize,
        total: usize,
        label: String,
    },
    AuditEntry(AuditOutcome),
    BatchFinished {
        summary: String,
    },
}

/// Root TUI application state.
pub struct App {
    pub should_quit: bool,
    pub active_tab: ActiveTab,
    pub show_help: bool,
    pub tick_count: u64,

    pub scan_dir: PathBuf,
    pub mapping_file: Option<PathBuf>,
    pub mappings: MappingTable,
    pub limits: IoLimits,

    // Tab 1: Packs
    pub packs: Vec<DiscoveredPack>,
    pub pack_filter: String,
    pub pack_sort: PackSortKey,
    pub selected_pack_idx: usize,
    pub cached_icon_path: Option<PathBuf>,
    pub cached_icon_lines: Vec<Line<'static>>,

    // Tab 2: Explorer + 3D Braille Canvas
    pub loaded_pack: Option<LoadedPack>,
    pub explorer_category: AssetCategoryFilter,
    pub explorer_filter: String,
    pub explorer_items: Vec<ExplorerItem>,
    pub selected_asset_idx: usize,
    pub detail_scroll: u16,
    pub model_yaw_deg: f32,
    pub model_pitch_deg: f32,
    pub show_head_ref: bool,
    pub cached_preview_key: Option<String>,
    pub cached_preview_lines: Vec<Line<'static>>,

    // Tab 3: Convert
    pub convert_form: ConvertForm,
    pub last_conversion: Option<Result<ConversionResultState, String>>,
    pub report_scroll: u16,

    // Tab 4: Batch & Audit
    pub audit_outcomes: Vec<AuditOutcome>,
    pub selected_audit_idx: usize,
    pub batch_progress: Option<(usize, usize, String)>,

    // Tab 5: Compare / Diff
    pub pack_diff: Option<Result<PackDiff, String>>,
    pub diff_scroll: u16,

    // Tab 6: Versions & Logs
    pub version_scroll: usize,
    pub logs: Vec<String>,

    // Shared UI status & async channel
    pub input_mode: InputMode,
    pub busy_message: Option<String>,
    pub status_banner: String,
    tx: Sender<WorkerMsg>,
    rx: Receiver<WorkerMsg>,
}

impl App {
    /// Initializes a new TUI `App` rooted at `scan_dir`.
    #[must_use]
    pub fn new(scan_dir: PathBuf, mapping_file: Option<PathBuf>, mappings: MappingTable) -> Self {
        let (tx, rx) = mpsc::channel();
        let convert_form = ConvertForm::new(&mappings);
        let mut app = Self {
            should_quit: false,
            active_tab: ActiveTab::Packs,
            show_help: false,
            tick_count: 0,
            scan_dir: scan_dir.clone(),
            mapping_file,
            mappings,
            limits: IoLimits::default(),
            packs: Vec::new(),
            pack_filter: String::new(),
            pack_sort: PackSortKey::Name,
            selected_pack_idx: 0,
            cached_icon_path: None,
            cached_icon_lines: Vec::new(),
            loaded_pack: None,
            explorer_category: AssetCategoryFilter::All,
            explorer_filter: String::new(),
            explorer_items: Vec::new(),
            selected_asset_idx: 0,
            detail_scroll: 0,
            model_yaw_deg: 35.0,
            model_pitch_deg: 22.0,
            show_head_ref: true,
            cached_preview_key: None,
            cached_preview_lines: Vec::new(),
            convert_form,
            last_conversion: None,
            report_scroll: 0,
            audit_outcomes: Vec::new(),
            selected_audit_idx: 0,
            batch_progress: None,
            pack_diff: None,
            diff_scroll: 0,
            version_scroll: 0,
            logs: Vec::new(),
            input_mode: InputMode::Normal,
            busy_message: None,
            status_banner: format!("Ready | Directory: {}", scan_dir.display()),
            tx,
            rx,
        };
        app.push_log(format!("Started rp-tui in {}", app.scan_dir.display()));
        app
    }

    /// Appends a message to the session log ring buffer.
    pub fn push_log(&mut self, msg: impl Into<String>) {
        let text = msg.into();
        self.logs.push(text);
        if self.logs.len() > 200 {
            let excess = self.logs.len() - 200;
            self.logs.drain(0..excess);
        }
    }

    /// Returns filtered indices into `self.packs`.
    #[must_use]
    pub fn filtered_pack_indices(&self) -> Vec<usize> {
        let needle = self.pack_filter.trim().to_ascii_lowercase();
        self.packs
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                if needle.is_empty() {
                    return true;
                }
                if p.file_name.to_ascii_lowercase().contains(&needle) {
                    return true;
                }
                if let Ok(ref s) = p.summary {
                    return s.name.to_ascii_lowercase().contains(&needle)
                        || s.edition.to_string().to_ascii_lowercase().contains(&needle);
                }
                false
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    /// Returns the currently highlighted `DiscoveredPack` if any.
    #[must_use]
    pub fn current_pack(&self) -> Option<&DiscoveredPack> {
        let indices = self.filtered_pack_indices();
        let real_idx = *indices.get(self.selected_pack_idx)?;
        self.packs.get(real_idx)
    }

    /// Refreshes the cached half-block icon preview for the highlighted pack in `[1:Packs]`.
    pub fn refresh_pack_icon_cache(&mut self) {
        let Some(pack) = self.current_pack() else {
            self.cached_icon_path = None;
            self.cached_icon_lines.clear();
            return;
        };
        if self.cached_icon_path.as_ref() == Some(&pack.path) {
            return;
        }
        let path = pack.path.clone();
        let lines = pack
            .icon_png
            .as_deref()
            .map(|b| render_png_bytes_halfblock(b, 16, 8))
            .unwrap_or_default();
        self.cached_icon_path = Some(path);
        self.cached_icon_lines = lines;
    }

    /// Spawns a background scan of `self.scan_dir`.
    pub fn start_scan(&mut self) {
        if self.busy_message.is_some() {
            return;
        }
        let dir = self.scan_dir.clone();
        let mappings = self.mappings.clone();
        let limits = self.limits;
        let tx = self.tx.clone();
        self.busy_message = Some(format!("Scanning {}...", dir.display()));
        self.push_log(format!("Scanning directory: {}", dir.display()));

        thread::spawn(move || {
            let result =
                scan_directory_for_packs(&dir, &mappings, limits).map_err(|err| format!("{err:#}"));
            let _ = tx.send(WorkerMsg::ScanComplete { dir, result });
        });
    }

    /// Spawns a background deep IR load for `path` and switches to Explorer tab.
    pub fn start_load_explorer(&mut self, path: PathBuf) {
        if self.busy_message.is_some() {
            return;
        }
        let mappings = self.mappings.clone();
        let limits = self.limits;
        let tx = self.tx.clone();
        self.busy_message = Some(format!("Loading {}...", path.display()));
        self.active_tab = ActiveTab::Explorer;
        self.push_log(format!("Inspecting pack: {}", path.display()));

        thread::spawn(move || {
            let result = load_pack_ir(&path, &mappings, limits)
                .map(|(edition, ir, report)| {
                    Box::new(LoadedPack {
                        path,
                        edition,
                        ir,
                        report,
                    })
                })
                .map_err(|err| format!("{err:#}"));
            let _ = tx.send(WorkerMsg::PackLoaded { result });
        });
    }

    /// Spawns a background Pack Diff between `path_a` and `path_b` and opens Tab 5 (`Compare`).
    pub fn start_diff(&mut self, path_a: PathBuf, path_b: PathBuf) {
        if self.busy_message.is_some() {
            return;
        }
        let mappings = self.mappings.clone();
        let limits = self.limits;
        let tx = self.tx.clone();
        self.active_tab = ActiveTab::Compare;
        self.busy_message = Some(String::from("Comparing packs..."));
        self.push_log(format!(
            "Comparing '{}' vs '{}'",
            path_a.display(),
            path_b.display()
        ));

        thread::spawn(move || {
            let result = diff_packs(&path_a, &path_b, &mappings, limits)
                .map(Box::new)
                .map_err(|err| format!("{err:#}"));
            let _ = tx.send(WorkerMsg::DiffComplete { result });
        });
    }

    /// Spawns a background unpack or repack operation.
    pub fn start_unpack(&mut self, path: PathBuf) {
        if self.busy_message.is_some() {
            return;
        }
        let limits = self.limits;
        let tx = self.tx.clone();
        self.busy_message = Some(format!("Unpacking {}...", path.display()));
        thread::spawn(move || {
            let result = unpack_pack_to_dir(&path, limits)
                .map(|out| format!("Unpacked -> {}", out.display()))
                .map_err(|err| format!("Unpack failed: {err:#}"));
            let _ = tx.send(WorkerMsg::ArchiveOpComplete {
                result,
                rescan: true,
            });
        });
    }

    /// Spawns a background repack of an unpacked directory into `.zip`/`.mcpack`.
    pub fn start_repack(&mut self, path: PathBuf) {
        if self.busy_message.is_some() {
            return;
        }
        let limits = self.limits;
        let tx = self.tx.clone();
        self.busy_message = Some(format!("Repacking {}...", path.display()));
        thread::spawn(move || {
            let result = repack_dir_to_archive(&path, limits)
                .map(|out| format!("Repacked -> {}", out.display()))
                .map_err(|err| format!("Repack failed: {err:#}"));
            let _ = tx.send(WorkerMsg::ArchiveOpComplete {
                result,
                rescan: true,
            });
        });
    }

    /// Spawns a background single-pack conversion using `self.convert_form`.
    pub fn start_convert(&mut self) {
        if self.busy_message.is_some() {
            return;
        }
        if self.convert_form.input_path.trim().is_empty()
            || self.convert_form.output_path.trim().is_empty()
        {
            self.status_banner =
                String::from("Input and Output paths are required before converting");
            return;
        }

        let options = ConvertOptions {
            input: PathBuf::from(self.convert_form.input_path.trim()),
            output: PathBuf::from(self.convert_form.output_path.trim()),
            target: self.convert_form.target_edition,
            target_version: self.convert_form.selected_version().map(str::to_string),
            mapping_file: self.mapping_file.clone(),
            strict: self.convert_form.strict,
            report_json: None,
            limits: self.limits,
            version_options: self.convert_form.version_options(),
        };
        let out_path = options.output.clone();
        let tx = self.tx.clone();
        self.busy_message = Some(format!("Converting {}...", options.input.display()));
        self.push_log(format!(
            "Converting '{}' -> '{}' (target={:?}, ver={:?})",
            options.input.display(),
            options.output.display(),
            options.target,
            options.target_version
        ));

        thread::spawn(move || {
            let start = Instant::now();
            let result = convert_pack(&options)
                .map(|(source, target, report)| ConversionResultState {
                    source,
                    target,
                    output_path: out_path,
                    report,
                    elapsed_ms: start.elapsed().as_millis(),
                })
                .map_err(|err| format!("{err:#}"));
            let _ = tx.send(WorkerMsg::ConvertComplete { result });
        });
    }

    /// Spawns a background round-trip & integrity audit for `paths`.
    pub fn start_audit(&mut self, paths: Vec<PathBuf>) {
        if self.busy_message.is_some() || paths.is_empty() {
            return;
        }
        let total = paths.len();
        let mappings = self.mappings.clone();
        let limits = self.limits;
        let target_ver = self.convert_form.selected_version().map(str::to_string);
        let ver_opts = self.convert_form.version_options();
        let tx = self.tx.clone();

        self.active_tab = ActiveTab::BatchAudit;
        self.audit_outcomes.clear();
        self.selected_audit_idx = 0;
        self.batch_progress = Some((0, total, String::from("Starting audit...")));
        self.busy_message = Some(format!("Auditing {total} pack(s)..."));
        self.push_log(format!("Started bidirectional audit on {total} pack(s)"));

        thread::spawn(move || {
            let mut passed = 0usize;
            for (idx, path) in paths.into_iter().enumerate() {
                let label = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("pack")
                    .to_string();
                let _ = tx.send(WorkerMsg::BatchProgress {
                    completed: idx,
                    total,
                    label: format!("Auditing {label}"),
                });
                let outcome = audit_pack_roundtrip(
                    &path,
                    target_ver.as_deref(),
                    &ver_opts,
                    &mappings,
                    limits,
                );
                if outcome.passed {
                    passed += 1;
                }
                let _ = tx.send(WorkerMsg::AuditEntry(outcome));
            }
            let _ = tx.send(WorkerMsg::BatchProgress {
                completed: total,
                total,
                label: String::from("Audit complete"),
            });
            let _ = tx.send(WorkerMsg::BatchFinished {
                summary: format!("Audit complete: {passed}/{total} passed"),
            });
        });
    }

    /// Spawns a background batch conversion of all checked packs (or all packs if none checked).
    pub fn start_batch_convert(&mut self) {
        if self.busy_message.is_some() {
            return;
        }
        let targets = self.selected_or_all_pack_paths();
        if targets.is_empty() {
            self.status_banner = String::from("No resource packs found to batch convert");
            return;
        }
        let total = targets.len();
        let out_dir = self.scan_dir.join("converted");
        let target_ver = self.convert_form.selected_version().map(str::to_string);
        let ver_opts = self.convert_form.version_options();
        let mapping_file = self.mapping_file.clone();
        let strict = self.convert_form.strict;
        let limits = self.limits;
        let tx = self.tx.clone();

        self.active_tab = ActiveTab::BatchAudit;
        self.audit_outcomes.clear();
        self.selected_audit_idx = 0;
        self.batch_progress = Some((0, total, String::from("Starting batch convert...")));
        self.busy_message = Some(format!("Batch converting {total} pack(s)..."));
        self.push_log(format!(
            "Batch converting {total} pack(s) into {}",
            out_dir.display()
        ));

        thread::spawn(move || {
            let _ = std::fs::create_dir_all(&out_dir);
            let mut ok_count = 0usize;
            for (idx, (path, src_ed)) in targets.into_iter().enumerate() {
                let start = Instant::now();
                let pack_name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("pack")
                    .to_string();
                let target_ed = match src_ed {
                    Edition::Java => Edition::Bedrock,
                    Edition::Bedrock => Edition::Java,
                };
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("converted");
                let out_file = match target_ed {
                    Edition::Java => out_dir.join(format!("{stem}-java.zip")),
                    Edition::Bedrock => out_dir.join(format!("{stem}-bedrock.mcpack")),
                };

                let _ = tx.send(WorkerMsg::BatchProgress {
                    completed: idx,
                    total,
                    label: format!("Converting {pack_name}"),
                });

                let options = ConvertOptions {
                    input: path.clone(),
                    output: out_file.clone(),
                    target: Some(target_ed),
                    target_version: target_ver.clone(),
                    mapping_file: mapping_file.clone(),
                    strict,
                    report_json: None,
                    limits,
                    version_options: ver_opts.clone(),
                };

                let outcome = match convert_pack(&options) {
                    Ok((source, target, report)) => {
                        ok_count += 1;
                        AuditOutcome {
                            pack_name,
                            path,
                            source_edition: source,
                            target_edition: target,
                            passed: true,
                            forward_report: Some(report),
                            detail: format!("Saved -> {}", out_file.display()),
                            elapsed_ms: start.elapsed().as_millis(),
                        }
                    }
                    Err(err) => AuditOutcome {
                        pack_name,
                        path,
                        source_edition: src_ed,
                        target_edition: target_ed,
                        passed: false,
                        forward_report: None,
                        detail: format!("Error: {err:#}"),
                        elapsed_ms: start.elapsed().as_millis(),
                    },
                };
                let _ = tx.send(WorkerMsg::AuditEntry(outcome));
            }
            let _ = tx.send(WorkerMsg::BatchProgress {
                completed: total,
                total,
                label: String::from("Batch convert complete"),
            });
            let _ = tx.send(WorkerMsg::BatchFinished {
                summary: format!(
                    "Batch convert done: {ok_count}/{total} saved to {}",
                    out_dir.display()
                ),
            });
        });
    }

    fn selected_or_all_pack_paths(&self) -> Vec<(PathBuf, Edition)> {
        let checked: Vec<(PathBuf, Edition)> = self
            .packs
            .iter()
            .filter(|p| p.selected)
            .filter_map(|p| p.summary.as_ref().ok().map(|s| (p.path.clone(), s.edition)))
            .collect();
        if !checked.is_empty() {
            return checked;
        }
        self.packs
            .iter()
            .filter_map(|p| p.summary.as_ref().ok().map(|s| (p.path.clone(), s.edition)))
            .collect()
    }

    /// Rebuilds the Explorer tab item list from `self.loaded_pack` and active filters.
    pub fn rebuild_explorer_items(&mut self) {
        self.explorer_items.clear();
        let Some(loaded) = &self.loaded_pack else {
            return;
        };
        let needle = self.explorer_filter.trim().to_ascii_lowercase();
        let matches = |id: &str| needle.is_empty() || id.to_ascii_lowercase().contains(&needle);

        let cat = self.explorer_category;
        if matches!(
            cat,
            AssetCategoryFilter::All | AssetCategoryFilter::Textures
        ) {
            for id in loaded.ir.textures.keys() {
                if matches(id) {
                    self.explorer_items.push(ExplorerItem::Texture(id.clone()));
                }
            }
        }
        if matches!(cat, AssetCategoryFilter::All | AssetCategoryFilter::Models) {
            for id in loaded.ir.models.keys() {
                if matches(id) {
                    self.explorer_items.push(ExplorerItem::Model(id.clone()));
                }
            }
        }
        if matches!(
            cat,
            AssetCategoryFilter::All | AssetCategoryFilter::Attachables
        ) {
            for id in loaded.ir.attachables.keys() {
                if matches(id) {
                    self.explorer_items
                        .push(ExplorerItem::Attachable(id.clone()));
                }
            }
        }
        if matches!(cat, AssetCategoryFilter::All | AssetCategoryFilter::Sounds) {
            for id in loaded.ir.sounds.files.keys() {
                if matches(id) {
                    self.explorer_items
                        .push(ExplorerItem::SoundFile(id.clone()));
                }
            }
            for id in loaded.ir.sounds.events.keys() {
                if matches(id) {
                    self.explorer_items
                        .push(ExplorerItem::SoundEvent(id.clone()));
                }
            }
        }
        if matches!(cat, AssetCategoryFilter::All | AssetCategoryFilter::Locales) {
            for id in loaded.ir.locales.keys() {
                if matches(id) {
                    self.explorer_items.push(ExplorerItem::Locale(id.clone()));
                }
            }
        }
        if matches!(
            cat,
            AssetCategoryFilter::All | AssetCategoryFilter::Passthrough
        ) {
            for id in loaded.ir.passthrough_files.keys() {
                if matches(id) {
                    self.explorer_items
                        .push(ExplorerItem::Passthrough(id.clone()));
                }
            }
        }

        if self.selected_asset_idx >= self.explorer_items.len() {
            self.selected_asset_idx = self.explorer_items.len().saturating_sub(1);
        }
        self.detail_scroll = 0;
        self.refresh_texture_preview_cache();
    }

    /// Updates the cached ANSI half-block texture preview if a texture is currently selected.
    pub fn refresh_texture_preview_cache(&mut self) {
        let Some(ExplorerItem::Texture(tex_id)) = self.explorer_items.get(self.selected_asset_idx)
        else {
            self.cached_preview_key = None;
            self.cached_preview_lines.clear();
            return;
        };

        if self.cached_preview_key.as_deref() == Some(tex_id.as_str()) {
            return;
        }

        if let Some(loaded) = &self.loaded_pack
            && let Some(tex) = loaded.ir.textures.get(tex_id)
        {
            self.cached_preview_lines = render_texture_halfblock(tex, 40, 18);
            self.cached_preview_key = Some(tex_id.clone());
        } else {
            self.cached_preview_key = None;
            self.cached_preview_lines.clear();
        }
    }

    /// Drains any completed background worker messages and updates UI state.
    pub fn poll_worker(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);
        let mut need_rescan = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                WorkerMsg::ScanComplete { dir, result } => {
                    self.busy_message = None;
                    match result {
                        Ok(mut packs) => {
                            sort_discovered_packs(&mut packs, self.pack_sort);
                            let count = packs.len();
                            self.packs = packs;
                            self.selected_pack_idx = 0;
                            self.cached_icon_path = None;
                            self.refresh_pack_icon_cache();
                            if let Some(first) = self.packs.first()
                                && self.convert_form.input_path.is_empty()
                            {
                                self.convert_form.load_from_pack(first);
                            }
                            self.status_banner =
                                format!("Found {count} resource pack(s) in {}", dir.display());
                            self.push_log(self.status_banner.clone());
                        }
                        Err(err) => {
                            self.status_banner = format!("Scan error: {err}");
                            self.push_log(self.status_banner.clone());
                        }
                    }
                }
                WorkerMsg::PackLoaded { result } => {
                    self.busy_message = None;
                    match result {
                        Ok(loaded) => {
                            let loaded = *loaded;
                            self.status_banner = format!(
                                "Loaded {} ({}, {} textures, {} models)",
                                loaded.path.display(),
                                loaded.edition,
                                loaded.ir.textures.len(),
                                loaded.ir.models.len()
                            );
                            self.push_log(self.status_banner.clone());
                            self.loaded_pack = Some(loaded);
                            self.explorer_category = AssetCategoryFilter::All;
                            self.explorer_filter.clear();
                            self.selected_asset_idx = 0;
                            self.rebuild_explorer_items();
                        }
                        Err(err) => {
                            self.status_banner = format!("Inspect error: {err}");
                            self.push_log(self.status_banner.clone());
                        }
                    }
                }
                WorkerMsg::ConvertComplete { result } => {
                    self.busy_message = None;
                    self.report_scroll = 0;
                    match &result {
                        Ok(res) => {
                            self.status_banner = format!(
                                "Converted {} -> {} in {}ms ({} converted, {} lossy, {} warnings)",
                                res.source,
                                res.target,
                                res.elapsed_ms,
                                res.report.converted_count,
                                res.report.lossy_count,
                                res.report.warnings.len()
                            );
                            self.push_log(format!(
                                "OK: {} -> {}",
                                self.status_banner,
                                res.output_path.display()
                            ));
                        }
                        Err(err) => {
                            self.status_banner = format!("Conversion failed: {err}");
                            self.push_log(self.status_banner.clone());
                        }
                    }
                    self.last_conversion = Some(result);
                }
                WorkerMsg::DiffComplete { result } => {
                    self.busy_message = None;
                    self.diff_scroll = 0;
                    match result {
                        Ok(diff) => {
                            let d = *diff;
                            self.status_banner = format!(
                                "Diff: {} vs {} (shared tex={}, models={})",
                                d.name_a, d.name_b, d.textures.shared_count, d.models.shared_count
                            );
                            self.push_log(self.status_banner.clone());
                            self.pack_diff = Some(Ok(d));
                        }
                        Err(err) => {
                            self.status_banner = format!("Diff failed: {err}");
                            self.push_log(self.status_banner.clone());
                            self.pack_diff = Some(Err(err));
                        }
                    }
                }
                WorkerMsg::ArchiveOpComplete { result, rescan } => {
                    self.busy_message = None;
                    match result {
                        Ok(msg) => {
                            self.status_banner = msg.clone();
                            self.push_log(msg);
                            if rescan {
                                need_rescan = true;
                            }
                        }
                        Err(err) => {
                            self.status_banner = err.clone();
                            self.push_log(err);
                        }
                    }
                }
                WorkerMsg::BatchProgress {
                    completed,
                    total,
                    label,
                } => {
                    self.batch_progress = Some((completed, total, label));
                }
                WorkerMsg::AuditEntry(outcome) => {
                    let tag = if outcome.passed { "PASS" } else { "FAIL" };
                    self.push_log(format!(
                        "[{tag}] {} ({} -> {}, {}ms): {}",
                        outcome.pack_name,
                        outcome.source_edition,
                        outcome.target_edition,
                        outcome.elapsed_ms,
                        outcome.detail
                    ));
                    self.audit_outcomes.push(outcome);
                }
                WorkerMsg::BatchFinished { summary } => {
                    self.busy_message = None;
                    self.status_banner = summary.clone();
                    self.push_log(summary);
                }
            }
        }
        if need_rescan {
            self.start_scan();
        }
    }

    /// Handles mouse scroll wheel up/down cleanly in the active tab.
    pub fn handle_scroll(&mut self, scroll_down: bool) {
        let code = if scroll_down {
            KeyCode::Down
        } else {
            KeyCode::Up
        };
        self.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// Handles a keyboard event from `crossterm`.
    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        if self.input_mode != InputMode::Normal {
            self.handle_text_input_key(key);
            return;
        }

        if self.show_help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter
            ) {
                self.show_help = false;
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('?') | KeyCode::F(1) => {
                self.show_help = !self.show_help;
                return;
            }
            KeyCode::Char('1') => {
                self.active_tab = ActiveTab::Packs;
                return;
            }
            KeyCode::Char('2') => {
                self.active_tab = ActiveTab::Explorer;
                return;
            }
            KeyCode::Char('3') => {
                self.active_tab = ActiveTab::Convert;
                return;
            }
            KeyCode::Char('4') => {
                self.active_tab = ActiveTab::BatchAudit;
                return;
            }
            KeyCode::Char('5') => {
                self.active_tab = ActiveTab::Compare;
                return;
            }
            KeyCode::Char('6') => {
                self.active_tab = ActiveTab::VersionsLogs;
                return;
            }
            KeyCode::Tab => {
                self.active_tab = self.active_tab.next();
                return;
            }
            KeyCode::BackTab => {
                self.active_tab = self.active_tab.prev();
                return;
            }
            _ => {}
        }

        match self.active_tab {
            ActiveTab::Packs => self.handle_packs_key(key),
            ActiveTab::Explorer => self.handle_explorer_key(key),
            ActiveTab::Convert => self.handle_convert_key(key),
            ActiveTab::BatchAudit => self.handle_batch_key(key),
            ActiveTab::Compare => self.handle_compare_key(key),
            ActiveTab::VersionsLogs => self.handle_versions_key(key),
        }
    }

    fn handle_text_input_key(&mut self, key: KeyEvent) {
        let mut mode = std::mem::replace(&mut self.input_mode, InputMode::Normal);
        if let InputMode::ConfirmDelete { path, is_dir } = mode {
            match key.code {
                KeyCode::Char('y' | 'Y') => {
                    let res = if is_dir {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
                    match res {
                        Ok(()) => {
                            self.status_banner = format!("Deleted '{}'", path.display());
                            self.push_log(self.status_banner.clone());
                            self.start_scan();
                        }
                        Err(err) => {
                            self.status_banner = format!("Delete failed: {err}");
                            self.push_log(self.status_banner.clone());
                        }
                    }
                }
                _ => {
                    self.status_banner = String::from("Delete cancelled");
                }
            }
            return;
        }

        match key.code {
            KeyCode::Esc => {
                self.status_banner = String::from("Cancelled input");
            }
            KeyCode::Enter => match mode {
                InputMode::EditScanDir(buf) => {
                    let trimmed = buf.trim();
                    if !trimmed.is_empty() {
                        let new_dir = PathBuf::from(trimmed);
                        if new_dir.is_dir() {
                            self.scan_dir = new_dir;
                            self.start_scan();
                        } else {
                            self.status_banner = format!("Directory not found: {trimmed}");
                        }
                    }
                }
                InputMode::FilterPacks(buf) => {
                    self.pack_filter = buf;
                    self.selected_pack_idx = 0;
                    self.refresh_pack_icon_cache();
                }
                InputMode::FilterAssets(buf) => {
                    self.explorer_filter = buf;
                    self.selected_asset_idx = 0;
                    self.rebuild_explorer_items();
                }
                InputMode::EditConvertInput(buf) => {
                    self.convert_form.input_path = buf.trim().to_string();
                }
                InputMode::EditConvertOutput(buf) => {
                    self.convert_form.output_path = buf.trim().to_string();
                }
                InputMode::Normal | InputMode::ConfirmDelete { .. } => {}
            },
            KeyCode::Backspace => {
                if let Some(buf) = input_mode_buf_mut(&mut mode) {
                    buf.pop();
                }
                self.input_mode = mode;
            }
            KeyCode::Char(c) => {
                if let Some(buf) = input_mode_buf_mut(&mut mode) {
                    buf.push(c);
                }
                self.input_mode = mode;
            }
            _ => {
                self.input_mode = mode;
            }
        }
    }

    fn handle_packs_key(&mut self, key: KeyEvent) {
        let count = self.filtered_pack_indices().len();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                if count > 0 {
                    self.selected_pack_idx = (self.selected_pack_idx + 1).min(count - 1);
                    self.refresh_pack_icon_cache();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected_pack_idx = self.selected_pack_idx.saturating_sub(1);
                self.refresh_pack_icon_cache();
            }
            KeyCode::Char(' ') => {
                let indices = self.filtered_pack_indices();
                if let Some(&real_idx) = indices.get(self.selected_pack_idx)
                    && let Some(p) = self.packs.get_mut(real_idx)
                {
                    p.selected = !p.selected;
                }
            }
            KeyCode::Char('a') => {
                let any_unselected = self.packs.iter().any(|p| !p.selected);
                for p in &mut self.packs {
                    p.selected = any_unselected;
                }
            }
            KeyCode::Char('s') => {
                self.pack_sort = self.pack_sort.next();
                sort_discovered_packs(&mut self.packs, self.pack_sort);
                self.selected_pack_idx = 0;
                self.refresh_pack_icon_cache();
                self.status_banner = format!("Sorted packs by {}", self.pack_sort.label());
            }
            KeyCode::Enter | KeyCode::Char('i') => {
                if let Some(pack) = self.current_pack().cloned() {
                    self.start_load_explorer(pack.path);
                }
            }
            KeyCode::Char('c') => {
                if let Some(pack) = self.current_pack().cloned() {
                    self.convert_form.load_from_pack(&pack);
                    self.active_tab = ActiveTab::Convert;
                    self.status_banner = format!("Loaded '{}' into Converter", pack.file_name);
                }
            }
            KeyCode::Char('t') => {
                if let Some(pack) = self.current_pack().cloned() {
                    self.start_audit(vec![pack.path]);
                }
            }
            KeyCode::Char('=') => {
                let marked: Vec<PathBuf> = self
                    .packs
                    .iter()
                    .filter(|p| p.selected)
                    .map(|p| p.path.clone())
                    .collect();
                if marked.len() >= 2 {
                    self.start_diff(marked[0].clone(), marked[1].clone());
                } else if self.packs.len() >= 2 {
                    let a = self.packs[0].path.clone();
                    let b = self.packs[1].path.clone();
                    self.start_diff(a, b);
                } else {
                    self.status_banner =
                        String::from("Mark 2 packs with Space (or have >=2 packs) to Compare");
                }
            }
            KeyCode::Char('u') => {
                if let Some(pack) = self.current_pack().cloned() {
                    if pack.is_dir {
                        self.status_banner = String::from("Already an unpacked directory");
                    } else {
                        self.start_unpack(pack.path);
                    }
                }
            }
            KeyCode::Char('p') => {
                if let Some(pack) = self.current_pack().cloned() {
                    if !pack.is_dir {
                        self.status_banner =
                            String::from("Select an unpacked pack directory to repack");
                    } else {
                        self.start_repack(pack.path);
                    }
                }
            }
            KeyCode::Delete => {
                if let Some(pack) = self.current_pack().cloned() {
                    self.input_mode = InputMode::ConfirmDelete {
                        path: pack.path,
                        is_dir: pack.is_dir,
                    };
                }
            }
            KeyCode::Char('r') => self.start_scan(),
            KeyCode::Char('d') => {
                self.input_mode = InputMode::EditScanDir(self.scan_dir.display().to_string());
            }
            KeyCode::Char('/') => {
                self.input_mode = InputMode::FilterPacks(self.pack_filter.clone());
            }
            KeyCode::Esc => {
                self.pack_filter.clear();
                self.selected_pack_idx = 0;
                self.refresh_pack_icon_cache();
            }
            _ => {}
        }
    }

    fn handle_explorer_key(&mut self, key: KeyEvent) {
        let count = self.explorer_items.len();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                if count > 0 {
                    self.selected_asset_idx = (self.selected_asset_idx + 1).min(count - 1);
                    self.detail_scroll = 0;
                    self.refresh_texture_preview_cache();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected_asset_idx = self.selected_asset_idx.saturating_sub(1);
                self.detail_scroll = 0;
                self.refresh_texture_preview_cache();
            }
            KeyCode::PageDown | KeyCode::Char('J') => {
                self.detail_scroll = self.detail_scroll.saturating_add(5);
            }
            KeyCode::PageUp | KeyCode::Char('K') => {
                self.detail_scroll = self.detail_scroll.saturating_sub(5);
            }
            // Orbit 3D Braille model viewport
            KeyCode::Char('a' | '[') => {
                self.model_yaw_deg = (self.model_yaw_deg - 15.0).rem_euclid(360.0);
            }
            KeyCode::Char('d' | ']') => {
                self.model_yaw_deg = (self.model_yaw_deg + 15.0).rem_euclid(360.0);
            }
            KeyCode::Char('w') => {
                self.model_pitch_deg = (self.model_pitch_deg + 10.0).clamp(-85.0, 85.0);
            }
            KeyCode::Char('s') => {
                self.model_pitch_deg = (self.model_pitch_deg - 10.0).clamp(-85.0, 85.0);
            }
            KeyCode::Char('h') => {
                self.show_head_ref = !self.show_head_ref;
            }
            KeyCode::Char('f') | KeyCode::Right => {
                self.explorer_category = self.explorer_category.next();
                self.selected_asset_idx = 0;
                self.rebuild_explorer_items();
            }
            KeyCode::Char('/') => {
                self.input_mode = InputMode::FilterAssets(self.explorer_filter.clone());
            }
            KeyCode::Esc => {
                self.explorer_filter.clear();
                self.rebuild_explorer_items();
            }
            KeyCode::Char('c') => {
                if let Some(loaded) = &self.loaded_pack {
                    self.convert_form.input_path = loaded.path.display().to_string();
                    let target = match loaded.edition {
                        Edition::Java => Edition::Bedrock,
                        Edition::Bedrock => Edition::Java,
                    };
                    self.convert_form.target_edition = Some(target);
                    self.convert_form.output_path = suggest_output_path(&loaded.path, target);
                    self.active_tab = ActiveTab::Convert;
                }
            }
            _ => {}
        }
    }

    fn handle_convert_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.convert_form.selected_field =
                    (self.convert_form.selected_field + 1) % ConvertForm::FIELD_COUNT;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.convert_form.selected_field =
                    (self.convert_form.selected_field + ConvertForm::FIELD_COUNT - 1)
                        % ConvertForm::FIELD_COUNT;
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
                self.convert_form.cycle_current_field(true);
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.convert_form.cycle_current_field(false);
            }
            KeyCode::Enter => match self.convert_form.selected_field {
                0 => {
                    self.input_mode =
                        InputMode::EditConvertInput(self.convert_form.input_path.clone());
                }
                1 => {
                    self.input_mode =
                        InputMode::EditConvertOutput(self.convert_form.output_path.clone());
                }
                _ => self.convert_form.cycle_current_field(true),
            },
            KeyCode::Char('x') | KeyCode::F(5) => {
                self.start_convert();
            }
            KeyCode::Char('e') => {
                if let Some(Ok(res)) = &self.last_conversion {
                    let out = self.scan_dir.join("conversion_report.json");
                    match export_report_json(&res.report, &out) {
                        Ok(()) => {
                            self.status_banner = format!("Exported report -> {}", out.display());
                            self.push_log(self.status_banner.clone());
                        }
                        Err(err) => {
                            self.status_banner = format!("Export failed: {err:#}");
                        }
                    }
                }
            }
            KeyCode::PageDown => {
                self.report_scroll = self.report_scroll.saturating_add(4);
            }
            KeyCode::PageUp => {
                self.report_scroll = self.report_scroll.saturating_sub(4);
            }
            _ => {}
        }
    }

    fn handle_batch_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.audit_outcomes.is_empty() {
                    self.selected_audit_idx =
                        (self.selected_audit_idx + 1).min(self.audit_outcomes.len() - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected_audit_idx = self.selected_audit_idx.saturating_sub(1);
            }
            KeyCode::Char('t') => {
                let paths: Vec<PathBuf> = self
                    .selected_or_all_pack_paths()
                    .into_iter()
                    .map(|(p, _)| p)
                    .collect();
                if paths.is_empty() {
                    self.status_banner = String::from("No valid packs to audit");
                } else {
                    self.start_audit(paths);
                }
            }
            KeyCode::Char('b') => {
                self.start_batch_convert();
            }
            KeyCode::Char('e') if !self.audit_outcomes.is_empty() => {
                let out = self.scan_dir.join("audit_results.json");
                match export_audit_json(&self.audit_outcomes, &out) {
                    Ok(()) => {
                        self.status_banner = format!("Exported audit -> {}", out.display());
                        self.push_log(self.status_banner.clone());
                    }
                    Err(err) => {
                        self.status_banner = format!("Export failed: {err:#}");
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_compare_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.diff_scroll = self.diff_scroll.saturating_add(2);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.diff_scroll = self.diff_scroll.saturating_sub(2);
            }
            KeyCode::Char('r') => {
                let marked: Vec<PathBuf> = self
                    .packs
                    .iter()
                    .filter(|p| p.selected)
                    .map(|p| p.path.clone())
                    .collect();
                if marked.len() >= 2 {
                    self.start_diff(marked[0].clone(), marked[1].clone());
                } else if self.packs.len() >= 2 {
                    self.start_diff(self.packs[0].path.clone(), self.packs[1].path.clone());
                }
            }
            _ => {}
        }
    }

    fn handle_versions_key(&mut self, key: KeyEvent) {
        let len = self.mappings.pack_format_versions().len();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                if len > 0 {
                    self.version_scroll = (self.version_scroll + 1).min(len - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.version_scroll = self.version_scroll.saturating_sub(1);
            }
            _ => {}
        }
    }
}

fn input_mode_buf_mut(mode: &mut InputMode) -> Option<&mut String> {
    match mode {
        InputMode::EditScanDir(s)
        | InputMode::FilterPacks(s)
        | InputMode::FilterAssets(s)
        | InputMode::EditConvertInput(s)
        | InputMode::EditConvertOutput(s) => Some(s),
        InputMode::Normal | InputMode::ConfirmDelete { .. } => None,
    }
}
