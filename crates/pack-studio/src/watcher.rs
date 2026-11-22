use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use anyhow::{Context, Result};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Non-blocking filesystem hot-reload watcher backed by `notify`.
pub struct PackWatcher {
    _watcher: RecommendedWatcher,
    rx: Receiver<Vec<PathBuf>>,
    root: PathBuf,
}

impl PackWatcher {
    /// Starts recursively watching `root` directory for file modifications and creations.
    pub fn watch_directory(root: &Path) -> Result<Self> {
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let (tx, rx) = mpsc::channel::<Vec<PathBuf>>();

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                let is_relevant = matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                );
                if is_relevant && !event.paths.is_empty() {
                    let _ = tx.send(event.paths);
                }
            }
        })
        .context("failed to initialize filesystem watcher")?;

        watcher
            .watch(&canonical_root, RecursiveMode::Recursive)
            .with_context(|| {
                format!(
                    "failed to watch pack directory '{}'",
                    canonical_root.display()
                )
            })?;

        Ok(Self {
            _watcher: watcher,
            rx,
            root: canonical_root,
        })
    }

    /// Returns the watched root directory path.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Drains pending filesystem change notifications and returns deduplicated forward-slash
    /// archive paths relative to the watched pack root.
    #[must_use]
    pub fn poll_changed_relative_paths(&self) -> Vec<String> {
        let mut changed = BTreeSet::new();
        while let Ok(paths) = self.rx.try_recv() {
            for p in paths {
                let rel_buf = if let Ok(r) = p.strip_prefix(&self.root) {
                    Some(r.to_path_buf())
                } else if let Ok(c) = p.canonicalize() {
                    c.strip_prefix(&self.root).ok().map(Path::to_path_buf)
                } else {
                    None
                };

                if let Some(rel_path) = rel_buf {
                    let norm = rel_path.to_string_lossy().replace('\\', "/");
                    if !norm.is_empty() && !rp_io::is_os_junk(&norm) {
                        changed.insert(norm);
                    }
                }
            }
        }
        changed.into_iter().collect()
    }
}
