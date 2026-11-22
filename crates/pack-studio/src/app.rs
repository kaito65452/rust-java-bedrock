use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use egui::{
    Align, Align2, Color32, ColorImage, Context, CornerRadius, FontId, Frame, Key, Layout, Margin,
    Order, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit, TextureHandle,
    TextureOptions, TopBottomPanel, Vec2,
};
use egui_dock::{DockArea, TabViewer};
use egui_extras::{Column, TableBuilder};
use glam::Vec2 as GlamVec2;
use image::RgbaImage;
use rp_core::{AuditStatus, Edition, RotationSnapMode, WarningSeverity};

use crate::artist::{
    BiomeTintPreset, TextureFormatKind, apply_biome_tint, build_alpha_mask_preview,
    composite_grass_side_tga, decode_texture_bytes, encode_texture_bytes, resize_nearest_neighbor,
};
use crate::blockbench::BlockbenchHost;
use crate::blockbench::webview::WebViewLaunchOutcome;
use crate::command_palette::{StudioCommand, filter_commands};
use crate::explorer::{
    ConflictResolution, ExplorerClipboard, ExplorerInlineEdit, FileTreeNode, PendingImportItem,
    create_blank_png_bytes, delete_pack_asset, duplicate_pack_asset, filter_quick_open_files,
    generate_minecraft_resource_id, make_unique_rel_path, paste_pack_asset, prepare_import_bytes,
    rename_pack_asset,
};
use crate::packager::{
    CompressionPreset, PackageFormat, export_pack_archive, import_pack_icon_file,
    render_output_filename,
};
use crate::pipeline::{
    BackgroundEvent, ConversionDirection, PreflightAssetStatus, spawn_convert, spawn_load_pack,
    spawn_preflight, spawn_semantic_audit,
};
use crate::state::{ActivityTab, BottomTab, CodeTabState, EditorTab, TextureTabState, WorkbenchState};
use crate::theme::{self, vscode_dock_style};
use crate::watcher::PackWatcher;
use crate::wizard::{
    ProjectTemplateKind, render_mc_formatting_toolbar, render_minecraft_formatted_preview,
    scaffold_new_project,
};
use crate::workspace::{
    LoadedPack, is_text_asset, materialize_temp_asset, open_in_external_app,
    reveal_in_file_manager,
};

#[derive(Debug, Clone)]
enum DialogTarget {
    OpenFolder,
    OpenArchive,
    SaveConvertOutput,
    WizardParentDir,
    WizardPickIcon,
    MetadataImportIcon,
    ExportOutputDir,
    ImportAssetsToDir(String),
}

struct DialogResult {
    target: DialogTarget,
    paths: Vec<PathBuf>,
}

/// Deferred UI actions emitted from inside `egui_dock::TabViewer`.
enum TabAction {
    OpenFolder,
    OpenArchive,
    OpenNewProjectWizard,
    OpenExportDialog,
    OpenRecent(PathBuf),
    RunPreflight,
    ExecuteConvert,
    PickConvertOutput,
    SaveCodeTab { path: String, text: String },
    DownscaleTexture { path: String, target_size: u32 },
    CompositeGrassSideTga { path: String, biome: BiomeTintPreset },
    OpenAssetPath(String),
    OpenInBlockbenchStudio(String),
    NewBedrockGeoInBlockbench,
}

/// Deferred Explorer tree actions collected while walking the recursive `FileTreeNode`.
enum ExplorerAction {
    SelectDir(String),
    SelectFile(String),
    OpenFile(String),
    OpenFileSide(String),
    StartInlineNewFile(String),
    StartInlineNewFolder(String),
    StartInlineRename { path: String, is_dir: bool },
    ImportToDir(String),
    NewBlankTexture(String),
    OpenInBlockbench(String),
    NewBedrockGeo(String),
    OpenExternalEditor(String),
    DuplicateFile(String),
    CutItem(String),
    CopyItem(String),
    PasteInto(String),
    DeleteItem(String),
    CopyRelativePath(String),
    CopyResourceId(String),
    RevealInFileManager(String),
}

/// Main `eframe::App` implementing the VS Code-inspired PackStudio workbench.
pub struct PackStudioApp {
    pub state: WorkbenchState,
    pub blockbench: BlockbenchHost,
    watcher: Option<PackWatcher>,
    bg_tx: Sender<BackgroundEvent>,
    bg_rx: Receiver<BackgroundEvent>,
    dialog_tx: Sender<DialogResult>,
    dialog_rx: Receiver<DialogResult>,
    texture_cache: HashMap<String, TextureHandle>,
    theme_applied: bool,
    palette_needs_focus: bool,
    quick_open_needs_focus: bool,
    inline_edit_needs_focus: bool,
    pending_commands: Vec<StudioCommand>,
}

impl PackStudioApp {
    /// Creates a new `PackStudioApp`, optionally scheduling an initial pack path to open.
    #[must_use]
    pub fn new(initial_path: Option<PathBuf>) -> Self {
        let (bg_tx, bg_rx) = mpsc::channel();
        let (dialog_tx, dialog_rx) = mpsc::channel();
        let mut app = Self {
            state: WorkbenchState::new(),
            blockbench: BlockbenchHost::new(),
            watcher: None,
            bg_tx,
            bg_rx,
            dialog_tx,
            dialog_rx,
            texture_cache: HashMap::new(),
            theme_applied: false,
            palette_needs_focus: false,
            quick_open_needs_focus: false,
            inline_edit_needs_focus: false,
            pending_commands: Vec::new(),
        };
        if let Some(path) = initial_path {
            app.start_load_pack(path);
        }
        app
    }

    /// Starts loading a folder or `.zip`/`.mcpack` archive on a background thread.
    pub fn start_load_pack(&mut self, path: PathBuf) {
        self.state.busy_label = Some(format!("Loading {}...", path.display()));
        spawn_load_pack(path, self.bg_tx.clone());
    }

    /// Starts a Preflight Scan on a background thread.
    pub fn start_preflight(&mut self) {
        let Some(pack) = &self.state.loaded_pack else {
            self.state.log_output(
                WarningSeverity::Warning,
                "Open a resource pack before running Preflight Scan.",
            );
            return;
        };
        let mut cfg = self.state.conversion_config.clone();
        cfg.convert_pbr = self.state.settings.convert_pbr || cfg.convert_pbr;
        cfg.uv_atlas_guard = self.state.settings.uv_atlas_guard;
        self.state.busy_label = Some(format!("Preflight ({})...", cfg.direction.label()));
        spawn_preflight(pack.container.clone(), cfg, self.bg_tx.clone());
    }

    /// Executes Java <-> Bedrock conversion + Semantic Audit Gate on a background thread.
    pub fn start_convert(&mut self) {
        let Some(pack) = &self.state.loaded_pack else {
            self.state.log_output(
                WarningSeverity::Warning,
                "Open a resource pack before executing conversion.",
            );
            return;
        };
        let mut cfg = self.state.conversion_config.clone();
        cfg.convert_pbr = self.state.settings.convert_pbr || cfg.convert_pbr;
        cfg.uv_atlas_guard = self.state.settings.uv_atlas_guard;
        self.state.busy_label = Some(format!("Converting ({})...", cfg.direction.label()));
        spawn_convert(pack.container.clone(), cfg, self.bg_tx.clone());
    }

    /// Runs Semantic Validation Audit on a background thread.
    pub fn start_semantic_audit(&mut self) {
        let Some(pack) = &self.state.loaded_pack else {
            self.state.log_output(
                WarningSeverity::Warning,
                "Open a resource pack before running Semantic Validation.",
            );
            return;
        };
        self.state.busy_label = Some(String::from("Auditing pack..."));
        spawn_semantic_audit(
            pack.container.clone(),
            self.state.settings.uv_atlas_guard,
            self.bg_tx.clone(),
        );
    }

    fn active_project_root(&self) -> Option<PathBuf> {
        self.state
            .loaded_pack
            .as_ref()
            .and_then(|p| p.source.directory_path())
            .map(Path::to_path_buf)
    }

    fn spawn_file_dialog(&self, target: DialogTarget) {
        let tx = self.dialog_tx.clone();
        let start_dir = self
            .state
            .loaded_pack
            .as_ref()
            .and_then(|p| p.source.disk_path())
            .and_then(|p| if p.is_dir() { Some(p) } else { p.parent() })
            .map(Path::to_path_buf);

        thread::spawn(move || {
            let mut dlg = rfd::FileDialog::new();
            if let Some(dir) = start_dir
                && dir.is_dir()
            {
                dlg = dlg.set_directory(dir);
            }
            let paths = match &target {
                DialogTarget::OpenFolder
                | DialogTarget::WizardParentDir
                | DialogTarget::ExportOutputDir => dlg.pick_folder().into_iter().collect(),
                DialogTarget::OpenArchive => dlg
                    .add_filter("Resource Pack Archive", &["zip", "mcpack"])
                    .pick_file()
                    .into_iter()
                    .collect(),
                DialogTarget::SaveConvertOutput => dlg
                    .set_file_name("converted_pack.mcpack")
                    .add_filter("Bedrock / Java Pack", &["mcpack", "zip"])
                    .save_file()
                    .into_iter()
                    .collect(),
                DialogTarget::WizardPickIcon | DialogTarget::MetadataImportIcon => dlg
                    .add_filter("Pack Icon Image", &["png", "jpg", "jpeg", "webp", "tga"])
                    .pick_file()
                    .into_iter()
                    .collect(),
                DialogTarget::ImportAssetsToDir(_) => dlg
                    .add_filter(
                        "Minecraft Pack Assets",
                        &["png", "tga", "jpg", "jpeg", "webp", "json", "ogg", "lang", "mcmeta"],
                    )
                    .pick_files()
                    .unwrap_or_default(),
            };
            let _ = tx.send(DialogResult { target, paths });
        });
    }

    fn attach_watcher_for_pack(&mut self, pack: &LoadedPack) {
        self.watcher = None;
        if let Some(dir) = pack.source.directory_path() {
            match PackWatcher::watch_directory(dir) {
                Ok(w) => {
                    self.state.watcher_status = String::from("Watching");
                    self.state.log_output(
                        WarningSeverity::Info,
                        format!("Hot-Reload watcher active on '{}'", w.root().display()),
                    );
                    self.watcher = Some(w);
                }
                Err(err) => {
                    self.state.watcher_status = String::from("Watcher Err");
                    self.state.log_output(
                        WarningSeverity::Warning,
                        format!("Could not start hot-reload watcher: {err:#}"),
                    );
                }
            }
        } else {
            self.state.watcher_status = String::from("Archive");
        }
    }

    fn poll_background_events(&mut self) {
        while let Ok(event) = self.bg_rx.try_recv() {
            match event {
                BackgroundEvent::Log { level, message } => {
                    self.state.log_output(level, message);
                }
                BackgroundEvent::PackLoaded(res) => {
                    self.state.busy_label = None;
                    match res {
                        Ok(pack) => {
                            let label = format!(
                                "Loaded '{}' ({}, {} files, {} textures)",
                                pack.pack_name,
                                pack.edition_label,
                                pack.container.len(),
                                pack.texture_count
                            );
                            let root = pack.source.directory_path().map(Path::to_path_buf);
                            self.attach_watcher_for_pack(&pack);
                            self.blockbench.sync_project_root(root);
                            self.state.set_loaded_pack(pack);
                            self.texture_cache.clear();
                            self.state.log_output(WarningSeverity::Info, label);
                            self.start_semantic_audit();
                        }
                        Err(err) => {
                            self.state.log_output(
                                WarningSeverity::Error,
                                format!("Failed to load pack: {err}"),
                            );
                        }
                    }
                }
                BackgroundEvent::PreflightFinished(res) => {
                    self.state.busy_label = None;
                    match res {
                        Ok(preflight) => {
                            self.state.log_output(
                                WarningSeverity::Info,
                                format!(
                                    "Preflight finished in {} ms: {} ready, {} lossy, {} skipped, {} warnings — {}",
                                    preflight.elapsed_ms,
                                    preflight.ready_count,
                                    preflight.lossy_count,
                                    preflight.skipped_count,
                                    preflight.warning_count,
                                    preflight.semantic_preview_summary
                                ),
                            );
                            self.state.apply_diagnostics(
                                preflight.semantic_preview_status,
                                preflight.semantic_preview_summary.clone(),
                                preflight.diagnostics.clone(),
                            );
                            self.state.preflight_report = Some(preflight);
                            self.state.open_conversion_dashboard();
                        }
                        Err(err) => {
                            self.state.log_output(
                                WarningSeverity::Error,
                                format!("Preflight failed: {err}"),
                            );
                        }
                    }
                }
                BackgroundEvent::ConversionFinished(res) => {
                    self.state.busy_label = None;
                    match res {
                        Ok(conv) => {
                            let level = match conv.gate_status {
                                AuditStatus::Passed => WarningSeverity::Info,
                                AuditStatus::Partial => WarningSeverity::Warning,
                                AuditStatus::Failed => WarningSeverity::Error,
                            };
                            let written_note = conv
                                .output_written_to
                                .as_ref()
                                .map(|p| format!(" -> saved '{}'", p.display()))
                                .unwrap_or_default();
                            self.state.log_output(
                                level,
                                format!(
                                    "Conversion ({} -> {}) finished in {} ms [{}]: {}{}",
                                    conv.source_edition,
                                    conv.target_edition,
                                    conv.elapsed_ms,
                                    conv.gate_status,
                                    conv.gate_summary,
                                    written_note
                                ),
                            );
                            self.state.apply_diagnostics(
                                conv.gate_status,
                                conv.gate_summary.clone(),
                                conv.report.warnings.clone(),
                            );
                            if conv.gate_status == AuditStatus::Failed {
                                self.state.bottom_panel_visible = true;
                                self.state.active_bottom_tab = BottomTab::Problems;
                            }
                            self.state.last_conversion = Some(conv);
                        }
                        Err(err) => {
                            self.state.apply_diagnostics(
                                AuditStatus::Failed,
                                format!("FAILED: {err}"),
                                Vec::new(),
                            );
                            self.state.log_output(
                                WarningSeverity::Error,
                                format!("Conversion error: {err}"),
                            );
                        }
                    }
                }
                BackgroundEvent::AuditFinished(res) => {
                    self.state.busy_label = None;
                    match res {
                        Ok(audit) => {
                            let level = match audit.status {
                                AuditStatus::Passed => WarningSeverity::Info,
                                AuditStatus::Partial => WarningSeverity::Warning,
                                AuditStatus::Failed => WarningSeverity::Error,
                            };
                            self.state.log_output(
                                level,
                                format!(
                                    "Semantic Audit ({}, {} ms) [{}]: {}",
                                    audit.edition, audit.elapsed_ms, audit.status, audit.summary
                                ),
                            );
                            self.state.apply_diagnostics(
                                audit.status,
                                audit.summary,
                                audit.diagnostics,
                            );
                        }
                        Err(err) => {
                            self.state.apply_diagnostics(
                                AuditStatus::Failed,
                                format!("FAILED: {err}"),
                                Vec::new(),
                            );
                            self.state.log_output(
                                WarningSeverity::Error,
                                format!("Semantic Audit error: {err}"),
                            );
                        }
                    }
                }
            }
        }
    }

    fn poll_blockbench_saves(&mut self) {
        let events = self.blockbench.poll_save_events();
        if events.is_empty() {
            return;
        }
        let mut any_saved = false;
        for ev in events {
            for (rel_path, bytes) in ev.saved_files {
                if let Some(pack) = &mut self.state.loaded_pack
                    && let Ok(()) = pack.container.insert(&rel_path, bytes.clone())
                {
                    any_saved = true;
                }
                self.state.hot_reload_asset(&rel_path, &bytes);
                self.texture_cache
                    .retain(|k, _| !k.starts_with(&format!("{rel_path}:")));
                self.state.log_output(
                    WarningSeverity::Info,
                    format!("Blockbench 2-way sync saved '{rel_path}'"),
                );
            }
        }
        if any_saved {
            if let Some(pack) = &mut self.state.loaded_pack {
                pack.refresh_index();
            }
            self.start_semantic_audit();
        }
    }

    fn poll_file_dialogs(&mut self) {
        while let Ok(res) = self.dialog_rx.try_recv() {
            if res.paths.is_empty() {
                continue;
            }
            match res.target {
                DialogTarget::OpenFolder | DialogTarget::OpenArchive => {
                    self.start_load_pack(res.paths[0].clone());
                }
                DialogTarget::SaveConvertOutput => {
                    self.state.conversion_config.output_path = res.paths[0].display().to_string();
                }
                DialogTarget::WizardParentDir => {
                    self.state.wizard.target_parent_dir = res.paths[0].display().to_string();
                }
                DialogTarget::WizardPickIcon => {
                    self.state.wizard.icon_path = Some(res.paths[0].clone());
                }
                DialogTarget::MetadataImportIcon => {
                    self.import_pack_icon_from_disk(&res.paths[0]);
                }
                DialogTarget::ExportOutputDir => {
                    self.state.export_config.output_dir = res.paths[0].display().to_string();
                }
                DialogTarget::ImportAssetsToDir(target_dir) => {
                    self.queue_or_import_disk_files(res.paths, &target_dir);
                }
            }
        }
    }

    fn poll_hot_reload(&mut self) {
        let changed_paths = self
            .watcher
            .as_ref()
            .map(PackWatcher::poll_changed_relative_paths)
            .unwrap_or_default();
        if changed_paths.is_empty() {
            return;
        }
        let Some(root) = self.active_project_root() else {
            return;
        };

        let mut any_reloaded = false;
        for rel in changed_paths {
            let full = root.join(&rel);
            if full.is_file()
                && let Ok(bytes) = fs::read(&full)
            {
                if let Some(pack) = &mut self.state.loaded_pack
                    && pack.container.insert(&rel, bytes.clone()).is_ok()
                {
                    any_reloaded = true;
                }
                self.state.hot_reload_asset(&rel, &bytes);
                self.texture_cache
                    .retain(|k, _| !k.starts_with(&format!("{rel}:")));
                self.state.log_output(
                    WarningSeverity::Info,
                    format!("Hot-reloaded '{rel}' from disk"),
                );
            }
        }
        if any_reloaded
            && let Some(pack) = &mut self.state.loaded_pack
        {
            pack.refresh_index();
        }
    }

    fn poll_os_dropped_files(&mut self, ctx: &Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        if self.state.loaded_pack.is_none() {
            self.start_load_pack(dropped[0].clone());
            return;
        }
        let target_dir = self.state.explorer_selected_dir.clone();
        self.queue_or_import_disk_files(dropped, &target_dir);
    }

    fn queue_or_import_disk_files(&mut self, files: Vec<PathBuf>, target_dir: &str) {
        for src in files {
            if !src.is_file() {
                continue;
            }
            let item = PendingImportItem::from_path(src, target_dir.to_string());
            let target_rel = item.resolved_target_rel_path();
            let has_collision = self
                .state
                .loaded_pack
                .as_ref()
                .is_some_and(|p| p.container.contains(&target_rel));

            // If file needs PNG conversion, sanitization, or collides, prompt via Smart Import Helper
            if item.needs_png_conversion || item.needs_sanitization || has_collision {
                self.state.pending_imports.push(item);
            } else {
                self.commit_import_item(&item, ConflictResolution::Overwrite);
            }
        }
    }

    fn commit_import_item(&mut self, item: &PendingImportItem, res: ConflictResolution) {
        let prepared = {
            let Some(pack) = &self.state.loaded_pack else {
                return;
            };
            prepare_import_bytes(item, &pack.container, res)
        };
        match prepared {
            Ok(Some((rel_path, bytes))) => {
                if let Some(pack) = &mut self.state.loaded_pack {
                    match pack.write_asset(&rel_path, bytes) {
                        Ok(()) => {
                            self.state.log_output(
                                WarningSeverity::Info,
                                format!("Imported asset -> '{rel_path}'"),
                            );
                            self.state.open_asset_tab(&rel_path);
                        }
                        Err(err) => {
                            self.state.log_output(
                                WarningSeverity::Error,
                                format!("Import write failed: {err:#}"),
                            );
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(err) => {
                self.state.log_output(
                    WarningSeverity::Error,
                    format!("Import failed: {err:#}"),
                );
            }
        }
    }

    fn import_pack_icon_from_disk(&mut self, src: &Path) {
        let resize = self.state.metadata_form.icon_resize_target;
        match import_pack_icon_file(src, resize) {
            Ok(png_bytes) => {
                if let Some(pack) = &mut self.state.loaded_pack {
                    let icon_rel = match pack.edition {
                        Edition::Java => "pack.png",
                        Edition::Bedrock => "pack_icon.png",
                    };
                    match pack.write_asset(icon_rel, png_bytes.clone()) {
                        Ok(()) => {
                            self.state.metadata_form.icon_revision =
                                self.state.metadata_form.icon_revision.saturating_add(1);
                            self.texture_cache
                                .retain(|k, _| !k.starts_with("__pack_icon__") && !k.starts_with(icon_rel));
                            self.state.hot_reload_asset(icon_rel, &png_bytes);
                            self.state.metadata_form.status_message =
                                Some(format!("Updated {icon_rel} successfully."));
                            self.state.log_output(
                                WarningSeverity::Info,
                                format!("Imported Pack Icon -> '{icon_rel}'"),
                            );
                        }
                        Err(err) => {
                            self.state.log_output(
                                WarningSeverity::Error,
                                format!("Failed saving {icon_rel}: {err:#}"),
                            );
                        }
                    }
                }
            }
            Err(err) => {
                self.state.log_output(
                    WarningSeverity::Error,
                    format!("Failed importing pack icon: {err:#}"),
                );
            }
        }
    }

    fn launch_asset_in_blockbench_studio(&mut self, rel_path: &str) {
        let root = self.active_project_root();
        let container_opt = self.state.loaded_pack.as_ref().map(|p| p.container.clone());
        let Some(container) = container_opt else {
            self.state.log_output(
                WarningSeverity::Warning,
                "Open a resource pack before launching Blockbench Studio.",
            );
            return;
        };
        match self
            .blockbench
            .open_asset_in_blockbench(root, &container, rel_path)
        {
            Ok(WebViewLaunchOutcome::OpenedNative) => {
                self.state.log_output(
                    WarningSeverity::Info,
                    format!("Opened '{rel_path}' in native Blockbench WebView Studio."),
                );
            }
            Ok(WebViewLaunchOutcome::FallbackToBrowser { reason }) => {
                self.state.log_output(
                    WarningSeverity::Info,
                    format!("Blockbench Bridge ready for '{rel_path}': {reason}"),
                );
            }
            Err(err) => {
                // Fallback to external Blockbench binary if local bundle isn't built yet
                self.state.log_output(
                    WarningSeverity::Warning,
                    format!("Blockbench Web Studio unavailable ({err:#}); falling back to external launcher."),
                );
                self.open_asset_externally(rel_path, true);
            }
        }
    }

    fn launch_new_bedrock_geo_in_blockbench(&mut self, parent_dir: &str) {
        let root = self.active_project_root();
        let base_dir = if parent_dir.is_empty() {
            "models/entity"
        } else {
            parent_dir.trim_matches('/')
        };
        let default_rel = format!("{base_dir}/custom_model.geo.json");
        let target_rel = self
            .state
            .loaded_pack
            .as_ref()
            .map_or(default_rel.clone(), |p| {
                if p.container.contains(&default_rel) {
                    make_unique_rel_path(&p.container, &default_rel)
                } else {
                    default_rel
                }
            });
        let stem = target_rel
            .rsplit('/')
            .next()
            .unwrap_or("custom_model")
            .trim_end_matches(".geo.json");
        let identifier = format!("geometry.{stem}");

        match self
            .blockbench
            .new_bedrock_geometry_in_blockbench(root, &target_rel, &identifier)
        {
            Ok(WebViewLaunchOutcome::OpenedNative) => {
                self.state.log_output(
                    WarningSeverity::Info,
                    format!("Created new Bedrock Geometry '{target_rel}' in Blockbench WebView."),
                );
            }
            Ok(WebViewLaunchOutcome::FallbackToBrowser { reason }) => {
                self.state.log_output(
                    WarningSeverity::Info,
                    format!("New Bedrock Geometry '{target_rel}' queued on Blockbench Bridge: {reason}"),
                );
            }
            Err(err) => {
                self.state.log_output(
                    WarningSeverity::Warning,
                    format!("Could not start Blockbench Web Studio: {err:#}"),
                );
            }
        }
    }

    fn execute_command(&mut self, cmd: StudioCommand) {
        match cmd {
            StudioCommand::OpenFolder => self.spawn_file_dialog(DialogTarget::OpenFolder),
            StudioCommand::OpenArchive => self.spawn_file_dialog(DialogTarget::OpenArchive),
            StudioCommand::NewProjectWizard => {
                self.state.wizard.open = true;
                self.state.wizard.error_message = None;
            }
            StudioCommand::ExportPackage => {
                self.state.export_config.open = true;
                self.state.export_config.last_error = None;
            }
            StudioCommand::QuickOpenFile => {
                self.state.open_quick_open();
                self.quick_open_needs_focus = true;
            }
            StudioCommand::ReloadWorkspace => {
                if let Some(disk_path) = self
                    .state
                    .loaded_pack
                    .as_ref()
                    .and_then(|p| p.source.disk_path())
                    .map(Path::to_path_buf)
                {
                    self.start_load_pack(disk_path);
                } else {
                    self.state.log_output(
                        WarningSeverity::Info,
                        "No disk-backed workspace to reload.",
                    );
                }
            }
            StudioCommand::ConvertJavaToBedrockPreflight => {
                self.state.conversion_config.direction = ConversionDirection::JavaToBedrock;
                self.state.open_conversion_dashboard();
                self.start_preflight();
            }
            StudioCommand::ConvertBedrockToJavaPreflight => {
                self.state.conversion_config.direction = ConversionDirection::BedrockToJava;
                self.state.open_conversion_dashboard();
                self.start_preflight();
            }
            StudioCommand::OpenConversionDashboard => {
                self.state.open_conversion_dashboard();
            }
            StudioCommand::RunSemanticAudit => {
                self.start_semantic_audit();
            }
            StudioCommand::OpenInBlockbenchStudio => {
                if let Some(rel) = self.state.active_asset_path() {
                    self.launch_asset_in_blockbench_studio(&rel);
                } else {
                    self.launch_new_bedrock_geo_in_blockbench("models/entity");
                }
            }
            StudioCommand::NewBedrockGeoInBlockbench => {
                let dir = self.state.explorer_selected_dir.clone();
                self.launch_new_bedrock_geo_in_blockbench(&dir);
            }
            StudioCommand::OpenActiveInExternalEditor => {
                if let Some(rel) = self.state.active_asset_path() {
                    self.open_asset_externally(&rel, false);
                } else {
                    self.state.log_output(
                        WarningSeverity::Warning,
                        "No active asset tab to open in external editor.",
                    );
                }
            }
            StudioCommand::SaveActiveEditor => match self.state.save_active_code_tab() {
                Ok(Some(saved_path)) => {
                    self.state.log_output(
                        WarningSeverity::Info,
                        format!("Saved '{saved_path}'"),
                    );
                }
                Ok(None) => {
                    self.state.log_output(
                        WarningSeverity::Info,
                        "No dirty text/JSON editor tab active to save.",
                    );
                }
                Err(err) => {
                    self.state.log_output(
                        WarningSeverity::Error,
                        format!("Save failed: {err:#}"),
                    );
                }
            },
            StudioCommand::ToggleSidebar => self.state.toggle_sidebar(),
            StudioCommand::ToggleBottomPanel => self.state.toggle_bottom_panel(),
            StudioCommand::FocusExplorer => self.state.focus_activity(ActivityTab::Explorer),
            StudioCommand::FocusSearch => self.state.focus_activity(ActivityTab::Search),
            StudioCommand::FocusConverter => self.state.focus_activity(ActivityTab::Converter),
            StudioCommand::FocusMetadata => self.state.focus_activity(ActivityTab::Metadata),
            StudioCommand::FocusSettings => self.state.focus_activity(ActivityTab::Settings),
        }
    }

    fn open_asset_externally(&mut self, rel_path: &str, prefer_blockbench: bool) {
        let exe = if prefer_blockbench {
            self.state.settings.blockbench_path.clone()
        } else {
            self.state.settings.external_image_editor.clone()
        };

        let disk_file = self
            .state
            .loaded_pack
            .as_ref()
            .and_then(|p| p.resolve_disk_file(rel_path))
            .filter(|p| p.exists());

        let target = if let Some(path) = disk_file {
            Ok(path)
        } else if let Some(bytes) = self
            .state
            .loaded_pack
            .as_ref()
            .and_then(|p| p.container.get(rel_path))
        {
            materialize_temp_asset(rel_path, bytes)
        } else {
            Err(anyhow::anyhow!("Asset '{rel_path}' not found in pack"))
        };

        match target.and_then(|p| open_in_external_app(&p, &exe)) {
            Ok(()) => {
                self.state.log_output(
                    WarningSeverity::Info,
                    format!(
                        "Opened '{rel_path}' in {}",
                        if prefer_blockbench {
                            "Blockbench"
                        } else {
                            "External Editor"
                        }
                    ),
                );
            }
            Err(err) => {
                self.state.log_output(
                    WarningSeverity::Warning,
                    format!("External launch error: {err:#}"),
                );
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &Context) {
        let wants_keyboard = ctx.wants_keyboard_input();
        let mut cmds = Vec::new();
        let mut trigger_f2_rename = false;
        let mut trigger_del_item = false;

        ctx.input(|i| {
            let ctrl = i.modifiers.ctrl || i.modifiers.command;
            let shift = i.modifiers.shift;

            if (ctrl && shift && i.key_pressed(Key::P)) || i.key_pressed(Key::F1) {
                if self.state.command_palette_open {
                    self.state.close_command_palette();
                } else {
                    self.state.open_command_palette();
                    self.palette_needs_focus = true;
                }
            } else if ctrl && !shift && i.key_pressed(Key::P) {
                if self.state.quick_open_visible {
                    self.state.close_quick_open();
                } else {
                    self.state.open_quick_open();
                    self.quick_open_needs_focus = true;
                }
            }

            if i.key_pressed(Key::Escape) {
                if self.state.command_palette_open {
                    self.state.close_command_palette();
                }
                if self.state.quick_open_visible {
                    self.state.close_quick_open();
                }
                if self.state.explorer_inline_edit.is_some() {
                    self.state.explorer_inline_edit = None;
                }
            }
            if ctrl && !shift && i.key_pressed(Key::B) {
                cmds.push(StudioCommand::ToggleSidebar);
            }
            if ctrl && !shift && i.key_pressed(Key::J) {
                cmds.push(StudioCommand::ToggleBottomPanel);
            }
            if ctrl && shift && i.key_pressed(Key::E) {
                cmds.push(StudioCommand::FocusExplorer);
            }
            if ctrl && shift && i.key_pressed(Key::F) {
                cmds.push(StudioCommand::FocusSearch);
            }
            if ctrl && !shift && i.key_pressed(Key::S) {
                cmds.push(StudioCommand::SaveActiveEditor);
            }
            if !wants_keyboard && i.key_pressed(Key::F2) {
                trigger_f2_rename = true;
            }
            if !wants_keyboard && i.key_pressed(Key::Delete) {
                trigger_del_item = true;
            }
        });

        for c in cmds {
            self.execute_command(c);
        }

        if trigger_f2_rename
            && let Some((path, is_dir)) = self.state.explorer_selected_item.clone()
        {
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            self.state.explorer_inline_edit = Some(ExplorerInlineEdit::Rename {
                old_path: path,
                is_dir,
                input: name,
            });
            self.inline_edit_needs_focus = true;
        }

        if trigger_del_item
            && let Some((path, _)) = self.state.explorer_selected_item.clone()
            && let Some(pack) = &mut self.state.loaded_pack
        {
            if let Err(err) = delete_pack_asset(pack, &path) {
                self.state.log_output(
                    WarningSeverity::Error,
                    format!("Delete failed for '{path}': {err:#}"),
                );
            } else {
                self.state.explorer_selected_item = None;
                self.state.explorer_extra_folders.remove(&path);
                self.state.log_output(WarningSeverity::Info, format!("Deleted '{path}'"));
            }
        }
    }

    fn apply_tab_actions(&mut self, actions: Vec<TabAction>) {
        for action in actions {
            match action {
                TabAction::OpenFolder => self.execute_command(StudioCommand::OpenFolder),
                TabAction::OpenArchive => self.execute_command(StudioCommand::OpenArchive),
                TabAction::OpenNewProjectWizard => {
                    self.execute_command(StudioCommand::NewProjectWizard);
                }
                TabAction::OpenExportDialog => self.execute_command(StudioCommand::ExportPackage),
                TabAction::OpenRecent(path) => self.start_load_pack(path),
                TabAction::RunPreflight => self.start_preflight(),
                TabAction::ExecuteConvert => self.start_convert(),
                TabAction::PickConvertOutput => {
                    self.spawn_file_dialog(DialogTarget::SaveConvertOutput);
                }
                TabAction::SaveCodeTab { path, text } => {
                    if let Some(pack) = &mut self.state.loaded_pack {
                        match pack.write_asset(&path, text.into_bytes()) {
                            Ok(()) => {
                                if path == "pack.mcmeta" || path == "manifest.json" {
                                    self.state.metadata_form.load_from_pack(pack);
                                }
                                for (_, tab) in self.state.dock_state.iter_all_tabs_mut() {
                                    if let EditorTab::Code(code) = tab
                                        && code.path == path
                                    {
                                        code.mark_saved();
                                    }
                                }
                                self.state.log_output(
                                    WarningSeverity::Info,
                                    format!("Saved '{path}'"),
                                );
                            }
                            Err(err) => {
                                self.state.log_output(
                                    WarningSeverity::Error,
                                    format!("Failed to save '{path}': {err:#}"),
                                );
                            }
                        }
                    }
                }
                TabAction::DownscaleTexture { path, target_size } => {
                    self.perform_downscale_texture(&path, target_size);
                }
                TabAction::CompositeGrassSideTga { path, biome } => {
                    self.perform_composite_grass_side_tga(&path, biome);
                }
                TabAction::OpenAssetPath(path) => self.state.open_asset_tab(&path),
                TabAction::OpenInBlockbenchStudio(path) => {
                    self.launch_asset_in_blockbench_studio(&path);
                }
                TabAction::NewBedrockGeoInBlockbench => {
                    let dir = self.state.explorer_selected_dir.clone();
                    self.launch_new_bedrock_geo_in_blockbench(&dir);
                }
            }
        }
    }

    fn perform_downscale_texture(&mut self, path: &str, target_size: u32) {
        let Some(src_img) = self
            .state
            .dock_state
            .iter_all_tabs()
            .find_map(|(_, tab)| match tab {
                EditorTab::Texture(t) if t.path == path => t.decoded.clone(),
                _ => None,
            })
        else {
            return;
        };

        let (src_w, src_h) = src_img.dimensions();
        let target_w = target_size.max(1);
        let target_h = if src_h > src_w && src_w > 0 {
            ((u64::from(src_h) * u64::from(target_w)) / u64::from(src_w)).max(1) as u32
        } else {
            target_w
        };

        let fmt = TextureFormatKind::from_path(path).unwrap_or(TextureFormatKind::Png);
        match resize_nearest_neighbor(&src_img, target_w, target_h).and_then(|resized| {
            let bytes = encode_texture_bytes(&resized, fmt)?;
            Ok((resized, bytes))
        }) {
            Ok((resized, bytes)) => {
                let byte_len = bytes.len();
                if let Some(pack) = &mut self.state.loaded_pack
                    && let Err(err) = pack.write_asset(path, bytes)
                {
                    self.state.log_output(
                        WarningSeverity::Error,
                        format!("Failed writing downscaled texture '{path}': {err:#}"),
                    );
                    return;
                }
                for (_, tab) in self.state.dock_state.iter_all_tabs_mut() {
                    if let EditorTab::Texture(tex) = tab
                        && tex.path == path
                    {
                        tex.set_transformed_image(resized.clone(), fmt, byte_len);
                    }
                }
                self.texture_cache
                    .retain(|k, _| !k.starts_with(&format!("{path}:")));
                self.state.log_output(
                    WarningSeverity::Info,
                    format!(
                        "Downscaled '{path}' ({src_w}x{src_h} -> {target_w}x{target_h}) using Nearest-Neighbor."
                    ),
                );
            }
            Err(err) => {
                self.state.log_output(
                    WarningSeverity::Error,
                    format!("Downscale failed on '{path}': {err:#}"),
                );
            }
        }
    }

    fn perform_composite_grass_side_tga(&mut self, path: &str, biome: BiomeTintPreset) {
        let Some(src_img) = self
            .state
            .dock_state
            .iter_all_tabs()
            .find_map(|(_, tab)| match tab {
                EditorTab::Texture(t) if t.path == path => t.decoded.clone(),
                _ => None,
            })
        else {
            return;
        };

        let overlay_img = self.state.loaded_pack.as_ref().and_then(|pack| {
            let candidates = [
                "assets/minecraft/textures/block/grass_block_side_overlay.png",
                "textures/blocks/grass_side_overlay.png",
            ];
            for c in candidates {
                if let Some(bytes) = pack.container.get(c)
                    && let Ok((img, _)) = decode_texture_bytes(bytes, c)
                {
                    return Some(img);
                }
            }
            None
        });

        match composite_grass_side_tga(&src_img, overlay_img.as_ref(), biome.rgb_multiplier(), true) {
            Ok((_img, tga_bytes)) => {
                let target_tga_path = if self
                    .state
                    .loaded_pack
                    .as_ref()
                    .is_some_and(|p| p.edition == Edition::Bedrock)
                {
                    String::from("textures/blocks/grass_side.tga")
                } else {
                    String::from("assets/minecraft/textures/block/grass_side.tga")
                };
                if let Some(pack) = &mut self.state.loaded_pack {
                    match pack.write_asset(&target_tga_path, tga_bytes) {
                        Ok(()) => {
                            self.state.log_output(
                                WarningSeverity::Info,
                                format!("Composited Bedrock Grass Side TGA -> '{target_tga_path}'"),
                            );
                            self.state.open_asset_tab(&target_tga_path);
                        }
                        Err(err) => {
                            self.state.log_output(
                                WarningSeverity::Error,
                                format!("Failed saving '{target_tga_path}': {err:#}"),
                            );
                        }
                    }
                }
            }
            Err(err) => {
                self.state.log_output(
                    WarningSeverity::Error,
                    format!("Composite Grass Side TGA failed: {err:#}"),
                );
            }
        }
    }

    fn render_blockbench_fallback_banner(&mut self, ctx: &Context) {
        let Some(notice) = self.blockbench.fallback_notice.clone() else {
            return;
        };
        let url_copy = self.blockbench.last_url.clone();

        TopBottomPanel::top("blockbench_fallback_banner")
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(0x1E, 0x3A, 0x5F))
                    .stroke(Stroke::new(1.0, theme::ACCENT_BLUE))
                    .inner_margin(Margin::symmetric(10, 5)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("🧊 Blockbench 2-Way Studio Bridge Active")
                            .strong()
                            .color(theme::TEXT_BRIGHT),
                    );
                    ui.label(RichText::new(&notice).small().color(theme::TEXT_PRIMARY));

                    if ui
                        .button(
                            RichText::new("🌐 Buka Blockbench Studio di Browser Lokal")
                                .strong()
                                .color(theme::TEXT_BRIGHT),
                        )
                        .clicked()
                    {
                        let root = self.active_project_root();
                        match self.blockbench.open_fallback_browser(root) {
                            Ok(url) => {
                                self.state.log_output(
                                    WarningSeverity::Info,
                                    format!("Opened Blockbench Studio in local browser: {url}"),
                                );
                            }
                            Err(err) => {
                                self.state.log_output(
                                    WarningSeverity::Warning,
                                    format!("Could not auto-launch browser: {err:#}"),
                                );
                            }
                        }
                    }

                    if let Some(url) = &url_copy
                        && ui.small_button("Copy URL").clicked()
                    {
                        ctx.copy_text(url.clone());
                        self.state.log_output(
                            WarningSeverity::Info,
                            format!("Copied local Blockbench URL: {url}"),
                        );
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            self.blockbench.fallback_notice = None;
                        }
                    });
                });
            });
    }

    fn render_status_bar(&mut self, ctx: &Context) {
        TopBottomPanel::bottom("pack_studio_status_bar")
            .exact_height(24.0)
            .frame(
                Frame::new()
                    .fill(theme::BG_STATUS_BAR)
                    .inner_margin(Margin::symmetric(8, 2)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let edition_text = self
                        .state
                        .loaded_pack
                        .as_ref()
                        .map_or_else(|| String::from("No Pack Open"), |p| p.edition_label.clone());
                    ui.label(
                        RichText::new(format!("⬡ {edition_text}"))
                            .color(theme::TEXT_BRIGHT)
                            .strong()
                            .size(12.0),
                    );

                    ui.separator();

                    let (errs, warns) = self.state.problem_counts();
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!("✖ {errs}  ⚠ {warns}"))
                                    .color(theme::TEXT_BRIGHT)
                                    .size(12.0),
                            )
                            .frame(false),
                        )
                        .on_hover_text("Toggle Problems Panel (Ctrl+J)")
                        .clicked()
                    {
                        self.state.bottom_panel_visible = true;
                        self.state.active_bottom_tab = BottomTab::Problems;
                    }

                    if let Some(busy) = &self.state.busy_label {
                        ui.separator();
                        ui.spinner();
                        ui.label(RichText::new(busy).color(theme::TEXT_BRIGHT).size(12.0));
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (audit_label, audit_bg) = match self.state.last_audit_status {
                            Some(AuditStatus::Passed) => ("PASSED", theme::BADGE_PASSED_BG),
                            Some(AuditStatus::Partial) => ("WARNING", theme::BADGE_WARN_BG),
                            Some(AuditStatus::Failed) => ("FAILED", theme::BADGE_FAIL_BG),
                            None => ("UNAUDITED", Color32::from_rgb(0x44, 0x44, 0x44)),
                        };

                        Frame::new()
                            .fill(audit_bg)
                            .corner_radius(CornerRadius::same(3))
                            .inner_margin(Margin::symmetric(6, 1))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!("Audit: {audit_label}"))
                                        .color(theme::TEXT_BRIGHT)
                                        .strong()
                                        .size(11.0),
                                )
                                .on_hover_text(&self.state.last_audit_summary);
                            });

                        ui.separator();

                        ui.label(
                            RichText::new(format!(
                                "{} | Watcher: {}",
                                self.blockbench.status_label(),
                                self.state.watcher_status
                            ))
                            .color(theme::TEXT_BRIGHT)
                            .size(11.5),
                        );

                        ui.separator();

                        if let Some(pack) = &self.state.loaded_pack {
                            let res_str = pack
                                .dominant_resolution
                                .map_or_else(|| String::from("n/a"), |r| r.to_string());
                            ui.label(
                                RichText::new(format!(
                                    "Textures: {} ({res_str})",
                                    pack.texture_count
                                ))
                                .color(theme::TEXT_BRIGHT)
                                .size(11.5),
                            );
                        } else {
                            ui.label(
                                RichText::new("Ctrl+P: Quick Open | Ctrl+Shift+P: Commands")
                                    .color(theme::TEXT_BRIGHT)
                                    .size(11.5),
                            );
                        }
                    });
                });
            });
    }

    fn render_activity_bar(&mut self, ctx: &Context) {
        egui::SidePanel::left("pack_studio_activity_bar")
            .exact_width(48.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(theme::BG_ACTIVITY_BAR)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .inner_margin(Margin::symmetric(0, 6)),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    for tab in ActivityTab::ALL {
                        let is_active =
                            self.state.sidebar_visible && self.state.active_activity == tab;
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(48.0, 44.0), Sense::click());

                        if is_active {
                            ui.painter().rect_filled(
                                Rect::from_min_size(rect.min, Vec2::new(2.5, rect.height())),
                                0.0,
                                theme::TEXT_BRIGHT,
                            );
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(0x3D, 0x3D, 0x3D));
                        }

                        let icon_color = if is_active || resp.hovered() {
                            theme::TEXT_BRIGHT
                        } else {
                            theme::TEXT_MUTED
                        };
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            tab.icon(),
                            FontId::proportional(20.0),
                            icon_color,
                        );

                        if resp.on_hover_text(tab.tooltip()).clicked() {
                            self.state.select_activity(tab);
                        }
                    }

                    ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(48.0, 40.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(0x3D, 0x3D, 0x3D));
                        }
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            "⌘",
                            FontId::proportional(18.0),
                            if resp.hovered() {
                                theme::TEXT_BRIGHT
                            } else {
                                theme::TEXT_MUTED
                            },
                        );
                        if resp
                            .on_hover_text("Command Palette (Ctrl+Shift+P / F1)")
                            .clicked()
                        {
                            self.state.open_command_palette();
                            self.palette_needs_focus = true;
                        }
                    });
                });
            });
    }

    fn render_primary_sidebar(&mut self, ctx: &Context) {
        if !self.state.sidebar_visible {
            return;
        }
        egui::SidePanel::left("pack_studio_primary_sidebar")
            .default_width(280.0)
            .min_width(200.0)
            .max_width(500.0)
            .resizable(true)
            .frame(
                Frame::new()
                    .fill(theme::BG_SIDEBAR)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .inner_margin(Margin::same(8)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(self.state.active_activity.title())
                            .strong()
                            .size(11.0)
                            .color(theme::TEXT_MUTED),
                    );
                });
                ui.separator();

                match self.state.active_activity {
                    ActivityTab::Explorer => self.render_sidebar_explorer(ui, ctx),
                    ActivityTab::Search => self.render_sidebar_search(ui),
                    ActivityTab::Converter => self.render_sidebar_converter(ui),
                    ActivityTab::Metadata => self.render_sidebar_metadata(ui, ctx),
                    ActivityTab::Settings => self.render_sidebar_settings(ui),
                }
            });
    }

    fn render_sidebar_explorer(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        // Workspace quick open / wizard / export strip
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("✨ New").on_hover_text("New Project Wizard (Java / Bedrock / Add-On)").clicked() {
                self.pending_commands.push(StudioCommand::NewProjectWizard);
            }
            if ui.small_button("📂 Folder").on_hover_text("Open Pack Folder").clicked() {
                self.pending_commands.push(StudioCommand::OpenFolder);
            }
            if ui.small_button("📦 Archive").on_hover_text("Open .zip / .mcpack").clicked() {
                self.pending_commands.push(StudioCommand::OpenArchive);
            }
            if self.state.loaded_pack.is_some()
                && ui.small_button("📤 Export").on_hover_text("Export / Package (.zip / .mcpack / .mcaddon)").clicked()
            {
                self.pending_commands.push(StudioCommand::ExportPackage);
            }
        });

        if self.state.loaded_pack.is_none() {
            ui.separator();
            ui.add_space(12.0);
            ui.label(RichText::new("No resource pack opened.").color(theme::TEXT_MUTED));
            ui.label(
                RichText::new(
                    "Use 'New' for the Java/Bedrock Project Wizard, or drag & drop a pack folder/.zip here.",
                )
                .small()
                .color(theme::TEXT_MUTED),
            );
            return;
        }

        // VS Code Explorer Header Action Icons: New File, New Folder, Import, New Blank PNG, Collapse All, Refresh
        ui.separator();
        let active_dir = self.state.explorer_selected_dir.clone();
        ui.horizontal(|ui| {
            let name = self
                .state
                .loaded_pack
                .as_ref()
                .map_or("PACK", |p| p.pack_name.as_str());
            ui.label(RichText::new(name).strong().size(12.0).color(theme::TEXT_BRIGHT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if ui.small_button("⟳").on_hover_text("Refresh Workspace").clicked() {
                    self.pending_commands.push(StudioCommand::ReloadWorkspace);
                }
                if ui.small_button("⊟").on_hover_text("Collapse All Folders").clicked() {
                    self.state.explorer_collapse_epoch =
                        self.state.explorer_collapse_epoch.saturating_add(1);
                }
                if ui.small_button("🎨+").on_hover_text("New Blank Texture (16x16 PNG)").clicked() {
                    self.create_blank_texture_in_dir(&active_dir);
                }
                if ui.small_button("📥").on_hover_text("Import Images / Assets Here...").clicked() {
                    self.spawn_file_dialog(DialogTarget::ImportAssetsToDir(active_dir.clone()));
                }
                if ui.small_button("📁+").on_hover_text("New Folder").clicked() {
                    self.state.explorer_inline_edit = Some(ExplorerInlineEdit::NewFolder {
                        parent_dir: active_dir.clone(),
                        input: String::new(),
                    });
                    self.inline_edit_needs_focus = true;
                }
                if ui.small_button("📄+").on_hover_text("New File").clicked() {
                    self.state.explorer_inline_edit = Some(ExplorerInlineEdit::NewFile {
                        parent_dir: active_dir.clone(),
                        input: String::new(),
                    });
                    self.inline_edit_needs_focus = true;
                }
            });
        });

        // Quick Filter box
        ui.add(
            TextEdit::singleline(&mut self.state.explorer_filter)
                .hint_text("Filter tree (or Ctrl+P Quick Open)...")
                .desired_width(f32::INFINITY),
        );
        if !self.state.explorer_selected_dir.is_empty() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Target: {}/", self.state.explorer_selected_dir))
                        .small()
                        .monospace()
                        .color(theme::SEVERITY_INFO),
                );
                if ui.small_button("×").on_hover_text("Reset target folder to pack root").clicked() {
                    self.state.explorer_selected_dir.clear();
                }
            });
        }
        ui.add_space(2.0);

        let Some(pack) = &self.state.loaded_pack else {
            return;
        };
        let tree = FileTreeNode::build(
            &pack.container,
            &self.state.explorer_extra_folders,
            &self.state.explorer_filter,
        );
        let severities = pack.problem_severity_by_path.clone();
        let has_clipboard = self.state.explorer_clipboard.is_some();
        let selected_item = self.state.explorer_selected_item.clone();
        let collapse_epoch = self.state.explorer_collapse_epoch;

        let mut actions: Vec<ExplorerAction> = Vec::new();
        let mut commit_inline: Option<ExplorerInlineEdit> = None;
        let mut cancel_inline = false;

        ScrollArea::vertical()
            .id_salt("explorer_hierarchical_tree")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Root-level inline create box when parent_dir is empty
                if let Some(edit) = &mut self.state.explorer_inline_edit {
                    let is_root_create = matches!(
                        edit,
                        ExplorerInlineEdit::NewFile { parent_dir, .. }
                        | ExplorerInlineEdit::NewFolder { parent_dir, .. } if parent_dir.is_empty()
                    );
                    if is_root_create {
                        render_inline_edit_row(
                            ui,
                            edit,
                            &mut self.inline_edit_needs_focus,
                            &mut commit_inline,
                            &mut cancel_inline,
                        );
                    }
                }

                render_tree_node(
                    ui,
                    &tree,
                    &severities,
                    &selected_item,
                    has_clipboard,
                    collapse_epoch,
                    &mut self.state.explorer_inline_edit,
                    &mut self.inline_edit_needs_focus,
                    &mut actions,
                    &mut commit_inline,
                    &mut cancel_inline,
                );
            });

        if cancel_inline {
            self.state.explorer_inline_edit = None;
        } else if let Some(edit) = commit_inline {
            self.state.explorer_inline_edit = None;
            self.commit_explorer_inline_edit(edit);
        }

        self.apply_explorer_actions(actions, ctx);
    }

    fn create_blank_texture_in_dir(&mut self, dir: &str) {
        let base_rel = if dir.is_empty() {
            if self
                .state
                .loaded_pack
                .as_ref()
                .is_some_and(|p| p.edition == Edition::Java)
            {
                String::from("assets/minecraft/textures/block/new_texture.png")
            } else {
                String::from("textures/blocks/new_texture.png")
            }
        } else {
            format!("{}/new_texture.png", dir.trim_matches('/'))
        };

        let target_rel = self.state.loaded_pack.as_ref().map_or(base_rel.clone(), |p| {
            if p.container.contains(&base_rel) {
                make_unique_rel_path(&p.container, &base_rel)
            } else {
                base_rel
            }
        });

        match create_blank_png_bytes(16) {
            Ok(bytes) => {
                if let Some(pack) = &mut self.state.loaded_pack {
                    match pack.write_asset(&target_rel, bytes) {
                        Ok(()) => {
                            self.state.log_output(
                                WarningSeverity::Info,
                                format!("Created blank 16x16 PNG '{target_rel}'"),
                            );
                            self.state.open_asset_tab(&target_rel);
                        }
                        Err(err) => {
                            self.state.log_output(
                                WarningSeverity::Error,
                                format!("Failed creating blank PNG: {err:#}"),
                            );
                        }
                    }
                }
            }
            Err(err) => {
                self.state.log_output(
                    WarningSeverity::Error,
                    format!("Failed encoding blank PNG: {err:#}"),
                );
            }
        }
    }

    fn commit_explorer_inline_edit(&mut self, edit: ExplorerInlineEdit) {
        match edit {
            ExplorerInlineEdit::NewFile { parent_dir, input } => {
                let name = input.trim().trim_matches('/');
                if name.is_empty() || name.contains("..") {
                    return;
                }
                let rel = if parent_dir.is_empty() {
                    name.to_string()
                } else {
                    format!("{}/{name}", parent_dir.trim_matches('/'))
                };
                let initial_bytes = if rel.to_ascii_lowercase().ends_with(".png") {
                    create_blank_png_bytes(16).unwrap_or_default()
                } else if rel.to_ascii_lowercase().ends_with(".json") {
                    b"{\n}\n".to_vec()
                } else {
                    Vec::new()
                };
                if let Some(pack) = &mut self.state.loaded_pack {
                    match pack.write_asset(&rel, initial_bytes) {
                        Ok(()) => {
                            self.state.log_output(
                                WarningSeverity::Info,
                                format!("Created file '{rel}'"),
                            );
                            self.state.open_asset_tab(&rel);
                        }
                        Err(err) => {
                            self.state.log_output(
                                WarningSeverity::Error,
                                format!("Create file failed: {err:#}"),
                            );
                        }
                    }
                }
            }
            ExplorerInlineEdit::NewFolder { parent_dir, input } => {
                let name = input.trim().trim_matches('/');
                if name.is_empty() || name.contains("..") {
                    return;
                }
                let rel = if parent_dir.is_empty() {
                    name.to_string()
                } else {
                    format!("{}/{name}", parent_dir.trim_matches('/'))
                };
                if let Some(root) = self.active_project_root() {
                    let _ = fs::create_dir_all(root.join(&rel));
                }
                self.state.explorer_extra_folders.insert(rel.clone());
                self.state.explorer_selected_dir = rel.clone();
                self.state.log_output(
                    WarningSeverity::Info,
                    format!("Created folder '{rel}/'"),
                );
            }
            ExplorerInlineEdit::Rename {
                old_path,
                is_dir,
                input,
            } => {
                if let Some(pack) = &mut self.state.loaded_pack {
                    match rename_pack_asset(pack, &old_path, &input, is_dir) {
                        Ok(new_rel) => {
                            if is_dir {
                                self.state.explorer_extra_folders.remove(&old_path);
                                self.state.explorer_extra_folders.insert(new_rel.clone());
                            }
                            self.state.explorer_selected_item = Some((new_rel.clone(), is_dir));
                            self.state.log_output(
                                WarningSeverity::Info,
                                format!("Renamed '{old_path}' -> '{new_rel}'"),
                            );
                        }
                        Err(err) => {
                            self.state.log_output(
                                WarningSeverity::Error,
                                format!("Rename failed: {err:#}"),
                            );
                        }
                    }
                }
            }
        }
    }

    fn apply_explorer_actions(&mut self, actions: Vec<ExplorerAction>, ctx: &Context) {
        let edition = self
            .state
            .loaded_pack
            .as_ref()
            .map_or(Edition::Java, |p| p.edition);

        for action in actions {
            match action {
                ExplorerAction::SelectDir(dir) => {
                    self.state.explorer_selected_dir = dir.clone();
                    self.state.explorer_selected_item = Some((dir, true));
                }
                ExplorerAction::SelectFile(path) => {
                    if let Some((parent, _)) = path.rsplit_once('/') {
                        self.state.explorer_selected_dir = parent.to_string();
                    } else {
                        self.state.explorer_selected_dir.clear();
                    }
                    self.state.explorer_selected_item = Some((path, false));
                }
                ExplorerAction::OpenFile(path) => {
                    if let Some((parent, _)) = path.rsplit_once('/') {
                        self.state.explorer_selected_dir = parent.to_string();
                    } else {
                        self.state.explorer_selected_dir.clear();
                    }
                    self.state.explorer_selected_item = Some((path.clone(), false));
                    self.state.open_asset_tab(&path);
                }
                ExplorerAction::OpenFileSide(path) => self.state.open_asset_to_side(&path),
                ExplorerAction::StartInlineNewFile(parent_dir) => {
                    self.state.explorer_inline_edit = Some(ExplorerInlineEdit::NewFile {
                        parent_dir,
                        input: String::new(),
                    });
                    self.inline_edit_needs_focus = true;
                }
                ExplorerAction::StartInlineNewFolder(parent_dir) => {
                    self.state.explorer_inline_edit = Some(ExplorerInlineEdit::NewFolder {
                        parent_dir,
                        input: String::new(),
                    });
                    self.inline_edit_needs_focus = true;
                }
                ExplorerAction::StartInlineRename { path, is_dir } => {
                    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                    self.state.explorer_inline_edit = Some(ExplorerInlineEdit::Rename {
                        old_path: path,
                        is_dir,
                        input: name,
                    });
                    self.inline_edit_needs_focus = true;
                }
                ExplorerAction::ImportToDir(dir) => {
                    self.spawn_file_dialog(DialogTarget::ImportAssetsToDir(dir));
                }
                ExplorerAction::NewBlankTexture(dir) => self.create_blank_texture_in_dir(&dir),
                ExplorerAction::OpenInBlockbench(path) => {
                    self.launch_asset_in_blockbench_studio(&path);
                }
                ExplorerAction::NewBedrockGeo(dir) => {
                    self.launch_new_bedrock_geo_in_blockbench(&dir);
                }
                ExplorerAction::OpenExternalEditor(path) => self.open_asset_externally(&path, false),
                ExplorerAction::DuplicateFile(path) => {
                    if let Some(pack) = &mut self.state.loaded_pack {
                        match duplicate_pack_asset(pack, &path) {
                            Ok(copy_path) => {
                                self.state.log_output(
                                    WarningSeverity::Info,
                                    format!("Duplicated '{path}' -> '{copy_path}'"),
                                );
                            }
                            Err(err) => {
                                self.state.log_output(
                                    WarningSeverity::Error,
                                    format!("Duplicate failed: {err:#}"),
                                );
                            }
                        }
                    }
                }
                ExplorerAction::CutItem(path) => {
                    self.state.explorer_clipboard = Some(ExplorerClipboard::Cut(path.clone()));
                    self.state.log_output(WarningSeverity::Info, format!("Cut '{path}'"));
                }
                ExplorerAction::CopyItem(path) => {
                    self.state.explorer_clipboard = Some(ExplorerClipboard::Copy(path.clone()));
                    self.state.log_output(WarningSeverity::Info, format!("Copied '{path}'"));
                }
                ExplorerAction::PasteInto(target_dir) => {
                    if let Some(clip) = self.state.explorer_clipboard.clone()
                        && let Some(pack) = &mut self.state.loaded_pack
                    {
                        match paste_pack_asset(pack, &clip, &target_dir) {
                            Ok(pasted) => {
                                if matches!(clip, ExplorerClipboard::Cut(_)) {
                                    self.state.explorer_clipboard = None;
                                }
                                self.state.log_output(
                                    WarningSeverity::Info,
                                    format!("Pasted -> '{pasted}'"),
                                );
                            }
                            Err(err) => {
                                self.state.log_output(
                                    WarningSeverity::Error,
                                    format!("Paste failed: {err:#}"),
                                );
                            }
                        }
                    }
                }
                ExplorerAction::DeleteItem(path) => {
                    if let Some(pack) = &mut self.state.loaded_pack {
                        match delete_pack_asset(pack, &path) {
                            Ok(()) => {
                                self.state.explorer_extra_folders.remove(&path);
                                self.state.log_output(
                                    WarningSeverity::Info,
                                    format!("Deleted '{path}'"),
                                );
                            }
                            Err(err) => {
                                self.state.log_output(
                                    WarningSeverity::Error,
                                    format!("Delete failed: {err:#}"),
                                );
                            }
                        }
                    }
                }
                ExplorerAction::CopyRelativePath(path) => {
                    ctx.copy_text(path.clone());
                    self.state.log_output(
                        WarningSeverity::Info,
                        format!("Copied relative path: {path}"),
                    );
                }
                ExplorerAction::CopyResourceId(path) => {
                    let id = generate_minecraft_resource_id(&path, edition);
                    ctx.copy_text(id.clone());
                    self.state.log_output(
                        WarningSeverity::Info,
                        format!("Copied Minecraft Resource ID: {id}"),
                    );
                }
                ExplorerAction::RevealInFileManager(path) => {
                    if let Some(full) = self
                        .state
                        .loaded_pack
                        .as_ref()
                        .and_then(|pk| pk.resolve_disk_file(&path))
                    {
                        let _ = reveal_in_file_manager(&full);
                    } else if let Some(disk) = self
                        .state
                        .loaded_pack
                        .as_ref()
                        .and_then(|pk| pk.source.disk_path())
                    {
                        let _ = reveal_in_file_manager(disk);
                    }
                }
            }
        }
    }

    fn render_sidebar_search(&mut self, ui: &mut egui::Ui) {
        ui.add(
            TextEdit::singleline(&mut self.state.search_query)
                .hint_text("Search files, textures, block/item IDs...")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);

        let Some(pack) = &self.state.loaded_pack else {
            ui.label(RichText::new("Open a pack to search.").color(theme::TEXT_MUTED));
            return;
        };

        let query = self.state.search_query.trim().to_ascii_lowercase();
        let edition = pack.edition;
        let matches: Vec<(String, String)> = pack
            .container
            .files()
            .keys()
            .filter_map(|path| {
                let canonical = generate_minecraft_resource_id(path, edition);
                if query.is_empty()
                    || path.to_ascii_lowercase().contains(&query)
                    || canonical.to_ascii_lowercase().contains(&query)
                {
                    Some((path.clone(), canonical))
                } else {
                    None
                }
            })
            .take(200)
            .collect();

        ui.label(
            RichText::new(format!("{} matching asset(s)", matches.len()))
                .small()
                .color(theme::TEXT_MUTED),
        );
        ui.separator();

        let mut open_path = None;
        ScrollArea::vertical()
            .id_salt("sidebar_search_results")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (path, canonical) in matches {
                    let resp = ui.selectable_label(false, RichText::new(&canonical).size(12.0));
                    ui.label(RichText::new(&path).small().color(theme::TEXT_MUTED));
                    if resp.clicked() {
                        open_path = Some(path);
                    }
                    ui.add_space(2.0);
                }
            });

        if let Some(p) = open_path {
            self.state.open_asset_tab(&p);
        }
    }

    fn render_sidebar_converter(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Conversion Direction").strong().size(12.0));
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.state.conversion_config.direction,
                ConversionDirection::JavaToBedrock,
                "Java → Bedrock",
            );
            ui.selectable_value(
                &mut self.state.conversion_config.direction,
                ConversionDirection::BedrockToJava,
                "Bedrock → Java",
            );
        });

        ui.add_space(6.0);
        ui.label(RichText::new("Target Version").small().color(theme::TEXT_MUTED));
        ui.add(
            TextEdit::singleline(&mut self.state.conversion_config.target_version)
                .hint_text("e.g. 1.21.0, 26.60, 1.21.4")
                .desired_width(f32::INFINITY),
        );

        ui.add_space(6.0);
        ui.label(RichText::new("Output Path (.mcpack / .zip / dir)").small().color(theme::TEXT_MUTED));
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut self.state.conversion_config.output_path)
                    .hint_text("Optional export path...")
                    .desired_width(ui.available_width() - 62.0),
            );
            if ui.button("Browse").clicked() {
                self.spawn_file_dialog(DialogTarget::SaveConvertOutput);
            }
        });

        ui.add_space(6.0);
        ui.checkbox(
            &mut self.state.conversion_config.convert_pbr,
            "Convert LabPBR -> Bedrock MER",
        );
        ui.checkbox(
            &mut self.state.conversion_config.uv_atlas_guard,
            "3D UV-Atlas Guard",
        );

        ui.add_space(10.0);
        if ui
            .add_sized([ui.available_width(), 28.0], egui::Button::new("1. Run Preflight Scan"))
            .clicked()
        {
            self.start_preflight();
        }
        ui.add_space(4.0);
        if ui
            .add_sized(
                [ui.available_width(), 28.0],
                egui::Button::new(RichText::new("2. Execute Convert").strong()),
            )
            .clicked()
        {
            self.start_convert();
        }
        ui.add_space(4.0);
        if ui
            .add_sized([ui.available_width(), 26.0], egui::Button::new("Open Full Dashboard"))
            .clicked()
        {
            self.state.open_conversion_dashboard();
        }
        ui.add_space(4.0);
        if ui
            .add_sized([ui.available_width(), 26.0], egui::Button::new("Run Semantic Audit"))
            .clicked()
        {
            self.start_semantic_audit();
        }
    }

    fn render_sidebar_metadata(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let edition = self
            .state
            .loaded_pack
            .as_ref()
            .map_or(Edition::Java, |p| p.edition);

        ScrollArea::vertical()
            .id_salt("metadata_editor_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Prominent Flexible Packager / Exporter button
                if ui
                    .add_sized(
                        [ui.available_width(), 30.0],
                        egui::Button::new(
                            RichText::new("📤 Export Pack (.zip / .mcpack / .mcaddon)...")
                                .strong(),
                        ),
                    )
                    .clicked()
                {
                    self.state.export_config.open = true;
                    self.state.export_config.last_error = None;
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("Active Edition: {edition}"))
                        .strong()
                        .color(theme::TEXT_BRIGHT),
                );
                ui.separator();

                // Pack Icon Manager (Nearest-Neighbor preview + Import/Replace + Resize)
                egui::CollapsingHeader::new(RichText::new("🖼 Pack Icon Manager").strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        let icon_bytes = self.state.loaded_pack.as_ref().and_then(|p| {
                            p.container
                                .get("pack_icon.png")
                                .or_else(|| p.container.get("pack.png"))
                                .map(ToOwned::to_owned)
                        });
                        ui.horizontal(|ui| {
                            let cache_key = format!(
                                "__pack_icon__:rev={}",
                                self.state.metadata_form.icon_revision
                            );
                            if let Some(bytes) = &icon_bytes {
                                let handle = if let Some(h) = self.texture_cache.get(&cache_key) {
                                    h.clone()
                                } else if let Ok((img, _)) = decode_texture_bytes(bytes, "pack.png") {
                                    let ci = ColorImage::from_rgba_unmultiplied(
                                        [img.width() as usize, img.height() as usize],
                                        img.as_raw(),
                                    );
                                    let h = ctx.load_texture(
                                        cache_key.clone(),
                                        ci,
                                        TextureOptions::NEAREST,
                                    );
                                    self.texture_cache.insert(cache_key, h.clone());
                                    h
                                } else {
                                    let ci = ColorImage::new([16, 16], Color32::DARK_GRAY);
                                    ctx.load_texture(cache_key, ci, TextureOptions::NEAREST)
                                };
                                ui.image((handle.id(), Vec2::splat(56.0)));
                            } else {
                                ui.label(RichText::new("[No Icon]").color(theme::TEXT_MUTED));
                            }

                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("Resize:").small());
                                    egui::ComboBox::from_id_salt("icon_resize_cb")
                                        .width(86.0)
                                        .selected_text(match self.state.metadata_form.icon_resize_target {
                                            None => "Keep",
                                            Some(128) => "128x128",
                                            Some(256) => "256x256",
                                            Some(_) => "Custom",
                                        })
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut self.state.metadata_form.icon_resize_target,
                                                None,
                                                "Keep Original",
                                            );
                                            ui.selectable_value(
                                                &mut self.state.metadata_form.icon_resize_target,
                                                Some(128),
                                                "128x128 (Nearest)",
                                            );
                                            ui.selectable_value(
                                                &mut self.state.metadata_form.icon_resize_target,
                                                Some(256),
                                                "256x256 (Nearest)",
                                            );
                                        });
                                });
                                if ui.button("Import / Replace Icon...").clicked() {
                                    self.spawn_file_dialog(DialogTarget::MetadataImportIcon);
                                }
                            });
                        });
                    });

                ui.add_space(4.0);

                // Shared Authors, License, Website/URL
                egui::CollapsingHeader::new(RichText::new("👤 Authors, License & URL").strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut remove_idx = None;
                        for (idx, author) in self.state.metadata_form.authors_list.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                ui.add(
                                    TextEdit::singleline(author)
                                        .desired_width(ui.available_width() - 28.0),
                                );
                                if ui.small_button("×").clicked() {
                                    remove_idx = Some(idx);
                                }
                            });
                        }
                        if let Some(idx) = remove_idx {
                            self.state.metadata_form.authors_list.remove(idx);
                        }
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::singleline(&mut self.state.metadata_form.new_author_input)
                                    .hint_text("Add author name...")
                                    .desired_width(ui.available_width() - 48.0),
                            );
                            if ui.small_button("+ Add").clicked() {
                                let name = self.state.metadata_form.new_author_input.trim().to_string();
                                if !name.is_empty() {
                                    self.state.metadata_form.authors_list.push(name);
                                    self.state.metadata_form.new_author_input.clear();
                                }
                            }
                        });
                        ui.label(RichText::new("License:").small().color(theme::TEXT_MUTED));
                        ui.add(
                            TextEdit::singleline(&mut self.state.metadata_form.license)
                                .hint_text("MIT / GPL-3.0 / All Rights Reserved")
                                .desired_width(f32::INFINITY),
                        );
                        ui.label(RichText::new("Website / URL:").small().color(theme::TEXT_MUTED));
                        ui.add(
                            TextEdit::singleline(&mut self.state.metadata_form.url)
                                .hint_text("https://...")
                                .desired_width(f32::INFINITY),
                        );
                    });

                ui.add_space(4.0);

                // Java pack.mcmeta section
                egui::CollapsingHeader::new(RichText::new("☕ Java pack.mcmeta").strong())
                    .default_open(edition == Edition::Java)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Pack Name:").small().color(theme::TEXT_MUTED));
                        ui.add(
                            TextEdit::singleline(&mut self.state.metadata_form.java_pack_name)
                                .desired_width(f32::INFINITY),
                        );
                        ui.horizontal(|ui| {
                            ui.label("Version:");
                            for v in &mut self.state.metadata_form.java_version {
                                ui.add(egui::DragValue::new(v).range(0..=999));
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("pack_format:");
                            ui.add(
                                egui::DragValue::new(&mut self.state.metadata_form.java_pack_format)
                                    .range(1..=200),
                            );
                        });
                        ui.checkbox(
                            &mut self.state.metadata_form.java_use_supported_formats,
                            "supported_formats range",
                        );
                        if self.state.metadata_form.java_use_supported_formats {
                            ui.horizontal(|ui| {
                                ui.label("min:");
                                ui.add(
                                    egui::DragValue::new(
                                        &mut self.state.metadata_form.java_supported_min,
                                    )
                                    .range(1..=200),
                                );
                                ui.label("max:");
                                ui.add(
                                    egui::DragValue::new(
                                        &mut self.state.metadata_form.java_supported_max,
                                    )
                                    .range(1..=1024),
                                );
                            });
                        }
                        ui.label("Description (§ formatting supported):");
                        render_mc_formatting_toolbar(
                            ui,
                            &mut self.state.metadata_form.java_description,
                        );
                        ui.add(
                            TextEdit::multiline(&mut self.state.metadata_form.java_description)
                                .desired_rows(2)
                                .desired_width(f32::INFINITY),
                        );
                        render_minecraft_formatted_preview(
                            ui,
                            &self.state.metadata_form.java_description,
                        );
                        ui.add_space(4.0);
                        if ui.button("💾 Apply & Save pack.mcmeta").clicked() {
                            self.apply_java_metadata_form();
                        }
                    });

                ui.add_space(4.0);

                // Bedrock manifest.json section
                egui::CollapsingHeader::new(RichText::new("🪨 Bedrock manifest.json").strong())
                    .default_open(edition == Edition::Bedrock)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("format_version:");
                            ui.selectable_value(
                                &mut self.state.metadata_form.bedrock_format_version,
                                1,
                                "v1",
                            );
                            ui.selectable_value(
                                &mut self.state.metadata_form.bedrock_format_version,
                                2,
                                "v2",
                            );
                            ui.selectable_value(
                                &mut self.state.metadata_form.bedrock_format_version,
                                3,
                                "v3",
                            );
                        });
                        ui.label("Pack Name:");
                        ui.add(
                            TextEdit::singleline(&mut self.state.metadata_form.bedrock_name)
                                .desired_width(f32::INFINITY),
                        );
                        ui.label("Description (§ formatting supported):");
                        render_mc_formatting_toolbar(
                            ui,
                            &mut self.state.metadata_form.bedrock_description,
                        );
                        ui.add(
                            TextEdit::multiline(&mut self.state.metadata_form.bedrock_description)
                                .desired_rows(2)
                                .desired_width(f32::INFINITY),
                        );
                        render_minecraft_formatted_preview(
                            ui,
                            &self.state.metadata_form.bedrock_description,
                        );

                        ui.horizontal(|ui| {
                            ui.label("Version:");
                            for v in &mut self.state.metadata_form.bedrock_version {
                                ui.add(egui::DragValue::new(v).range(0..=999));
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("min_engine_version:");
                            ui.label(RichText::new("1 .").strong().color(theme::SEVERITY_OK));
                            ui.add(
                                egui::DragValue::new(
                                    &mut self.state.metadata_form.bedrock_min_engine_version[1],
                                )
                                .range(14..=99),
                            );
                            ui.add(
                                egui::DragValue::new(
                                    &mut self.state.metadata_form.bedrock_min_engine_version[2],
                                )
                                .range(0..=999),
                            );
                        });
                        self.state.metadata_form.enforce_bedrock_engine_version_invariant();

                        ui.label(RichText::new("Header UUID (v4):").small().color(theme::TEXT_MUTED));
                        ui.add(
                            TextEdit::singleline(
                                &mut self.state.metadata_form.bedrock_header_uuid,
                            )
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                        );
                        ui.label(RichText::new("Module UUID (v4):").small().color(theme::TEXT_MUTED));
                        ui.add(
                            TextEdit::singleline(
                                &mut self.state.metadata_form.bedrock_module_uuid,
                            )
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                        );
                        if ui.button("🎲 Regenerate UUIDv4 Pair").clicked() {
                            self.state.metadata_form.regenerate_uuids();
                        }

                        ui.checkbox(&mut self.state.metadata_form.bedrock_cap_pbr, "Capability: pbr");
                        ui.checkbox(
                            &mut self.state.metadata_form.bedrock_cap_raytraced,
                            "Capability: raytraced",
                        );

                        ui.add_space(4.0);
                        if ui.button("💾 Apply & Save manifest.json").clicked() {
                            self.apply_bedrock_metadata_form();
                        }
                    });

                if let Some(msg) = &self.state.metadata_form.status_message {
                    ui.add_space(6.0);
                    ui.label(RichText::new(msg).small().color(theme::SEVERITY_OK));
                }
            });
    }

    fn apply_java_metadata_form(&mut self) {
        match self.state.metadata_form.build_java_mcmeta_bytes() {
            Ok(bytes) => {
                if let Some(pack) = &mut self.state.loaded_pack {
                    match pack.write_asset("pack.mcmeta", bytes.clone()) {
                        Ok(()) => {
                            self.state.hot_reload_asset("pack.mcmeta", &bytes);
                            self.state.metadata_form.status_message =
                                Some(String::from("Saved pack.mcmeta."));
                            self.state.log_output(
                                WarningSeverity::Info,
                                "Updated pack.mcmeta via Metadata & Packaging editor.",
                            );
                        }
                        Err(err) => {
                            self.state.log_output(
                                WarningSeverity::Error,
                                format!("Failed saving pack.mcmeta: {err:#}"),
                            );
                        }
                    }
                } else {
                    self.state.metadata_form.status_message =
                        Some(String::from("Open a pack first to save pack.mcmeta."));
                }
            }
            Err(err) => {
                self.state.log_output(
                    WarningSeverity::Error,
                    format!("Failed building pack.mcmeta: {err:#}"),
                );
            }
        }
    }

    fn apply_bedrock_metadata_form(&mut self) {
        match self.state.metadata_form.build_bedrock_manifest_bytes() {
            Ok(bytes) => {
                if let Some(pack) = &mut self.state.loaded_pack {
                    match pack.write_asset("manifest.json", bytes.clone()) {
                        Ok(()) => {
                            self.state.hot_reload_asset("manifest.json", &bytes);
                            self.state.metadata_form.status_message = Some(String::from(
                                "Saved manifest.json (min_engine_version=[1,x,x]).",
                            ));
                            self.state.log_output(
                                WarningSeverity::Info,
                                "Updated manifest.json via Metadata & Packaging editor.",
                            );
                            self.start_semantic_audit();
                        }
                        Err(err) => {
                            self.state.log_output(
                                WarningSeverity::Error,
                                format!("Failed saving manifest.json: {err:#}"),
                            );
                        }
                    }
                } else {
                    self.state.metadata_form.status_message =
                        Some(String::from("Open a Bedrock pack first to save manifest.json."));
                }
            }
            Err(err) => {
                self.state.log_output(
                    WarningSeverity::Error,
                    format!("Failed building manifest.json: {err:#}"),
                );
            }
        }
    }

    fn render_sidebar_settings(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical()
            .id_salt("settings_sidebar_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(RichText::new("Blockbench Integrated Web Studio").strong().size(12.0));
                ui.label(
                    RichText::new(format!("Status: {}", self.blockbench.status_label()))
                        .small()
                        .color(theme::SEVERITY_INFO),
                );
                if ui.button("🌐 Buka Blockbench Studio di Browser Lokal").clicked() {
                    let root = self.active_project_root();
                    match self.blockbench.open_fallback_browser(root) {
                        Ok(url) => self.state.log_output(
                            WarningSeverity::Info,
                            format!("Opened local Blockbench Studio: {url}"),
                        ),
                        Err(err) => self.state.log_output(
                            WarningSeverity::Warning,
                            format!("Blockbench Studio launch failed: {err:#}"),
                        ),
                    }
                }

                ui.separator();
                ui.label(RichText::new("External Artist Tools").strong().size(12.0));
                ui.label(RichText::new("Fallback Blockbench binary path:").small().color(theme::TEXT_MUTED));
                ui.add(
                    TextEdit::singleline(&mut self.state.settings.blockbench_path)
                        .hint_text("blockbench or /path/to/blockbench")
                        .desired_width(f32::INFINITY),
                );

                ui.add_space(4.0);
                ui.label(RichText::new("External Image Editor (Aseprite / GIMP / Krita):").small().color(theme::TEXT_MUTED));
                ui.add(
                    TextEdit::singleline(&mut self.state.settings.external_image_editor)
                        .hint_text("Leave empty for OS default")
                        .desired_width(f32::INFINITY),
                );

                ui.separator();
                ui.label(RichText::new("Texture & Conversion Pipeline").strong().size(12.0));
                ui.checkbox(
                    &mut self.state.settings.convert_pbr,
                    "Enable LabPBR -> Bedrock MER PBR Conversion",
                );
                self.state.conversion_config.convert_pbr = self.state.settings.convert_pbr;

                ui.checkbox(
                    &mut self.state.settings.uv_atlas_guard,
                    "Enable 3D UV-Atlas Guard (check out-of-bounds UVs)",
                );
                self.state.conversion_config.uv_atlas_guard = self.state.settings.uv_atlas_guard;

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Default Downscale:");
                    egui::ComboBox::from_id_salt("settings_downscale_size")
                        .selected_text(format!("{}x", self.state.settings.default_downscale_size))
                        .show_ui(ui, |ui| {
                            for sz in [16u32, 32, 64, 128, 256] {
                                ui.selectable_value(
                                    &mut self.state.settings.default_downscale_size,
                                    sz,
                                    format!("{sz}x{sz}"),
                                );
                            }
                        });
                });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Rotation Mode:");
                    egui::ComboBox::from_id_salt("settings_rotation_mode")
                        .selected_text(format!("{:?}", self.state.conversion_config.rotation_mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.state.conversion_config.rotation_mode,
                                RotationSnapMode::Preserve,
                                "Preserve (1.21.6+ / 26.x)",
                            );
                            ui.selectable_value(
                                &mut self.state.conversion_config.rotation_mode,
                                RotationSnapMode::Snap22_5,
                                "Snap 22.5° (Legacy)",
                            );
                            ui.selectable_value(
                                &mut self.state.conversion_config.rotation_mode,
                                RotationSnapMode::Clamp45,
                                "Clamp ±45°",
                            );
                        });
                });
            });
    }

    fn render_bottom_panel(&mut self, ctx: &Context) {
        if !self.state.bottom_panel_visible {
            return;
        }
        TopBottomPanel::bottom("pack_studio_bottom_panel")
            .default_height(185.0)
            .min_height(110.0)
            .max_height(420.0)
            .resizable(true)
            .frame(
                Frame::new()
                    .fill(theme::BG_EDITOR)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .inner_margin(Margin::symmetric(8, 4)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (errs, warns) = self.state.problem_counts();
                    let prob_label = format!("PROBLEMS ({})", errs + warns);
                    let prob_active = self.state.active_bottom_tab == BottomTab::Problems;
                    if ui
                        .selectable_label(
                            prob_active,
                            RichText::new(prob_label)
                                .strong()
                                .size(11.5)
                                .color(if prob_active {
                                    theme::TEXT_BRIGHT
                                } else {
                                    theme::TEXT_MUTED
                                }),
                        )
                        .clicked()
                    {
                        self.state.active_bottom_tab = BottomTab::Problems;
                    }

                    let out_active = self.state.active_bottom_tab == BottomTab::Output;
                    if ui
                        .selectable_label(
                            out_active,
                            RichText::new("OUTPUT")
                                .strong()
                                .size(11.5)
                                .color(if out_active {
                                    theme::TEXT_BRIGHT
                                } else {
                                    theme::TEXT_MUTED
                                }),
                        )
                        .clicked()
                    {
                        self.state.active_bottom_tab = BottomTab::Output;
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("Hide Panel (Ctrl+J)").clicked() {
                            self.state.bottom_panel_visible = false;
                        }
                        if self.state.active_bottom_tab == BottomTab::Problems {
                            ui.add(
                                TextEdit::singleline(&mut self.state.problems_filter)
                                    .hint_text("Filter problems...")
                                    .desired_width(180.0),
                            );
                        } else if ui.small_button("Clear").clicked() {
                            self.state.output_logs.clear();
                        }
                    });
                });
                ui.separator();

                match self.state.active_bottom_tab {
                    BottomTab::Problems => self.render_bottom_problems(ui),
                    BottomTab::Output => self.render_bottom_output(ui),
                }
            });
    }

    fn render_bottom_problems(&mut self, ui: &mut egui::Ui) {
        let needle = self.state.problems_filter.trim().to_ascii_lowercase();
        let filtered: Vec<_> = self
            .state
            .problems
            .iter()
            .filter(|w| {
                needle.is_empty()
                    || w.source_path.to_ascii_lowercase().contains(&needle)
                    || w.reason.to_ascii_lowercase().contains(&needle)
                    || w.actionable_fix.to_ascii_lowercase().contains(&needle)
            })
            .cloned()
            .collect();

        if filtered.is_empty() {
            ui.label(
                RichText::new("No problems detected in the workspace.")
                    .color(theme::TEXT_MUTED),
            );
            return;
        }

        let mut clicked_path: Option<String> = None;

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(28.0))
            .column(Column::initial(220.0).at_least(120.0))
            .column(Column::exact(74.0))
            .column(Column::remainder().at_least(180.0))
            .column(Column::remainder().at_least(180.0))
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.label(RichText::new("Sev").strong().size(11.0));
                });
                header.col(|ui| {
                    ui.label(RichText::new("source_path").strong().size(11.0));
                });
                header.col(|ui| {
                    ui.label(RichText::new("stage").strong().size(11.0));
                });
                header.col(|ui| {
                    ui.label(RichText::new("reason").strong().size(11.0));
                });
                header.col(|ui| {
                    ui.label(RichText::new("actionable_fix").strong().size(11.0));
                });
            })
            .body(|mut body| {
                for item in &filtered {
                    body.row(22.0, |mut row| {
                        let (icon, col) = match item.severity {
                            WarningSeverity::Error => ("✖", theme::SEVERITY_ERROR),
                            WarningSeverity::Warning => ("⚠", theme::SEVERITY_WARNING),
                            WarningSeverity::Info => ("ℹ", theme::SEVERITY_INFO),
                        };
                        row.col(|ui| {
                            ui.label(RichText::new(icon).color(col));
                        });
                        row.col(|ui| {
                            if ui
                                .selectable_label(
                                    false,
                                    RichText::new(&item.source_path).monospace().size(11.5),
                                )
                                .clicked()
                            {
                                clicked_path = Some(item.source_path.clone());
                            }
                        });
                        row.col(|ui| {
                            ui.label(
                                RichText::new(item.stage.to_string())
                                    .small()
                                    .color(theme::TEXT_MUTED),
                            );
                        });
                        row.col(|ui| {
                            ui.label(RichText::new(&item.reason).size(11.5))
                                .on_hover_text(&item.reason);
                        });
                        row.col(|ui| {
                            ui.label(
                                RichText::new(&item.actionable_fix)
                                    .size(11.5)
                                    .color(theme::TEXT_MUTED),
                            )
                            .on_hover_text(&item.actionable_fix);
                        });
                    });
                }
            });

        if let Some(path) = clicked_path {
            let exists_in_pack = self
                .state
                .loaded_pack
                .as_ref()
                .is_some_and(|p| p.container.contains(&path));
            if exists_in_pack {
                self.state.open_asset_tab(&path);
            } else {
                self.state.log_output(
                    WarningSeverity::Info,
                    format!("Diagnostic path '{path}' is an emitted/virtual artifact (not in source pack)."),
                );
            }
        }
    }

    fn render_bottom_output(&self, ui: &mut egui::Ui) {
        ScrollArea::vertical()
            .id_salt("bottom_output_log")
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for entry in &self.state.output_logs {
                    let (tag, color) = match entry.level {
                        WarningSeverity::Error => ("ERR ", theme::SEVERITY_ERROR),
                        WarningSeverity::Warning => ("WARN", theme::SEVERITY_WARNING),
                        WarningSeverity::Info => ("INFO", theme::TEXT_PRIMARY),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format!("[{:03}] [{tag}]", entry.seq))
                                .monospace()
                                .size(11.5)
                                .color(color),
                        );
                        ui.label(
                            RichText::new(&entry.message)
                                .monospace()
                                .size(11.5)
                                .color(color),
                        );
                    });
                }
            });
    }

    fn render_command_palette(&mut self, ctx: &Context) {
        if !self.state.command_palette_open {
            return;
        }
        let matches = filter_commands(&self.state.command_palette_query);
        if matches.is_empty() {
            self.state.command_palette_selected = 0;
        } else if self.state.command_palette_selected >= matches.len() {
            self.state.command_palette_selected = matches.len() - 1;
        }

        let mut chosen_cmd: Option<StudioCommand> = None;
        ctx.input(|i| {
            if i.key_pressed(Key::ArrowDown) && !matches.is_empty() {
                self.state.command_palette_selected =
                    (self.state.command_palette_selected + 1) % matches.len();
            }
            if i.key_pressed(Key::ArrowUp) && !matches.is_empty() {
                self.state.command_palette_selected = if self.state.command_palette_selected == 0 {
                    matches.len() - 1
                } else {
                    self.state.command_palette_selected - 1
                };
            }
            if i.key_pressed(Key::Enter)
                && let Some(entry) = matches.get(self.state.command_palette_selected)
            {
                chosen_cmd = Some(entry.command);
            }
        });

        egui::Area::new(egui::Id::new("pack_studio_command_palette"))
            .order(Order::Foreground)
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 28.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(theme::BG_SIDEBAR)
                    .stroke(Stroke::new(1.0, theme::ACCENT_BLUE))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(Margin::same(8))
                    .shadow(egui::Shadow {
                        offset: [0, 8],
                        blur: 20,
                        spread: 2,
                        color: Color32::from_black_alpha(160),
                    })
                    .show(ui, |ui| {
                        ui.set_width(560.0);
                        let resp = ui.add(
                            TextEdit::singleline(&mut self.state.command_palette_query)
                                .hint_text("> Type a command (New Project, Blockbench, Export, Preflight, Audit)...")
                                .desired_width(544.0),
                        );
                        if self.palette_needs_focus {
                            resp.request_focus();
                            self.palette_needs_focus = false;
                        }
                        if resp.changed() {
                            self.state.command_palette_selected = 0;
                        }
                        ui.separator();

                        ScrollArea::vertical()
                            .max_height(280.0)
                            .id_salt("command_palette_list")
                            .show(ui, |ui| {
                                if matches.is_empty() {
                                    ui.label(
                                        RichText::new("No matching commands")
                                            .color(theme::TEXT_MUTED),
                                    );
                                }
                                for (idx, entry) in matches.iter().enumerate() {
                                    let selected = idx == self.state.command_palette_selected;
                                    let row_bg = if selected {
                                        theme::BG_SELECTION
                                    } else {
                                        Color32::TRANSPARENT
                                    };
                                    Frame::new()
                                        .fill(row_bg)
                                        .corner_radius(CornerRadius::same(2))
                                        .inner_margin(Margin::symmetric(6, 4))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                let lbl = ui.add(
                                                    egui::Label::new(
                                                        RichText::new(entry.title).color(
                                                            if selected {
                                                                theme::TEXT_BRIGHT
                                                            } else {
                                                                theme::TEXT_PRIMARY
                                                            },
                                                        ),
                                                    )
                                                    .sense(Sense::click()),
                                                );
                                                if lbl.clicked() {
                                                    chosen_cmd = Some(entry.command);
                                                }
                                                if !entry.shortcut.is_empty() {
                                                    ui.with_layout(
                                                        Layout::right_to_left(Align::Center),
                                                        |ui| {
                                                            ui.label(
                                                                RichText::new(entry.shortcut)
                                                                    .small()
                                                                    .monospace()
                                                                    .color(theme::TEXT_MUTED),
                                                            );
                                                        },
                                                    );
                                                }
                                            });
                                        });
                                }
                            });
                    });
            });

        if let Some(cmd) = chosen_cmd {
            self.state.close_command_palette();
            self.execute_command(cmd);
        }
    }

    fn render_quick_open_modal(&mut self, ctx: &Context) {
        if !self.state.quick_open_visible {
            return;
        }
        let matches = self
            .state
            .loaded_pack
            .as_ref()
            .map(|p| filter_quick_open_files(&p.container, &self.state.quick_open_query))
            .unwrap_or_default();

        if matches.is_empty() {
            self.state.quick_open_selected = 0;
        } else if self.state.quick_open_selected >= matches.len() {
            self.state.quick_open_selected = matches.len() - 1;
        }

        let mut chosen_path: Option<String> = None;
        ctx.input(|i| {
            if i.key_pressed(Key::ArrowDown) && !matches.is_empty() {
                self.state.quick_open_selected =
                    (self.state.quick_open_selected + 1) % matches.len();
            }
            if i.key_pressed(Key::ArrowUp) && !matches.is_empty() {
                self.state.quick_open_selected = if self.state.quick_open_selected == 0 {
                    matches.len() - 1
                } else {
                    self.state.quick_open_selected - 1
                };
            }
            if i.key_pressed(Key::Enter)
                && let Some(path) = matches.get(self.state.quick_open_selected)
            {
                chosen_path = Some(path.clone());
            }
        });

        egui::Area::new(egui::Id::new("pack_studio_quick_open"))
            .order(Order::Foreground)
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 28.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(theme::BG_SIDEBAR)
                    .stroke(Stroke::new(1.0, theme::ACCENT_BLUE))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(540.0);
                        let resp = ui.add(
                            TextEdit::singleline(&mut self.state.quick_open_query)
                                .hint_text("Quick Open File (Ctrl+P) — type texture or JSON name...")
                                .desired_width(524.0),
                        );
                        if self.quick_open_needs_focus {
                            resp.request_focus();
                            self.quick_open_needs_focus = false;
                        }
                        if resp.changed() {
                            self.state.quick_open_selected = 0;
                        }
                        ui.separator();

                        ScrollArea::vertical()
                            .max_height(280.0)
                            .id_salt("quick_open_list")
                            .show(ui, |ui| {
                                if matches.is_empty() {
                                    ui.label(
                                        RichText::new("No matching files in active pack")
                                            .color(theme::TEXT_MUTED),
                                    );
                                }
                                for (idx, path) in matches.iter().enumerate() {
                                    let selected = idx == self.state.quick_open_selected;
                                    Frame::new()
                                        .fill(if selected {
                                            theme::BG_SELECTION
                                        } else {
                                            Color32::TRANSPARENT
                                        })
                                        .inner_margin(Margin::symmetric(6, 3))
                                        .show(ui, |ui| {
                                            let lbl = ui.add(
                                                egui::Label::new(
                                                    RichText::new(path).monospace().size(12.0).color(
                                                        if selected {
                                                            theme::TEXT_BRIGHT
                                                        } else {
                                                            theme::TEXT_PRIMARY
                                                        },
                                                    ),
                                                )
                                                .sense(Sense::click()),
                                            );
                                            if lbl.clicked() {
                                                chosen_path = Some(path.clone());
                                            }
                                        });
                                }
                            });
                    });
            });

        if let Some(p) = chosen_path {
            self.state.close_quick_open();
            self.state.open_asset_tab(&p);
        }
    }

    fn render_new_project_wizard_modal(&mut self, ctx: &Context) {
        if !self.state.wizard.open {
            return;
        }
        let mut should_create = false;
        let mut should_close = false;
        let mut pick_parent = false;
        let mut pick_icon = false;

        egui::Window::new("✨ New Project Wizard — Java & Bedrock")
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(620.0, 560.0))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ScrollArea::vertical().max_height(470.0).show(ui, |ui| {
                    ui.label(RichText::new("1. Choose Edition & Project Type").strong());
                    for kind in ProjectTemplateKind::ALL {
                        let selected = self.state.wizard.kind == kind;
                        Frame::new()
                            .fill(if selected {
                                theme::BG_SELECTION
                            } else {
                                theme::BG_EDITOR
                            })
                            .stroke(Stroke::new(
                                1.0,
                                if selected {
                                    theme::ACCENT_BLUE
                                } else {
                                    theme::BORDER
                                },
                            ))
                            .corner_radius(CornerRadius::same(4))
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                if ui
                                    .radio_value(&mut self.state.wizard.kind, kind, RichText::new(kind.label()).strong())
                                    .clicked()
                                {
                                    self.state.wizard.error_message = None;
                                }
                                ui.label(RichText::new(kind.subtitle()).small().color(theme::TEXT_MUTED));
                            });
                        ui.add_space(4.0);
                    }

                    ui.separator();
                    ui.label(RichText::new("2. Metadata & Identity").strong());
                    ui.horizontal(|ui| {
                        ui.label("Pack Name:");
                        ui.add(
                            TextEdit::singleline(&mut self.state.wizard.pack_name)
                                .desired_width(f32::INFINITY),
                        );
                    });

                    ui.label("Description (§ colors supported):");
                    render_mc_formatting_toolbar(ui, &mut self.state.wizard.description);
                    ui.add(
                        TextEdit::multiline(&mut self.state.wizard.description)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                    render_minecraft_formatted_preview(ui, &self.state.wizard.description);

                    ui.horizontal(|ui| {
                        ui.label("Author(s) (comma-separated):");
                        ui.add(
                            TextEdit::singleline(&mut self.state.wizard.authors)
                                .desired_width(f32::INFINITY),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("License:");
                        ui.add(
                            TextEdit::singleline(&mut self.state.wizard.license)
                                .desired_width(180.0),
                        );
                        ui.label("Website/URL:");
                        ui.add(
                            TextEdit::singleline(&mut self.state.wizard.url)
                                .hint_text("https://...")
                                .desired_width(f32::INFINITY),
                        );
                    });

                    ui.horizontal(|ui| {
                        ui.label("Version:");
                        for v in &mut self.state.wizard.version {
                            ui.add(egui::DragValue::new(v).range(0..=999));
                        }
                    });

                    ui.separator();
                    ui.label(RichText::new("3. Target Engine & Namespace").strong());
                    if self.state.wizard.kind == ProjectTemplateKind::JavaResourcePack {
                        ui.horizontal(|ui| {
                            ui.label("Java pack_format:");
                            ui.add(
                                egui::DragValue::new(&mut self.state.wizard.java_pack_format)
                                    .range(1..=200),
                            );
                            ui.checkbox(
                                &mut self.state.wizard.java_use_supported_formats,
                                "supported_formats:",
                            );
                            if self.state.wizard.java_use_supported_formats {
                                ui.add(
                                    egui::DragValue::new(&mut self.state.wizard.java_supported_min)
                                        .range(1..=200),
                                );
                                ui.label("..");
                                ui.add(
                                    egui::DragValue::new(&mut self.state.wizard.java_supported_max)
                                        .range(1..=1024),
                                );
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Namespace:");
                            ui.add(
                                TextEdit::singleline(&mut self.state.wizard.java_namespace)
                                    .hint_text("minecraft")
                                    .desired_width(180.0),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "assets/{}/...",
                                    self.state.wizard.sanitized_namespace()
                                ))
                                .small()
                                .monospace()
                                .color(theme::TEXT_MUTED),
                            );
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.label("Bedrock min_engine_version:");
                            ui.label(RichText::new("1 .").strong().color(theme::SEVERITY_OK));
                            ui.add(
                                egui::DragValue::new(
                                    &mut self.state.wizard.bedrock_min_engine_version[1],
                                )
                                .range(14..=99),
                            );
                            ui.add(
                                egui::DragValue::new(
                                    &mut self.state.wizard.bedrock_min_engine_version[2],
                                )
                                .range(0..=999),
                            );
                            self.state.wizard.bedrock_min_engine_version[0] = 1;
                        });
                    }

                    ui.separator();
                    ui.label(RichText::new("4. Pack Icon & Location").strong());
                    ui.horizontal(|ui| {
                        let icon_label = self
                            .state
                            .wizard
                            .icon_path
                            .as_ref()
                            .map_or_else(
                                || String::from("Default PackStudio Icon"),
                                |p| p.display().to_string(),
                            );
                        ui.label(format!("Icon: {icon_label}"));
                        if ui.small_button("Choose PNG...").clicked() {
                            pick_icon = true;
                        }
                        if self.state.wizard.icon_path.is_some() && ui.small_button("Reset").clicked() {
                            self.state.wizard.icon_path = None;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Project Folder:");
                        ui.add(
                            TextEdit::singleline(&mut self.state.wizard.target_parent_dir)
                                .desired_width(ui.available_width() - 78.0),
                        );
                        if ui.button("Browse").clicked() {
                            pick_parent = true;
                        }
                    });

                    if let Some(err) = &self.state.wizard.error_message {
                        ui.add_space(4.0);
                        ui.colored_label(theme::SEVERITY_ERROR, err);
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("✨ Create & Open Project").strong()).clicked() {
                        should_create = true;
                    }
                    if ui.button("Cancel").clicked() {
                        should_close = true;
                    }
                });
            });

        if pick_parent {
            self.spawn_file_dialog(DialogTarget::WizardParentDir);
        }
        if pick_icon {
            self.spawn_file_dialog(DialogTarget::WizardPickIcon);
        }
        if should_create {
            match scaffold_new_project(&self.state.wizard) {
                Ok(res) => {
                    self.state.wizard.open = false;
                    self.state.wizard.error_message = None;
                    self.state.companion_bp_dir = res.companion_bp_dir;
                    self.state.log_output(
                        WarningSeverity::Info,
                        format!(
                            "Scaffolded new {} project at '{}'",
                            self.state.wizard.kind.label(),
                            res.workspace_root.display()
                        ),
                    );
                    self.start_load_pack(res.open_dir);
                }
                Err(err) => {
                    self.state.wizard.error_message = Some(format!("{err:#}"));
                }
            }
        } else if should_close {
            self.state.wizard.open = false;
        }
    }

    fn render_export_package_modal(&mut self, ctx: &Context) {
        if !self.state.export_config.open {
            return;
        }
        let edition = self
            .state
            .loaded_pack
            .as_ref()
            .map_or(Edition::Java, |p| p.edition);
        let (pack_name, version) = self.state.metadata_form.active_name_and_version(edition);

        let mut do_export = false;
        let mut do_close = false;
        let mut pick_dir = false;
        let mut open_out_dir: Option<PathBuf> = None;

        egui::Window::new("📤 Export / Package Resource Pack")
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(560.0, 420.0))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(RichText::new("Output Format").strong());
                ui.horizontal(|ui| {
                    for fmt in [PackageFormat::Zip, PackageFormat::Mcpack, PackageFormat::Mcaddon] {
                        ui.selectable_value(
                            &mut self.state.export_config.format,
                            fmt,
                            fmt.label(),
                        );
                    }
                });

                ui.add_space(6.0);
                ui.label(RichText::new("Filename Template ({pack_name}, {version}, {edition}):").small());
                ui.add(
                    TextEdit::singleline(&mut self.state.export_config.filename_template)
                        .desired_width(f32::INFINITY),
                );
                let preview = render_output_filename(
                    &self.state.export_config.filename_template,
                    &pack_name,
                    version,
                    edition,
                    self.state.export_config.format,
                );
                ui.label(
                    RichText::new(format!("Output filename: {preview}"))
                        .small()
                        .monospace()
                        .color(theme::SEVERITY_INFO),
                );

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Output Folder:");
                    ui.add(
                        TextEdit::singleline(&mut self.state.export_config.output_dir)
                            .desired_width(ui.available_width() - 78.0),
                    );
                    if ui.button("Browse").clicked() {
                        pick_dir = true;
                    }
                });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Compression:");
                    for preset in [
                        CompressionPreset::Fast,
                        CompressionPreset::Balanced,
                        CompressionPreset::Maximum,
                    ] {
                        ui.selectable_value(
                            &mut self.state.export_config.compression,
                            preset,
                            preset.label(),
                        );
                    }
                });

                ui.checkbox(
                    &mut self.state.export_config.strip_junk_files,
                    "Strip junk files (.git, desktop.ini, .DS_Store, Thumbs.db, temp backups)",
                );
                ui.checkbox(
                    &mut self.state.export_config.run_semantic_audit,
                    "Run Semantic Audit Gate before Packaging (recommended)",
                );
                if self.state.export_config.run_semantic_audit {
                    ui.checkbox(
                        &mut self.state.export_config.allow_export_with_errors,
                        "Allow export even if Semantic Audit reports fatal errors (not recommended)",
                    );
                }

                if let Some(err) = &self.state.export_config.last_error {
                    ui.add_space(6.0);
                    ui.colored_label(theme::SEVERITY_ERROR, err);
                }

                if let Some(summary) = &self.state.export_config.last_summary {
                    ui.add_space(8.0);
                    Frame::new()
                        .fill(theme::BADGE_PASSED_BG)
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(Margin::same(8))
                        .show(ui, |ui| {
                            let mb = summary.size_bytes as f64 / (1024.0 * 1024.0);
                            ui.label(
                                RichText::new(format!(
                                    "✓ Exported: {}",
                                    summary.output_path.display()
                                ))
                                .strong()
                                .color(theme::TEXT_BRIGHT),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "Size: {mb:.2} MB ({} bytes) | Assets: {} | Junk Stripped: {}",
                                    summary.size_bytes,
                                    summary.asset_count,
                                    summary.stripped_junk_count
                                ))
                                .small()
                                .color(theme::TEXT_BRIGHT),
                            );
                            ui.label(
                                RichText::new(format!("Audit: {}", summary.audit_summary))
                                    .small()
                                    .color(theme::TEXT_BRIGHT),
                            );
                            if ui.small_button("📂 Open Output Folder").clicked() {
                                open_out_dir = Some(summary.output_path.clone());
                            }
                        });
                }

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("📤 Export Now").strong()).clicked() {
                        do_export = true;
                    }
                    if ui.button("Close").clicked() {
                        do_close = true;
                    }
                });
            });

        if pick_dir {
            self.spawn_file_dialog(DialogTarget::ExportOutputDir);
        }
        if let Some(p) = open_out_dir {
            let _ = reveal_in_file_manager(&p);
        }
        if do_export {
            if let Some(pack) = &self.state.loaded_pack {
                match export_pack_archive(
                    &pack.container,
                    self.state.companion_bp_dir.as_deref(),
                    &self.state.export_config,
                    edition,
                    &pack_name,
                    version,
                    self.state.settings.uv_atlas_guard,
                ) {
                    Ok(summary) => {
                        self.state.log_output(
                            WarningSeverity::Info,
                            format!(
                                "Packaged '{}' ({} assets, {} bytes)",
                                summary.output_path.display(),
                                summary.asset_count,
                                summary.size_bytes
                            ),
                        );
                        self.state.export_config.last_error = None;
                        self.state.export_config.last_summary = Some(summary);
                    }
                    Err(err) => {
                        self.state.export_config.last_error = Some(format!("{err:#}"));
                    }
                }
            } else {
                self.state.export_config.last_error =
                    Some(String::from("No resource pack currently loaded."));
            }
        }
        if do_close {
            self.state.export_config.open = false;
        }
    }

    fn render_smart_import_modal(&mut self, ctx: &Context) {
        if self.state.pending_imports.is_empty() {
            return;
        }
        let mut resolution: Option<ConflictResolution> = None;
        let has_collision = {
            let item = &self.state.pending_imports[0];
            let target = item.resolved_target_rel_path();
            self.state
                .loaded_pack
                .as_ref()
                .is_some_and(|p| p.container.contains(&target))
        };

        let remaining_count = self.state.pending_imports.len();
        egui::Window::new("📥 Smart Minecraft Asset Import Helper")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let item = &mut self.state.pending_imports[0];
                ui.label(
                    RichText::new(format!(
                        "Importing '{}' ({remaining_count} remaining)",
                        item.original_name,
                    ))
                    .strong(),
                );
                if item.needs_png_conversion {
                    ui.checkbox(
                        &mut item.convert_to_png,
                        "Auto-convert .jpg / .webp to Minecraft-compatible .png",
                    );
                }
                if item.needs_sanitization {
                    ui.checkbox(
                        &mut item.auto_sanitize,
                        format!(
                            "Auto-Sanitize filename to valid Minecraft ID ('{}')",
                            item.sanitized_name
                        ),
                    );
                }
                let final_path = item.resolved_target_rel_path();
                ui.label(
                    RichText::new(format!("Destination: {final_path}"))
                        .monospace()
                        .small()
                        .color(theme::SEVERITY_INFO),
                );
                if has_collision {
                    ui.colored_label(
                        theme::SEVERITY_WARNING,
                        "⚠ A file with this exact name already exists in the pack.",
                    );
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if has_collision {
                        if ui.button("Overwrite").clicked() {
                            resolution = Some(ConflictResolution::Overwrite);
                        }
                        if ui.button("Keep Both (Rename)").clicked() {
                            resolution = Some(ConflictResolution::KeepBoth);
                        }
                    } else if ui.button("Import Asset").clicked() {
                        resolution = Some(ConflictResolution::Overwrite);
                    }
                    if ui.button("Cancel").clicked() {
                        resolution = Some(ConflictResolution::Cancel);
                    }
                });
            });

        if let Some(res) = resolution {
            let item = self.state.pending_imports.remove(0);
            self.commit_import_item(&item, res);
        }
    }

    /// Renders one complete frame of the VS Code workbench UI (callable from headless tests too).
    pub fn render_ui(&mut self, ctx: &Context) {
        if !self.theme_applied {
            theme::apply_vscode_dark_plus(ctx);
            self.theme_applied = true;
        }

        self.poll_background_events();
        self.poll_blockbench_saves();
        self.poll_file_dialogs();
        self.poll_hot_reload();
        self.poll_os_dropped_files(ctx);
        self.handle_shortcuts(ctx);

        let queued = std::mem::take(&mut self.pending_commands);
        for cmd in queued {
            self.execute_command(cmd);
        }

        self.render_blockbench_fallback_banner(ctx);
        self.render_status_bar(ctx);
        self.render_activity_bar(ctx);
        self.render_primary_sidebar(ctx);
        self.render_bottom_panel(ctx);

        let mut tab_actions = Vec::new();
        let dock_style = vscode_dock_style(&ctx.style());

        egui::CentralPanel::default()
            .frame(Frame::new().fill(theme::BG_EDITOR).inner_margin(Margin::ZERO))
            .show(ctx, |ui| {
                let mut viewer = PackTabViewer {
                    recent_projects: &self.state.recent_projects,
                    conversion_config: &mut self.state.conversion_config,
                    preflight_report: self.state.preflight_report.as_ref(),
                    last_conversion: self.state.last_conversion.as_ref(),
                    loaded_pack_name: self.state.loaded_pack.as_ref().map(|p| p.pack_name.as_str()),
                    loaded_pack_edition: self.state.loaded_pack.as_ref().map(|p| p.edition),
                    texture_cache: &mut self.texture_cache,
                    actions: &mut tab_actions,
                };
                DockArea::new(&mut self.state.dock_state)
                    .style(dock_style)
                    .show_inside(ui, &mut viewer);
            });

        self.apply_tab_actions(tab_actions);
        self.render_command_palette(ctx);
        self.render_quick_open_modal(ctx);
        self.render_new_project_wizard_modal(ctx);
        self.render_export_package_modal(ctx);
        self.render_smart_import_modal(ctx);

        if self.state.busy_label.is_some()
            || self.watcher.is_some()
            || self.blockbench.last_url.is_some()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }
    }
}

impl eframe::App for PackStudioApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.render_ui(ctx);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_tree_node(
    ui: &mut egui::Ui,
    node: &FileTreeNode,
    severities: &std::collections::BTreeMap<String, WarningSeverity>,
    selected_item: &Option<(String, bool)>,
    has_clipboard: bool,
    collapse_epoch: u64,
    inline_edit: &mut Option<ExplorerInlineEdit>,
    inline_needs_focus: &mut bool,
    actions: &mut Vec<ExplorerAction>,
    commit_inline: &mut Option<ExplorerInlineEdit>,
    cancel_inline: &mut bool,
) {
    for (dir_name, child) in &node.dirs {
        let dir_path = child.rel_path.clone();
        let is_renaming = matches!(
            inline_edit,
            Some(ExplorerInlineEdit::Rename { old_path, is_dir: true, .. }) if *old_path == dir_path
        );

        if is_renaming && let Some(edit) = inline_edit.as_mut() {
            render_inline_edit_row(ui, edit, inline_needs_focus, commit_inline, cancel_inline);
            continue;
        }

        let header_id = ui.make_persistent_id(format!("dir:{dir_path}:c={collapse_epoch}"));
        let default_open = dir_path.matches('/').count() <= 1;
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            header_id,
            default_open,
        );
        let is_selected = selected_item
            .as_ref()
            .is_some_and(|(p, d)| *d && p == &dir_path);

        let header_resp = state.show_header(ui, |ui| {
            let lbl = ui.selectable_label(is_selected, RichText::new(format!("📁 {dir_name}")).size(12.0));
            if lbl.clicked() {
                actions.push(ExplorerAction::SelectDir(dir_path.clone()));
            }
            lbl.context_menu(|ui| {
                if ui.button("📄 New File").clicked() {
                    actions.push(ExplorerAction::StartInlineNewFile(dir_path.clone()));
                    ui.close_menu();
                }
                if ui.button("📁 New Folder").clicked() {
                    actions.push(ExplorerAction::StartInlineNewFolder(dir_path.clone()));
                    ui.close_menu();
                }
                if ui.button("📥 Import Images / Assets Here...").clicked() {
                    actions.push(ExplorerAction::ImportToDir(dir_path.clone()));
                    ui.close_menu();
                }
                if ui.button("🎨 New Blank Texture (16x16 PNG)").clicked() {
                    actions.push(ExplorerAction::NewBlankTexture(dir_path.clone()));
                    ui.close_menu();
                }
                if ui.button("🧊 New Bedrock Geometry (.geo.json) in Blockbench").clicked() {
                    actions.push(ExplorerAction::NewBedrockGeo(dir_path.clone()));
                    ui.close_menu();
                }
                ui.separator();
                if has_clipboard && ui.button("📋 Paste (Ctrl+V)").clicked() {
                    actions.push(ExplorerAction::PasteInto(dir_path.clone()));
                    ui.close_menu();
                }
                if ui.button("✏ Rename (F2)").clicked() {
                    actions.push(ExplorerAction::StartInlineRename {
                        path: dir_path.clone(),
                        is_dir: true,
                    });
                    ui.close_menu();
                }
                if ui.button("🗑 Delete (Del)").clicked() {
                    actions.push(ExplorerAction::DeleteItem(dir_path.clone()));
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Copy Relative Path").clicked() {
                    actions.push(ExplorerAction::CopyRelativePath(dir_path.clone()));
                    ui.close_menu();
                }
                if ui.button("Reveal in OS File Manager").clicked() {
                    actions.push(ExplorerAction::RevealInFileManager(dir_path.clone()));
                    ui.close_menu();
                }
            });
        });

        header_resp.body(|ui| {
            // Render inline create box directly inside this folder if targeted here
            if let Some(edit) = inline_edit.as_mut() {
                let is_child_create = matches!(
                    edit,
                    ExplorerInlineEdit::NewFile { parent_dir, .. }
                    | ExplorerInlineEdit::NewFolder { parent_dir, .. } if *parent_dir == dir_path
                );
                if is_child_create {
                    render_inline_edit_row(
                        ui,
                        edit,
                        inline_needs_focus,
                        commit_inline,
                        cancel_inline,
                    );
                }
            }

            render_tree_node(
                ui,
                child,
                severities,
                selected_item,
                has_clipboard,
                collapse_epoch,
                inline_edit,
                inline_needs_focus,
                actions,
                commit_inline,
                cancel_inline,
            );
        });
    }

    for (file_name, full_path) in &node.files {
        let is_renaming = matches!(
            inline_edit,
            Some(ExplorerInlineEdit::Rename { old_path, is_dir: false, .. }) if *old_path == *full_path
        );
        if is_renaming && let Some(edit) = inline_edit.as_mut() {
            render_inline_edit_row(ui, edit, inline_needs_focus, commit_inline, cancel_inline);
            continue;
        }

        let badge = severities.get(full_path).copied();
        let is_selected = selected_item
            .as_ref()
            .is_some_and(|(p, d)| !*d && p == full_path);

        let icon = if file_name.ends_with(".png") || file_name.ends_with(".tga") {
            "🎨"
        } else if file_name.ends_with(".geo.json") {
            "🧊"
        } else if file_name.ends_with(".json") || file_name.ends_with(".mcmeta") {
            "📄"
        } else if file_name.ends_with(".ogg") {
            "🔊"
        } else {
            "📝"
        };

        ui.horizontal(|ui| {
            if let Some(sev) = badge {
                let color = match sev {
                    WarningSeverity::Error => theme::SEVERITY_ERROR,
                    WarningSeverity::Warning => theme::SEVERITY_WARNING,
                    WarningSeverity::Info => theme::SEVERITY_INFO,
                };
                ui.label(RichText::new("●").color(color).size(10.0));
            }
            let resp = ui
                .selectable_label(
                    is_selected,
                    RichText::new(format!("{icon} {file_name}")).size(12.0),
                )
                .on_hover_text(full_path);

            if resp.clicked() {
                actions.push(ExplorerAction::OpenFile(full_path.clone()));
            } else if resp.secondary_clicked() {
                actions.push(ExplorerAction::SelectFile(full_path.clone()));
            }

            let parent_dir = full_path
                .rsplit_once('/')
                .map_or_else(String::new, |(d, _)| d.to_string());

            resp.context_menu(|ui| {
                if ui.button("Open in New Tab").clicked() {
                    actions.push(ExplorerAction::OpenFile(full_path.clone()));
                    ui.close_menu();
                }
                if ui.button("Open to the Side (Split)").clicked() {
                    actions.push(ExplorerAction::OpenFileSide(full_path.clone()));
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("🧊 Open in Blockbench (3D & Paint)").clicked() {
                    actions.push(ExplorerAction::OpenInBlockbench(full_path.clone()));
                    ui.close_menu();
                }
                if ui.button("Open in External Image Editor").clicked() {
                    actions.push(ExplorerAction::OpenExternalEditor(full_path.clone()));
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("✏ Rename (F2)").clicked() {
                    actions.push(ExplorerAction::StartInlineRename {
                        path: full_path.clone(),
                        is_dir: false,
                    });
                    ui.close_menu();
                }
                if ui.button("Duplicate").clicked() {
                    actions.push(ExplorerAction::DuplicateFile(full_path.clone()));
                    ui.close_menu();
                }
                if ui.button("Cut (Ctrl+X)").clicked() {
                    actions.push(ExplorerAction::CutItem(full_path.clone()));
                    ui.close_menu();
                }
                if ui.button("Copy (Ctrl+C)").clicked() {
                    actions.push(ExplorerAction::CopyItem(full_path.clone()));
                    ui.close_menu();
                }
                if has_clipboard && ui.button("Paste Here (Ctrl+V)").clicked() {
                    actions.push(ExplorerAction::PasteInto(parent_dir.clone()));
                    ui.close_menu();
                }
                if ui.button("🗑 Delete (Del)").clicked() {
                    actions.push(ExplorerAction::DeleteItem(full_path.clone()));
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Copy Relative Path").clicked() {
                    actions.push(ExplorerAction::CopyRelativePath(full_path.clone()));
                    ui.close_menu();
                }
                if ui.button("Copy Minecraft Resource ID").clicked() {
                    actions.push(ExplorerAction::CopyResourceId(full_path.clone()));
                    ui.close_menu();
                }
                if ui.button("Reveal in OS File Manager").clicked() {
                    actions.push(ExplorerAction::RevealInFileManager(full_path.clone()));
                    ui.close_menu();
                }
            });
        });
    }
}

fn render_inline_edit_row(
    ui: &mut egui::Ui,
    edit: &mut ExplorerInlineEdit,
    needs_focus: &mut bool,
    commit_inline: &mut Option<ExplorerInlineEdit>,
    cancel_inline: &mut bool,
) {
    let mut submitted = false;
    {
        let (prefix, hint, buf) = match edit {
            ExplorerInlineEdit::NewFile { input, .. } => ("📄", "new_file.json / .png", input),
            ExplorerInlineEdit::NewFolder { input, .. } => ("📁", "new_folder", input),
            ExplorerInlineEdit::Rename { input, .. } => ("✏", "new_name", input),
        };
        ui.horizontal(|ui| {
            ui.label(prefix);
            let resp = ui.add(
                TextEdit::singleline(buf)
                    .hint_text(hint)
                    .desired_width(ui.available_width() - 8.0),
            );
            if *needs_focus {
                resp.request_focus();
                *needs_focus = false;
            }
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                submitted = true;
            } else if ui.input(|i| i.key_pressed(Key::Escape)) {
                *cancel_inline = true;
            }
        });
    }
    if submitted {
        *commit_inline = Some(edit.clone());
    }
}

struct PackTabViewer<'a> {
    recent_projects: &'a [PathBuf],
    conversion_config: &'a mut crate::pipeline::ConversionConfig,
    preflight_report: Option<&'a crate::pipeline::PreflightReport>,
    last_conversion: Option<&'a crate::pipeline::ConversionResult>,
    loaded_pack_name: Option<&'a str>,
    loaded_pack_edition: Option<Edition>,
    texture_cache: &'a mut HashMap<String, TextureHandle>,
    actions: &'a mut Vec<TabAction>,
}

impl TabViewer for PackTabViewer<'_> {
    type Tab = EditorTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        tab.title().into()
    }

    fn id(&mut self, tab: &mut Self::Tab) -> egui::Id {
        egui::Id::new(tab.key())
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            EditorTab::Welcome => self.render_welcome_tab(ui),
            EditorTab::Texture(tex) => self.render_texture_tab(ui, tex),
            EditorTab::Code(code) => self.render_code_tab(ui, code),
            EditorTab::ConversionDashboard => self.render_conversion_dashboard(ui),
        }
    }
}

impl PackTabViewer<'_> {
    fn render_welcome_tab(&mut self, ui: &mut egui::Ui) {
        Frame::new()
            .fill(theme::BG_EDITOR)
            .inner_margin(Margin::same(28))
            .show(ui, |ui| {
                ScrollArea::vertical().show(ui, |ui| {
                    ui.label(
                        RichText::new("PackStudio")
                            .size(30.0)
                            .strong()
                            .color(theme::TEXT_BRIGHT),
                    );
                    ui.label(
                        RichText::new(
                            "VS Code-style Resource Pack Workbench for Java & Bedrock Artists (with Blockbench 3D & Paint Bridge)",
                        )
                        .size(14.0)
                        .color(theme::TEXT_MUTED),
                    );
                    ui.add_space(22.0);

                    ui.columns(2, |cols| {
                        cols[0].label(RichText::new("Start").size(16.0).strong());
                        cols[0].add_space(6.0);
                        if cols[0]
                            .button(RichText::new("✨  New Project Wizard (Java / Bedrock / Add-On)...").size(13.5))
                            .clicked()
                        {
                            self.actions.push(TabAction::OpenNewProjectWizard);
                        }
                        cols[0].add_space(4.0);
                        if cols[0]
                            .button(RichText::new("📂  Open Pack Folder...").size(13.5))
                            .clicked()
                        {
                            self.actions.push(TabAction::OpenFolder);
                        }
                        cols[0].add_space(4.0);
                        if cols[0]
                            .button(RichText::new("📦  Open .zip / .mcpack Archive...").size(13.5))
                            .clicked()
                        {
                            self.actions.push(TabAction::OpenArchive);
                        }
                        cols[0].add_space(4.0);
                        if cols[0]
                            .button(RichText::new("📤  Export / Package Active Pack...").size(13.5))
                            .clicked()
                        {
                            self.actions.push(TabAction::OpenExportDialog);
                        }
                        cols[0].add_space(4.0);
                        if cols[0]
                            .button(RichText::new("🧊  New Bedrock Geometry (.geo.json) in Blockbench").size(13.5))
                            .clicked()
                        {
                            self.actions.push(TabAction::NewBedrockGeoInBlockbench);
                        }

                        cols[0].add_space(18.0);
                        cols[0].label(RichText::new("Recent Projects").size(16.0).strong());
                        cols[0].add_space(6.0);
                        if self.recent_projects.is_empty() {
                            cols[0].label(
                                RichText::new("No recent projects yet.")
                                    .color(theme::TEXT_MUTED),
                            );
                        } else {
                            for path in self.recent_projects {
                                let name = path
                                    .file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("Pack");
                                if cols[0]
                                    .link(format!("{name}  —  {}", path.display()))
                                    .clicked()
                                {
                                    self.actions.push(TabAction::OpenRecent(path.clone()));
                                }
                            }
                        }

                        cols[1].label(RichText::new("Keyboard Shortcuts").size(16.0).strong());
                        cols[1].add_space(6.0);
                        for (key, desc) in [
                            ("Ctrl+P", "Quick Open File (Fuzzy Search)"),
                            ("Ctrl+Shift+P / F1", "Open Command Palette"),
                            ("Ctrl+B", "Toggle Primary Side Bar"),
                            ("Ctrl+J", "Toggle Bottom Panel (Problems / Output)"),
                            ("Ctrl+Shift+E", "Focus Explorer File Tree"),
                            ("Ctrl+Shift+F", "Focus Asset Search"),
                            ("Ctrl+S", "Save Active JSON / Text Editor"),
                            ("F2 / Del", "Rename / Delete Selected Explorer Item"),
                            ("Ctrl+Scroll", "Zoom Texture Inspector (Nearest-Neighbor)"),
                            ("Middle / Space+Drag", "Pan Texture Canvas"),
                        ] {
                            cols[1].horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("{key:<22}"))
                                        .monospace()
                                        .color(theme::SEVERITY_INFO),
                                );
                                ui.label(RichText::new(desc).color(theme::TEXT_PRIMARY));
                            });
                        }
                    });
                });
            });
    }

    fn render_texture_tab(&mut self, ui: &mut egui::Ui, tex: &mut TextureTabState) {
        Frame::new()
            .fill(theme::BG_SIDEBAR)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .inner_margin(Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("Fit").clicked() {
                        tex.fit_to_view = true;
                        tex.pan = GlamVec2::ZERO;
                    }
                    for (label, factor) in [("100%", 1.0), ("400%", 4.0), ("800%", 8.0)] {
                        if ui.small_button(label).clicked() {
                            tex.fit_to_view = false;
                            tex.zoom = factor;
                        }
                    }
                    ui.label(
                        RichText::new(format!("{}%", (tex.zoom * 100.0).round() as i32))
                            .monospace()
                            .small()
                            .color(theme::TEXT_MUTED),
                    );

                    ui.separator();

                    ui.toggle_value(&mut tex.show_pixel_grid, "Grid");
                    ui.toggle_value(&mut tex.show_alpha_mask, "Alpha Mask");
                    ui.toggle_value(&mut tex.show_biome_tint, "Biome Tint");
                    if tex.show_biome_tint {
                        egui::ComboBox::from_id_salt(format!("biome_cb_{}", tex.path))
                            .selected_text(tex.biome_preset.label())
                            .show_ui(ui, |ui| {
                                for preset in BiomeTintPreset::ALL {
                                    ui.selectable_value(
                                        &mut tex.biome_preset,
                                        preset,
                                        preset.label(),
                                    );
                                }
                            });
                    }

                    ui.separator();

                    egui::ComboBox::from_id_salt(format!("downscale_cb_{}", tex.path))
                        .width(64.0)
                        .selected_text(format!("{}x", tex.downscale_target))
                        .show_ui(ui, |ui| {
                            for sz in [16u32, 32, 64, 128, 256] {
                                ui.selectable_value(&mut tex.downscale_target, sz, format!("{sz}x"));
                            }
                        });
                    if ui
                        .small_button("⬇ Downscale")
                        .on_hover_text("Nearest-Neighbor resize via fast_image_resize")
                        .clicked()
                    {
                        self.actions.push(TabAction::DownscaleTexture {
                            path: tex.path.clone(),
                            target_size: tex.downscale_target,
                        });
                    }

                    if ui
                        .small_button("🌿 Grass Side TGA")
                        .on_hover_text("Composite Bedrock grass_side.tga from current texture + overlay")
                        .clicked()
                    {
                        self.actions.push(TabAction::CompositeGrassSideTga {
                            path: tex.path.clone(),
                            biome: tex.biome_preset,
                        });
                    }

                    if ui
                        .small_button("🧊 Open in Blockbench (3D & Paint)")
                        .clicked()
                    {
                        self.actions
                            .push(TabAction::OpenInBlockbenchStudio(tex.path.clone()));
                    }

                    if let Some(stats) = &tex.stats {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(stats.format_badge())
                                    .monospace()
                                    .small()
                                    .color(theme::SEVERITY_INFO),
                            );
                            if stats.partial_alpha_pixels > 0 {
                                ui.label(
                                    RichText::new(format!(
                                        "⚠ {} semi-alpha px",
                                        stats.partial_alpha_pixels
                                    ))
                                    .small()
                                    .color(theme::SEVERITY_WARNING),
                                );
                            }
                        });
                    }
                });
            });

        if let Some(err) = &tex.load_error {
            ui.centered_and_justified(|ui| {
                ui.colored_label(theme::SEVERITY_ERROR, format!("Failed to load texture: {err}"));
            });
            return;
        }

        let Some(base_img) = &tex.decoded else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("No texture payload loaded.").color(theme::TEXT_MUTED));
            });
            return;
        };

        let cache_key = tex.render_cache_key();
        let tex_handle = if let Some(handle) = self.texture_cache.get(&cache_key) {
            handle.clone()
        } else {
            let preview_img: RgbaImage = if tex.show_alpha_mask {
                build_alpha_mask_preview(base_img, 128)
            } else if tex.show_biome_tint {
                apply_biome_tint(base_img, tex.biome_preset.rgb_multiplier())
            } else {
                base_img.clone()
            };
            let color_image = ColorImage::from_rgba_unmultiplied(
                [preview_img.width() as usize, preview_img.height() as usize],
                preview_img.as_raw(),
            );
            let handle = ui
                .ctx()
                .load_texture(cache_key.clone(), color_image, TextureOptions::NEAREST);
            if self.texture_cache.len() > 64 {
                self.texture_cache.clear();
            }
            self.texture_cache.insert(cache_key, handle.clone());
            handle
        };

        let avail = ui.available_size();
        let (canvas_rect, resp) =
            ui.allocate_exact_size(avail.max(Vec2::new(64.0, 64.0)), Sense::click_and_drag());
        let painter = ui.painter_at(canvas_rect);
        painter.rect_filled(canvas_rect, 0.0, Color32::from_rgb(0x16, 0x16, 0x16));

        let img_w = base_img.width().max(1) as f32;
        let img_h = base_img.height().max(1) as f32;

        if tex.fit_to_view {
            let pad = 32.0;
            let sx = ((canvas_rect.width() - pad).max(16.0)) / img_w;
            let sy = ((canvas_rect.height() - pad).max(16.0)) / img_h;
            tex.zoom = sx.min(sy).clamp(0.25, 64.0);
        }

        if resp.hovered() {
            let scroll_y = ui.input(|i| {
                if i.modifiers.ctrl || i.modifiers.command {
                    i.raw_scroll_delta.y
                } else {
                    0.0
                }
            });
            if scroll_y.abs() > 0.1 {
                tex.fit_to_view = false;
                let factor = if scroll_y > 0.0 { 1.18 } else { 1.0 / 1.18 };
                tex.zoom = (tex.zoom * factor).clamp(0.25, 64.0);
            }
        }
        let space_held = ui.input(|i| i.key_down(Key::Space));
        if resp.dragged_by(egui::PointerButton::Middle)
            || (space_held && resp.dragged_by(egui::PointerButton::Primary))
        {
            tex.fit_to_view = false;
            let delta = resp.drag_delta();
            tex.pan += GlamVec2::new(delta.x, delta.y);
        }

        let draw_size = Vec2::new(img_w * tex.zoom, img_h * tex.zoom);
        let center = canvas_rect.center() + Vec2::new(tex.pan.x, tex.pan.y);
        let img_rect = Rect::from_center_size(center, draw_size);

        let tile = 10.0_f32;
        let light = Color32::from_rgb(0x3A, 0x3A, 0x3A);
        let dark = Color32::from_rgb(0x28, 0x28, 0x28);
        let cols = (img_rect.width() / tile).ceil().max(1.0) as i32;
        let rows = (img_rect.height() / tile).ceil().max(1.0) as i32;
        for r in 0..rows.min(180) {
            for c in 0..cols.min(180) {
                let x0 = img_rect.left() + c as f32 * tile;
                let y0 = img_rect.top() + r as f32 * tile;
                let cell = Rect::from_min_max(
                    Pos2::new(x0, y0),
                    Pos2::new(
                        (x0 + tile).min(img_rect.right()),
                        (y0 + tile).min(img_rect.bottom()),
                    ),
                );
                painter.rect_filled(cell, 0.0, if (r + c) % 2 == 0 { light } else { dark });
            }
        }

        painter.image(
            tex_handle.id(),
            img_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );

        if tex.show_pixel_grid
            && tex.zoom >= 4.0
            && base_img.width() <= 256
            && base_img.height() <= 256
        {
            let grid_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 36));
            for x in 0..=base_img.width() {
                let px = img_rect.left() + x as f32 * tex.zoom;
                painter.line_segment(
                    [Pos2::new(px, img_rect.top()), Pos2::new(px, img_rect.bottom())],
                    grid_stroke,
                );
            }
            for y in 0..=base_img.height() {
                let py = img_rect.top() + y as f32 * tex.zoom;
                painter.line_segment(
                    [Pos2::new(img_rect.left(), py), Pos2::new(img_rect.right(), py)],
                    grid_stroke,
                );
            }
        }

        painter.rect_stroke(
            img_rect,
            0.0,
            Stroke::new(1.0, theme::BORDER),
            egui::StrokeKind::Outside,
        );
    }

    fn render_code_tab(&mut self, ui: &mut egui::Ui, code: &mut CodeTabState) {
        let lower = code.path.to_ascii_lowercase();
        let is_model_file = lower.ends_with(".geo.json")
            || lower.ends_with(".bbmodel")
            || (lower.contains("models/") && lower.ends_with(".json"));

        Frame::new()
            .fill(theme::BG_SIDEBAR)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .inner_margin(Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(&code.path)
                            .monospace()
                            .small()
                            .color(theme::TEXT_MUTED),
                    );
                    ui.separator();
                    if code.is_json_like && ui.small_button("Format JSON").clicked() {
                        let _ = code.format_json();
                    }
                    if ui.small_button("💾 Save (Ctrl+S)").clicked() {
                        self.actions.push(TabAction::SaveCodeTab {
                            path: code.path.clone(),
                            text: code.text.clone(),
                        });
                    }
                    if is_model_file
                        && ui.small_button("🧊 Open in Blockbench (3D & Paint)").clicked()
                    {
                        self.actions
                            .push(TabAction::OpenInBlockbenchStudio(code.path.clone()));
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some(err) = &code.json_error {
                            ui.label(
                                RichText::new(format!("✖ {err}"))
                                    .small()
                                    .color(theme::SEVERITY_ERROR),
                            );
                        } else if code.is_json_like {
                            ui.label(
                                RichText::new("✓ Valid JSON")
                                    .small()
                                    .color(theme::SEVERITY_OK),
                            );
                        } else if is_text_asset(&code.path) {
                            ui.label(
                                RichText::new("Plain Text / Lang")
                                    .small()
                                    .color(theme::TEXT_MUTED),
                            );
                        }
                    });
                });
            });

        let line_count = code.text.lines().count().max(1);
        let gutter_text: String = (1..=line_count)
            .map(|n| format!("{n:>4}"))
            .collect::<Vec<_>>()
            .join("\n");

        ScrollArea::both()
            .id_salt(format!("code_editor_scroll_{}", code.path))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    Frame::new()
                        .fill(Color32::from_rgb(0x1B, 0x1B, 0x1C))
                        .inner_margin(Margin::symmetric(6, 4))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(gutter_text)
                                    .monospace()
                                    .color(theme::TEXT_MUTED),
                            );
                        });

                    let edit = ui.add(
                        TextEdit::multiline(&mut code.text)
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .lock_focus(true)
                            .desired_rows(line_count.max(24))
                            .desired_width(f32::INFINITY),
                    );
                    if edit.changed() {
                        code.validate_syntax();
                    }
                });
            });
    }

    fn render_conversion_dashboard(&mut self, ui: &mut egui::Ui) {
        Frame::new()
            .fill(theme::BG_EDITOR)
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Conversion & Preflight Dashboard")
                            .size(18.0)
                            .strong()
                            .color(theme::TEXT_BRIGHT),
                    );
                    if let (Some(name), Some(ed)) = (self.loaded_pack_name, self.loaded_pack_edition)
                    {
                        ui.label(
                            RichText::new(format!("— {name} ({ed})")).color(theme::TEXT_MUTED),
                        );
                    }
                });
                ui.separator();

                ui.horizontal_wrapped(|ui| {
                    ui.label("Direction:");
                    ui.selectable_value(
                        &mut self.conversion_config.direction,
                        ConversionDirection::JavaToBedrock,
                        "Java → Bedrock",
                    );
                    ui.selectable_value(
                        &mut self.conversion_config.direction,
                        ConversionDirection::BedrockToJava,
                        "Bedrock → Java",
                    );
                    ui.separator();
                    ui.label("Target Version:");
                    ui.add(
                        TextEdit::singleline(&mut self.conversion_config.target_version)
                            .desired_width(90.0),
                    );
                    ui.separator();
                    ui.label("Output:");
                    ui.add(
                        TextEdit::singleline(&mut self.conversion_config.output_path)
                            .hint_text("Optional .mcpack/.zip path")
                            .desired_width(200.0),
                    );
                    if ui.button("Browse").clicked() {
                        self.actions.push(TabAction::PickConvertOutput);
                    }
                    ui.separator();
                    if ui.button("1. Run Preflight Scan").clicked() {
                        self.actions.push(TabAction::RunPreflight);
                    }
                    if ui
                        .button(RichText::new("2. Execute Convert").strong())
                        .clicked()
                    {
                        self.actions.push(TabAction::ExecuteConvert);
                    }
                });

                ui.add_space(10.0);

                if let Some(conv) = self.last_conversion {
                    let (bg, title) = match conv.gate_status {
                        AuditStatus::Passed => {
                            (theme::BADGE_PASSED_BG, "PASSED (Semantic Audit Gate)")
                        }
                        AuditStatus::Partial => {
                            (theme::BADGE_WARN_BG, "PARTIAL / WARNING (Semantic Audit Gate)")
                        }
                        AuditStatus::Failed => {
                            (theme::BADGE_FAIL_BG, "FAILED (Semantic Audit Gate)")
                        }
                    };
                    Frame::new()
                        .fill(bg)
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.label(RichText::new(title).strong().color(theme::TEXT_BRIGHT));
                            ui.label(RichText::new(&conv.gate_summary).color(theme::TEXT_BRIGHT));
                            if let Some(out_path) = &conv.output_written_to {
                                ui.label(
                                    RichText::new(format!("Output written: {}", out_path.display()))
                                        .small()
                                        .color(theme::TEXT_BRIGHT),
                                );
                            }
                        });
                    ui.add_space(10.0);
                }

                if let Some(pf) = self.preflight_report {
                    ui.horizontal_wrapped(|ui| {
                        let gate_col = match pf.semantic_preview_status {
                            AuditStatus::Passed => theme::SEVERITY_OK,
                            AuditStatus::Partial => theme::SEVERITY_WARNING,
                            AuditStatus::Failed => theme::SEVERITY_ERROR,
                        };
                        ui.label(
                            RichText::new(format!(
                                "Preflight ({} → {}, {} ms) | Ready: {}  |  Lossy: {}  |  Skipped: {}  |  Warnings: {}  |  Gate Preview: {}",
                                pf.source_edition,
                                pf.target_edition,
                                pf.elapsed_ms,
                                pf.ready_count,
                                pf.lossy_count,
                                pf.skipped_count,
                                pf.warning_count,
                                pf.semantic_preview_status
                            ))
                            .strong()
                            .color(gate_col),
                        );
                    });
                    ui.add_space(6.0);

                    TableBuilder::new(ui)
                        .striped(true)
                        .resizable(true)
                        .cell_layout(Layout::left_to_right(Align::Center))
                        .column(Column::exact(76.0))
                        .column(Column::initial(240.0).at_least(120.0))
                        .column(Column::initial(200.0).at_least(120.0))
                        .column(Column::remainder().at_least(180.0))
                        .header(22.0, |mut header| {
                            header.col(|ui| {
                                ui.label(RichText::new("Status").strong().size(11.5));
                            });
                            header.col(|ui| {
                                ui.label(RichText::new("Source Asset").strong().size(11.5));
                            });
                            header.col(|ui| {
                                ui.label(RichText::new("Target Mapping").strong().size(11.5));
                            });
                            header.col(|ui| {
                                ui.label(RichText::new("Detail").strong().size(11.5));
                            });
                        })
                        .body(|mut body| {
                            for item in &pf.rows {
                                body.row(22.0, |mut row| {
                                    let col = match item.status {
                                        PreflightAssetStatus::Ready => theme::SEVERITY_OK,
                                        PreflightAssetStatus::Lossy => theme::SEVERITY_WARNING,
                                        PreflightAssetStatus::Skipped => theme::SEVERITY_ERROR,
                                        PreflightAssetStatus::Warning => theme::SEVERITY_WARNING,
                                    };
                                    row.col(|ui| {
                                        ui.label(
                                            RichText::new(item.status.label())
                                                .strong()
                                                .size(11.0)
                                                .color(col),
                                        );
                                    });
                                    row.col(|ui| {
                                        if ui
                                            .selectable_label(
                                                false,
                                                RichText::new(&item.source_path)
                                                    .monospace()
                                                    .size(11.5),
                                            )
                                            .clicked()
                                        {
                                            self.actions.push(TabAction::OpenAssetPath(
                                                item.source_path.clone(),
                                            ));
                                        }
                                    });
                                    row.col(|ui| {
                                        ui.label(
                                            RichText::new(&item.target_hint)
                                                .monospace()
                                                .size(11.5)
                                                .color(theme::TEXT_MUTED),
                                        );
                                    });
                                    row.col(|ui| {
                                        ui.label(RichText::new(&item.detail).size(11.5));
                                    });
                                });
                            }
                        });
                } else {
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new(
                            "Click '1. Run Preflight Scan' to inspect every texture, model, lossy approximation, and Semantic Audit Gate preview before executing conversion.",
                        )
                        .color(theme::TEXT_MUTED),
                    );
                }
            });
    }
}
