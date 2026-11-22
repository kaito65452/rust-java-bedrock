pub mod bundle;
pub mod server;
pub mod webview;

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use anyhow::Result;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use rp_io::PackContainer;

use self::bundle::{default_cache_dir, ensure_default_bundle_extracted};
use self::server::{
    BlockbenchServer, BridgeSaveEvent, BridgeTexturePayload, StudioToBlockbenchCommand,
};
use self::webview::{WebViewLaunchOutcome, open_in_local_browser, try_open_native_webview};

/// Live runtime host for the embedded/bundled Blockbench Web Studio and its 2-way loopback bridge.
pub struct BlockbenchHost {
    server: Option<BlockbenchServer>,
    save_tx: Sender<BridgeSaveEvent>,
    save_rx: Receiver<BridgeSaveEvent>,
    pub last_url: Option<String>,
    pub fallback_notice: Option<String>,
    pub cache_sha256: Option<String>,
}

impl Default for BlockbenchHost {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockbenchHost {
    /// Creates an idle `BlockbenchHost`; extraction and server startup happen lazily on first use.
    #[must_use]
    pub fn new() -> Self {
        let (save_tx, save_rx) = mpsc::channel();
        Self {
            server: None,
            save_tx,
            save_rx,
            last_url: None,
            fallback_notice: None,
            cache_sha256: None,
        }
    }

    /// Ensures the Blockbench bundle is lazily extracted (0 ms if cached) and the loopback
    /// static + WebSocket server is running. Returns `(launch_url, extracted_now)`.
    pub fn ensure_started(&mut self, project_root: Option<PathBuf>) -> Result<(String, bool)> {
        if let Some(srv) = &self.server {
            srv.set_project_root(project_root);
            let url = srv.launch_url();
            self.last_url = Some(url.clone());
            return Ok((url, false));
        }

        let cache_dir = default_cache_dir();
        let outcome = ensure_default_bundle_extracted(&cache_dir)?;
        self.cache_sha256 = Some(outcome.sha256);

        let srv = BlockbenchServer::start(
            outcome.cache_dir,
            project_root,
            self.save_tx.clone(),
        )?;
        let url = srv.launch_url();
        self.last_url = Some(url.clone());
        self.server = Some(srv);
        Ok((url, outcome.extracted))
    }

    /// Updates the active project directory root whenever the user switches workspaces.
    pub fn sync_project_root(&self, project_root: Option<PathBuf>) {
        if let Some(srv) = &self.server {
            srv.set_project_root(project_root);
        }
    }

    /// Queues an asset (`.geo.json`, Java `.json`, `.bbmodel`, or `.png`) to open directly inside
    /// Blockbench without a browser file picker, and launches the WebView (or browser fallback).
    pub fn open_asset_in_blockbench(
        &mut self,
        project_root: Option<PathBuf>,
        container: &PackContainer,
        rel_path: &str,
    ) -> Result<WebViewLaunchOutcome> {
        let (url, _) = self.ensure_started(project_root)?;
        let lower = rel_path.to_ascii_lowercase();

        let (format_hint, content, textures) = if lower.ends_with(".png") {
            let mut tex_list = Vec::new();
            if let Some(bytes) = container.get(rel_path) {
                tex_list.push(BridgeTexturePayload {
                    path: rel_path.to_string(),
                    name: rel_path.rsplit('/').next().map(ToString::to_string),
                    data_url: Some(format!(
                        "data:image/png;base64,{}",
                        BASE64_STANDARD.encode(bytes)
                    )),
                    png_base64: None,
                });
            }
            (String::from("image"), None, tex_list)
        } else {
            let hint = if lower.ends_with(".bbmodel") {
                "bbmodel"
            } else if lower.ends_with(".geo.json") {
                "bedrock"
            } else {
                "java_block"
            };
            let text = container
                .get(rel_path)
                .map(|b| rp_core::decode_minecraft_text(b).into_owned());
            let textures = collect_associated_textures(container, rel_path);
            (hint.to_string(), text, textures)
        };

        if let Some(srv) = &self.server {
            srv.push_command(StudioToBlockbenchCommand::OpenModel {
                path: rel_path.to_string(),
                format_hint,
                content,
                textures,
            });
        }

        let outcome = try_open_native_webview(&url);
        if let WebViewLaunchOutcome::FallbackToBrowser { reason } = &outcome {
            self.fallback_notice = Some(reason.clone());
        } else {
            self.fallback_notice = None;
        }
        Ok(outcome)
    }

    /// Queues creation of a brand-new Bedrock Geometry (`.geo.json`) inside Blockbench.
    pub fn new_bedrock_geometry_in_blockbench(
        &mut self,
        project_root: Option<PathBuf>,
        rel_path: &str,
        identifier: &str,
    ) -> Result<WebViewLaunchOutcome> {
        let (url, _) = self.ensure_started(project_root)?;
        if let Some(srv) = &self.server {
            srv.push_command(StudioToBlockbenchCommand::NewBedrockGeo {
                path: rel_path.to_string(),
                identifier: identifier.to_string(),
            });
        }
        let outcome = try_open_native_webview(&url);
        if let WebViewLaunchOutcome::FallbackToBrowser { reason } = &outcome {
            self.fallback_notice = Some(reason.clone());
        } else {
            self.fallback_notice = None;
        }
        Ok(outcome)
    }

    /// Opens the live local Blockbench Studio URL in the OS browser (fallback action).
    pub fn open_fallback_browser(&mut self, project_root: Option<PathBuf>) -> Result<String> {
        let (url, _) = self.ensure_started(project_root)?;
        open_in_local_browser(&url)?;
        Ok(url)
    }

    /// Drains any validated file saves received from `studio_bridge.js`.
    #[must_use]
    pub fn poll_save_events(&self) -> Vec<BridgeSaveEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.save_rx.try_recv() {
            out.push(ev);
        }
        out
    }

    /// Returns a concise status badge for the Status Bar.
    #[must_use]
    pub fn status_label(&self) -> String {
        if let Some(srv) = &self.server {
            format!("BB Bridge :{}", srv.port())
        } else if bundle::locate_archive_on_disk().is_some()
            || cfg!(feature = "embed-blockbench")
        {
            String::from("BB Web: Ready")
        } else {
            String::from("BB: External")
        }
    }
}

fn collect_associated_textures(container: &PackContainer, model_path: &str) -> Vec<BridgeTexturePayload> {
    let mut out = Vec::new();
    let stem = model_path
        .rsplit('/')
        .next()
        .unwrap_or(model_path)
        .trim_end_matches(".geo.json")
        .trim_end_matches(".json")
        .to_ascii_lowercase();

    for (path, bytes) in container.files() {
        let lower = path.to_ascii_lowercase();
        if !lower.ends_with(".png") {
            continue;
        }
        let fname = lower.rsplit('/').next().unwrap_or(&lower);
        if fname.starts_with(&stem) {
            out.push(BridgeTexturePayload {
                path: path.clone(),
                name: path.rsplit('/').next().map(ToString::to_string),
                data_url: Some(format!(
                    "data:image/png;base64,{}",
                    BASE64_STANDARD.encode(bytes)
                )),
                png_base64: None,
            });
            if out.len() >= 4 {
                break;
            }
        }
    }
    out
}
