use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use egui::{
    Color32, ColorImage, Context, Pos2, Rect, RichText, ScrollArea, Stroke, TextureHandle,
    TextureOptions, Ui, Vec2,
};

use rp_bedrock::{audit_bedrock_container, emit_bedrock_pack};
use rp_cli::manager::{
    AuditOutcome, DiscoveredPack, PackDiff, PackSortKey, audit_pack_roundtrip, diff_packs,
    export_audit_json, export_report_json, format_bytes, load_pack_ir, repack_dir_to_archive,
    scan_directory_for_packs, sort_discovered_packs, suggest_output_path, unpack_pack_to_dir,
};
use rp_cli::{ConvertOptions, VersionSpecificOptions, convert_pack};
use rp_core::{
    AuditStatus, BedrockGeoVersion, ConversionReport, Edition, ElementRotation, FaceDirection,
    MappingTable, ResourcePackIR, RotationAxis, RotationSnapMode, TextureAsset, TextureCategory,
};
use rp_io::IoLimits;
use rp_java::emit_java_pack;
use rp_transforms::model::java_to_bedrock_box;

use crate::diagnostics::{DiagnosticFilter, DiagnosticKind, NeedsReviewIndex};
use crate::file_picker::{FilePickerResult, FilePickerTarget, InAppFileBrowser};
use crate::studio::{
    ActivitySection, ManualOverrideStore, SideBarFilter, StudioEditorTab, TextAssetTarget,
    validate_json_str,
};
use crate::theme::{
    M3Palette, apply_material3_theme, m3_card_frame, m3_filled_button, m3_pill_badge,
    m3_rail_frame, m3_segmented_button, m3_tonal_button,
};
use crate::viewport3d::ModelViewportState;

/// Top-level workspace mode: streamlined Quick Manager or VS Code-style Studio Mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceMode {
    Quick,
    Studio,
}

/// Navigation pages inside Quick Mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiPage {
    Packs,
    Convert,
    BatchAudit,
    Compare,
    VersionsLogs,
}

impl GuiPage {
    pub const ALL: [Self; 5] = [
        Self::Packs,
        Self::Convert,
        Self::BatchAudit,
        Self::Compare,
        Self::VersionsLogs,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Packs => "Packs Manager",
            Self::Convert => "Quick Convert",
            Self::BatchAudit => "Batch & Audit",
            Self::Compare => "Compare (Diff)",
            Self::VersionsLogs => "Versions & Logs",
        }
    }
}

/// Deep-loaded pack inside Studio Mode.
#[derive(Debug, Clone)]
pub struct LoadedPack {
    pub path: PathBuf,
    pub edition: Edition,
    pub ir: ResourcePackIR,
    pub report: ConversionReport,
}

/// Isolated per-edition Studio workspace state so Java and Bedrock packs never overwrite each other.
#[derive(Debug, Clone, Default)]
pub struct EditionStudioSlot {
    pub loaded_pack: Option<LoadedPack>,
    pub open_tabs: Vec<StudioEditorTab>,
    pub active_tab_idx: usize,
    pub overrides: ManualOverrideStore,
}

/// Last single conversion outcome.
#[derive(Debug, Clone)]
pub struct ConversionOutcome {
    pub source: Edition,
    pub target: Edition,
    pub output_path: PathBuf,
    pub report: ConversionReport,
    pub elapsed_ms: u128,
}

#[derive(Debug)]
enum WorkerMsg {
    ScanDone {
        dir: PathBuf,
        result: Result<Vec<DiscoveredPack>, String>,
    },
    PackLoaded {
        result: Result<Box<LoadedPack>, String>,
        is_fresh_refresh: bool,
        refreshed_packs: Option<Result<Vec<DiscoveredPack>, String>>,
    },
    ConvertDone(Result<ConversionOutcome, String>),
    DiffDone(Result<Box<PackDiff>, String>),
    ArchiveOpDone {
        result: Result<String, String>,
        rescan: bool,
    },
    BatchProgress {
        completed: usize,
        total: usize,
        label: String,
    },
    AuditEntry(AuditOutcome),
    BatchDone(String),
}

struct CachedTexPreview {
    asset_id: String,
    dims: (u32, u32),
    frames: Vec<TextureHandle>,
    frametime_ticks: u32,
}

/// Desktop GUI Studio implementing `eframe::App`.
pub struct RpGuiApp {
    pub mode: WorkspaceMode,
    pub page: GuiPage,
    pub dark_mode: bool,
    applied_dark_mode: Option<bool>,
    palette: M3Palette,

    pub scan_dir: PathBuf,
    pub scan_dir_input: String,
    pub recent_paths: Vec<PathBuf>,
    pub mapping_file: Option<PathBuf>,
    pub mappings: MappingTable,
    pub limits: IoLimits,

    // Non-blocking file browser & async OS picker channel
    pub file_browser: InAppFileBrowser,
    picker_tx: Sender<FilePickerResult>,
    picker_rx: Receiver<FilePickerResult>,

    // Packs Manager
    pub packs: Vec<DiscoveredPack>,
    pub pack_filter: String,
    pub pack_sort: PackSortKey,
    pub pending_delete: Option<(PathBuf, bool)>,
    icon_cache: HashMap<PathBuf, TextureHandle>,

    // Studio Mode Workbench (VS Code layout + Blockbench 3D, separated Java vs Bedrock slots)
    pub studio_edition: Edition,
    stashed_java_studio: EditionStudioSlot,
    stashed_bedrock_studio: EditionStudioSlot,
    pub activity_section: ActivitySection,
    pub show_sidebar: bool,
    pub show_bottom_panel: bool,
    pub sidebar_filter: SideBarFilter,
    pub studio_search: String,
    pub open_tabs: Vec<StudioEditorTab>,
    pub active_tab_idx: usize,
    pub loaded_pack: Option<LoadedPack>,
    pub overrides: ManualOverrideStore,
    pub review_index: NeedsReviewIndex,
    pub diagnostic_filter: DiagnosticFilter,

    // Blockbench 3D & Texture Inspector state
    pub viewport_3d: ModelViewportState,
    pub selected_element_idx: Option<usize>,
    pub selected_face_dir: FaceDirection,
    pub tex_zoom: f32,
    pub tex_pixel_grid: bool,
    pub anim_playing: bool,
    pub anim_frame_idx: usize,
    pub anim_last_step: Instant,
    pub custom_mapping_input: String,
    tex_preview: Option<CachedTexPreview>,

    // Converter
    pub input_path: String,
    pub output_path: String,
    pub target_edition: Option<Edition>,
    pub target_version: String,
    pub rotation_mode: RotationSnapMode,
    pub bedrock_geo_version: Option<BedrockGeoVersion>,
    pub bedrock_manifest_version: Option<u32>,
    pub emit_java_items: Option<bool>,
    pub strict_mode: bool,
    pub last_conversion: Option<Result<ConversionOutcome, String>>,

    // Batch & Audit
    pub audit_outcomes: Vec<AuditOutcome>,
    pub batch_progress: Option<(usize, usize, String)>,

    // Compare / Diff
    pub diff_path_a: String,
    pub diff_path_b: String,
    pub pack_diff: Option<Result<PackDiff, String>>,

    // Versions & Logs
    pub version_filter: String,
    pub logs: Vec<String>,

    // Status & Worker Channel
    pub busy_message: Option<String>,
    pub status_banner: String,
    tx: Sender<WorkerMsg>,
    rx: Receiver<WorkerMsg>,
}

impl RpGuiApp {
    /// Creates a new desktop `RpGuiApp` rooted at `scan_dir`.
    #[must_use]
    pub fn new(scan_dir: PathBuf, mapping_file: Option<PathBuf>, mappings: MappingTable) -> Self {
        let (tx, rx) = mpsc::channel();
        let (picker_tx, picker_rx) = mpsc::channel();
        let scan_dir_input = scan_dir.display().to_string();
        let mut app = Self {
            mode: WorkspaceMode::Quick,
            page: GuiPage::Packs,
            dark_mode: true,
            applied_dark_mode: None,
            palette: M3Palette::dark(),
            scan_dir: scan_dir.clone(),
            scan_dir_input,
            recent_paths: vec![scan_dir.clone()],
            mapping_file,
            mappings,
            limits: IoLimits::default(),
            file_browser: InAppFileBrowser::default(),
            picker_tx,
            picker_rx,
            packs: Vec::new(),
            pack_filter: String::new(),
            pack_sort: PackSortKey::Name,
            pending_delete: None,
            icon_cache: HashMap::new(),
            studio_edition: Edition::Java,
            stashed_java_studio: EditionStudioSlot::default(),
            stashed_bedrock_studio: EditionStudioSlot::default(),
            activity_section: ActivitySection::Explorer,
            show_sidebar: true,
            show_bottom_panel: true,
            sidebar_filter: SideBarFilter::All,
            studio_search: String::new(),
            open_tabs: Vec::new(),
            active_tab_idx: 0,
            loaded_pack: None,
            overrides: ManualOverrideStore::default(),
            review_index: NeedsReviewIndex::default(),
            diagnostic_filter: DiagnosticFilter::All,
            viewport_3d: ModelViewportState::default(),
            selected_element_idx: Some(0),
            selected_face_dir: FaceDirection::North,
            tex_zoom: 6.0,
            tex_pixel_grid: true,
            anim_playing: true,
            anim_frame_idx: 0,
            anim_last_step: Instant::now(),
            custom_mapping_input: String::new(),
            tex_preview: None,
            input_path: String::new(),
            output_path: String::new(),
            target_edition: None,
            target_version: String::from("26.2"),
            rotation_mode: RotationSnapMode::Preserve,
            bedrock_geo_version: None,
            bedrock_manifest_version: None,
            emit_java_items: None,
            strict_mode: false,
            last_conversion: None,
            audit_outcomes: Vec::new(),
            batch_progress: None,
            diff_path_a: String::new(),
            diff_path_b: String::new(),
            pack_diff: None,
            version_filter: String::new(),
            logs: Vec::new(),
            busy_message: None,
            status_banner: format!("Ready | {}", scan_dir.display()),
            tx,
            rx,
        };
        app.push_log(format!(
            "Launched rp-gui Studio in {}",
            app.scan_dir.display()
        ));
        app
    }

    fn record_recent(&mut self, path: &Path) {
        self.recent_paths.retain(|p| p != path);
        self.recent_paths.insert(0, path.to_path_buf());
        if self.recent_paths.len() > 8 {
            self.recent_paths.truncate(8);
        }
    }

    fn push_log(&mut self, msg: impl Into<String>) {
        self.logs.push(msg.into());
        if self.logs.len() > 250 {
            let excess = self.logs.len() - 250;
            self.logs.drain(0..excess);
        }
    }

    fn rebuild_review_index(&mut self) {
        let mut reports: Vec<&ConversionReport> = Vec::new();
        if let Some(loaded) = &self.loaded_pack {
            reports.push(&loaded.report);
        }
        if let Some(Ok(conv)) = &self.last_conversion {
            reports.push(&conv.report);
        }
        self.review_index = NeedsReviewIndex::from_reports(reports);
    }

    fn open_or_focus_tab(&mut self, tab: StudioEditorTab) {
        if let Some(pos) = self.open_tabs.iter().position(|t| *t == tab) {
            self.active_tab_idx = pos;
        } else {
            self.open_tabs.push(tab);
            self.active_tab_idx = self.open_tabs.len().saturating_sub(1);
        }
        self.mode = WorkspaceMode::Studio;
        self.selected_element_idx = Some(0);
        self.custom_mapping_input.clear();
    }

    fn open_asset_by_report_hint(&mut self, raw_asset: &str) {
        let Some(loaded) = &self.loaded_pack else {
            return;
        };
        let norm = crate::diagnostics::normalize_asset_key(raw_asset);
        if let Some(id) = loaded
            .ir
            .models
            .keys()
            .find(|k| crate::diagnostics::normalize_asset_key(k) == norm || k.ends_with(&norm))
            .cloned()
        {
            self.open_or_focus_tab(StudioEditorTab::Model(id));
            return;
        }
        if let Some(id) = loaded
            .ir
            .textures
            .keys()
            .find(|k| crate::diagnostics::normalize_asset_key(k) == norm || k.ends_with(&norm))
            .cloned()
        {
            self.open_or_focus_tab(StudioEditorTab::Texture(id));
            return;
        }
        if let Some(id) = loaded
            .ir
            .attachables
            .keys()
            .find(|k| {
                raw_asset
                    .to_ascii_lowercase()
                    .contains(&k.to_ascii_lowercase())
            })
            .cloned()
        {
            self.open_or_focus_tab(StudioEditorTab::Text(TextAssetTarget::AttachableJson(id)));
            return;
        }
        if let Some(p) = loaded
            .ir
            .passthrough_files
            .keys()
            .find(|k| k.eq_ignore_ascii_case(raw_asset) || k.ends_with(raw_asset))
            .cloned()
        {
            self.open_or_focus_tab(StudioEditorTab::Text(TextAssetTarget::Passthrough(p)));
        }
    }

    /// Starts scanning `self.scan_dir` asynchronously.
    pub fn start_scan(&mut self) {
        if self.busy_message.is_some() {
            return;
        }
        let dir = self.scan_dir.clone();
        let mappings = self.mappings.clone();
        let limits = self.limits;
        let tx = self.tx.clone();
        self.busy_message = Some(format!("Scanning {}...", dir.display()));
        self.push_log(format!("Scanning {}", dir.display()));
        thread::spawn(move || {
            let result =
                scan_directory_for_packs(&dir, &mappings, limits).map_err(|err| format!("{err:#}"));
            let _ = tx.send(WorkerMsg::ScanDone { dir, result });
        });
    }

    fn version_options(&self) -> VersionSpecificOptions {
        VersionSpecificOptions {
            supported_formats: None,
            rotation_mode: self.rotation_mode,
            bedrock_geo_version: self.bedrock_geo_version,
            bedrock_manifest_version: self.bedrock_manifest_version,
            emit_java_items_definitions: self.emit_java_items,
            convert_pbr: false,
        }
    }

    /// Switches between separated Java Edition and Bedrock Edition Studio workspaces,
    /// preserving open tabs, loaded pack IR, and manual overrides independently per edition.
    pub fn switch_studio_edition(&mut self, target: Edition) {
        if self.studio_edition == target {
            return;
        }
        let outgoing = EditionStudioSlot {
            loaded_pack: self.loaded_pack.take(),
            open_tabs: std::mem::take(&mut self.open_tabs),
            active_tab_idx: self.active_tab_idx,
            overrides: std::mem::take(&mut self.overrides),
        };
        match self.studio_edition {
            Edition::Java => self.stashed_java_studio = outgoing,
            Edition::Bedrock => self.stashed_bedrock_studio = outgoing,
        }

        let incoming = match target {
            Edition::Java => std::mem::take(&mut self.stashed_java_studio),
            Edition::Bedrock => std::mem::take(&mut self.stashed_bedrock_studio),
        };
        self.loaded_pack = incoming.loaded_pack;
        self.open_tabs = incoming.open_tabs;
        self.active_tab_idx = incoming.active_tab_idx;
        self.overrides = incoming.overrides;
        self.studio_edition = target;
        self.tex_preview = None;
        self.selected_element_idx = Some(0);
        self.custom_mapping_input.clear();

        if let Some(loaded) = &self.loaded_pack {
            let convert_target = match loaded.edition {
                Edition::Java => Edition::Bedrock,
                Edition::Bedrock => Edition::Java,
            };
            self.input_path = loaded.path.display().to_string();
            self.target_edition = Some(convert_target);
            self.output_path = suggest_output_path(&loaded.path, convert_target);
        }
        self.rebuild_review_index();
    }

    fn edition_loaded_label(&self, edition: Edition) -> Option<String> {
        let pack = if self.studio_edition == edition {
            self.loaded_pack.as_ref()
        } else {
            match edition {
                Edition::Java => self.stashed_java_studio.loaded_pack.as_ref(),
                Edition::Bedrock => self.stashed_bedrock_studio.loaded_pack.as_ref(),
            }
        };
        pack.map(|p| {
            p.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.ir.metadata.name.clone())
        })
    }

    /// Performs a fresh reload in Studio Mode: clears texture/icon caches, reloads the active
    /// pack directly from disk (re-running all parsers/diagnostics), and refreshes workspace packs.
    pub fn fresh_refresh_studio(&mut self) {
        if self.busy_message.is_some() {
            return;
        }
        self.tex_preview = None;
        self.icon_cache.clear();
        self.anim_frame_idx = 0;
        self.custom_mapping_input.clear();

        if let Some(path) = self.loaded_pack.as_ref().map(|l| l.path.clone()) {
            let mappings = self.mappings.clone();
            let limits = self.limits;
            let scan_dir = self.scan_dir.clone();
            let tx = self.tx.clone();
            self.busy_message = Some(format!("Fresh refreshing {}...", path.display()));
            self.push_log(format!("Studio fresh refresh: {}", path.display()));
            thread::spawn(move || {
                let refreshed_packs = Some(
                    scan_directory_for_packs(&scan_dir, &mappings, limits)
                        .map_err(|err| format!("{err:#}")),
                );
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
                let _ = tx.send(WorkerMsg::PackLoaded {
                    result,
                    is_fresh_refresh: true,
                    refreshed_packs,
                });
            });
        } else {
            self.status_banner = String::from("Refreshing Studio workspace packs...");
            self.start_scan();
        }
    }

    /// Loads `path` into Studio Mode asynchronously.
    pub fn start_load_studio(&mut self, path: PathBuf) {
        if self.busy_message.is_some() {
            return;
        }
        self.record_recent(&path);
        let mappings = self.mappings.clone();
        let limits = self.limits;
        let tx = self.tx.clone();
        self.mode = WorkspaceMode::Studio;
        self.busy_message = Some(format!("Opening {} in Studio...", path.display()));
        self.push_log(format!("Studio open: {}", path.display()));
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
            let _ = tx.send(WorkerMsg::PackLoaded {
                result,
                is_fresh_refresh: false,
                refreshed_packs: None,
            });
        });
    }

    fn load_pack_into_converter(&mut self, pack: &DiscoveredPack) {
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
        self.mode = WorkspaceMode::Quick;
        self.page = GuiPage::Convert;
    }

    fn start_convert(&mut self) {
        if self.busy_message.is_some() {
            return;
        }
        if self.input_path.trim().is_empty() || self.output_path.trim().is_empty() {
            self.status_banner = String::from("Select both Input Pack and Output Path first");
            return;
        }
        let options = ConvertOptions {
            input: PathBuf::from(self.input_path.trim()),
            output: PathBuf::from(self.output_path.trim()),
            target: self.target_edition,
            target_version: Some(self.target_version.clone()),
            mapping_file: self.mapping_file.clone(),
            strict: self.strict_mode,
            report_json: None,
            limits: self.limits,
            version_options: self.version_options(),
        };
        let out_path = options.output.clone();
        let tx = self.tx.clone();
        self.busy_message = Some(format!("Converting {}...", options.input.display()));
        self.push_log(format!(
            "Converting '{}' -> '{}' ({})",
            options.input.display(),
            options.output.display(),
            self.target_version
        ));
        thread::spawn(move || {
            let start = Instant::now();
            let res = convert_pack(&options)
                .map(|(source, target, report)| ConversionOutcome {
                    source,
                    target,
                    output_path: out_path,
                    report,
                    elapsed_ms: start.elapsed().as_millis(),
                })
                .map_err(|err| format!("{err:#}"));
            let _ = tx.send(WorkerMsg::ConvertDone(res));
        });
    }

    /// Exports the currently edited Studio `ResourcePackIR` (with all manual overrides preserved)
    /// into the chosen target edition and output archive path!
    fn start_export_studio_ir(&mut self) {
        if self.busy_message.is_some() {
            return;
        }
        let Some(loaded) = self.loaded_pack.clone() else {
            self.status_banner = String::from("No pack loaded in Studio to export");
            return;
        };
        let target = self.target_edition.unwrap_or(match loaded.edition {
            Edition::Java => Edition::Bedrock,
            Edition::Bedrock => Edition::Java,
        });
        let out_path = if self.output_path.trim().is_empty() {
            PathBuf::from(suggest_output_path(&loaded.path, target))
        } else {
            PathBuf::from(self.output_path.trim())
        };
        let mut ir = loaded.ir;
        let applied = self.overrides.apply_to_ir(&mut ir);
        let mut mappings = self.mappings.clone();
        if let Ok((java_fmt, bver)) = mappings.resolve_target_version(target, &self.target_version)
        {
            if let Some(f) = java_fmt {
                ir.metadata.java_pack_format = Some(f);
            }
            if let Some(b) = bver {
                ir.metadata.bedrock_min_engine_version = Some(b);
                if self.bedrock_geo_version.is_none() {
                    ir.metadata.bedrock_geo_version =
                        Some(BedrockGeoVersion::for_engine_version(b));
                }
            }
        }
        // Apply any custom texture mapping rules from Studio
        if !self.overrides.custom_texture_mappings.is_empty() {
            let tex_entries: Vec<serde_json::Value> = self
                .overrides
                .custom_texture_mappings
                .iter()
                .map(|(j, b)| serde_json::json!({ "java": j, "bedrock": b }))
                .collect();
            let raw = serde_json::json!({ "schema_version": "1.0", "textures": tex_entries });
            if let Ok(ov) = MappingTable::from_json_str(&raw.to_string()) {
                mappings.merge_override(ov);
            }
        }

        let source_ed = loaded.edition;
        let tx = self.tx.clone();
        self.busy_message = Some(format!(
            "Exporting Studio pack ({applied} manual override(s))..."
        ));
        self.push_log(format!(
            "Exporting Studio IR ({applied} manual overrides) -> {}",
            out_path.display()
        ));

        thread::spawn(move || {
            let start = Instant::now();
            let res: Result<ConversionOutcome, String> = (|| {
                let total_in_tex = ir.textures.len();
                let (container, mut report) = match target {
                    Edition::Java => emit_java_pack(&ir, &mappings).map_err(|e| format!("{e}"))?,
                    Edition::Bedrock => {
                        emit_bedrock_pack(&ir, &mappings).map_err(|e| format!("{e}"))?
                    }
                };
                if target == Edition::Bedrock {
                    let audit = audit_bedrock_container(&container, total_in_tex, &mappings);
                    let failed_msg = (audit.status == AuditStatus::Failed)
                        .then(|| format!("Bedrock Semantic Audit FAILED: {}", audit.summary));
                    report.semantic_audit = Some(audit);
                    if let Some(msg) = failed_msg {
                        return Err(msg);
                    }
                }
                let bytes = container
                    .write_to_zip_bytes()
                    .map_err(|e| format!("zip encode: {e}"))?;
                std::fs::write(&out_path, bytes)
                    .map_err(|e| format!("write '{}': {e}", out_path.display()))?;
                Ok(ConversionOutcome {
                    source: source_ed,
                    target,
                    output_path: out_path,
                    report,
                    elapsed_ms: start.elapsed().as_millis(),
                })
            })();
            let _ = tx.send(WorkerMsg::ConvertDone(res));
        });
    }

    fn start_diff(&mut self, a: PathBuf, b: PathBuf) {
        if self.busy_message.is_some() {
            return;
        }
        let mappings = self.mappings.clone();
        let limits = self.limits;
        let tx = self.tx.clone();
        self.mode = WorkspaceMode::Quick;
        self.page = GuiPage::Compare;
        self.busy_message = Some(String::from("Comparing packs..."));
        thread::spawn(move || {
            let res = diff_packs(&a, &b, &mappings, limits)
                .map(Box::new)
                .map_err(|err| format!("{err:#}"));
            let _ = tx.send(WorkerMsg::DiffDone(res));
        });
    }

    fn start_unpack(&mut self, path: PathBuf) {
        if self.busy_message.is_some() {
            return;
        }
        let limits = self.limits;
        let tx = self.tx.clone();
        self.busy_message = Some(format!("Unpacking {}...", path.display()));
        thread::spawn(move || {
            let res = unpack_pack_to_dir(&path, limits)
                .map(|p| format!("Unpacked -> {}", p.display()))
                .map_err(|e| format!("Unpack failed: {e:#}"));
            let _ = tx.send(WorkerMsg::ArchiveOpDone {
                result: res,
                rescan: true,
            });
        });
    }

    fn start_repack(&mut self, path: PathBuf) {
        if self.busy_message.is_some() {
            return;
        }
        let limits = self.limits;
        let tx = self.tx.clone();
        self.busy_message = Some(format!("Repacking {}...", path.display()));
        thread::spawn(move || {
            let res = repack_dir_to_archive(&path, limits)
                .map(|p| format!("Repacked -> {}", p.display()))
                .map_err(|e| format!("Repack failed: {e:#}"));
            let _ = tx.send(WorkerMsg::ArchiveOpDone {
                result: res,
                rescan: true,
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

    fn start_audit(&mut self, paths: Vec<PathBuf>) {
        if self.busy_message.is_some() || paths.is_empty() {
            return;
        }
        let total = paths.len();
        let mappings = self.mappings.clone();
        let limits = self.limits;
        let target_ver = Some(self.target_version.clone());
        let ver_opts = self.version_options();
        let tx = self.tx.clone();

        self.audit_outcomes.clear();
        self.batch_progress = Some((0, total, String::from("Starting round-trip audit...")));
        self.busy_message = Some(format!("Auditing {total} pack(s)..."));
        self.push_log(format!("Auditing {total} pack(s)"));

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
                label: String::from("Complete"),
            });
            let _ = tx.send(WorkerMsg::BatchDone(format!(
                "Round-trip audit complete: {passed}/{total} passed"
            )));
        });
    }

    fn start_batch_convert(&mut self) {
        if self.busy_message.is_some() {
            return;
        }
        let targets = self.selected_or_all_pack_paths();
        if targets.is_empty() {
            self.status_banner = String::from("No valid packs to batch convert");
            return;
        }
        let total = targets.len();
        let out_dir = self.scan_dir.join("converted");
        let target_ver = Some(self.target_version.clone());
        let ver_opts = self.version_options();
        let mapping_file = self.mapping_file.clone();
        let strict = self.strict_mode;
        let limits = self.limits;
        let tx = self.tx.clone();

        self.audit_outcomes.clear();
        self.batch_progress = Some((0, total, String::from("Starting batch convert...")));
        self.busy_message = Some(format!("Batch converting {total} pack(s)..."));

        thread::spawn(move || {
            let _ = std::fs::create_dir_all(&out_dir);
            let mut ok = 0usize;
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
                        let status = report.effective_status();
                        if status == AuditStatus::Passed {
                            ok += 1;
                        }
                        let detail = match &report.semantic_audit {
                            Some(a) => format!("{} | Saved -> {}", a.summary, out_file.display()),
                            None => format!("Saved -> {}", out_file.display()),
                        };
                        AuditOutcome {
                            pack_name,
                            path,
                            source_edition: source,
                            target_edition: target,
                            passed: status == AuditStatus::Passed,
                            status,
                            forward_report: Some(report),
                            detail,
                            elapsed_ms: start.elapsed().as_millis(),
                        }
                    }
                    Err(err) => AuditOutcome {
                        pack_name,
                        path,
                        source_edition: src_ed,
                        target_edition: target_ed,
                        passed: false,
                        status: AuditStatus::Failed,
                        forward_report: None,
                        detail: format!("FAILED: {err:#}"),
                        elapsed_ms: start.elapsed().as_millis(),
                    },
                };
                let _ = tx.send(WorkerMsg::AuditEntry(outcome));
            }
            let _ = tx.send(WorkerMsg::BatchProgress {
                completed: total,
                total,
                label: String::from("Complete"),
            });
            let _ = tx.send(WorkerMsg::BatchDone(format!(
                "Batch convert complete: {ok}/{total} saved in {}",
                out_dir.display()
            )));
        });
    }

    fn apply_picked_path(&mut self, target: FilePickerTarget, path: PathBuf) {
        self.record_recent(&path);
        match target {
            FilePickerTarget::WorkspaceDir => {
                self.scan_dir = path.clone();
                self.scan_dir_input = path.display().to_string();
                self.start_scan();
            }
            FilePickerTarget::ConvertInput => {
                self.input_path = path.display().to_string();
                let ed = self.target_edition.unwrap_or(Edition::Java);
                self.output_path = suggest_output_path(&path, ed);
            }
            FilePickerTarget::ConvertOutput => {
                self.output_path = path.display().to_string();
            }
            FilePickerTarget::DiffPackA => {
                self.diff_path_a = path.display().to_string();
            }
            FilePickerTarget::DiffPackB => {
                self.diff_path_b = path.display().to_string();
            }
            FilePickerTarget::ExportOverrides => match self.overrides.export_json(&path) {
                Ok(()) => {
                    self.status_banner = format!("Exported overrides -> {}", path.display());
                    self.push_log(self.status_banner.clone());
                }
                Err(err) => {
                    self.status_banner = format!("Export overrides failed: {err:#}");
                }
            },
        }
    }

    fn poll_workers(&mut self) {
        while let Ok(pick) = self.picker_rx.try_recv() {
            if let Some(p) = pick.path {
                self.apply_picked_path(pick.target, p);
            }
        }

        let mut need_rescan = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                WorkerMsg::ScanDone { dir, result } => {
                    self.busy_message = None;
                    match result {
                        Ok(mut packs) => {
                            sort_discovered_packs(&mut packs, self.pack_sort);
                            let n = packs.len();
                            if self.input_path.is_empty()
                                && let Some(first) = packs.first()
                            {
                                let prev_mode = self.mode;
                                let prev_page = self.page;
                                self.load_pack_into_converter(first);
                                self.mode = prev_mode;
                                self.page = prev_page;
                            }
                            if self.diff_path_a.is_empty() && packs.len() >= 2 {
                                self.diff_path_a = packs[0].path.display().to_string();
                                self.diff_path_b = packs[1].path.display().to_string();
                            }
                            self.packs = packs;
                            self.icon_cache.clear();
                            self.status_banner = format!("Found {n} pack(s) in {}", dir.display());
                            self.push_log(self.status_banner.clone());
                        }
                        Err(err) => {
                            self.status_banner = format!("Scan error: {err}");
                            self.push_log(self.status_banner.clone());
                        }
                    }
                }
                WorkerMsg::PackLoaded {
                    result,
                    is_fresh_refresh,
                    refreshed_packs,
                } => {
                    self.busy_message = None;
                    if let Some(Ok(mut packs)) = refreshed_packs {
                        sort_discovered_packs(&mut packs, self.pack_sort);
                        self.packs = packs;
                        self.icon_cache.clear();
                    }
                    match result {
                        Ok(loaded) => {
                            let loaded = *loaded;
                            // Route pack into its dedicated Java or Bedrock Studio workspace.
                            self.switch_studio_edition(loaded.edition);

                            let action_label = if is_fresh_refresh {
                                "Fresh refreshed"
                            } else {
                                "Studio loaded"
                            };
                            self.status_banner = format!(
                                "{action_label} {} ({}, {} tex, {} 3D models, {} warnings)",
                                loaded.ir.metadata.name,
                                loaded.edition,
                                loaded.ir.textures.len(),
                                loaded.ir.models.len(),
                                loaded.report.warnings.len(),
                            );
                            self.push_log(self.status_banner.clone());
                            self.overrides.clear();
                            self.open_tabs.clear();
                            self.active_tab_idx = 0;
                            self.tex_preview = None;
                            self.selected_element_idx = Some(0);

                            // Auto-open first 3D model (or first texture, or metadata)
                            let default_tab = loaded
                                .ir
                                .models
                                .iter()
                                .find(|(_, m)| !m.elements.is_empty())
                                .map(|(id, _)| StudioEditorTab::Model(id.clone()))
                                .or_else(|| {
                                    loaded
                                        .ir
                                        .textures
                                        .keys()
                                        .next()
                                        .map(|id| StudioEditorTab::Texture(id.clone()))
                                })
                                .unwrap_or(StudioEditorTab::Text(TextAssetTarget::MetadataJson));
                            self.open_tabs.push(default_tab);

                            let target = match loaded.edition {
                                Edition::Java => Edition::Bedrock,
                                Edition::Bedrock => Edition::Java,
                            };
                            self.input_path = loaded.path.display().to_string();
                            self.target_edition = Some(target);
                            self.output_path = suggest_output_path(&loaded.path, target);

                            self.loaded_pack = Some(loaded);
                            self.rebuild_review_index();
                        }
                        Err(err) => {
                            self.status_banner = format!("Studio load error: {err}");
                            self.push_log(self.status_banner.clone());
                        }
                    }
                }
                WorkerMsg::ConvertDone(res) => {
                    self.busy_message = None;
                    match &res {
                        Ok(ok) => {
                            self.status_banner = format!(
                                "Converted {} -> {} in {}ms ({} converted, {} lossy, {} warnings)",
                                ok.source,
                                ok.target,
                                ok.elapsed_ms,
                                ok.report.converted_count,
                                ok.report.lossy_count,
                                ok.report.warnings.len()
                            );
                            self.push_log(format!(
                                "OK: {} -> {}",
                                self.status_banner,
                                ok.output_path.display()
                            ));
                        }
                        Err(err) => {
                            self.status_banner = format!("Convert failed: {err}");
                            self.push_log(self.status_banner.clone());
                        }
                    }
                    self.last_conversion = Some(res);
                    self.rebuild_review_index();
                }
                WorkerMsg::DiffDone(res) => {
                    self.busy_message = None;
                    match res {
                        Ok(diff) => {
                            let d = *diff;
                            self.status_banner = format!(
                                "Compared {} vs {} ({} shared tex, {} shared 3D)",
                                d.name_a, d.name_b, d.textures.shared_count, d.models.shared_count
                            );
                            self.pack_diff = Some(Ok(d));
                        }
                        Err(err) => {
                            self.status_banner = format!("Diff failed: {err}");
                            self.pack_diff = Some(Err(err));
                        }
                    }
                }
                WorkerMsg::ArchiveOpDone { result, rescan } => {
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
                        "[{tag}] {} ({}ms): {}",
                        outcome.pack_name, outcome.elapsed_ms, outcome.detail
                    ));
                    self.audit_outcomes.push(outcome);
                }
                WorkerMsg::BatchDone(summary) => {
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

    fn ensure_icon_texture(
        &mut self,
        ctx: &Context,
        path: &Path,
        png_bytes: &[u8],
    ) -> Option<TextureHandle> {
        if let Some(handle) = self.icon_cache.get(path) {
            return Some(handle.clone());
        }
        let img = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png)
            .ok()?
            .into_rgba8();
        let (w, h) = img.dimensions();
        let color_img = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
        let handle = ctx.load_texture(
            format!("icon:{}", path.display()),
            color_img,
            TextureOptions::NEAREST,
        );
        self.icon_cache.insert(path.to_path_buf(), handle.clone());
        Some(handle)
    }

    fn ensure_asset_texture_preview(&mut self, ctx: &Context, tex: &TextureAsset) {
        if self
            .tex_preview
            .as_ref()
            .is_some_and(|c| c.asset_id == tex.id)
        {
            return;
        }
        let Ok(decoded) = tex.payload.decode() else {
            self.tex_preview = None;
            return;
        };
        let (w, h) = decoded.dimensions();
        let mut frames = Vec::new();
        let frametime_ticks = tex.animation.as_ref().map_or(2, |a| a.frametime.max(1));

        const MAX_PREVIEW_DIM: u32 = 512;
        let downscale_if_needed = |img: image::RgbaImage| -> ColorImage {
            let (iw, ih) = img.dimensions();
            if iw > MAX_PREVIEW_DIM || ih > MAX_PREVIEW_DIM {
                let scale =
                    (MAX_PREVIEW_DIM as f32 / iw as f32).min(MAX_PREVIEW_DIM as f32 / ih as f32);
                let nw = ((iw as f32 * scale).round() as u32).max(1);
                let nh = ((ih as f32 * scale).round() as u32).max(1);
                let resized =
                    image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Nearest);
                ColorImage::from_rgba_unmultiplied([nw as usize, nh as usize], resized.as_raw())
            } else {
                ColorImage::from_rgba_unmultiplied([iw as usize, ih as usize], img.as_raw())
            }
        };

        if let Some(anim) = &tex.animation {
            let fh = anim.frame_height.unwrap_or(w).min(h).max(1);
            let count = (h / fh).clamp(1, 64);
            for i in 0..count {
                let cropped =
                    image::imageops::crop_imm(decoded.as_ref(), 0, i * fh, w, fh).to_image();
                let ci = downscale_if_needed(cropped);
                frames.push(ctx.load_texture(
                    format!("tex:{}:f{i}", tex.id),
                    ci,
                    TextureOptions::NEAREST,
                ));
            }
        } else {
            let ci = downscale_if_needed(decoded.into_owned());
            frames.push(ctx.load_texture(format!("tex:{}", tex.id), ci, TextureOptions::NEAREST));
        }

        self.anim_frame_idx = 0;
        self.anim_last_step = Instant::now();
        self.tex_preview = Some(CachedTexPreview {
            asset_id: tex.id.clone(),
            dims: (w, h),
            frames,
            frametime_ticks,
        });
    }

    /// Renders the entire desktop UI frame (public for headless unit testing).
    pub fn render_ui(&mut self, ctx: &Context) {
        self.poll_workers();

        if self.applied_dark_mode != Some(self.dark_mode) {
            self.palette = apply_material3_theme(ctx, self.dark_mode);
            self.applied_dark_mode = Some(self.dark_mode);
        }
        let palette = self.palette;

        // Keyboard shortcut: F5 or Ctrl+R triggers Fresh Refresh inside Studio Mode
        if self.mode == WorkspaceMode::Studio
            && ctx.input(|i| {
                i.key_pressed(egui::Key::F5) || (i.modifiers.ctrl && i.key_pressed(egui::Key::R))
            })
        {
            self.fresh_refresh_studio();
        }

        // Drag-and-drop .zip / .mcpack / folder anywhere in the window
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if let Some(first_drop) = dropped.first() {
            if first_drop.is_dir()
                && !first_drop.join("pack.mcmeta").is_file()
                && !first_drop.join("manifest.json").is_file()
            {
                self.scan_dir = first_drop.clone();
                self.scan_dir_input = first_drop.display().to_string();
                self.start_scan();
            } else {
                self.start_load_studio(first_drop.clone());
            }
        }

        if self.busy_message.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }

        // Top App Bar (Material 3 tonal surface container)
        egui::TopBottomPanel::top("top_bar")
            .frame(
                egui::Frame::new()
                    .fill(palette.surface_container)
                    .stroke(Stroke::new(1.0, palette.outline_variant))
                    .inner_margin(egui::Margin::symmetric(12, 7)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("mc-rp-converter")
                            .strong()
                            .size(15.5)
                            .color(palette.primary),
                    );
                    m3_pill_badge(
                        ui,
                        "M3",
                        palette.primary_container,
                        palette.on_primary_container,
                    );
                    ui.separator();

                    if m3_segmented_button(
                        ui,
                        self.mode == WorkspaceMode::Quick,
                        "📦 Quick Manager",
                        palette.primary_container,
                        palette.on_primary_container,
                        &palette,
                    )
                    .clicked()
                    {
                        self.mode = WorkspaceMode::Quick;
                    }
                    let total_ov = self.overrides.total_override_count()
                        + self.stashed_java_studio.overrides.total_override_count()
                        + self.stashed_bedrock_studio.overrides.total_override_count();
                    let studio_badge = if total_ov > 0 {
                        format!("🛠 Studio ({total_ov})")
                    } else {
                        String::from("🛠 Studio Mode")
                    };
                    if m3_segmented_button(
                        ui,
                        self.mode == WorkspaceMode::Studio,
                        &studio_badge,
                        palette.primary_container,
                        palette.on_primary_container,
                        &palette,
                    )
                    .clicked()
                    {
                        self.mode = WorkspaceMode::Studio;
                    }

                    ui.separator();
                    let path_w = (ui.available_width() - 310.0).clamp(130.0, 340.0);
                    let resp = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.scan_dir_input)
                                .desired_width(path_w)
                                .hint_text("/path/to/packs"),
                        )
                        .on_hover_text(self.scan_dir_input.as_str());
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let p = PathBuf::from(self.scan_dir_input.trim());
                        if p.is_dir() {
                            self.scan_dir = p;
                            self.start_scan();
                        } else {
                            self.status_banner =
                                format!("Directory not found: {}", self.scan_dir_input);
                        }
                    }
                    if ui
                        .button("Browse")
                        .on_hover_text("Non-blocking in-app folder browser (no X11 freeze)")
                        .clicked()
                    {
                        let start = self.scan_dir.clone();
                        self.file_browser
                            .open_for(FilePickerTarget::WorkspaceDir, &start, "");
                    }
                    if ui.button("Rescan").clicked() {
                        let p = PathBuf::from(self.scan_dir_input.trim());
                        if p.is_dir() {
                            self.scan_dir = p;
                        }
                        self.start_scan();
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let theme_label = if self.dark_mode {
                            "☀ Light"
                        } else {
                            "🌙 Dark"
                        };
                        if m3_tonal_button(ui, true, theme_label, &palette)
                            .on_hover_text("Toggle Material 3 Dark / Light Theme")
                            .clicked()
                        {
                            self.dark_mode = !self.dark_mode;
                        }
                        if let Some(busy) = &self.busy_message {
                            ui.spinner();
                            ui.label(RichText::new(busy).color(palette.warning).small());
                        }
                    });
                });
            });

        // Bottom Status Strip (Material 3 surface container low)
        egui::TopBottomPanel::bottom("status_strip")
            .frame(
                egui::Frame::new()
                    .fill(palette.surface_container_low)
                    .stroke(Stroke::new(1.0, palette.outline_variant))
                    .inner_margin(egui::Margin::symmetric(12, 5)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&self.status_banner)
                            .color(palette.success)
                            .small(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ov = self.overrides.total_override_count();
                        let rev = self
                            .review_index
                            .count_matching(DiagnosticFilter::NeedsReview);
                        ui.label(
                            RichText::new(format!(
                                "Studio: {} | Needs Review: {rev} | Overrides: {ov} | Target: {}",
                                self.studio_edition, self.target_version
                            ))
                            .small()
                            .color(palette.on_surface_variant),
                        );
                    });
                });
            });

        // Render active workspace mode
        match self.mode {
            WorkspaceMode::Quick => self.render_quick_mode(ctx),
            WorkspaceMode::Studio => self.render_studio_mode(ctx),
        }

        // Non-blocking In-App File Browser Modal (zero X11 freeze!)
        let picked = self
            .file_browser
            .show(ctx, &self.recent_paths, &self.picker_tx);
        if let Some((target, path)) = picked {
            self.apply_picked_path(target, path);
        }

        // Safe Delete Confirmation Modal
        if let Some((del_path, is_dir)) = self.pending_delete.clone() {
            egui::Window::new("Confirm Delete")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    let kind = if is_dir { "directory" } else { "file" };
                    ui.label(format!("Permanently delete this {kind}?"));
                    ui.code(del_path.display().to_string());
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new("Delete").color(Color32::LIGHT_RED))
                            .clicked()
                        {
                            let res = if is_dir {
                                std::fs::remove_dir_all(&del_path)
                            } else {
                                std::fs::remove_file(&del_path)
                            };
                            match res {
                                Ok(()) => {
                                    self.status_banner = format!("Deleted {}", del_path.display());
                                    self.push_log(self.status_banner.clone());
                                    self.pending_delete = None;
                                    self.start_scan();
                                }
                                Err(err) => {
                                    self.status_banner = format!("Delete failed: {err}");
                                    self.pending_delete = None;
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending_delete = None;
                        }
                    });
                });
        }
    }

    // =========================================================================
    // QUICK MODE (Streamlined Pack Manager + Clean Converter + Grouped Warnings)
    // =========================================================================

    fn render_quick_mode(&mut self, ctx: &Context) {
        egui::SidePanel::left("quick_nav_rail")
            .resizable(false)
            .default_width(158.0)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                for page in GuiPage::ALL {
                    let btn = ui.add_sized(
                        Vec2::new(ui.available_width(), 30.0),
                        egui::Button::new(RichText::new(page.label()).strong())
                            .selected(self.page == page),
                    );
                    if btn.clicked() {
                        self.page = page;
                    }
                    ui.add_space(3.0);
                }
                ui.separator();
                ui.label(RichText::new(format!("Packs: {}", self.packs.len())).small());
                let sel = self.packs.iter().filter(|p| p.selected).count();
                ui.label(RichText::new(format!("Checked: {sel}")).small());
            });

        egui::CentralPanel::default().show(ctx, |ui| match self.page {
            GuiPage::Packs => self.show_packs_page(ctx, ui),
            GuiPage::Convert => self.show_convert_page(ui),
            GuiPage::BatchAudit => self.show_batch_page(ui),
            GuiPage::Compare => self.show_compare_page(ui),
            GuiPage::VersionsLogs => self.show_versions_page(ui),
        });
    }

    fn show_packs_page(&mut self, ctx: &Context, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Search:");
            ui.add(egui::TextEdit::singleline(&mut self.pack_filter).desired_width(190.0));
            ui.separator();
            ui.label("Sort:");
            if ui.button(self.pack_sort.label()).clicked() {
                self.pack_sort = self.pack_sort.next();
                sort_discovered_packs(&mut self.packs, self.pack_sort);
            }
            if ui.button("Select All").clicked() {
                let any_unsel = self.packs.iter().any(|p| !p.selected);
                for p in &mut self.packs {
                    p.selected = any_unsel;
                }
            }
            if ui.button("Compare Checked (2)").clicked() {
                let marked: Vec<PathBuf> = self
                    .packs
                    .iter()
                    .filter(|p| p.selected)
                    .map(|p| p.path.clone())
                    .collect();
                if marked.len() >= 2 {
                    self.diff_path_a = marked[0].display().to_string();
                    self.diff_path_b = marked[1].display().to_string();
                    self.start_diff(marked[0].clone(), marked[1].clone());
                } else {
                    self.status_banner = String::from("Check 2 packs first to Compare");
                }
            }
        });
        ui.separator();

        let needle = self.pack_filter.trim().to_ascii_lowercase();
        let mut action_studio: Option<PathBuf> = None;
        let mut action_convert: Option<usize> = None;
        let mut action_audit: Option<PathBuf> = None;
        let mut action_unpack: Option<PathBuf> = None;
        let mut action_repack: Option<PathBuf> = None;
        let mut action_delete: Option<(PathBuf, bool)> = None;

        let icon_jobs: Vec<(PathBuf, Vec<u8>)> = self
            .packs
            .iter()
            .filter_map(|p| {
                if !self.icon_cache.contains_key(&p.path) {
                    p.icon_png.clone().map(|b| (p.path.clone(), b))
                } else {
                    None
                }
            })
            .collect();
        for (p, bytes) in icon_jobs {
            let _ = self.ensure_icon_texture(ctx, &p, &bytes);
        }

        ScrollArea::vertical().show(ui, |ui| {
            if self.packs.is_empty() {
                ui.label("No resource packs found here. Click 'Browse' or drag & drop a .zip/.mcpack.");
                return;
            }
            for (idx, pack) in self.packs.iter_mut().enumerate() {
                if !needle.is_empty() {
                    let hit = pack.file_name.to_ascii_lowercase().contains(&needle)
                        || pack.summary.as_ref().is_ok_and(|s| {
                            s.name.to_ascii_lowercase().contains(&needle)
                                || s.edition.to_string().to_ascii_lowercase().contains(&needle)
                        });
                    if !hit {
                        continue;
                    }
                }

                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut pack.selected, "");
                        if let Some(tex) = self.icon_cache.get(&pack.path) {
                            ui.image((tex.id(), Vec2::splat(36.0)));
                        }
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                let name = if pack.is_dir {
                                    format!("{}/", pack.file_name)
                                } else {
                                    pack.file_name.clone()
                                };
                                ui.label(RichText::new(name).strong().size(14.0));
                                match &pack.summary {
                                    Ok(s) => {
                                        let col = match s.edition {
                                            Edition::Java => Color32::from_rgb(80, 200, 255),
                                            Edition::Bedrock => Color32::from_rgb(100, 225, 130),
                                        };
                                        ui.label(
                                            RichText::new(format!("[{}]", s.edition))
                                                .color(col)
                                                .strong(),
                                        );
                                        let ver = match s.edition {
                                            Edition::Java => s
                                                .java_pack_format
                                                .map(|f| format!("fmt {f}"))
                                                .unwrap_or_default(),
                                            Edition::Bedrock => s
                                                .bedrock_min_engine_version
                                                .map(|v| format!("v{}.{}.{}", v[0], v[1], v[2]))
                                                .unwrap_or_default(),
                                        };
                                        ui.label(RichText::new(ver).weak());
                                    }
                                    Err(e) => {
                                        ui.label(
                                            RichText::new(format!("ERROR: {e}"))
                                                .color(Color32::LIGHT_RED),
                                        );
                                    }
                                }
                            });
                            if let Ok(s) = &pack.summary {
                                ui.label(
                                    RichText::new(format!(
                                        "{} tex ({} anim) | {} 3D | {} sounds | {} locales | {} warnings | {}",
                                        s.texture_count,
                                        s.animated_texture_count,
                                        s.model_count,
                                        s.sound_file_count,
                                        s.locale_count,
                                        s.warnings.len(),
                                        format_bytes(pack.size_bytes)
                                    ))
                                    .small()
                                    .weak(),
                                );
                            }
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(RichText::new("Del").color(Color32::LIGHT_RED))
                                .clicked()
                            {
                                action_delete = Some((pack.path.clone(), pack.is_dir));
                            }
                            if pack.is_dir {
                                if ui.button("Repack").clicked() {
                                    action_repack = Some(pack.path.clone());
                                }
                            } else if ui.button("Unpack").clicked() {
                                action_unpack = Some(pack.path.clone());
                            }
                            if ui.button("Audit").clicked() {
                                action_audit = Some(pack.path.clone());
                            }
                            if ui.button("Convert").clicked() {
                                action_convert = Some(idx);
                            }
                            if ui
                                .button(RichText::new("Open Studio").color(Color32::from_rgb(120, 220, 255)))
                                .clicked()
                            {
                                action_studio = Some(pack.path.clone());
                            }
                        });
                    });
                });
                ui.add_space(3.0);
            }
        });

        if let Some(p) = action_studio {
            self.start_load_studio(p);
        } else if let Some(idx) = action_convert {
            if let Some(pack) = self.packs.get(idx).cloned() {
                self.load_pack_into_converter(&pack);
            }
        } else if let Some(p) = action_audit {
            self.page = GuiPage::BatchAudit;
            self.start_audit(vec![p]);
        } else if let Some(p) = action_unpack {
            self.start_unpack(p);
        } else if let Some(p) = action_repack {
            self.start_repack(p);
        } else if let Some(del) = action_delete {
            self.pending_delete = Some(del);
        }
    }

    fn show_convert_page(&mut self, ui: &mut Ui) {
        let mut jump_to_studio_asset: Option<String> = None;

        ui.columns(2, |cols| {
            // Left column: Streamlined stacked full-width inputs (no clipping!) + Collapsible Advanced
            ScrollArea::vertical()
                .id_salt("convert_form_scroll")
                .show(&mut cols[0], |ui| {
                    ui.heading("Quick Convert");
                    ui.add_space(6.0);

                    // Stacked Input Pack field — full width + hover tooltip
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Input Pack").strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Browse...").clicked() {
                                let start = PathBuf::from(&self.input_path);
                                self.file_browser.open_for(
                                    FilePickerTarget::ConvertInput,
                                    if start.exists() {
                                        &start
                                    } else {
                                        &self.scan_dir
                                    },
                                    "",
                                );
                            }
                        });
                    });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.input_path)
                            .desired_width(f32::INFINITY)
                            .hint_text("/path/to/input.zip or .mcpack"),
                    )
                    .on_hover_text(self.input_path.as_str());

                    ui.add_space(6.0);

                    // Stacked Output Path field — full width + hover tooltip
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Output Path").strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Save As...").clicked() {
                                let out_p = PathBuf::from(&self.output_path);
                                let fname = out_p
                                    .file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("converted-pack.zip")
                                    .to_string();
                                self.file_browser.open_for(
                                    FilePickerTarget::ConvertOutput,
                                    out_p.parent().unwrap_or(&self.scan_dir),
                                    &fname,
                                );
                            }
                        });
                    });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.output_path)
                            .desired_width(f32::INFINITY)
                            .hint_text("/path/to/output.zip or .mcpack"),
                    )
                    .on_hover_text(self.output_path.as_str());

                    ui.add_space(8.0);
                    egui::Grid::new("convert_primary_grid")
                        .num_columns(2)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("Target Edition:");
                            egui::ComboBox::from_id_salt("target_ed")
                                .selected_text(match self.target_edition {
                                    None => "Auto (Flip)",
                                    Some(Edition::Java) => "Java Edition",
                                    Some(Edition::Bedrock) => "Bedrock Edition",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.target_edition,
                                        None,
                                        "Auto (Flip)",
                                    );
                                    if ui
                                        .selectable_value(
                                            &mut self.target_edition,
                                            Some(Edition::Java),
                                            "Java Edition",
                                        )
                                        .clicked()
                                        && !self.input_path.is_empty()
                                    {
                                        self.output_path = suggest_output_path(
                                            Path::new(&self.input_path),
                                            Edition::Java,
                                        );
                                    }
                                    if ui
                                        .selectable_value(
                                            &mut self.target_edition,
                                            Some(Edition::Bedrock),
                                            "Bedrock Edition",
                                        )
                                        .clicked()
                                        && !self.input_path.is_empty()
                                    {
                                        self.output_path = suggest_output_path(
                                            Path::new(&self.input_path),
                                            Edition::Bedrock,
                                        );
                                    }
                                });
                            ui.end_row();

                            ui.label("Target Version:");
                            egui::ComboBox::from_id_salt("target_ver")
                                .selected_text(&self.target_version)
                                .show_ui(ui, |ui| {
                                    for v in self.mappings.pack_format_versions().iter().rev() {
                                        ui.selectable_value(
                                            &mut self.target_version,
                                            v.minecraft_version.clone(),
                                            format!(
                                                "{} (Java {} / Bedrock {}.{}.{})",
                                                v.minecraft_version,
                                                v.java_pack_format,
                                                v.bedrock_min_engine_version[0],
                                                v.bedrock_min_engine_version[1],
                                                v.bedrock_min_engine_version[2]
                                            ),
                                        );
                                    }
                                });
                            ui.end_row();
                        });

                    ui.add_space(6.0);
                    // Collapsible Advanced Options so browsing/conversion stays uncluttered
                    egui::CollapsingHeader::new(
                        "Advanced Options (Rotation / Geo / Manifest / Strict)",
                    )
                    .default_open(false)
                    .show(ui, |ui| {
                        egui::Grid::new("convert_adv_grid")
                            .num_columns(2)
                            .spacing([12.0, 6.0])
                            .show(ui, |ui| {
                                ui.label("Rotation Mode:");
                                egui::ComboBox::from_id_salt("rot_mode")
                                    .selected_text(format!("{:?}", self.rotation_mode))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.rotation_mode,
                                            RotationSnapMode::Preserve,
                                            "Preserve (Exact / 26.x {x,y,z})",
                                        );
                                        ui.selectable_value(
                                            &mut self.rotation_mode,
                                            RotationSnapMode::Snap22_5,
                                            "Snap 22.5° (Legacy)",
                                        );
                                        ui.selectable_value(
                                            &mut self.rotation_mode,
                                            RotationSnapMode::Clamp45,
                                            "Clamp [-45..45]",
                                        );
                                    });
                                ui.end_row();

                                ui.label("Bedrock Geo Ver:");
                                egui::ComboBox::from_id_salt("geo_ver")
                                    .selected_text(
                                        self.bedrock_geo_version
                                            .map_or("Auto", BedrockGeoVersion::as_str),
                                    )
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.bedrock_geo_version,
                                            None,
                                            "Auto",
                                        );
                                        for v in [
                                            BedrockGeoVersion::V1_21_0,
                                            BedrockGeoVersion::V1_16_0,
                                            BedrockGeoVersion::V1_12_0,
                                            BedrockGeoVersion::V1_10_0,
                                        ] {
                                            ui.selectable_value(
                                                &mut self.bedrock_geo_version,
                                                Some(v),
                                                v.as_str(),
                                            );
                                        }
                                    });
                                ui.end_row();

                                ui.label("Bedrock Manifest:");
                                egui::ComboBox::from_id_salt("manifest_ver")
                                    .selected_text(match self.bedrock_manifest_version {
                                        None => "Auto (v2)",
                                        Some(3) => "v3 (SemVer)",
                                        Some(2) => "v2",
                                        Some(1) => "v1",
                                        Some(_) => "Auto",
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.bedrock_manifest_version,
                                            None,
                                            "Auto",
                                        );
                                        ui.selectable_value(
                                            &mut self.bedrock_manifest_version,
                                            Some(3),
                                            "v3 (SemVer)",
                                        );
                                        ui.selectable_value(
                                            &mut self.bedrock_manifest_version,
                                            Some(2),
                                            "v2",
                                        );
                                        ui.selectable_value(
                                            &mut self.bedrock_manifest_version,
                                            Some(1),
                                            "v1",
                                        );
                                    });
                                ui.end_row();

                                ui.label("Java items/*.json:");
                                egui::ComboBox::from_id_salt("items_def")
                                    .selected_text(match self.emit_java_items {
                                        None => "Auto (1.21.4+)",
                                        Some(true) => "Force On",
                                        Some(false) => "Force Off",
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.emit_java_items,
                                            None,
                                            "Auto (1.21.4+)",
                                        );
                                        ui.selectable_value(
                                            &mut self.emit_java_items,
                                            Some(true),
                                            "Force On",
                                        );
                                        ui.selectable_value(
                                            &mut self.emit_java_items,
                                            Some(false),
                                            "Force Off",
                                        );
                                    });
                                ui.end_row();

                                ui.label("Strict Mode:");
                                ui.checkbox(&mut self.strict_mode, "Fail on warnings / lossy");
                                ui.end_row();
                            });
                    });

                    ui.add_space(10.0);
                    let can_run = self.busy_message.is_none();
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                can_run,
                                egui::Button::new(
                                    RichText::new("Convert Pack").strong().size(14.0),
                                )
                                .min_size(Vec2::new(150.0, 34.0)),
                            )
                            .clicked()
                        {
                            self.start_convert();
                        }
                        if !self.input_path.trim().is_empty()
                            && ui
                                .add_enabled(
                                    can_run,
                                    egui::Button::new("Inspect in Studio")
                                        .min_size(Vec2::new(130.0, 34.0)),
                                )
                                .clicked()
                        {
                            let p = PathBuf::from(self.input_path.trim());
                            self.start_load_studio(p);
                        }
                    });
                });

            // Right column: Grouped Diagnostics & Conversion Report
            cols[1].heading("Conversion Report & Grouped Warnings");
            cols[1].add_space(4.0);

            if let Some(Ok(res)) = &self.last_conversion {
                let status = res.report.effective_status();
                let (status_color, status_label) = match status {
                    AuditStatus::Passed => (Color32::LIGHT_GREEN, "PASSED"),
                    AuditStatus::Partial => (Color32::YELLOW, "PARTIAL / WARNING"),
                    AuditStatus::Failed => (Color32::LIGHT_RED, "FAILED"),
                };
                cols[1].colored_label(
                    status_color,
                    format!(
                        "{status_label}: {} -> {} ({} ms)",
                        res.source, res.target, res.elapsed_ms
                    ),
                );
                if let Some(audit) = &res.report.semantic_audit {
                    cols[1].colored_label(status_color, &audit.summary);
                }
                cols[1]
                    .monospace(res.output_path.display().to_string())
                    .on_hover_text(res.output_path.display().to_string());
                cols[1].horizontal_wrapped(|ui| {
                    ui.label(format!("Converted: {}", res.report.converted_count));
                    ui.separator();
                    ui.label(format!("Pass-through: {}", res.report.passthrough_count));
                    ui.separator();
                    ui.colored_label(
                        Color32::YELLOW,
                        format!("Lossy: {}", res.report.lossy_count),
                    );
                    ui.separator();
                    ui.colored_label(
                        Color32::LIGHT_RED,
                        format!("Skipped: {}", res.report.skipped_count),
                    );
                    if ui.small_button("Export JSON").clicked() {
                        let out = self.scan_dir.join("conversion_report.json");
                        if let Ok(()) = export_report_json(&res.report, &out) {
                            self.status_banner = format!("Exported -> {}", out.display());
                        }
                    }
                });
                cols[1].separator();
            } else if let Some(Err(err)) = &self.last_conversion {
                cols[1].colored_label(Color32::LIGHT_RED, format!("FAILED: {err}"));
                cols[1].separator();
            }

            // Severity filter pills
            cols[1].horizontal_wrapped(|ui| {
                for f in DiagnosticFilter::ALL {
                    let count = self.review_index.count_matching(f);
                    let label = format!("{} ({count})", f.label());
                    if ui
                        .selectable_label(self.diagnostic_filter == f, label)
                        .clicked()
                    {
                        self.diagnostic_filter = f;
                    }
                }
            });
            cols[1].add_space(4.0);

            ScrollArea::vertical()
                .id_salt("grouped_diag_scroll")
                .show(&mut cols[1], |ui| {
                    if self.review_index.groups.is_empty() {
                        ui.label("No diagnostics yet. Run a conversion or open a pack in Studio.");
                    } else {
                        jump_to_studio_asset = render_grouped_diagnostics(
                            ui,
                            &self.review_index,
                            self.diagnostic_filter,
                            self.loaded_pack.is_some(),
                        );
                    }
                });
        });

        if let Some(asset) = jump_to_studio_asset {
            self.open_asset_by_report_hint(&asset);
        }
    }

    fn show_batch_page(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            let enabled = self.busy_message.is_none();
            if ui
                .add_enabled(enabled, egui::Button::new("Run Round-Trip Audit"))
                .clicked()
            {
                let paths: Vec<PathBuf> = self
                    .selected_or_all_pack_paths()
                    .into_iter()
                    .map(|(p, _)| p)
                    .collect();
                self.start_audit(paths);
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Batch Convert -> ./converted/"))
                .clicked()
            {
                self.start_batch_convert();
            }
            if !self.audit_outcomes.is_empty() && ui.button("Export Audit JSON").clicked() {
                let out = self.scan_dir.join("audit_results.json");
                if let Ok(()) = export_audit_json(&self.audit_outcomes, &out) {
                    self.status_banner = format!("Exported audit -> {}", out.display());
                }
            }
        });

        if let Some((done, total, label)) = &self.batch_progress {
            let frac = if *total > 0 {
                *done as f32 / *total as f32
            } else {
                0.0
            };
            ui.add(
                egui::ProgressBar::new(frac)
                    .text(format!("{done}/{total} — {label}"))
                    .animate(self.busy_message.is_some()),
            );
        }
        ui.separator();

        ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("audit_table")
                .striped(true)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.strong("Status");
                    ui.strong("Pack");
                    ui.strong("Direction");
                    ui.strong("Conv / Lossy");
                    ui.strong("Time");
                    ui.strong("Details");
                    ui.end_row();

                    for o in &self.audit_outcomes {
                        match o.status {
                            AuditStatus::Passed => {
                                ui.colored_label(Color32::LIGHT_GREEN, "PASS");
                            }
                            AuditStatus::Partial => {
                                ui.colored_label(Color32::YELLOW, "WARN");
                            }
                            AuditStatus::Failed => {
                                ui.colored_label(Color32::LIGHT_RED, "FAIL");
                            }
                        }
                        ui.label(&o.pack_name);
                        ui.label(format!("{} -> {}", o.source_edition, o.target_edition));
                        let counts = match &o.forward_report {
                            Some(r) => format!("{} / {}", r.converted_count, r.lossy_count),
                            None => String::from("-"),
                        };
                        ui.label(counts);
                        ui.label(format!("{} ms", o.elapsed_ms));
                        ui.label(&o.detail);
                        ui.end_row();
                    }
                });
        });
    }

    fn show_compare_page(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Pack A:");
            ui.add(egui::TextEdit::singleline(&mut self.diff_path_a).desired_width(240.0))
                .on_hover_text(self.diff_path_a.as_str());
            if ui.small_button("Browse A").clicked() {
                let start = self.scan_dir.clone();
                self.file_browser
                    .open_for(FilePickerTarget::DiffPackA, &start, "");
            }
            ui.label("Pack B:");
            ui.add(egui::TextEdit::singleline(&mut self.diff_path_b).desired_width(240.0))
                .on_hover_text(self.diff_path_b.as_str());
            if ui.small_button("Browse B").clicked() {
                let start = self.scan_dir.clone();
                self.file_browser
                    .open_for(FilePickerTarget::DiffPackB, &start, "");
            }
            let enabled = self.busy_message.is_none()
                && !self.diff_path_a.trim().is_empty()
                && !self.diff_path_b.trim().is_empty();
            if ui
                .add_enabled(enabled, egui::Button::new("Compare"))
                .clicked()
            {
                self.start_diff(
                    PathBuf::from(self.diff_path_a.trim()),
                    PathBuf::from(self.diff_path_b.trim()),
                );
            }
        });
        ui.separator();

        ScrollArea::vertical().show(ui, |ui| match &self.pack_diff {
            None => {
                ui.label("Select Pack A and Pack B (or check 2 packs in Packs Manager and click 'Compare Checked').");
            }
            Some(Err(err)) => {
                ui.colored_label(Color32::LIGHT_RED, format!("Diff error: {err}"));
            }
            Some(Ok(d)) => {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        Color32::from_rgb(80, 200, 255),
                        format!("A: {} ({})", d.name_a, d.edition_a),
                    );
                    ui.label("vs");
                    ui.colored_label(
                        Color32::from_rgb(100, 225, 130),
                        format!("B: {} ({})", d.name_b, d.edition_b),
                    );
                });
                ui.add_space(6.0);

                for (title, cat) in [
                    ("Textures", &d.textures),
                    ("3D Models", &d.models),
                    ("Attachables", &d.attachables),
                    ("Sound Files", &d.sound_files),
                    ("Locales", &d.locales),
                ] {
                    ui.collapsing(
                        format!(
                            "{title} — {} shared | -{} only A | +{} only B",
                            cat.shared_count,
                            cat.only_in_a.len(),
                            cat.only_in_b.len()
                        ),
                        |ui| {
                            for id in &cat.only_in_a {
                                ui.colored_label(Color32::LIGHT_RED, format!("- [A] {id}"));
                            }
                            for id in &cat.only_in_b {
                                ui.colored_label(Color32::LIGHT_GREEN, format!("+ [B] {id}"));
                            }
                        },
                    );
                }
            }
        });
    }

    fn show_versions_page(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Filter Version:");
            ui.add(egui::TextEdit::singleline(&mut self.version_filter).desired_width(160.0));
        });
        ui.separator();

        let needle = self.version_filter.trim().to_ascii_lowercase();
        ScrollArea::vertical()
            .max_height(250.0)
            .id_salt("versions_scroll")
            .show(ui, |ui| {
                egui::Grid::new("versions_grid")
                    .striped(true)
                    .spacing([14.0, 5.0])
                    .show(ui, |ui| {
                        ui.strong("Version");
                        ui.strong("Java pack_format");
                        ui.strong("Bedrock Engine");
                        ui.strong("Default Geo");
                        ui.end_row();
                        for v in self.mappings.pack_format_versions().iter().rev() {
                            if !needle.is_empty()
                                && !v.minecraft_version.to_ascii_lowercase().contains(&needle)
                                && !v.java_pack_format.to_string().contains(&needle)
                            {
                                continue;
                            }
                            let b = v.bedrock_min_engine_version;
                            ui.label(&v.minecraft_version);
                            ui.label(v.java_pack_format.to_string());
                            ui.label(format!("{}.{}.{}", b[0], b[1], b[2]));
                            ui.label(BedrockGeoVersion::for_engine_version(b).as_str());
                            ui.end_row();
                        }
                    });
            });

        ui.separator();
        ui.heading(format!("Session Activity Log ({})", self.logs.len()));
        ScrollArea::vertical()
            .id_salt("gui_logs_scroll")
            .show(ui, |ui| {
                for line in self.logs.iter().rev() {
                    ui.monospace(line);
                }
            });
    }

    // =========================================================================
    // STUDIO MODE (VS Code Workbench + Blockbench 3D Model/Asset Inspector)
    // =========================================================================

    fn render_studio_mode(&mut self, ctx: &Context) {
        let palette = self.palette;

        // 0. Material 3 Studio Edition Bar (Separated Java Studio vs Bedrock Studio + Fresh Refresh)
        egui::TopBottomPanel::top("studio_edition_bar")
            .frame(
                egui::Frame::new()
                    .fill(palette.surface_container_low)
                    .stroke(Stroke::new(1.0, palette.outline_variant))
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let java_label = match self.edition_loaded_label(Edition::Java) {
                        Some(name) => format!("☕ Java Studio • {name}"),
                        None => String::from("☕ Java Studio"),
                    };
                    let bedrock_label = match self.edition_loaded_label(Edition::Bedrock) {
                        Some(name) => format!("🪨 Bedrock Studio • {name}"),
                        None => String::from("🪨 Bedrock Studio"),
                    };

                    if m3_segmented_button(
                        ui,
                        self.studio_edition == Edition::Java,
                        &java_label,
                        palette.java_container,
                        palette.on_java_container,
                        &palette,
                    )
                    .on_hover_text("Switch to isolated Java Edition Studio workspace (pack.mcmeta / assets/minecraft/)")
                    .clicked()
                    {
                        self.switch_studio_edition(Edition::Java);
                    }

                    if m3_segmented_button(
                        ui,
                        self.studio_edition == Edition::Bedrock,
                        &bedrock_label,
                        palette.bedrock_container,
                        palette.on_bedrock_container,
                        &palette,
                    )
                    .on_hover_text("Switch to isolated Bedrock Edition Studio workspace (manifest.json / textures/ / models/entity/)")
                    .clicked()
                    {
                        self.switch_studio_edition(Edition::Bedrock);
                    }

                    ui.separator();
                    let schema_hint = match self.studio_edition {
                        Edition::Java => "pack.mcmeta • assets/minecraft/{models,textures,lang}",
                        Edition::Bedrock => "manifest.json • models/entity/*.geo.json • textures/",
                    };
                    ui.label(
                        RichText::new(schema_hint)
                            .small()
                            .color(palette.on_surface_variant),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let enabled = self.busy_message.is_none();
                        if m3_tonal_button(ui, enabled, "↻ Fresh Refresh", &palette)
                            .on_hover_text(
                                "Reload active pack fresh from disk, clear texture/icon caches & rescan workspace (F5)",
                            )
                            .clicked()
                        {
                            self.fresh_refresh_studio();
                        }
                        if ui
                            .button("Open Pack...")
                            .on_hover_text("Open any Java or Bedrock .zip / .mcpack / folder")
                            .clicked()
                        {
                            let start = self.scan_dir.clone();
                            self.file_browser
                                .open_for(FilePickerTarget::ConvertInput, &start, "");
                        }
                    });
                });
            });

        // 1. Material 3 Navigation Rail (Left Activity Bar)
        egui::SidePanel::left("studio_activity_bar")
            .resizable(false)
            .exact_width(50.0)
            .frame(m3_rail_frame(&palette))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(4.0);
                    for sec in ActivitySection::ALL {
                        let active = self.show_sidebar && self.activity_section == sec;
                        let btn = egui::Button::new(RichText::new(sec.icon()).size(16.0))
                            .fill(if active {
                                palette.primary_container
                            } else {
                                Color32::TRANSPARENT
                            })
                            .stroke(if active {
                                Stroke::new(1.0, palette.primary)
                            } else {
                                Stroke::NONE
                            })
                            .corner_radius(egui::CornerRadius::same(14));
                        if ui
                            .add_sized(Vec2::splat(34.0), btn)
                            .on_hover_text(sec.tooltip())
                            .clicked()
                        {
                            if self.activity_section == sec {
                                self.show_sidebar = !self.show_sidebar;
                            } else {
                                self.activity_section = sec;
                                self.show_sidebar = true;
                            }
                        }
                        ui.add_space(4.0);
                    }

                    ui.separator();
                    let refresh_btn = egui::Button::new(RichText::new("↻").size(16.0))
                        .fill(palette.surface_container_high)
                        .corner_radius(egui::CornerRadius::same(14));
                    if ui
                        .add_enabled(self.busy_message.is_none(), refresh_btn)
                        .on_hover_text("Fresh Refresh Studio (F5)")
                        .clicked()
                    {
                        self.fresh_refresh_studio();
                    }
                    ui.add_space(4.0);

                    let panel_btn = egui::Button::new(RichText::new("⬓").size(15.0))
                        .fill(if self.show_bottom_panel {
                            palette.secondary_container
                        } else {
                            Color32::TRANSPARENT
                        })
                        .corner_radius(egui::CornerRadius::same(14));
                    if ui
                        .add_sized(Vec2::splat(34.0), panel_btn)
                        .on_hover_text("Toggle Bottom Panel (Summary, Warnings, Export)")
                        .clicked()
                    {
                        self.show_bottom_panel = !self.show_bottom_panel;
                    }
                });
            });

        // 2. Collapsible Left Side Bar (Explorer Tree / Grouped Warnings / Batch & Presets)
        if self.show_sidebar {
            egui::SidePanel::left("studio_sidebar")
                .resizable(true)
                .default_width(265.0)
                .min_width(190.0)
                .max_width(430.0)
                .frame(m3_rail_frame(&palette))
                .show(ctx, |ui| match self.activity_section {
                    ActivitySection::Explorer => self.show_studio_explorer_sidebar(ui),
                    ActivitySection::Warnings => self.show_studio_warnings_sidebar(ui),
                    ActivitySection::BatchPresets => self.show_studio_presets_sidebar(ui),
                });
        }

        // 3. Collapsible Bottom Panel (Summary Cards, Grouped Warnings, Manual Override Export)
        if self.show_bottom_panel {
            egui::TopBottomPanel::bottom("studio_bottom_panel")
                .resizable(true)
                .default_height(155.0)
                .min_height(90.0)
                .max_height(320.0)
                .frame(m3_rail_frame(&palette))
                .show(ctx, |ui| {
                    self.show_studio_bottom_panel(ui);
                });
        }

        // 4. Central Tabbed Workbench (Model 3D + Inspector / Texture + Grid / Text + JSON Editor)
        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_studio_editor_area(ctx, ui);
        });
    }

    fn show_studio_explorer_sidebar(&mut self, ui: &mut Ui) {
        let palette = self.palette;
        let active_ed = self.studio_edition;
        let (ed_bg, ed_fg, _) = palette.edition_colors(active_ed);
        let ed_badge = match active_ed {
            Edition::Java => "☕ JAVA EXPLORER",
            Edition::Bedrock => "🪨 BEDROCK EXPLORER",
        };

        ui.horizontal(|ui| {
            m3_pill_badge(ui, ed_badge, ed_bg, ed_fg);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(self.busy_message.is_none(), egui::Button::new("↻").small())
                    .on_hover_text("Fresh Refresh current pack & workspace (F5)")
                    .clicked()
                {
                    self.fresh_refresh_studio();
                }
            });
        });

        // Separate Java vs Bedrock workspace pack quick-switcher
        let matching_packs: Vec<(String, PathBuf)> = self
            .packs
            .iter()
            .filter(|p| p.summary.as_ref().is_ok_and(|s| s.edition == active_ed))
            .map(|p| (p.file_name.clone(), p.path.clone()))
            .collect();
        let other_ed = match active_ed {
            Edition::Java => Edition::Bedrock,
            Edition::Bedrock => Edition::Java,
        };
        let other_count = self
            .packs
            .iter()
            .filter(|p| p.summary.as_ref().is_ok_and(|s| s.edition == other_ed))
            .count();

        if !matching_packs.is_empty() {
            let mut open_p: Option<PathBuf> = None;
            let active_path = self.loaded_pack.as_ref().map(|l| l.path.clone());
            egui::CollapsingHeader::new(format!(
                "{} Packs in Workspace ({})",
                active_ed,
                matching_packs.len()
            ))
            .id_salt(format!("studio_packs_{active_ed}"))
            .default_open(self.loaded_pack.is_none())
            .show(ui, |ui| {
                for (name, path) in &matching_packs {
                    let is_curr = active_path.as_ref() == Some(path);
                    let icon = match active_ed {
                        Edition::Java => "☕",
                        Edition::Bedrock => "🪨",
                    };
                    if ui
                        .selectable_label(is_curr, format!("{icon} {name}"))
                        .clicked()
                    {
                        open_p = Some(path.clone());
                    }
                }
            });
            if let Some(path) = open_p {
                self.start_load_studio(path);
            }
        }

        // Filter pills: All vs Needs Review vs Manual Overrides
        ui.horizontal_wrapped(|ui| {
            for f in SideBarFilter::ALL {
                if ui
                    .selectable_label(self.sidebar_filter == f, f.label())
                    .clicked()
                {
                    self.sidebar_filter = f;
                }
            }
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.studio_search)
                .desired_width(f32::INFINITY)
                .hint_text("Search assets..."),
        );
        ui.separator();

        let Some(loaded) = self.loaded_pack.clone() else {
            m3_card_frame(&palette).show(ui, |ui| {
                ui.label(
                    RichText::new(format!("No {active_ed} pack open in Studio."))
                        .strong(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(match active_ed {
                        Edition::Java => {
                            "Select a Java pack (.zip / folder with pack.mcmeta) above or click Open Pack."
                        }
                        Edition::Bedrock => {
                            "Select a Bedrock pack (.mcpack / folder with manifest.json) above or click Open Pack."
                        }
                    })
                    .small()
                    .color(palette.on_surface_variant),
                );
                if other_count > 0 {
                    ui.add_space(6.0);
                    let switch_label = format!("Switch to {other_ed} Studio ({other_count} pack(s))");
                    if m3_tonal_button(ui, true, &switch_label, &palette).clicked() {
                        self.switch_studio_edition(other_ed);
                    }
                }
            });
            return;
        };

        let needle = self.studio_search.trim().to_ascii_lowercase();
        let filter = self.sidebar_filter;
        let mut clicked_tab: Option<StudioEditorTab> = None;

        ScrollArea::vertical()
            .id_salt("studio_tree_scroll")
            .show(ui, |ui| {
                // Pack Metadata / Manifest (edition-specific label!)
                let meta_ov = self
                    .overrides
                    .is_text_overridden(&TextAssetTarget::MetadataJson.key());
                if filter != SideBarFilter::NeedsReview
                    && (filter != SideBarFilter::ManualOverrides || meta_ov)
                {
                    let base_name = match loaded.edition {
                        Edition::Java => "📝 pack.mcmeta (Metadata)",
                        Edition::Bedrock => "📝 manifest.json (Metadata)",
                    };
                    let label = if meta_ov {
                        format!("{base_name}  ✎")
                    } else {
                        String::from(base_name)
                    };
                    if ui.selectable_label(false, label).clicked() {
                        clicked_tab = Some(StudioEditorTab::Text(TextAssetTarget::MetadataJson));
                    }
                }

                // 3D Models tree
                let model_ids: Vec<String> = loaded
                    .ir
                    .models
                    .keys()
                    .filter(|id| {
                        let m_search =
                            needle.is_empty() || id.to_ascii_lowercase().contains(&needle);
                        let m_filter = match filter {
                            SideBarFilter::All => true,
                            SideBarFilter::NeedsReview => self.review_index.asset_needs_review(id),
                            SideBarFilter::ManualOverrides => {
                                self.overrides.is_model_overridden(id)
                            }
                        };
                        m_search && m_filter
                    })
                    .cloned()
                    .collect();

                let models_header = match loaded.edition {
                    Edition::Java => format!("🧊 Java Models ({})", model_ids.len()),
                    Edition::Bedrock => format!("🧊 Bedrock Geo Models ({})", model_ids.len()),
                };
                egui::CollapsingHeader::new(models_header)
                    .default_open(true)
                    .show(ui, |ui| {
                        for id in &model_ids {
                            let warn = self.review_index.asset_needs_review(id);
                            let edited = self.overrides.is_model_overridden(id);
                            let mut text = id.clone();
                            if warn {
                                text.push_str(" ⚠");
                            }
                            if edited {
                                text.push_str(" ✎");
                            }
                            let rt = if edited {
                                RichText::new(text).color(palette.primary)
                            } else if warn {
                                RichText::new(text).color(palette.warning)
                            } else {
                                RichText::new(text)
                            };
                            if ui.selectable_label(false, rt).clicked() {
                                clicked_tab = Some(StudioEditorTab::Model(id.clone()));
                            }
                        }
                    });

                // Textures tree (with ResolutionTier badge)
                let tex_ids: Vec<String> = loaded
                    .ir
                    .textures
                    .keys()
                    .filter(|id| {
                        let m_search =
                            needle.is_empty() || id.to_ascii_lowercase().contains(&needle);
                        let m_filter = match filter {
                            SideBarFilter::All => true,
                            SideBarFilter::NeedsReview => self.review_index.asset_needs_review(id),
                            SideBarFilter::ManualOverrides => {
                                self.overrides.is_texture_overridden(id)
                            }
                        };
                        m_search && m_filter
                    })
                    .cloned()
                    .collect();

                let tex_header = match loaded.edition {
                    Edition::Java => format!("🖼 Java Textures ({})", tex_ids.len()),
                    Edition::Bedrock => format!("🖼 Bedrock Textures ({})", tex_ids.len()),
                };
                egui::CollapsingHeader::new(tex_header)
                    .default_open(true)
                    .show(ui, |ui| {
                        for id in &tex_ids {
                            let warn = self.review_index.asset_needs_review(id);
                            let edited = self.overrides.is_texture_overridden(id);
                            let tier_suffix = loaded
                                .ir
                                .textures
                                .get(id)
                                .and_then(|t| t.resolution_tier)
                                .map(|tier| format!(" [{tier}]"))
                                .unwrap_or_default();
                            let mut text = format!("{id}{tier_suffix}");
                            if warn {
                                text.push_str(" ⚠");
                            }
                            if edited {
                                text.push_str(" ✎");
                            }
                            let rt = if edited {
                                RichText::new(text).color(palette.primary)
                            } else if warn {
                                RichText::new(text).color(palette.warning)
                            } else {
                                RichText::new(text)
                            };
                            if ui.selectable_label(false, rt).clicked() {
                                clicked_tab = Some(StudioEditorTab::Texture(id.clone()));
                            }
                        }
                    });

                // Attachables tree (Bedrock-native)
                if !loaded.ir.attachables.is_empty() {
                    let att_ids: Vec<String> = loaded
                        .ir
                        .attachables
                        .keys()
                        .filter(|id| needle.is_empty() || id.to_ascii_lowercase().contains(&needle))
                        .cloned()
                        .collect();
                    egui::CollapsingHeader::new(format!("🎭 Attachables ({})", att_ids.len()))
                        .default_open(false)
                        .show(ui, |ui| {
                            for id in &att_ids {
                                if ui.selectable_label(false, id).clicked() {
                                    clicked_tab = Some(StudioEditorTab::Text(
                                        TextAssetTarget::AttachableJson(id.clone()),
                                    ));
                                }
                            }
                        });
                }

                // Locales tree
                if !loaded.ir.locales.is_empty() {
                    let loc_ids: Vec<String> = loaded
                        .ir
                        .locales
                        .keys()
                        .filter(|id| needle.is_empty() || id.to_ascii_lowercase().contains(&needle))
                        .cloned()
                        .collect();
                    let loc_header = match loaded.edition {
                        Edition::Java => format!("🌐 Java Locales ({})", loc_ids.len()),
                        Edition::Bedrock => format!("🌐 Bedrock Texts ({})", loc_ids.len()),
                    };
                    egui::CollapsingHeader::new(loc_header)
                        .default_open(false)
                        .show(ui, |ui| {
                            for code in &loc_ids {
                                let ext = match loaded.edition {
                                    Edition::Java => "json",
                                    Edition::Bedrock => "lang",
                                };
                                if ui
                                    .selectable_label(false, format!("{code}.{ext}"))
                                    .clicked()
                                {
                                    clicked_tab = Some(StudioEditorTab::Text(
                                        TextAssetTarget::LocaleJson(code.clone()),
                                    ));
                                }
                            }
                        });
                }

                // Raw / Passthrough files
                if !loaded.ir.passthrough_files.is_empty() {
                    let raw_ids: Vec<String> = loaded
                        .ir
                        .passthrough_files
                        .keys()
                        .filter(|id| needle.is_empty() || id.to_ascii_lowercase().contains(&needle))
                        .cloned()
                        .collect();
                    egui::CollapsingHeader::new(format!("📄 Raw / JSON ({})", raw_ids.len()))
                        .default_open(false)
                        .show(ui, |ui| {
                            for p in &raw_ids {
                                if ui.selectable_label(false, p).clicked() {
                                    clicked_tab = Some(StudioEditorTab::Text(
                                        TextAssetTarget::Passthrough(p.clone()),
                                    ));
                                }
                            }
                        });
                }
            });

        if let Some(tab) = clicked_tab {
            self.open_or_focus_tab(tab);
        }
    }

    fn show_studio_warnings_sidebar(&mut self, ui: &mut Ui) {
        ui.strong("WARNINGS & NEEDS REVIEW");
        ui.horizontal_wrapped(|ui| {
            for f in [
                DiagnosticFilter::NeedsReview,
                DiagnosticFilter::Lossy,
                DiagnosticFilter::All,
            ] {
                let count = self.review_index.count_matching(f);
                if ui
                    .selectable_label(
                        self.diagnostic_filter == f,
                        format!("{} ({count})", f.label()),
                    )
                    .clicked()
                {
                    self.diagnostic_filter = f;
                }
            }
        });
        ui.separator();

        let mut open_asset: Option<String> = None;
        ScrollArea::vertical()
            .id_salt("studio_warn_sidebar")
            .show(ui, |ui| {
                open_asset = render_grouped_diagnostics(
                    ui,
                    &self.review_index,
                    self.diagnostic_filter,
                    self.loaded_pack.is_some(),
                );
            });
        if let Some(asset) = open_asset {
            self.open_asset_by_report_hint(&asset);
        }
    }

    fn show_studio_presets_sidebar(&mut self, ui: &mut Ui) {
        ui.strong("PRESETS & BATCH");
        ui.separator();

        ui.label(RichText::new("Quick Target Presets:").small().strong());
        if ui.button("Java 26.3 (97.1 + Euler + Shade)").clicked() {
            self.target_edition = Some(Edition::Java);
            self.target_version = String::from("26.3");
            self.rotation_mode = RotationSnapMode::Preserve;
            self.status_banner = String::from("Preset: Java 26.3 (pack_format 97.1)");
        }
        if ui.button("Java 26.2 (88 + Multi-Axis)").clicked() {
            self.target_edition = Some(Edition::Java);
            self.target_version = String::from("26.2");
            self.rotation_mode = RotationSnapMode::Preserve;
            self.status_banner = String::from("Preset: Java 26.2 (pack_format 88)");
        }
        if ui.button("Bedrock 26.50 (v3 SemVer + 1.21)").clicked() {
            self.target_edition = Some(Edition::Bedrock);
            self.target_version = String::from("26.50");
            self.bedrock_manifest_version = Some(3);
            self.bedrock_geo_version = Some(BedrockGeoVersion::V1_21_0);
            self.status_banner = String::from("Preset: Bedrock 26.50 (v3 + 1.21.0 geo)");
        }
        if ui.button("Legacy Java 1.8.9 (Snap 22.5°)").clicked() {
            self.target_edition = Some(Edition::Java);
            self.target_version = String::from("1.8.9");
            self.rotation_mode = RotationSnapMode::Snap22_5;
            self.status_banner = String::from("Preset: Legacy Java 1.8.9");
        }

        ui.separator();
        ui.label(RichText::new("Workspace Actions:").small().strong());
        let enabled = self.busy_message.is_none();
        if ui
            .add_enabled(enabled, egui::Button::new("Run Round-Trip Audit"))
            .clicked()
        {
            let paths: Vec<PathBuf> = self
                .selected_or_all_pack_paths()
                .into_iter()
                .map(|(p, _)| p)
                .collect();
            self.mode = WorkspaceMode::Quick;
            self.page = GuiPage::BatchAudit;
            self.start_audit(paths);
        }
        if ui
            .add_enabled(enabled, egui::Button::new("Batch Convert All"))
            .clicked()
        {
            self.mode = WorkspaceMode::Quick;
            self.page = GuiPage::BatchAudit;
            self.start_batch_convert();
        }
    }

    fn show_studio_bottom_panel(&mut self, ui: &mut Ui) {
        let palette = self.palette;
        let mut open_asset: Option<String> = None;

        ui.horizontal_wrapped(|ui| {
            let (ed_bg, ed_fg, _) = palette.edition_colors(self.studio_edition);
            m3_pill_badge(
                ui,
                match self.studio_edition {
                    Edition::Java => "☕ JAVA CONSOLE",
                    Edition::Bedrock => "🪨 BEDROCK CONSOLE",
                },
                ed_bg,
                ed_fg,
            );
            ui.separator();
            let lossy = self.review_index.count_matching(DiagnosticFilter::Lossy);
            let warn = self.review_index.count_matching(DiagnosticFilter::Warning);
            let ov_count = self.overrides.total_override_count();
            ui.colored_label(palette.warning, format!("Lossy: {lossy}"));
            ui.colored_label(palette.error, format!("Warnings/Skip: {warn}"));
            ui.colored_label(palette.primary, format!("Manual Overrides: {ov_count}"));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = self.busy_message.is_none() && self.loaded_pack.is_some();
                let export_label = match self.studio_edition {
                    Edition::Java => "Export to Bedrock (with Overrides)",
                    Edition::Bedrock => "Export to Java (with Overrides)",
                };
                if m3_filled_button(ui, enabled, export_label, &palette).clicked() {
                    self.start_export_studio_ir();
                }
                if m3_tonal_button(ui, ov_count > 0, "Export Overrides JSON", &palette).clicked() {
                    let start = self.scan_dir.clone();
                    self.file_browser.open_for(
                        FilePickerTarget::ExportOverrides,
                        &start,
                        "studio_overrides.json",
                    );
                }
            });
        });
        ui.separator();

        ScrollArea::vertical()
            .id_salt("studio_bottom_diag")
            .show(ui, |ui| {
                if !self.review_index.groups.is_empty() {
                    open_asset = render_grouped_diagnostics(
                        ui,
                        &self.review_index,
                        DiagnosticFilter::NeedsReview,
                        self.loaded_pack.is_some(),
                    );
                } else {
                    for line in self.logs.iter().rev().take(8) {
                        ui.monospace(line);
                    }
                }
            });

        if let Some(asset) = open_asset {
            self.open_asset_by_report_hint(&asset);
        }
    }

    fn show_studio_editor_area(&mut self, ctx: &Context, ui: &mut Ui) {
        let palette = self.palette;
        if self.loaded_pack.is_none() {
            let active_ed = self.studio_edition;
            let (ed_bg, ed_fg, _) = palette.edition_colors(active_ed);
            ui.centered_and_justified(|ui| {
                m3_card_frame(&palette).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        m3_pill_badge(
                            ui,
                            match active_ed {
                                Edition::Java => "☕ JAVA EDITION STUDIO",
                                Edition::Bedrock => "🪨 BEDROCK EDITION STUDIO",
                            },
                            ed_bg,
                            ed_fg,
                        );
                        ui.add_space(8.0);
                        ui.heading(format!("mc-rp-converter {active_ed} Studio"));
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(match active_ed {
                                Edition::Java => {
                                    "Inspect & edit Java Edition packs (pack.mcmeta, JSON 3D models, up to 2048x textures & lang/*.json)."
                                }
                                Edition::Bedrock => {
                                    "Inspect & edit Bedrock Edition packs (manifest.json, .geo.json models, flipbooks, attachables & texts/*.lang)."
                                }
                            })
                            .color(palette.on_surface_variant),
                        );
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if m3_filled_button(ui, true, "Open Pack...", &palette).clicked() {
                                let start = self.scan_dir.clone();
                                self.file_browser.open_for(
                                    FilePickerTarget::ConvertInput,
                                    &start,
                                    "",
                                );
                            }
                            if m3_tonal_button(
                                ui,
                                self.busy_message.is_none(),
                                "↻ Fresh Refresh Workspace",
                                &palette,
                            )
                            .clicked()
                            {
                                self.fresh_refresh_studio();
                            }
                        });
                    });
                });
            });
            return;
        }

        // VS Code-style Editor Tab Bar
        let mut close_idx: Option<usize> = None;
        ui.horizontal_wrapped(|ui| {
            for (idx, tab) in self.open_tabs.iter().enumerate() {
                let active = idx == self.active_tab_idx;
                let key = tab.override_key();
                let is_mod = match tab {
                    StudioEditorTab::Model(id) => self.overrides.is_model_overridden(id),
                    StudioEditorTab::Texture(id) => self.overrides.is_texture_overridden(id),
                    StudioEditorTab::Text(_) => self.overrides.is_text_overridden(&key),
                };
                let label = if is_mod {
                    format!("{} ●", tab.title())
                } else {
                    tab.title()
                };
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(active, label).clicked() {
                            self.active_tab_idx = idx;
                            self.selected_element_idx = Some(0);
                        }
                        if ui.small_button("×").clicked() {
                            close_idx = Some(idx);
                        }
                    });
                });
            }
        });
        if let Some(idx) = close_idx {
            self.open_tabs.remove(idx);
            if self.active_tab_idx >= self.open_tabs.len() {
                self.active_tab_idx = self.open_tabs.len().saturating_sub(1);
            }
        }
        ui.separator();

        let Some(active_tab) = self.open_tabs.get(self.active_tab_idx).cloned() else {
            ui.centered_and_justified(|ui| {
                ui.label("Click any 3D model, texture, locale, or JSON file in the left Explorer to open a tab.");
            });
            return;
        };

        match active_tab {
            StudioEditorTab::Model(model_id) => self.show_studio_model_tab(ui, &model_id),
            StudioEditorTab::Texture(tex_id) => self.show_studio_texture_tab(ctx, ui, &tex_id),
            StudioEditorTab::Text(target) => self.show_studio_text_tab(ui, &target),
        }
    }

    fn show_studio_model_tab(&mut self, ui: &mut Ui, model_id: &str) {
        let Some(orig_model) = self
            .loaded_pack
            .as_ref()
            .and_then(|l| l.ir.models.get(model_id))
            .cloned()
        else {
            ui.label(format!("Model '{model_id}' not found."));
            return;
        };

        let mut model = orig_model.clone();
        let is_overridden = self.overrides.is_model_overridden(model_id);
        let mut modified = false;
        let mut reset_requested = false;

        // Top model status bar (Edition Pill + Manual Override badge + Reset to Auto)
        let palette = self.palette;
        let active_ed = self
            .loaded_pack
            .as_ref()
            .map_or(self.studio_edition, |l| l.edition);
        let (ed_bg, ed_fg, _) = palette.edition_colors(active_ed);
        ui.horizontal_wrapped(|ui| {
            m3_pill_badge(
                ui,
                match active_ed {
                    Edition::Java => "☕ JAVA MODEL",
                    Edition::Bedrock => "🪨 BEDROCK GEO",
                },
                ed_bg,
                ed_fg,
            );
            ui.label(RichText::new(model_id).strong().size(14.0));
            if is_overridden {
                ui.colored_label(palette.primary, "✎ MANUAL OVERRIDE");
                if ui.small_button("Reset to Auto").clicked() {
                    reset_requested = true;
                }
            } else {
                ui.label(
                    RichText::new("Auto-Converted")
                        .small()
                        .color(palette.on_surface_variant),
                );
            }
            ui.separator();
            ui.label(
                RichText::new(format!(
                    "Tex: {}x{} | Category: {:?}",
                    model.texture_size[0], model.texture_size[1], model.category
                ))
                .small()
                .color(palette.on_surface_variant),
            );
        });

        if reset_requested {
            if let Some(reverted) = self.overrides.reset_model_to_auto(model_id)
                && let Some(loaded) = &mut self.loaded_pack
            {
                loaded.ir.models.insert(model_id.to_string(), reverted);
                self.status_banner = format!("Reset model '{model_id}' to auto-converted state");
            }
            return;
        }

        // Top half: Interactive Blockbench-style 3D Viewport
        let sel_idx = self
            .selected_element_idx
            .filter(|&i| i < model.elements.len());
        self.viewport_3d.show(ui, &model, sel_idx, 270.0);
        ui.add_space(4.0);

        // Bottom half: Left = Bone/Element Outliner | Right = Live Coordinate/Rotation/UV Inspector
        ui.columns(2, |cols| {
            // Bone / Element Outliner
            cols[0].strong(format!("Outliner ({} cubes)", model.elements.len()));
            ScrollArea::vertical()
                .id_salt("model_outliner_scroll")
                .max_height(240.0)
                .show(&mut cols[0], |ui| {
                    for (idx, el) in model.elements.iter().enumerate() {
                        let grp = el.group.as_deref().unwrap_or("root");
                        let rot_tag = if el.rotation.is_some() { " ⟳" } else { "" };
                        let label = format!("[{idx:02}] {grp}{rot_tag}");
                        let selected = self.selected_element_idx == Some(idx);
                        if ui.selectable_label(selected, label).clicked() {
                            self.selected_element_idx = Some(idx);
                        }
                    }
                });

            // Blockbench-style Live Property Inspector
            cols[1].strong("Cube Property Inspector");
            ScrollArea::vertical()
                .id_salt("model_props_scroll")
                .max_height(240.0)
                .show(&mut cols[1], |ui| {
                    let Some(idx) = self
                        .selected_element_idx
                        .filter(|&i| i < model.elements.len())
                    else {
                        ui.label("Select a cube in the Outliner to inspect or edit.");
                        return;
                    };
                    let el = &mut model.elements[idx];
                    let (b_orig, b_size) = java_to_bedrock_box(el.from, el.to);

                    // Java From / To
                    ui.horizontal(|ui| {
                        ui.label("From (Java):");
                        for axis in 0..3 {
                            if ui
                                .add(
                                    egui::DragValue::new(&mut el.from[axis])
                                        .speed(0.1)
                                        .range(-16.0..=32.0),
                                )
                                .changed()
                            {
                                modified = true;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("To   (Java):");
                        for axis in 0..3 {
                            if ui
                                .add(
                                    egui::DragValue::new(&mut el.to[axis])
                                        .speed(0.1)
                                        .range(-16.0..=32.0),
                                )
                                .changed()
                            {
                                modified = true;
                            }
                        }
                    });
                    ui.label(
                        RichText::new(format!(
                            "Bedrock Origin: [{:.2}, {:.2}, {:.2}] | Size: [{:.2}, {:.2}, {:.2}]",
                            b_orig[0], b_orig[1], b_orig[2], b_size[0], b_size[1], b_size[2]
                        ))
                        .small()
                        .weak(),
                    );

                    ui.separator();

                    // Pivot & 3-axis Euler Rotation (Java 1.21.11+ / 26.x {x,y,z})
                    let mut has_rot = el.rotation.is_some();
                    if ui.checkbox(&mut has_rot, "Element Rotation").changed() {
                        if has_rot {
                            let c = [
                                0.5 * (el.from[0] + el.to[0]),
                                0.5 * (el.from[1] + el.to[1]),
                                0.5 * (el.from[2] + el.to[2]),
                            ];
                            el.rotation = Some(ElementRotation {
                                origin: c,
                                axis: RotationAxis::Y,
                                angle: 0.0,
                                rescale: false,
                                extra_bedrock_euler: Some([0.0, 0.0, 0.0]),
                            });
                        } else {
                            el.rotation = None;
                        }
                        modified = true;
                    }
                    if let Some(rot) = &mut el.rotation {
                        ui.horizontal(|ui| {
                            ui.label("Pivot:      ");
                            for axis in 0..3 {
                                if ui
                                    .add(egui::DragValue::new(&mut rot.origin[axis]).speed(0.1))
                                    .changed()
                                {
                                    modified = true;
                                }
                            }
                        });
                        let mut euler_java = if let Some(b) = rot.extra_bedrock_euler {
                            [-b[0], -b[1], b[2]]
                        } else {
                            match rot.axis {
                                RotationAxis::X => [rot.angle, 0.0, 0.0],
                                RotationAxis::Y => [0.0, rot.angle, 0.0],
                                RotationAxis::Z => [0.0, 0.0, rot.angle],
                            }
                        };
                        let mut rot_changed = false;
                        ui.horizontal(|ui| {
                            ui.label("Rot (X/Y/Z):");
                            for v in &mut euler_java {
                                if ui
                                    .add(egui::DragValue::new(v).speed(0.5).range(-180.0..=180.0))
                                    .changed()
                                {
                                    rot_changed = true;
                                }
                            }
                        });
                        if rot_changed {
                            let ax = euler_java[0].abs();
                            let ay = euler_java[1].abs();
                            let az = euler_java[2].abs();
                            if ax >= ay && ax >= az {
                                rot.axis = RotationAxis::X;
                                rot.angle = euler_java[0];
                            } else if ay >= ax && ay >= az {
                                rot.axis = RotationAxis::Y;
                                rot.angle = euler_java[1];
                            } else {
                                rot.axis = RotationAxis::Z;
                                rot.angle = euler_java[2];
                            }
                            rot.extra_bedrock_euler =
                                Some([-euler_java[0], -euler_java[1], euler_java[2]]);
                            modified = true;
                        }
                    }

                    ui.separator();

                    // Per-face UV inspector & live editor
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Face UV:");
                        for dir in FaceDirection::ALL {
                            if ui
                                .selectable_label(self.selected_face_dir == dir, dir.as_str())
                                .clicked()
                            {
                                self.selected_face_dir = dir;
                            }
                        }
                    });
                    if let Some(face) = el.faces.get_mut(&self.selected_face_dir) {
                        let mut uv = face.uv.unwrap_or([0.0, 0.0, 16.0, 16.0]);
                        let mut uv_changed = false;
                        ui.horizontal(|ui| {
                            ui.label("UV [u1,v1,u2,v2]:");
                            for c in &mut uv {
                                if ui
                                    .add(egui::DragValue::new(c).speed(0.25).range(0.0..=16.0))
                                    .changed()
                                {
                                    uv_changed = true;
                                }
                            }
                        });
                        if uv_changed {
                            face.uv = Some(uv);
                            modified = true;
                        }
                        ui.horizontal(|ui| {
                            ui.label("UV Rot:");
                            for deg in [0u16, 90, 180, 270] {
                                if ui
                                    .selectable_label(face.rotation == deg, format!("{deg}°"))
                                    .clicked()
                                {
                                    face.rotation = deg;
                                    modified = true;
                                }
                            }
                            ui.label(format!("Tex: {}", face.texture));
                        });
                    } else {
                        ui.label(
                            RichText::new("Face not defined on this cube")
                                .small()
                                .weak(),
                        );
                    }

                    // Shading & 26.3 properties
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut el.shade, "Shade").changed() {
                            modified = true;
                        }
                        let mut le = el.light_emission.unwrap_or(0);
                        ui.label("Light:");
                        if ui
                            .add(egui::DragValue::new(&mut le).range(0..=15))
                            .changed()
                        {
                            el.light_emission = if le == 0 { None } else { Some(le) };
                            modified = true;
                        }
                    });
                });
        });

        if modified {
            self.overrides
                .record_model_edit(model_id, &orig_model, model.clone());
            if let Some(loaded) = &mut self.loaded_pack {
                loaded.ir.models.insert(model_id.to_string(), model);
            }
            self.status_banner = format!("Manual override saved on model '{model_id}'");
        }
    }

    fn show_studio_texture_tab(&mut self, ctx: &Context, ui: &mut Ui, tex_id: &str) {
        let Some(tex) = self
            .loaded_pack
            .as_ref()
            .and_then(|l| l.ir.textures.get(tex_id))
            .cloned()
        else {
            ui.label(format!("Texture '{tex_id}' not found."));
            return;
        };

        self.ensure_asset_texture_preview(ctx, &tex);

        let mapped_bedrock = self.mappings.java_texture_to_bedrock(&tex.id);
        let is_overridden = self.overrides.is_texture_overridden(&tex.id);

        let palette = self.palette;
        let active_ed = self
            .loaded_pack
            .as_ref()
            .map_or(self.studio_edition, |l| l.edition);
        let (ed_bg, ed_fg, _) = palette.edition_colors(active_ed);

        ui.horizontal_wrapped(|ui| {
            m3_pill_badge(
                ui,
                match active_ed {
                    Edition::Java => "☕ JAVA TEXTURE",
                    Edition::Bedrock => "🪨 BEDROCK TEXTURE",
                },
                ed_bg,
                ed_fg,
            );
            ui.label(RichText::new(&tex.id).strong().size(15.0));
            if is_overridden {
                ui.colored_label(palette.primary, "✎ CUSTOM MAPPING");
            }
            ui.separator();
            ui.checkbox(&mut self.tex_pixel_grid, "Pixel Grid");
            ui.add(egui::Slider::new(&mut self.tex_zoom, 1.0..=16.0).text("Zoom"));
        });

        // Bedrock & Java Atlas Index Status (Material 3 tonal card)
        m3_card_frame(&palette).show(ui, |ui| {
            let atlas_file = match tex.category {
                TextureCategory::Block => "terrain_texture.json",
                TextureCategory::Item => "item_texture.json",
                _ => "atlases/blocks.json + items.json",
            };
            let flip_status = if tex.animation.is_some() {
                "flipbook_textures.json (Animated)"
            } else {
                "Static PNG"
            };
            let res_label = match (tex.width, tex.height, tex.resolution_tier) {
                (Some(w), Some(h), Some(tier)) => format!("{w}x{h} ({tier})"),
                (Some(w), Some(h), None) => format!("{w}x{h}"),
                _ => String::from("?"),
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Res:").strong());
                ui.monospace(res_label);
                ui.separator();
                ui.label(RichText::new("Index:").strong());
                ui.monospace(atlas_file);
                ui.separator();
                ui.label(RichText::new("Strip:").strong());
                ui.monospace(flip_status);
                ui.separator();
                ui.label(RichText::new("Bedrock Path:").strong());
                ui.monospace(&mapped_bedrock.path);
            });

            // Editable Custom Mapping Rule Override
            if self.custom_mapping_input.is_empty() {
                self.custom_mapping_input = self
                    .overrides
                    .custom_texture_mappings
                    .get(&tex.id)
                    .cloned()
                    .unwrap_or_else(|| mapped_bedrock.path.clone());
            }
            ui.horizontal(|ui| {
                ui.label("Override Bedrock Path:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.custom_mapping_input).desired_width(260.0),
                );
                if ui.small_button("Save Mapping Override").clicked() {
                    let val = self.custom_mapping_input.trim().to_string();
                    if !val.is_empty() {
                        self.overrides
                            .custom_texture_mappings
                            .insert(tex.id.clone(), val.clone());
                        self.status_banner = format!("Saved mapping override: {} -> {val}", tex.id);
                    }
                }
                if is_overridden && ui.small_button("Reset to Auto").clicked() {
                    self.overrides.custom_texture_mappings.remove(&tex.id);
                    self.custom_mapping_input = mapped_bedrock.path.clone();
                    self.status_banner = format!("Reset texture mapping for '{}'", tex.id);
                }
            });
        });

        ui.add_space(6.0);

        if let Some(prev) = &self.tex_preview {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "PNG: {}×{} | Frames: {}",
                    prev.dims.0,
                    prev.dims.1,
                    prev.frames.len()
                ));
                if prev.frames.len() > 1 {
                    if ui
                        .button(if self.anim_playing { "Pause" } else { "Play" })
                        .clicked()
                    {
                        self.anim_playing = !self.anim_playing;
                    }
                    let max_f = prev.frames.len() - 1;
                    ui.add(egui::Slider::new(&mut self.anim_frame_idx, 0..=max_f).text("Frame"));
                }
            });

            if prev.frames.len() > 1 && self.anim_playing {
                let ms_per_frame = u64::from(prev.frametime_ticks).saturating_mul(50).max(50);
                if self.anim_last_step.elapsed().as_millis() >= u128::from(ms_per_frame) {
                    self.anim_frame_idx = (self.anim_frame_idx + 1) % prev.frames.len();
                    self.anim_last_step = Instant::now();
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(ms_per_frame));
            }

            let idx = self.anim_frame_idx.min(prev.frames.len().saturating_sub(1));
            if let Some(handle) = prev.frames.get(idx) {
                let base = handle.size_vec2();
                let disp = (base * self.tex_zoom).min(Vec2::new(480.0, 480.0));
                ScrollArea::both().show(ui, |ui| {
                    let (rect, _) = ui.allocate_exact_size(disp, egui::Sense::hover());
                    draw_checkerboard(ui, rect);
                    ui.painter().image(
                        handle.id(),
                        rect,
                        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    // Optional crisp pixel grid when zoomed >= 4x
                    if self.tex_pixel_grid && self.tex_zoom >= 4.0 {
                        let px_w = base.x.max(1.0);
                        let px_h = base.y.max(1.0);
                        let step_x = rect.width() / px_w;
                        let step_y = rect.height() / px_h;
                        if step_x >= 3.5 && step_y >= 3.5 {
                            let p = ui.painter_at(rect);
                            let grid_stroke = Stroke::new(
                                0.6,
                                Color32::from_rgba_unmultiplied(255, 255, 255, 45),
                            );
                            for x in 1..(px_w as i32) {
                                let gx = rect.left() + x as f32 * step_x;
                                p.line_segment(
                                    [Pos2::new(gx, rect.top()), Pos2::new(gx, rect.bottom())],
                                    grid_stroke,
                                );
                            }
                            for y in 1..(px_h as i32) {
                                let gy = rect.top() + y as f32 * step_y;
                                p.line_segment(
                                    [Pos2::new(rect.left(), gy), Pos2::new(rect.right(), gy)],
                                    grid_stroke,
                                );
                            }
                        }
                    }
                });
            }
        }
    }

    fn show_studio_text_tab(&mut self, ui: &mut Ui, target: &TextAssetTarget) {
        let Some(ir) = self.loaded_pack.as_ref().map(|l| &l.ir) else {
            return;
        };
        let key = target.key();
        let is_overridden = self.overrides.is_text_overridden(&key);
        let expects_json = target.expects_json();
        let buf = self.overrides.ensure_text_buffer(target, ir);
        let json_status = if expects_json {
            Some(validate_json_str(buf))
        } else {
            None
        };

        let mut do_apply = false;
        let mut do_reset = false;

        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(target.title()).strong().size(14.0));
            if is_overridden {
                ui.colored_label(Color32::from_rgb(100, 225, 255), "✎ MANUAL OVERRIDE");
            }
            if let Some(res) = &json_status {
                match res {
                    Ok(()) => {
                        ui.colored_label(Color32::LIGHT_GREEN, "✓ Valid JSON");
                    }
                    Err(e) => {
                        ui.colored_label(Color32::LIGHT_RED, format!("✗ {e}"));
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_apply = json_status.as_ref().is_none_or(Result::is_ok);
                if ui
                    .add_enabled(can_apply, egui::Button::new("Apply Edit"))
                    .clicked()
                {
                    do_apply = true;
                }
                if is_overridden && ui.button("Reset to Auto").clicked() {
                    do_reset = true;
                }
            });
        });
        ui.separator();

        ScrollArea::vertical()
            .id_salt(format!("text_editor_{key}"))
            .show(ui, |ui| {
                if let Some(buf_mut) = self.overrides.text_buffers.get_mut(&key) {
                    ui.add(
                        egui::TextEdit::multiline(buf_mut)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .desired_rows(22),
                    );
                }
            });

        if do_apply && let Some(loaded) = &mut self.loaded_pack {
            match self.overrides.commit_text_edit(target, &mut loaded.ir) {
                Ok(()) => {
                    self.status_banner = format!("Applied manual edit to '{}'", target.title());
                    self.push_log(self.status_banner.clone());
                }
                Err(err) => {
                    self.status_banner = format!("Cannot apply edit: {err}");
                }
            }
        } else if do_reset && let Some(loaded) = &mut self.loaded_pack {
            match self.overrides.reset_text_to_auto(target, &mut loaded.ir) {
                Ok(()) => {
                    self.status_banner =
                        format!("Reset '{}' to auto-loaded content", target.title());
                }
                Err(err) => {
                    self.status_banner = format!("Reset error: {err}");
                }
            }
        }
    }
}

impl eframe::App for RpGuiApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.render_ui(ctx);
    }
}

/// Renders collapsed diagnostic clusters filtered by `filter`; returns an asset ID if clicked.
fn render_grouped_diagnostics(
    ui: &mut Ui,
    index: &NeedsReviewIndex,
    filter: DiagnosticFilter,
    can_open_in_studio: bool,
) -> Option<String> {
    let mut clicked_asset: Option<String> = None;
    let mut shown = 0usize;

    for (g_idx, group) in index.groups.iter().enumerate() {
        if !group.kind.matches_filter(filter) {
            continue;
        }
        shown += 1;
        let color = match group.kind {
            DiagnosticKind::Error | DiagnosticKind::Skipped => Color32::LIGHT_RED,
            DiagnosticKind::Lossy | DiagnosticKind::Warning => Color32::YELLOW,
            DiagnosticKind::Info => Color32::from_rgb(140, 190, 230),
        };
        let header = RichText::new(format!(
            "[{}] ×{} — {}",
            group.kind.badge(),
            group.assets.len(),
            group.message
        ))
        .color(color);

        egui::CollapsingHeader::new(header)
            .id_salt(format!("diag_grp_{g_idx}"))
            .default_open(group.assets.len() <= 3 && group.kind.needs_review())
            .show(ui, |ui| {
                for asset in group.assets.iter().take(30) {
                    ui.horizontal(|ui| {
                        ui.monospace(asset);
                        if can_open_in_studio && ui.small_button("Inspect").clicked() {
                            clicked_asset = Some(asset.clone());
                        }
                    });
                }
                if group.assets.len() > 30 {
                    ui.label(
                        RichText::new(format!("... and {} more asset(s)", group.assets.len() - 30))
                            .small()
                            .weak(),
                    );
                }
            });
    }

    if shown == 0 {
        ui.label(RichText::new("No diagnostics matching this filter.").weak());
    }
    clicked_asset
}

fn draw_checkerboard(ui: &Ui, rect: Rect) {
    let painter = ui.painter_at(rect);
    let cell = 12.0_f32;
    let cols = (rect.width() / cell).ceil() as i32;
    let rows = (rect.height() / cell).ceil() as i32;
    for r in 0..rows {
        for c in 0..cols {
            let x0 = rect.left() + c as f32 * cell;
            let y0 = rect.top() + r as f32 * cell;
            let tile = Rect::from_min_max(
                Pos2::new(x0, y0),
                Pos2::new(
                    (x0 + cell).min(rect.right()),
                    (y0 + cell).min(rect.bottom()),
                ),
            );
            let col = if (r + c) % 2 == 0 {
                Color32::from_rgb(56, 56, 62)
            } else {
                Color32::from_rgb(76, 76, 84)
            };
            painter.rect_filled(tile, 0.0, col);
        }
    }
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, Color32::GRAY),
        egui::StrokeKind::Inside,
    );
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{ImageFormat, Rgba, RgbaImage};
    use tempfile::tempdir;

    use rp_io::PackContainer;

    use super::*;

    fn make_png(w: u32, h: u32, color: Rgba<u8>) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, color);
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .expect("png");
        out
    }

    #[test]
    fn headless_quick_mode_and_studio_workbench_render_cleanly() {
        let dir = tempdir().expect("tempdir");
        let mut container = PackContainer::new();
        container
            .insert(
                "pack.mcmeta",
                br#"{"pack":{"description":"Studio Test Pack","min_format":[97,0],"max_format":[97,1]}}"#
                    .to_vec(),
            )
            .expect("mcmeta");
        container
            .insert("pack.png", make_png(16, 16, Rgba([60, 180, 250, 255])))
            .expect("icon");
        container
            .insert(
                "assets/minecraft/textures/block/glow.png",
                make_png(16, 32, Rgba([240, 200, 50, 255])),
            )
            .expect("tex");
        container
            .insert(
                "assets/minecraft/textures/block/glow.png.mcmeta",
                br#"{"animation":{"frametime":2}}"#.to_vec(),
            )
            .expect("anim");
        container
            .insert(
                "assets/minecraft/models/block/glow.json",
                br##"{"textures":{"all":"block/glow"},"elements":[{"from":[4,4,4],"to":[12,12,12],"rotation":{"x":20,"y":60,"z":15,"origin":[8,8,8]},"shade_direction_override":"up","faces":{"north":{"uv":[0,0,16,16],"texture":"#all"}}}]}"##
                    .to_vec(),
            )
            .expect("model");
        container
            .insert(
                "assets/minecraft/lang/en_us.json",
                br#"{"block.minecraft.glow": "Glow Block"}"#.to_vec(),
            )
            .expect("lang");

        let zip_path = dir.path().join("studio_sample.zip");
        std::fs::write(&zip_path, container.write_to_zip_bytes().expect("zip")).expect("save");

        let mappings = MappingTable::default_embedded().expect("mappings");
        let packs =
            scan_directory_for_packs(dir.path(), &mappings, IoLimits::default()).expect("scan");
        let (ed, ir, mut rep) =
            load_pack_ir(&zip_path, &mappings, IoLimits::default()).expect("ir");
        rep.record_lossy("block/glow", "Sample lossy warning for Needs Review test");

        let mut app = RpGuiApp::new(dir.path().to_path_buf(), None, mappings);
        app.packs = packs;
        app.loaded_pack = Some(LoadedPack {
            path: zip_path,
            edition: ed,
            ir,
            report: rep,
        });
        app.rebuild_review_index();
        assert!(app.review_index.asset_needs_review("block/glow"));

        let ctx = Context::default();

        // 1. Render all Quick Mode pages + non-blocking in-app file browser
        app.mode = WorkspaceMode::Quick;
        for page in GuiPage::ALL {
            app.page = page;
            let _ = ctx.run(egui::RawInput::default(), |c| {
                app.render_ui(c);
            });
        }
        app.file_browser
            .open_for(FilePickerTarget::WorkspaceDir, dir.path(), "");
        let _ = ctx.run(egui::RawInput::default(), |c| {
            app.render_ui(c);
        });
        app.file_browser.open = false;

        // 2. Render Studio Mode: all 3 ActivityBar sections + Model, Texture, and Text/JSON tabs
        app.mode = WorkspaceMode::Studio;
        app.open_tabs = vec![
            StudioEditorTab::Model(String::from("block/glow")),
            StudioEditorTab::Texture(String::from("block/glow")),
            StudioEditorTab::Text(TextAssetTarget::LocaleJson(String::from("en_us"))),
            StudioEditorTab::Text(TextAssetTarget::MetadataJson),
        ];

        for sec in ActivitySection::ALL {
            app.activity_section = sec;
            for tab_idx in 0..app.open_tabs.len() {
                app.active_tab_idx = tab_idx;
                let _ = ctx.run(egui::RawInput::default(), |c| {
                    app.render_ui(c);
                });
            }
        }

        // 3. Verify separated Java vs Bedrock Studio slots & Material 3 Light mode
        app.switch_studio_edition(Edition::Bedrock);
        assert_eq!(app.studio_edition, Edition::Bedrock);
        assert!(app.loaded_pack.is_none());
        assert!(app.open_tabs.is_empty());
        app.dark_mode = false;
        let _ = ctx.run(egui::RawInput::default(), |c| {
            app.render_ui(c);
        });

        app.switch_studio_edition(Edition::Java);
        assert_eq!(app.studio_edition, Edition::Java);
        assert!(app.loaded_pack.is_some());
        assert_eq!(app.open_tabs.len(), 4);

        // 4. Verify Fresh Refresh clears caches and reloads cleanly
        app.fresh_refresh_studio();
        assert!(app.tex_preview.is_none());
        assert!(app.busy_message.is_some());
    }

    #[test]
    fn gui_negative_test_renders_failed_status_not_passed() {
        let dir = tempdir().expect("tempdir");
        let mut bad = PackContainer::new();
        bad.insert(
            "pack.mcmeta",
            br#"{"pack":{"pack_format":34,"description":"Unmappable GUI Pack"}}"#.to_vec(),
        )
        .expect("mcmeta");
        bad.insert(
            "assets/minecraft/textures/block/unmapped_custom_gui.png",
            make_png(16, 16, Rgba([220, 30, 30, 255])),
        )
        .expect("tex");
        let bad_zip = dir.path().join("bad_gui.zip");
        std::fs::write(&bad_zip, bad.write_to_zip_bytes().expect("zip")).expect("save");

        let mappings = MappingTable::default_embedded().expect("mappings");
        let audit = audit_pack_roundtrip(
            &bad_zip,
            Some("26.60"),
            &VersionSpecificOptions::default(),
            &mappings,
            IoLimits::default(),
        );
        assert!(!audit.passed);
        assert_eq!(audit.status, AuditStatus::Failed);

        let mut app = RpGuiApp::new(dir.path().to_path_buf(), None, mappings);
        app.last_conversion = Some(Err(audit.detail.clone()));
        app.audit_outcomes.push(audit);

        let ctx = Context::default();
        app.mode = WorkspaceMode::Quick;
        app.page = GuiPage::Convert;
        let _ = ctx.run(egui::RawInput::default(), |c| {
            app.render_ui(c);
        });
        app.page = GuiPage::BatchAudit;
        let _ = ctx.run(egui::RawInput::default(), |c| {
            app.render_ui(c);
        });
        assert_eq!(app.audit_outcomes[0].status, AuditStatus::Failed);
        assert!(!app.audit_outcomes[0].passed);
    }
}
