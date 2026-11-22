use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;

use egui::{Color32, Context, RichText, ScrollArea, Vec2};

/// Identifies which UI field initiated a non-blocking file/folder pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePickerTarget {
    WorkspaceDir,
    ConvertInput,
    ConvertOutput,
    DiffPackA,
    DiffPackB,
    ExportOverrides,
}

impl FilePickerTarget {
    /// Whether this target expects a directory rather than a file.
    #[must_use]
    pub const fn picks_directory(self) -> bool {
        matches!(self, Self::WorkspaceDir)
    }

    /// Whether this target is a save-file path.
    #[must_use]
    pub const fn is_save(self) -> bool {
        matches!(self, Self::ConvertOutput | Self::ExportOverrides)
    }

    /// Human-readable modal title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::WorkspaceDir => "Select Workspace Folder",
            Self::ConvertInput => "Select Input Resource Pack",
            Self::ConvertOutput => "Choose Output Resource Pack Path",
            Self::DiffPackA => "Select Pack A for Compare",
            Self::DiffPackB => "Select Pack B for Compare",
            Self::ExportOverrides => "Export Manual Overrides JSON",
        }
    }
}

/// Result delivered asynchronously from a background file picker thread.
#[derive(Debug, Clone)]
pub struct FilePickerResult {
    pub target: FilePickerTarget,
    pub path: Option<PathBuf>,
}

/// Spawns `rfd::FileDialog` on a background OS thread so X11 / Wayland / `egui`
/// repaint never freezes or leaves trail artifacts while the system dialog is open.
pub fn spawn_async_system_dialog(
    target: FilePickerTarget,
    start_dir: PathBuf,
    tx: Sender<FilePickerResult>,
) {
    thread::spawn(move || {
        let mut dlg = rfd::FileDialog::new();
        if start_dir.is_dir() {
            dlg = dlg.set_directory(&start_dir);
        }
        let picked = if target.picks_directory() {
            dlg.pick_folder()
        } else if target.is_save() {
            match target {
                FilePickerTarget::ExportOverrides => dlg
                    .set_file_name("studio_overrides.json")
                    .add_filter("JSON", &["json"])
                    .save_file(),
                _ => dlg
                    .set_file_name("converted-pack.zip")
                    .add_filter("Minecraft Pack", &["zip", "mcpack"])
                    .save_file(),
            }
        } else {
            dlg.add_filter("Minecraft Pack", &["zip", "mcpack"])
                .pick_file()
        };
        let _ = tx.send(FilePickerResult {
            target,
            path: picked,
        });
    });
}

/// Pure-`egui` non-blocking in-app path browser modal.
/// Works 100% natively under Termux-X11 / proot even without `xdg-desktop-portal`,
/// with zero X11 window-freeze or repaint trail artifacts.
#[derive(Debug, Clone)]
pub struct InAppFileBrowser {
    pub open: bool,
    pub target: FilePickerTarget,
    pub current_dir: PathBuf,
    pub path_edit: String,
    pub filename_edit: String,
    pub entries: Vec<(PathBuf, String, bool)>,
    pub filter: String,
    pub error_msg: Option<String>,
}

impl Default for InAppFileBrowser {
    fn default() -> Self {
        Self {
            open: false,
            target: FilePickerTarget::WorkspaceDir,
            current_dir: PathBuf::from("."),
            path_edit: String::from("."),
            filename_edit: String::new(),
            entries: Vec::new(),
            filter: String::new(),
            error_msg: None,
        }
    }
}

impl InAppFileBrowser {
    /// Opens the in-app browser targeting `target` rooted at `start_dir`.
    pub fn open_for(&mut self, target: FilePickerTarget, start_dir: &Path, suggested_name: &str) {
        self.open = true;
        self.target = target;
        let dir = if start_dir.is_dir() {
            start_dir.to_path_buf()
        } else {
            start_dir
                .parent()
                .filter(|p| p.is_dir())
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
        };
        self.filename_edit = suggested_name.to_string();
        self.filter.clear();
        self.navigate(dir);
    }

    fn navigate(&mut self, dir: PathBuf) {
        let canonical = dir.canonicalize().unwrap_or(dir);
        self.current_dir = canonical.clone();
        self.path_edit = canonical.display().to_string();
        self.entries.clear();
        self.error_msg = None;

        match fs::read_dir(&canonical) {
            Ok(rd) => {
                let mut dirs = Vec::new();
                let mut files = Vec::new();
                for entry in rd.flatten() {
                    let p = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_dir() {
                            dirs.push((p, name, true));
                        } else if !self.target.picks_directory() {
                            files.push((p, name, false));
                        }
                    }
                }
                dirs.sort_by(|a, b| a.1.to_ascii_lowercase().cmp(&b.1.to_ascii_lowercase()));
                files.sort_by(|a, b| a.1.to_ascii_lowercase().cmp(&b.1.to_ascii_lowercase()));
                self.entries.extend(dirs);
                self.entries.extend(files);
            }
            Err(err) => {
                self.error_msg = Some(format!("Cannot read folder: {err}"));
            }
        }
    }

    /// Renders the modal if open; returns `Some((target, chosen_path))` when the user confirms.
    pub fn show(
        &mut self,
        ctx: &Context,
        recent_paths: &[PathBuf],
        async_tx: &Sender<FilePickerResult>,
    ) -> Option<(FilePickerTarget, PathBuf)> {
        if !self.open {
            return None;
        }
        let mut chosen: Option<(FilePickerTarget, PathBuf)> = None;
        let mut nav_to: Option<PathBuf> = None;
        let mut should_close = false;

        egui::Window::new(self.target.title())
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(620.0, 440.0))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                // Path bar + Up + Go + Async System Dialog fallback
                ui.horizontal(|ui| {
                    if ui.button("⬆ Up").clicked()
                        && let Some(parent) = self.current_dir.parent()
                    {
                        nav_to = Some(parent.to_path_buf());
                    }
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.path_edit)
                            .desired_width(ui.available_width() - 165.0),
                    );
                    if (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        || ui.button("Go").clicked()
                    {
                        let p = PathBuf::from(self.path_edit.trim());
                        if p.is_dir() {
                            nav_to = Some(p);
                        } else if p.is_file() && !self.target.picks_directory() {
                            chosen = Some((self.target, p));
                            should_close = true;
                        } else {
                            self.error_msg = Some(format!("Path not found: {}", self.path_edit));
                        }
                    }
                    if ui
                        .button("System Dialog")
                        .on_hover_text("Open OS picker on a background thread (non-blocking)")
                        .clicked()
                    {
                        spawn_async_system_dialog(
                            self.target,
                            self.current_dir.clone(),
                            async_tx.clone(),
                        );
                        should_close = true;
                    }
                });

                // Quick recent paths strip
                if !recent_paths.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Recent:").small().weak());
                        for rp in recent_paths.iter().take(5) {
                            let short = rp
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or_else(|| rp.to_str().unwrap_or("."));
                            if ui
                                .small_button(short)
                                .on_hover_text(rp.display().to_string())
                                .clicked()
                            {
                                if rp.is_dir() {
                                    nav_to = Some(rp.clone());
                                } else if !self.target.picks_directory() {
                                    chosen = Some((self.target, rp.clone()));
                                    should_close = true;
                                }
                            }
                        }
                    });
                }

                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    ui.add(egui::TextEdit::singleline(&mut self.filter).desired_width(220.0));
                });

                if let Some(err) = &self.error_msg {
                    ui.colored_label(Color32::LIGHT_RED, err);
                }
                ui.separator();

                let needle = self.filter.trim().to_ascii_lowercase();
                ScrollArea::vertical()
                    .max_height(250.0)
                    .id_salt("in_app_browser_entries")
                    .show(ui, |ui| {
                        for (path, name, is_dir) in &self.entries {
                            if !needle.is_empty() && !name.to_ascii_lowercase().contains(&needle) {
                                continue;
                            }
                            let is_pack_dir = *is_dir
                                && (path.join("pack.mcmeta").is_file()
                                    || path.join("manifest.json").is_file());
                            let label = if is_pack_dir {
                                format!("📦 {name}/ (Resource Pack)")
                            } else if *is_dir {
                                format!("📁 {name}/")
                            } else {
                                format!("📄 {name}")
                            };
                            ui.horizontal(|ui| {
                                let resp = ui.selectable_label(false, label);
                                if resp.clicked() {
                                    if *is_dir {
                                        nav_to = Some(path.clone());
                                    } else if self.target.is_save() {
                                        self.filename_edit = name.clone();
                                    } else {
                                        chosen = Some((self.target, path.clone()));
                                        should_close = true;
                                    }
                                }
                                if is_pack_dir
                                    && !self.target.picks_directory()
                                    && !self.target.is_save()
                                    && ui.small_button("Use Folder").clicked()
                                {
                                    chosen = Some((self.target, path.clone()));
                                    should_close = true;
                                }
                            });
                        }
                    });

                ui.separator();
                if self.target.is_save() {
                    ui.horizontal(|ui| {
                        ui.label("File name:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.filename_edit)
                                .desired_width(ui.available_width() - 90.0),
                        );
                    });
                }

                ui.horizontal(|ui| {
                    if self.target.picks_directory() && ui.button("Select This Folder").clicked() {
                        chosen = Some((self.target, self.current_dir.clone()));
                        should_close = true;
                    }
                    if self.target.is_save()
                        && !self.filename_edit.trim().is_empty()
                        && ui.button("Save Here").clicked()
                    {
                        chosen = Some((
                            self.target,
                            self.current_dir.join(self.filename_edit.trim()),
                        ));
                        should_close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        should_close = true;
                    }
                });
            });

        if let Some(d) = nav_to {
            self.navigate(d);
        }
        if should_close {
            self.open = false;
        }
        chosen
    }
}
