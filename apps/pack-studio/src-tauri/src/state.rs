use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rp_core::{Edition, ResolutionTier};
use rp_io::PackContainer;
use serde::{Deserialize, Serialize};

/// Explicit 3-mode pack classification for PackStudio workspaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackType {
    /// Java Edition Resource Pack (`pack.mcmeta` + `assets/`).
    JavaResourcePack,
    /// Single Bedrock Edition Resource Pack (`manifest.json` with `"resources"` module at root).
    BedrockResourcePack,
    /// Bedrock Edition Add-on workspace (`BP/` + `RP/`, optional `pstdr.toml` + `src/*.pstdr`).
    BedrockAddon,
}

use crate::blockbench::BlockbenchHost;
use crate::extensions::{BUILTIN_MINECRAFT_EXT_ID, default_extensions_dir, load_persisted_state};
use crate::lsp::LspClientHandle;
use crate::pty::PtySession;

/// Hierarchical tree node sent to the VS Code-style Explorer frontend.
#[derive(Debug, Clone, Serialize)]
pub struct FileTreeNodeDto {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub children: Vec<FileTreeNodeDto>,
}

/// Snapshot of the currently active resource pack workspace returned by workspace commands.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectSnapshot {
    pub pack_name: String,
    pub source_path: Option<String>,
    pub is_directory: bool,
    pub companion_bp_dir: Option<String>,
    pub edition: Edition,
    pub pack_type: PackType,
    pub edition_label: String,
    pub file_count: usize,
    pub texture_count: usize,
    pub dominant_resolution: Option<String>,
    pub all_files: Vec<String>,
    pub tree: Vec<FileTreeNodeDto>,
}

/// Active resource pack workspace held in Tauri managed state.
pub struct ActivePackState {
    pub source_path: Option<PathBuf>,
    pub is_directory: bool,
    pub companion_bp_dir: Option<PathBuf>,
    pub container: PackContainer,
    pub edition: Edition,
    pub pack_type: PackType,
    pub pack_name: String,
    pub edition_label: String,
    pub extra_folders: BTreeSet<String>,
}

/// Holds the active `notify` filesystem watcher for directory-backed packs.
pub struct PackWatcherHandle {
    pub _watcher: RecommendedWatcher,
    pub root: PathBuf,
}

impl PackWatcherHandle {
    /// Starts a recursive filesystem watcher that emits `fs://changed` events to the frontend.
    pub fn start(
        root: PathBuf,
        on_change: impl Fn(Vec<String>) + Send + 'static,
    ) -> anyhow::Result<Self> {
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.clone());
        let root_for_cb = canonical_root.clone();

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                let relevant = matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                );
                if relevant && !event.paths.is_empty() {
                    let mut rels = Vec::new();
                    for p in event.paths {
                        let rel_opt = p
                            .strip_prefix(&root_for_cb)
                            .ok()
                            .map(std::path::Path::to_path_buf)
                            .or_else(|| {
                                p.canonicalize().ok().and_then(|c| {
                                    c.strip_prefix(&root_for_cb)
                                        .ok()
                                        .map(std::path::Path::to_path_buf)
                                })
                            });
                        if let Some(rel) = rel_opt {
                            let norm = rel.to_string_lossy().replace('\\', "/");
                            if !norm.is_empty() && !rp_io::is_os_junk(&norm) {
                                rels.push(norm);
                            }
                        }
                    }
                    if !rels.is_empty() {
                        on_change(rels);
                    }
                }
            }
        })?;

        watcher.watch(&canonical_root, RecursiveMode::Recursive)?;
        Ok(Self {
            _watcher: watcher,
            root: canonical_root,
        })
    }
}

/// Global thread-safe Tauri state shared across all commands.
pub struct StudioState {
    pub active_pack: Arc<Mutex<Option<ActivePackState>>>,
    pub watcher: Mutex<Option<PackWatcherHandle>>,
    pub blockbench: Mutex<Option<BlockbenchHost>>,
    pub pty_sessions: Mutex<HashMap<u32, PtySession>>,
    pub next_pty_id: AtomicU32,
    pub lsp_clients: Arc<Mutex<HashMap<String, LspClientHandle>>>,
    pub builtin_lsp_enabled: Arc<AtomicBool>,
}

impl Default for StudioState {
    fn default() -> Self {
        let persisted = load_persisted_state(&default_extensions_dir());
        let builtin_on = !persisted.disabled_ids.contains(BUILTIN_MINECRAFT_EXT_ID);
        Self {
            active_pack: Arc::new(Mutex::new(None)),
            watcher: Mutex::new(None),
            blockbench: Mutex::new(None),
            pty_sessions: Mutex::new(HashMap::new()),
            next_pty_id: AtomicU32::new(1),
            lsp_clients: Arc::new(Mutex::new(HashMap::new())),
            builtin_lsp_enabled: Arc::new(AtomicBool::new(builtin_on)),
        }
    }
}

impl ActivePackState {
    /// Writes/updates a file inside the in-memory container and flushes it to disk if backed by a directory.
    pub fn write_asset(&mut self, rel_path: &str, bytes: Vec<u8>) -> anyhow::Result<()> {
        let clean = rp_io::sanitize_archive_path(rel_path)?;
        if self.is_directory
            && let Some(root) = &self.source_path
        {
            let full = root.join(&clean);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&full, &bytes)?;
        }
        self.container.insert(&clean, bytes)?;
        Ok(())
    }

    /// Rebuilds the hierarchical directory tree and pack stats snapshot for the UI.
    #[must_use]
    pub fn to_snapshot(&self, dominant_res: Option<ResolutionTier>, texture_count: usize) -> ProjectSnapshot {
        let mut tree_builder = TempTreeNode::default();
        for folder in &self.extra_folders {
            tree_builder.insert_folder(folder);
        }
        for path in self.container.files().keys() {
            tree_builder.insert_file(path);
        }

        ProjectSnapshot {
            pack_name: self.pack_name.clone(),
            source_path: self.source_path.as_ref().map(|p| p.display().to_string()),
            is_directory: self.is_directory,
            companion_bp_dir: self
                .companion_bp_dir
                .as_ref()
                .map(|p| p.display().to_string()),
            edition: self.edition,
            pack_type: self.pack_type,
            edition_label: self.edition_label.clone(),
            file_count: self.container.len(),
            texture_count,
            dominant_resolution: dominant_res.map(|r| r.to_string()),
            all_files: self.container.files().keys().cloned().collect(),
            tree: tree_builder.into_dtos(),
        }
    }
}

#[derive(Default)]
struct TempTreeNode {
    dirs: BTreeMap<String, (String, TempTreeNode)>,
    files: Vec<(String, String)>,
}

impl TempTreeNode {
    fn insert_folder(&mut self, rel_folder: &str) {
        let mut cur = self;
        let mut acc = String::new();
        for seg in rel_folder.split('/').filter(|s| !s.is_empty()) {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            let (_, next) = cur
                .dirs
                .entry(seg.to_string())
                .or_insert_with(|| (acc.clone(), TempTreeNode::default()));
            cur = next;
        }
    }

    fn insert_file(&mut self, rel_path: &str) {
        let mut parts: Vec<&str> = rel_path.split('/').filter(|s| !s.is_empty()).collect();
        let Some(fname) = parts.pop() else {
            return;
        };
        let mut cur = self;
        let mut acc = String::new();
        for seg in parts {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            let (_, next) = cur
                .dirs
                .entry(seg.to_string())
                .or_insert_with(|| (acc.clone(), TempTreeNode::default()));
            cur = next;
        }
        cur.files.push((fname.to_string(), rel_path.to_string()));
    }

    fn into_dtos(self) -> Vec<FileTreeNodeDto> {
        let mut out = Vec::with_capacity(self.dirs.len() + self.files.len());
        for (name, (rel_path, child)) in self.dirs {
            out.push(FileTreeNodeDto {
                name,
                rel_path,
                is_dir: true,
                children: child.into_dtos(),
            });
        }
        for (name, rel_path) in self.files {
            out.push(FileTreeNodeDto {
                name,
                rel_path,
                is_dir: false,
                children: Vec::new(),
            });
        }
        out
    }
}
