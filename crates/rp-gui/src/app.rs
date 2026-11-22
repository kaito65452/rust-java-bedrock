use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use egui::{
    Color32, ColorImage, Context, Pos2, Rect, RichText, ScrollArea, Stroke, TextureHandle,
    TextureOptions, Ui, Vec2,
};

use rp_cli::manager::{
    AuditOutcome, DiscoveredPack, PackDiff, PackSortKey, audit_pack_roundtrip, diff_packs,
    export_audit_json, export_report_json, format_bytes, load_pack_ir, repack_dir_to_archive,
    scan_directory_for_packs, sort_discovered_packs, suggest_output_path, unpack_pack_to_dir,
};
use rp_cli::{ConvertOptions, VersionSpecificOptions, convert_pack};
use rp_core::{
    BedrockGeoVersion, ConversionReport, Edition, MappingTable, ResourcePackIR, RotationSnapMode,
    TextureAsset,
};
use rp_io::IoLimits;

use crate::viewport3d::ModelViewportState;

/// Navigation pages inside `rp-gui`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiPage {
    Packs,
    Explorer,
    Convert,
    BatchAudit,
    Compare,
    VersionsLogs,
}

impl GuiPage {
    /// All pages in sidebar order.
    pub const ALL: [Self; 6] = [
        Self::Packs,
        Self::Explorer,
        Self::Convert,
        Self::BatchAudit,
        Self::Compare,
        Self::VersionsLogs,
    ];

    /// Display label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Packs => "Packs Manager",
            Self::Explorer => "Explorer & 3D",
            Self::Convert => "Converter",
            Self::BatchAudit => "Batch & Audit",
            Self::Compare => "Compare (Diff)",
            Self::VersionsLogs => "Versions & Logs",
        }
    }
}

/// Asset filter pills inside Explorer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetCategory {
    All,
    Textures,
    Models,
    Attachables,
    Sounds,
    Locales,
    Passthrough,
}

impl AssetCategory {
    pub const ALL: [Self; 7] = [
        Self::All,
        Self::Textures,
        Self::Models,
        Self::Attachables,
        Self::Sounds,
        Self::Locales,
        Self::Passthrough,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Textures => "Textures",
            Self::Models => "3D Models",
            Self::Attachables => "Attachables",
            Self::Sounds => "Sounds",
            Self::Locales => "Locales",
            Self::Passthrough => "Raw",
        }
    }
}

/// Flattened explorer asset reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetRef {
    Texture(String),
    Model(String),
    Attachable(String),
    SoundFile(String),
    SoundEvent(String),
    Locale(String),
    Passthrough(String),
}

impl AssetRef {
    #[must_use]
    pub const fn badge(&self) -> &'static str {
        match self {
            Self::Texture(_) => "TEX",
            Self::Model(_) => "3D",
            Self::Attachable(_) => "ATT",
            Self::SoundFile(_) => "OGG",
            Self::SoundEvent(_) => "SFX",
            Self::Locale(_) => "LNG",
            Self::Passthrough(_) => "RAW",
        }
    }

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

/// Deep-loaded pack inside Explorer.
#[derive(Debug, Clone)]
pub struct LoadedPack {
    pub path: PathBuf,
    pub edition: Edition,
    pub ir: ResourcePackIR,
    pub report: ConversionReport,
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

/// Background thread messages.
#[derive(Debug)]
enum WorkerMsg {
    ScanDone {
        dir: PathBuf,
        result: Result<Vec<DiscoveredPack>, String>,
    },
    PackLoaded(Result<Box<LoadedPack>, String>),
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

/// Cached GPU texture preview (single frame or animated strip frames).
struct CachedTexPreview {
    asset_id: String,
    dims: (u32, u32),
    frames: Vec<TextureHandle>,
    frametime_ticks: u32,
}

/// Desktop GUI application state implementing `eframe::App`.
pub struct RpGuiApp {
    pub page: GuiPage,
    pub dark_mode: bool,

    pub scan_dir: PathBuf,
    pub scan_dir_input: String,
    pub mapping_file: Option<PathBuf>,
    pub mappings: MappingTable,
    pub limits: IoLimits,

    // Packs Manager
    pub packs: Vec<DiscoveredPack>,
    pub pack_filter: String,
    pub pack_sort: PackSortKey,
    pub pending_delete: Option<(PathBuf, bool)>,
    icon_cache: HashMap<PathBuf, TextureHandle>,

    // Explorer & 3D Studio
    pub loaded_pack: Option<LoadedPack>,
    pub asset_category: AssetCategory,
    pub asset_filter: String,
    pub selected_asset: Option<AssetRef>,
    pub tex_zoom: f32,
    pub anim_playing: bool,
    pub anim_frame_idx: usize,
    pub anim_last_step: Instant,
    tex_preview: Option<CachedTexPreview>,
    pub viewport_3d: ModelViewportState,

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
        let scan_dir_input = scan_dir.display().to_string();
        let mut app = Self {
            page: GuiPage::Packs,
            dark_mode: true,
            scan_dir: scan_dir.clone(),
            scan_dir_input,
            mapping_file,
            mappings,
            limits: IoLimits::default(),
            packs: Vec::new(),
            pack_filter: String::new(),
            pack_sort: PackSortKey::Name,
            pending_delete: None,
            icon_cache: HashMap::new(),
            loaded_pack: None,
            asset_category: AssetCategory::All,
            asset_filter: String::new(),
            selected_asset: None,
            tex_zoom: 4.0,
            anim_playing: true,
            anim_frame_idx: 0,
            anim_last_step: Instant::now(),
            tex_preview: None,
            viewport_3d: ModelViewportState::default(),
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
        app.push_log(format!("Launched rp-gui in {}", app.scan_dir.display()));
        app
    }

    /// Starts scanning the configured directory asynchronously.
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

    fn push_log(&mut self, msg: impl Into<String>) {
        self.logs.push(msg.into());
        if self.logs.len() > 250 {
            let excess = self.logs.len() - 250;
            self.logs.drain(0..excess);
        }
    }

    fn version_options(&self) -> VersionSpecificOptions {
        VersionSpecificOptions {
            supported_formats: None,
            rotation_mode: self.rotation_mode,
            bedrock_geo_version: self.bedrock_geo_version,
            bedrock_manifest_version: self.bedrock_manifest_version,
            emit_java_items_definitions: self.emit_java_items,
        }
    }

    fn start_load_explorer(&mut self, path: PathBuf) {
        if self.busy_message.is_some() {
            return;
        }
        let mappings = self.mappings.clone();
        let limits = self.limits;
        let tx = self.tx.clone();
        self.page = GuiPage::Explorer;
        self.busy_message = Some(format!("Loading {}...", path.display()));
        self.push_log(format!("Exploring {}", path.display()));
        thread::spawn(move || {
            let res = load_pack_ir(&path, &mappings, limits)
                .map(|(edition, ir, report)| {
                    Box::new(LoadedPack {
                        path,
                        edition,
                        ir,
                        report,
                    })
                })
                .map_err(|err| format!("{err:#}"));
            let _ = tx.send(WorkerMsg::PackLoaded(res));
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
        self.page = GuiPage::Convert;
    }

    fn start_convert(&mut self) {
        if self.busy_message.is_some() {
            return;
        }
        if self.input_path.trim().is_empty() || self.output_path.trim().is_empty() {
            self.status_banner = String::from("Select both input and output paths first");
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

    fn start_diff(&mut self, a: PathBuf, b: PathBuf) {
        if self.busy_message.is_some() {
            return;
        }
        let mappings = self.mappings.clone();
        let limits = self.limits;
        let tx = self.tx.clone();
        self.page = GuiPage::Compare;
        self.busy_message = Some(String::from("Comparing packs..."));
        self.push_log(format!("Diff '{}' vs '{}'", a.display(), b.display()));
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

        self.page = GuiPage::BatchAudit;
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

        self.page = GuiPage::BatchAudit;
        self.audit_outcomes.clear();
        self.batch_progress = Some((0, total, String::from("Starting batch convert...")));
        self.busy_message = Some(format!("Batch converting {total} pack(s)..."));
        self.push_log(format!(
            "Batch converting {total} pack(s) -> {}",
            out_dir.display()
        ));

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
                        ok += 1;
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
                label: String::from("Complete"),
            });
            let _ = tx.send(WorkerMsg::BatchDone(format!(
                "Batch convert complete: {ok}/{total} saved in {}",
                out_dir.display()
            )));
        });
    }

    fn poll_worker(&mut self) {
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
                                self.load_pack_into_converter(first);
                                self.page = GuiPage::Packs;
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
                WorkerMsg::PackLoaded(res) => {
                    self.busy_message = None;
                    match res {
                        Ok(loaded) => {
                            let loaded = *loaded;
                            self.status_banner = format!(
                                "Loaded {} ({} textures, {} models)",
                                loaded.ir.metadata.name,
                                loaded.ir.textures.len(),
                                loaded.ir.models.len()
                            );
                            self.push_log(self.status_banner.clone());
                            self.selected_asset = loaded
                                .ir
                                .models
                                .keys()
                                .next()
                                .map(|k| AssetRef::Model(k.clone()))
                                .or_else(|| {
                                    loaded
                                        .ir
                                        .textures
                                        .keys()
                                        .next()
                                        .map(|k| AssetRef::Texture(k.clone()))
                                });
                            self.loaded_pack = Some(loaded);
                            self.tex_preview = None;
                            self.anim_frame_idx = 0;
                        }
                        Err(err) => {
                            self.status_banner = format!("Load error: {err}");
                            self.push_log(self.status_banner.clone());
                        }
                    }
                }
                WorkerMsg::ConvertDone(res) => {
                    self.busy_message = None;
                    match &res {
                        Ok(ok) => {
                            self.status_banner = format!(
                                "Converted {} -> {} in {}ms ({} converted, {} lossy)",
                                ok.source,
                                ok.target,
                                ok.elapsed_ms,
                                ok.report.converted_count,
                                ok.report.lossy_count
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
                }
                WorkerMsg::DiffDone(res) => {
                    self.busy_message = None;
                    match res {
                        Ok(diff) => {
                            let d = *diff;
                            self.status_banner = format!(
                                "Compared {} vs {} ({} shared tex, {} shared models)",
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

        if let Some(anim) = &tex.animation {
            let fh = anim.frame_height.unwrap_or(w).min(h).max(1);
            let count = (h / fh).clamp(1, 64);
            for i in 0..count {
                let cropped =
                    image::imageops::crop_imm(decoded.as_ref(), 0, i * fh, w, fh).to_image();
                let ci =
                    ColorImage::from_rgba_unmultiplied([w as usize, fh as usize], cropped.as_raw());
                frames.push(ctx.load_texture(
                    format!("tex:{}:f{i}", tex.id),
                    ci,
                    TextureOptions::NEAREST,
                ));
            }
        } else {
            let ci = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], decoded.as_raw());
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

    /// Renders the full GUI frame (exposed as public for headless `egui::Context` tests).
    pub fn render_ui(&mut self, ctx: &Context) {
        self.poll_worker();

        if self.dark_mode {
            ctx.set_visuals(egui::Visuals::dark());
        } else {
            ctx.set_visuals(egui::Visuals::light());
        }

        // Handle drag-and-drop .zip / .mcpack files directly onto window
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if let Some(first_drop) = dropped.first() {
            if first_drop.is_dir() {
                self.scan_dir = first_drop.clone();
                self.scan_dir_input = first_drop.display().to_string();
                self.start_scan();
            } else {
                self.start_load_explorer(first_drop.clone());
            }
        }

        if self.busy_message.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }

        // Top bar
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("mc-rp-converter Studio")
                        .strong()
                        .size(16.0)
                        .color(Color32::from_rgb(90, 210, 255)),
                );
                ui.separator();
                ui.label("Workspace:");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.scan_dir_input)
                        .desired_width(280.0)
                        .hint_text("/path/to/resource-packs"),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let p = PathBuf::from(self.scan_dir_input.trim());
                    if p.is_dir() {
                        self.scan_dir = p;
                        self.start_scan();
                    } else {
                        self.status_banner = format!("Not a directory: {}", self.scan_dir_input);
                    }
                }
                if ui.button("Browse...").clicked()
                    && let Some(folder) = rfd::FileDialog::new()
                        .set_directory(&self.scan_dir)
                        .pick_folder()
                {
                    self.scan_dir = folder.clone();
                    self.scan_dir_input = folder.display().to_string();
                    self.start_scan();
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
                        "Light Mode"
                    } else {
                        "Dark Mode"
                    };
                    if ui.button(theme_label).clicked() {
                        self.dark_mode = !self.dark_mode;
                    }
                    if let Some(busy) = &self.busy_message {
                        ui.spinner();
                        ui.label(RichText::new(busy).color(Color32::YELLOW));
                    }
                });
            });
            ui.add_space(4.0);
        });

        // Bottom status bar
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&self.status_banner)
                        .color(Color32::from_rgb(120, 225, 140))
                        .small(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(
                            "Drag & drop .zip/.mcpack anywhere | Java 1.6.1–26.3 <-> Bedrock",
                        )
                        .weak()
                        .small(),
                    );
                });
            });
        });

        // Left navigation rail
        egui::SidePanel::left("nav_rail")
            .resizable(false)
            .default_width(165.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                for page in GuiPage::ALL {
                    let selected = self.page == page;
                    let btn = ui.add_sized(
                        Vec2::new(ui.available_width(), 32.0),
                        egui::Button::new(RichText::new(page.label()).strong()).selected(selected),
                    );
                    if btn.clicked() {
                        self.page = page;
                    }
                    ui.add_space(4.0);
                }
                ui.separator();
                ui.label(RichText::new(format!("Packs: {}", self.packs.len())).small());
                let checked = self.packs.iter().filter(|p| p.selected).count();
                ui.label(RichText::new(format!("Selected: {checked}")).small());
                if let Some(loaded) = &self.loaded_pack {
                    ui.separator();
                    ui.label(RichText::new("Loaded Pack:").small().strong());
                    ui.label(RichText::new(&loaded.ir.metadata.name).small());
                    ui.label(
                        RichText::new(format!(
                            "{} tex | {} 3D",
                            loaded.ir.textures.len(),
                            loaded.ir.models.len()
                        ))
                        .small()
                        .weak(),
                    );
                }
            });

        // Central view
        egui::CentralPanel::default().show(ctx, |ui| match self.page {
            GuiPage::Packs => self.show_packs_page(ctx, ui),
            GuiPage::Explorer => self.show_explorer_page(ctx, ui),
            GuiPage::Convert => self.show_convert_page(ui),
            GuiPage::BatchAudit => self.show_batch_page(ui),
            GuiPage::Compare => self.show_compare_page(ui),
            GuiPage::VersionsLogs => self.show_versions_page(ui),
        });

        // Delete confirmation dialog
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

    fn show_packs_page(&mut self, ctx: &Context, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Search:");
            ui.add(egui::TextEdit::singleline(&mut self.pack_filter).desired_width(200.0));
            ui.separator();
            ui.label("Sort:");
            if ui.button(self.pack_sort.label()).clicked() {
                self.pack_sort = self.pack_sort.next();
                sort_discovered_packs(&mut self.packs, self.pack_sort);
            }
            if ui.button("Toggle Select All").clicked() {
                let any_unsel = self.packs.iter().any(|p| !p.selected);
                for p in &mut self.packs {
                    p.selected = any_unsel;
                }
            }
            if ui.button("Compare Selected (2)").clicked() {
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
                    self.status_banner = String::from("Check 2 packs first to run Compare (Diff)");
                }
            }
        });
        ui.separator();

        let needle = self.pack_filter.trim().to_ascii_lowercase();
        let mut action_explore: Option<PathBuf> = None;
        let mut action_convert: Option<usize> = None;
        let mut action_audit: Option<PathBuf> = None;
        let mut action_unpack: Option<PathBuf> = None;
        let mut action_repack: Option<PathBuf> = None;
        let mut action_delete: Option<(PathBuf, bool)> = None;

        // Pre-warm icons cleanly without overlapping borrows
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
                ui.label("No resource packs found in this folder. Click 'Browse...' or drop a .zip/.mcpack file.");
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
                                                .map(|f| format!("pack_format {f}"))
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
                                        "{} | {} tex ({} anim) | {} 3D models | {} sounds | {} locales | {}",
                                        s.description.lines().next().unwrap_or(""),
                                        s.texture_count,
                                        s.animated_texture_count,
                                        s.model_count,
                                        s.sound_file_count,
                                        s.locale_count,
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
                            if ui.button("Explore").clicked() {
                                action_explore = Some(pack.path.clone());
                            }
                        });
                    });
                });
                ui.add_space(3.0);
            }
        });

        if let Some(p) = action_explore {
            self.start_load_explorer(p);
        } else if let Some(idx) = action_convert {
            if let Some(pack) = self.packs.get(idx).cloned() {
                self.load_pack_into_converter(&pack);
            }
        } else if let Some(p) = action_audit {
            self.start_audit(vec![p]);
        } else if let Some(p) = action_unpack {
            self.start_unpack(p);
        } else if let Some(p) = action_repack {
            self.start_repack(p);
        } else if let Some(del) = action_delete {
            self.pending_delete = Some(del);
        }
    }

    fn show_explorer_page(&mut self, ctx: &Context, ui: &mut Ui) {
        let Some(loaded) = self.loaded_pack.clone() else {
            ui.centered_and_justified(|ui| {
                ui.label("No pack loaded. Open Packs Manager and click 'Explore' on any pack, or drag & drop a .zip/.mcpack here.");
            });
            return;
        };

        ui.horizontal_wrapped(|ui| {
            for cat in AssetCategory::ALL {
                if ui
                    .selectable_label(self.asset_category == cat, cat.label())
                    .clicked()
                {
                    self.asset_category = cat;
                }
            }
            ui.separator();
            ui.label("Search:");
            ui.add(egui::TextEdit::singleline(&mut self.asset_filter).desired_width(180.0));
        });
        ui.separator();

        // Build filtered asset list
        let needle = self.asset_filter.trim().to_ascii_lowercase();
        let matches = |id: &str| needle.is_empty() || id.to_ascii_lowercase().contains(&needle);
        let mut items: Vec<AssetRef> = Vec::new();
        if matches!(
            self.asset_category,
            AssetCategory::All | AssetCategory::Textures
        ) {
            for id in loaded.ir.textures.keys() {
                if matches(id) {
                    items.push(AssetRef::Texture(id.clone()));
                }
            }
        }
        if matches!(
            self.asset_category,
            AssetCategory::All | AssetCategory::Models
        ) {
            for id in loaded.ir.models.keys() {
                if matches(id) {
                    items.push(AssetRef::Model(id.clone()));
                }
            }
        }
        if matches!(
            self.asset_category,
            AssetCategory::All | AssetCategory::Attachables
        ) {
            for id in loaded.ir.attachables.keys() {
                if matches(id) {
                    items.push(AssetRef::Attachable(id.clone()));
                }
            }
        }
        if matches!(
            self.asset_category,
            AssetCategory::All | AssetCategory::Sounds
        ) {
            for id in loaded.ir.sounds.files.keys() {
                if matches(id) {
                    items.push(AssetRef::SoundFile(id.clone()));
                }
            }
            for id in loaded.ir.sounds.events.keys() {
                if matches(id) {
                    items.push(AssetRef::SoundEvent(id.clone()));
                }
            }
        }
        if matches!(
            self.asset_category,
            AssetCategory::All | AssetCategory::Locales
        ) {
            for id in loaded.ir.locales.keys() {
                if matches(id) {
                    items.push(AssetRef::Locale(id.clone()));
                }
            }
        }
        if matches!(
            self.asset_category,
            AssetCategory::All | AssetCategory::Passthrough
        ) {
            for id in loaded.ir.passthrough_files.keys() {
                if matches(id) {
                    items.push(AssetRef::Passthrough(id.clone()));
                }
            }
        }

        ui.columns(2, |cols| {
            // Left column: Scrollable asset list
            cols[0].label(RichText::new(format!("Assets ({})", items.len())).strong());
            ScrollArea::vertical()
                .id_salt("explorer_asset_list")
                .show(&mut cols[0], |ui| {
                    for item in &items {
                        let is_sel = self.selected_asset.as_ref() == Some(item);
                        let label = format!("[{}] {}", item.badge(), item.id());
                        if ui.selectable_label(is_sel, label).clicked() {
                            self.selected_asset = Some(item.clone());
                        }
                    }
                });

            // Right column: Visual Inspector / 3D Viewport / Texture Preview
            ScrollArea::vertical()
                .id_salt("explorer_asset_detail")
                .show(&mut cols[1], |ui| {
                    match self.selected_asset.clone() {
                        Some(AssetRef::Texture(id)) => {
                            if let Some(tex) = loaded.ir.textures.get(&id) {
                                self.ensure_asset_texture_preview(ctx, tex);
                                ui.heading(&tex.id);
                                if let Some(prev) = &self.tex_preview {
                                    ui.label(format!(
                                        "Dimensions: {}x{} | Category: {:?} | Frames: {}",
                                        prev.dims.0,
                                        prev.dims.1,
                                        tex.category,
                                        prev.frames.len()
                                    ));
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::Slider::new(&mut self.tex_zoom, 1.0..=16.0)
                                                .text("Zoom"),
                                        );
                                        if prev.frames.len() > 1 {
                                            if ui
                                                .button(if self.anim_playing { "Pause" } else { "Play" })
                                                .clicked()
                                            {
                                                self.anim_playing = !self.anim_playing;
                                            }
                                            let max_f = prev.frames.len() - 1;
                                            ui.add(
                                                egui::Slider::new(&mut self.anim_frame_idx, 0..=max_f)
                                                    .text("Frame"),
                                            );
                                        }
                                    });

                                    // Advance animation playback
                                    if prev.frames.len() > 1 && self.anim_playing {
                                        let ms_per_frame =
                                            u64::from(prev.frametime_ticks).saturating_mul(50).max(50);
                                        if self.anim_last_step.elapsed().as_millis()
                                            >= u128::from(ms_per_frame)
                                        {
                                            self.anim_frame_idx =
                                                (self.anim_frame_idx + 1) % prev.frames.len();
                                            self.anim_last_step = Instant::now();
                                        }
                                        ctx.request_repaint_after(
                                            std::time::Duration::from_millis(ms_per_frame),
                                        );
                                    }

                                    let frame_idx = self
                                        .anim_frame_idx
                                        .min(prev.frames.len().saturating_sub(1));
                                    if let Some(handle) = prev.frames.get(frame_idx) {
                                        let base = handle.size_vec2();
                                        let disp = (base * self.tex_zoom)
                                            .min(Vec2::new(420.0, 420.0));
                                        let (rect, _) =
                                            ui.allocate_exact_size(disp, egui::Sense::hover());
                                        draw_checkerboard(ui, rect);
                                        ui.painter().image(
                                            handle.id(),
                                            rect,
                                            Rect::from_min_max(
                                                Pos2::new(0.0, 0.0),
                                                Pos2::new(1.0, 1.0),
                                            ),
                                            Color32::WHITE,
                                        );
                                    }
                                }
                            }
                        }
                        Some(AssetRef::Model(id)) => {
                            if let Some(model) = loaded.ir.models.get(&id) {
                                ui.heading(&model.id);
                                self.viewport_3d.show(ui, model, 300.0);
                                ui.add_space(6.0);
                                ui.label(format!(
                                    "Category: {:?} | Texture Size: {}x{} | Elements: {}",
                                    model.category,
                                    model.texture_size[0],
                                    model.texture_size[1],
                                    model.elements.len()
                                ));
                                if let Some(parent) = &model.parent {
                                    ui.label(format!("Parent: {parent}"));
                                }
                                let multi_axis = model
                                    .elements
                                    .iter()
                                    .filter(|e| {
                                        e.rotation
                                            .as_ref()
                                            .is_some_and(|r| r.extra_bedrock_euler.is_some())
                                    })
                                    .count();
                                let shade_263 = model
                                    .elements
                                    .iter()
                                    .filter(|e| e.shade_direction_override.is_some())
                                    .count();
                                ui.label(format!(
                                    "Multi-axis {{x,y,z}} elements: {multi_axis} | 26.3 shade_direction_override: {shade_263}"
                                ));
                                if !model.textures.is_empty() {
                                    ui.collapsing("Texture Bindings", |ui| {
                                        for (k, v) in &model.textures {
                                            ui.monospace(format!("#{k} -> {v}"));
                                        }
                                    });
                                }
                                if !model.display.is_empty() {
                                    ui.collapsing("Display Transforms", |ui| {
                                        for (slot, dt) in &model.display {
                                            ui.monospace(format!(
                                                "{slot}: rot={:?} trans={:?} scale={:?}",
                                                dt.rotation, dt.translation, dt.scale
                                            ));
                                        }
                                    });
                                }
                                if !model.elements.is_empty() {
                                    ui.collapsing(
                                        format!("Elements ({})", model.elements.len()),
                                        |ui| {
                                            for (i, el) in model.elements.iter().take(40).enumerate() {
                                                let grp = el.group.as_deref().unwrap_or("-");
                                                ui.monospace(format!(
                                                    "[{i:02}] {grp} [{:.1},{:.1},{:.1}]..[{:.1},{:.1},{:.1}] faces={}",
                                                    el.from[0],
                                                    el.from[1],
                                                    el.from[2],
                                                    el.to[0],
                                                    el.to[1],
                                                    el.to[2],
                                                    el.faces.len()
                                                ));
                                            }
                                        },
                                    );
                                }
                            }
                        }
                        Some(AssetRef::Attachable(id)) => {
                            if let Some(att) = loaded.ir.attachables.get(&id) {
                                ui.heading(&att.id);
                                ui.monospace(format!("item:     {}", att.item_identifier));
                                ui.monospace(format!(
                                    "texture:  {}",
                                    att.default_texture.as_deref().unwrap_or("-")
                                ));
                                ui.monospace(format!(
                                    "geometry: {}",
                                    att.default_geometry.as_deref().unwrap_or("-")
                                ));
                            }
                        }
                        Some(AssetRef::SoundFile(id)) => {
                            if let Some(sf) = loaded.ir.sounds.files.get(&id) {
                                ui.heading(&sf.id);
                                ui.label(format!("OGG size: {}", format_bytes(sf.data.len() as u64)));
                            }
                        }
                        Some(AssetRef::SoundEvent(id)) => {
                            if let Some(ev) = loaded.ir.sounds.events.get(&id) {
                                ui.heading(&id);
                                ui.label(format!("Variants: {}", ev.sounds.len()));
                                for s in &ev.sounds {
                                    ui.monospace(format!(
                                        "{} (vol={}, pitch={}, w={})",
                                        s.name, s.volume, s.pitch, s.weight
                                    ));
                                }
                            }
                        }
                        Some(AssetRef::Locale(code)) => {
                            if let Some(tbl) = loaded.ir.locales.get(&code) {
                                ui.heading(format!("{code} ({} keys)", tbl.entries.len()));
                                let mut pairs: Vec<(&String, &String)> = tbl.entries.iter().collect();
                                pairs.sort_by(|a, b| a.0.cmp(b.0));
                                for (k, v) in pairs.into_iter().take(60) {
                                    ui.monospace(format!("{k} = {v}"));
                                }
                            }
                        }
                        Some(AssetRef::Passthrough(path)) => {
                            if let Some(bytes) = loaded.ir.passthrough_files.get(&path) {
                                ui.heading(&path);
                                ui.label(format!("Size: {}", format_bytes(bytes.len() as u64)));
                                if let Ok(txt) = std::str::from_utf8(bytes) {
                                    ui.code(txt.chars().take(2000).collect::<String>());
                                }
                            }
                        }
                        None => {
                            ui.label("Select an asset on the left to inspect it.");
                        }
                    }
                });
        });
    }

    fn show_convert_page(&mut self, ui: &mut Ui) {
        ui.columns(2, |cols| {
            // Left: Conversion Form
            cols[0].heading("Conversion Configuration");
            cols[0].add_space(6.0);

            cols[0].horizontal(|ui| {
                ui.label("Input Pack: ");
                ui.add(egui::TextEdit::singleline(&mut self.input_path).desired_width(240.0));
                if ui.button("File...").clicked()
                    && let Some(file) = rfd::FileDialog::new()
                        .set_directory(&self.scan_dir)
                        .add_filter("Minecraft Pack", &["zip", "mcpack"])
                        .pick_file()
                {
                    self.input_path = file.display().to_string();
                }
            });

            cols[0].horizontal(|ui| {
                ui.label("Output Path:");
                ui.add(egui::TextEdit::singleline(&mut self.output_path).desired_width(240.0));
                if ui.button("Save As...").clicked()
                    && let Some(file) = rfd::FileDialog::new()
                        .set_directory(&self.scan_dir)
                        .save_file()
                {
                    self.output_path = file.display().to_string();
                }
            });

            cols[0].add_space(6.0);
            egui::Grid::new("convert_opts_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(&mut cols[0], |ui| {
                    ui.label("Target Edition:");
                    egui::ComboBox::from_id_salt("target_ed")
                        .selected_text(match self.target_edition {
                            None => "Auto (Flip)",
                            Some(Edition::Java) => "Java Edition",
                            Some(Edition::Bedrock) => "Bedrock Edition",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.target_edition, None, "Auto (Flip)");
                            if ui
                                .selectable_value(
                                    &mut self.target_edition,
                                    Some(Edition::Java),
                                    "Java Edition",
                                )
                                .clicked()
                                && !self.input_path.is_empty()
                            {
                                self.output_path =
                                    suggest_output_path(Path::new(&self.input_path), Edition::Java);
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
                            ui.selectable_value(&mut self.bedrock_geo_version, None, "Auto");
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
                            ui.selectable_value(&mut self.bedrock_manifest_version, None, "Auto");
                            ui.selectable_value(
                                &mut self.bedrock_manifest_version,
                                Some(3),
                                "v3 (SemVer)",
                            );
                            ui.selectable_value(&mut self.bedrock_manifest_version, Some(2), "v2");
                            ui.selectable_value(&mut self.bedrock_manifest_version, Some(1), "v1");
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
                            ui.selectable_value(&mut self.emit_java_items, None, "Auto (1.21.4+)");
                            ui.selectable_value(&mut self.emit_java_items, Some(true), "Force On");
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

            cols[0].add_space(12.0);
            let can_run = self.busy_message.is_none();
            if cols[0]
                .add_enabled(
                    can_run,
                    egui::Button::new(RichText::new("Convert Pack").strong().size(15.0))
                        .min_size(Vec2::new(180.0, 36.0)),
                )
                .clicked()
            {
                self.start_convert();
            }

            // Right: Conversion Report
            cols[1].heading("Conversion Report");
            cols[1].add_space(6.0);
            ScrollArea::vertical()
                .id_salt("convert_report_scroll")
                .show(&mut cols[1], |ui| match &self.last_conversion {
                    None => {
                        ui.label("Run a conversion on the left to view diagnostics.");
                    }
                    Some(Err(err)) => {
                        ui.colored_label(Color32::LIGHT_RED, format!("Failed: {err}"));
                    }
                    Some(Ok(res)) => {
                        ui.colored_label(
                            Color32::LIGHT_GREEN,
                            format!(
                                "SUCCESS: {} -> {} ({} ms)",
                                res.source, res.target, res.elapsed_ms
                            ),
                        );
                        ui.monospace(format!("Output: {}", res.output_path.display()));
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("Converted: {}", res.report.converted_count));
                            ui.separator();
                            ui.label(format!("Pass-through: {}", res.report.passthrough_count));
                            ui.separator();
                            ui.label(format!("Lossy: {}", res.report.lossy_count));
                            ui.separator();
                            ui.label(format!("Skipped: {}", res.report.skipped_count));
                            ui.separator();
                            ui.label(format!("Warnings: {}", res.report.warnings.len()));
                        });
                        if ui.button("Export Report JSON").clicked() {
                            let out = self.scan_dir.join("conversion_report.json");
                            if let Ok(()) = export_report_json(&res.report, &out) {
                                self.status_banner = format!("Exported -> {}", out.display());
                            }
                        }
                        if !res.report.lossy_conversions.is_empty() {
                            ui.add_space(6.0);
                            ui.label(RichText::new("Lossy Conversions:").color(Color32::YELLOW));
                            for item in &res.report.lossy_conversions {
                                ui.monospace(format!("• [{}] {}", item.asset, item.detail));
                            }
                        }
                        if !res.report.warnings.is_empty() {
                            ui.add_space(6.0);
                            ui.label(RichText::new("Warnings:").color(Color32::LIGHT_BLUE));
                            for w in &res.report.warnings {
                                ui.monospace(format!(
                                    "• [{:?}] {}: {}",
                                    w.severity, w.asset, w.message
                                ));
                            }
                        }
                    }
                });
        });
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
                        if o.passed {
                            ui.colored_label(Color32::LIGHT_GREEN, "PASS");
                        } else {
                            ui.colored_label(Color32::LIGHT_RED, "FAIL");
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
        ui.horizontal_wrapped(|ui| {
            ui.label("Pack A:");
            ui.add(egui::TextEdit::singleline(&mut self.diff_path_a).desired_width(230.0));
            ui.label("Pack B:");
            ui.add(egui::TextEdit::singleline(&mut self.diff_path_b).desired_width(230.0));
            let enabled = self.busy_message.is_none()
                && !self.diff_path_a.trim().is_empty()
                && !self.diff_path_b.trim().is_empty();
            if ui
                .add_enabled(enabled, egui::Button::new("Compare Packs"))
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
                ui.label("Choose Pack A and Pack B (or check 2 packs in Packs Manager and click 'Compare Selected').");
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
            .max_height(260.0)
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
}

impl eframe::App for RpGuiApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.render_ui(ctx);
    }
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
    fn headless_egui_renders_all_six_desktop_pages_and_3d_viewport() {
        let dir = tempdir().expect("tempdir");
        let mut container = PackContainer::new();
        container
            .insert(
                "pack.mcmeta",
                br#"{"pack":{"description":"GUI Test Pack","min_format":[97,0],"max_format":[97,1]}}"#
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

        let zip_path = dir.path().join("gui_sample.zip");
        std::fs::write(&zip_path, container.write_to_zip_bytes().expect("zip")).expect("save");

        let mappings = MappingTable::default_embedded().expect("mappings");
        let packs =
            scan_directory_for_packs(dir.path(), &mappings, IoLimits::default()).expect("scan");
        let (ed, ir, rep) = load_pack_ir(&zip_path, &mappings, IoLimits::default()).expect("ir");
        let diff = diff_packs(&zip_path, &zip_path, &mappings, IoLimits::default()).expect("diff");
        let audit = audit_pack_roundtrip(
            &zip_path,
            Some("26.3"),
            &VersionSpecificOptions::default(),
            &mappings,
            IoLimits::default(),
        );
        assert!(audit.passed);

        let mut app = RpGuiApp::new(dir.path().to_path_buf(), None, mappings);
        app.packs = packs;
        app.loaded_pack = Some(LoadedPack {
            path: zip_path,
            edition: ed,
            ir,
            report: rep,
        });
        app.selected_asset = Some(AssetRef::Model(String::from("block/glow")));
        app.pack_diff = Some(Ok(diff));
        app.audit_outcomes.push(audit);

        let ctx = Context::default();
        for page in GuiPage::ALL {
            app.page = page;
            let _ = ctx.run(egui::RawInput::default(), |c| {
                app.render_ui(c);
            });
        }
        // Also render animated texture preview path
        app.page = GuiPage::Explorer;
        app.selected_asset = Some(AssetRef::Texture(String::from("block/glow")));
        let _ = ctx.run(egui::RawInput::default(), |c| {
            app.render_ui(c);
        });
    }
}
