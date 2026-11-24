use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use rp_io::PackContainer;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::state::{ActivePackState, StudioState};

#[cfg(feature = "embed-blockbench")]
const EMBEDDED_BLOCKBENCH_ARCHIVE: &[u8] =
    include_bytes!("../../../../assets/vendor/blockbench-web.tar.zst");

/// Always-current PackStudio <-> Blockbench bridge served directly from `BlockbenchServer`
/// so stale cached `blockbench-web.tar.zst` extractions never run outdated bridge code.
const EMBEDDED_STUDIO_BRIDGE_JS: &str = include_str!("../../../../scripts/studio_bridge.js");

/// Injected into Blockbench's `index.html` just before `</head>` to apply compact UI overrides
/// and wire up the `ps_set_zoom` / `ps_set_compact` postMessage API.
const BLOCKBENCH_COMPACT_INJECTION: &str = r#"<style>
/* PackStudio Compact Blockbench Overrides */
html, body { zoom: 0.84 !important; }
#work_screen { --toolbar-height: 32px !important; grid-template-columns: 280px auto 260px !important; }
#tab_bar { height: 28px !important; min-height: 28px !important; }
.toolbar { height: 28px !important; min-height: 28px !important; }
:root { --status-bar-height: 20px !important; }
/* Thinner scrollbars */
* { scrollbar-width: thin !important; }
::-webkit-scrollbar { width: 6px !important; height: 6px !important; }
</style>
<script>
(function () {
    window.addEventListener('message', function (event) {
        var data = event.data;
        if (!data || typeof data !== 'object') return;
        if (data.type === 'ps_set_zoom') {
            document.documentElement.style.zoom = data.zoom;
        } else if (data.type === 'ps_set_compact') {
            if (data.compact) {
                document.body.classList.add('ps-compact');
            } else {
                document.body.classList.remove('ps-compact');
            }
        }
    });
})();
</script>"#;

/// Result returned by Blockbench archive extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractOutcome {
    pub cache_dir: PathBuf,
    pub sha256: String,
    pub extracted: bool,
}

/// Info returned to the frontend so it can embed Blockbench in a tab via loopback URL.
#[derive(Debug, Clone, Serialize)]
pub struct BlockbenchLaunchInfo {
    pub url: String,
    pub port: u16,
    pub sha256: String,
    pub extracted_now: bool,
}

/// Event emitted to frontend when Blockbench saves/syncs files back to the project.
#[derive(Debug, Clone, Serialize)]
pub struct BlockbenchSavedEvent {
    pub saved_files: Vec<String>,
}

/// Texture attachment pushed Studio <-> Blockbench.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeTexturePayload {
    pub path: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub data_url: Option<String>,
    #[serde(default)]
    pub png_base64: Option<String>,
}

/// Command sent from PackStudio -> `studio_bridge.js`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StudioToBlockbenchCommand {
    OpenModel {
        path: String,
        format_hint: String,
        content: Option<String>,
        textures: Vec<BridgeTexturePayload>,
    },
    NewBedrockGeo {
        path: String,
        identifier: String,
    },
}

/// Payload POSTed by `studio_bridge.js` when the artist saves or clicks Sync.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BridgeSavePayload {
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub model_content: Option<String>,
    #[serde(default)]
    pub textures: Vec<BridgeTexturePayload>,
}

/// Returns the default local cache directory for Blockbench (`<data_local_dir>/pack-studio/blockbench-web`).
#[must_use]
pub fn default_cache_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("pack-studio")
        .join("blockbench-web")
}

/// Locates `blockbench-web.tar.zst` on disk (next to binary or workspace `assets/vendor/`).
#[must_use]
pub fn locate_archive_on_disk() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("blockbench-web.tar.zst"));
        candidates.push(dir.join("assets").join("vendor").join("blockbench-web.tar.zst"));
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join("assets").join("vendor").join("blockbench-web.tar.zst"));
        }
    }
    candidates.push(PathBuf::from("assets/vendor/blockbench-web.tar.zst"));
    candidates.push(PathBuf::from("../assets/vendor/blockbench-web.tar.zst"));
    candidates.push(PathBuf::from("../../assets/vendor/blockbench-web.tar.zst"));
    candidates.push(PathBuf::from("../../../assets/vendor/blockbench-web.tar.zst"));

    candidates.into_iter().find(|p| p.is_file())
}

/// Extracts default Blockbench web bundle into `cache_dir`.
pub fn ensure_default_bundle_extracted(cache_dir: &Path) -> Result<ExtractOutcome> {
    #[cfg(feature = "embed-blockbench")]
    {
        ensure_extracted_from_bytes(EMBEDDED_BLOCKBENCH_ARCHIVE, cache_dir)
    }
    #[cfg(not(feature = "embed-blockbench"))]
    {
        let Some(archive_path) = locate_archive_on_disk() else {
            bail!(
                "Blockbench web bundle not found (expected assets/vendor/blockbench-web.tar.zst). Run scripts/build_blockbench.sh first."
            );
        };
        ensure_extracted_from_path(&archive_path, cache_dir)
    }
}

/// Computes SHA-256 hex digest.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{b:02x}");
    }
    hex
}

/// Extracts `.tar.zst` archive from disk into `cache_dir`, skipping if `.version` matches.
pub fn ensure_extracted_from_path(archive_path: &Path, cache_dir: &Path) -> Result<ExtractOutcome> {
    let bytes = fs::read(archive_path).with_context(|| {
        format!(
            "failed to read Blockbench archive '{}'",
            archive_path.display()
        )
    })?;
    ensure_extracted_from_bytes(&bytes, cache_dir)
}

/// Lazily extracts `.tar.zst` bytes into `cache_dir` with `.version` cache check & Tar-Slip guard.
pub fn ensure_extracted_from_bytes(
    archive_bytes: &[u8],
    cache_dir: &Path,
) -> Result<ExtractOutcome> {
    let digest = sha256_hex(archive_bytes);
    let version_file = cache_dir.join(".version");
    let index_html = cache_dir.join("index.html");

    if version_file.is_file()
        && index_html.is_file()
        && let Ok(existing) = fs::read_to_string(&version_file)
        && existing.trim() == digest
    {
        return Ok(ExtractOutcome {
            cache_dir: cache_dir.to_path_buf(),
            sha256: digest,
            extracted: false,
        });
    }

    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)?;
    }
    fs::create_dir_all(cache_dir)?;

    extract_tar_zst_stream(Cursor::new(archive_bytes), cache_dir)?;
    fs::write(&version_file, format!("{digest}\n"))?;

    Ok(ExtractOutcome {
        cache_dir: cache_dir.to_path_buf(),
        sha256: digest,
        extracted: true,
    })
}

fn extract_tar_zst_stream<R: Read>(reader: R, dest_root: &Path) -> Result<()> {
    let decoder = zstd::stream::read::Decoder::new(reader)?;
    let mut archive = tar::Archive::new(decoder);

    for entry_res in archive.entries()? {
        let mut entry = entry_res?;
        let rel_path = entry.path()?.into_owned();
        let sanitized = sanitize_tar_rel_path(&rel_path)?;
        if sanitized.as_os_str().is_empty() {
            continue;
        }
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            bail!("refusing symlink/hardlink in Blockbench archive");
        }
        let target = dest_root.join(&sanitized);
        if entry_type.is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry_type.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out = fs::File::create(&target)?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }
    Ok(())
}

fn sanitize_tar_rel_path(raw: &Path) -> Result<PathBuf> {
    let mut clean = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(seg) => clean.push(seg),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("unsafe path traversal in tar archive: '{}'", raw.display());
            }
        }
    }
    Ok(clean)
}

/// Validates and normalizes a pack-relative path from the Blockbench bridge, strictly rejecting
/// any path traversal (`..`), absolute paths, drive prefixes, or NUL bytes.
pub fn validate_bridge_relative_path(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("bridge save path must not be empty");
    }
    if trimmed.contains('\0') {
        bail!("bridge save path contains NUL byte");
    }
    let normalized = trimmed.replace('\\', "/");
    if normalized.starts_with('/') || normalized.starts_with('~') {
        bail!("absolute or home-prefixed path rejected by traversal guard: '{raw}'");
    }
    if normalized.len() >= 2 && normalized.as_bytes()[1] == b':' {
        bail!("drive-prefixed path rejected by traversal guard: '{raw}'");
    }

    let mut clean_segments = Vec::new();
    for seg in normalized.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            bail!("path traversal ('..') rejected by traversal guard: '{raw}'");
        }
        clean_segments.push(seg);
    }

    if clean_segments.is_empty() {
        bail!("bridge save path resolves to empty path: '{raw}'");
    }

    let joined = clean_segments.join("/");
    for comp in Path::new(&joined).components() {
        if !matches!(comp, Component::Normal(_)) {
            bail!("unsafe path component rejected by traversal guard: '{raw}'");
        }
    }

    let verified = rp_io::sanitize_archive_path(&joined)?;
    Ok(verified)
}

/// Validates and writes a `BridgeSavePayload` into `project_root` (if `Some`), returning all
/// validated `(relative_path, raw_bytes)` pairs.
pub fn apply_bridge_save_payload(
    project_root: Option<&Path>,
    payload: &BridgeSavePayload,
) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files_to_write: Vec<(String, Vec<u8>)> = Vec::new();

    if let (Some(path), Some(content)) = (&payload.model_path, &payload.model_content) {
        let clean = validate_bridge_relative_path(path)?;
        files_to_write.push((clean, content.as_bytes().to_vec()));
    }

    for tex in &payload.textures {
        let clean = validate_bridge_relative_path(&tex.path)?;
        let raw_b64 = tex
            .png_base64
            .as_deref()
            .or_else(|| {
                tex.data_url
                    .as_deref()
                    .and_then(|u| u.split_once(',').map(|(_, b)| b))
            })
            .context("texture save payload missing png_base64/data_url")?;
        let png_bytes = BASE64_STANDARD
            .decode(raw_b64.trim())
            .with_context(|| format!("invalid base64 PNG payload for '{clean}'"))?;
        files_to_write.push((clean, png_bytes));
    }

    if files_to_write.is_empty() {
        bail!("bridge save payload contained neither model_content nor textures");
    }

    if let Some(root) = project_root {
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        for (rel, bytes) in &files_to_write {
            let target = canonical_root.join(rel);
            if !target.starts_with(&canonical_root) {
                bail!("resolved target escapes project root: '{rel}'");
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
                let canon_parent = parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf());
                if !canon_parent.starts_with(&canonical_root) {
                    bail!("symlink target escapes project root: '{rel}'");
                }
            }
            fs::write(&target, bytes)?;
        }
    }

    Ok(files_to_write)
}

type SaveCallback = Arc<dyn Fn(Vec<(String, Vec<u8>)>) + Send + Sync>;

struct ServerShared {
    web_root: PathBuf,
    token: String,
    project_root: Mutex<Option<PathBuf>>,
    pending_commands: Mutex<VecDeque<StudioToBlockbenchCommand>>,
    last_open_model: Mutex<Option<StudioToBlockbenchCommand>>,
    on_save: SaveCallback,
    shutdown: AtomicBool,
}

/// Loopback-only (`127.0.0.1:0`) static + WebSocket/HTTP server for the Blockbench web bundle.
pub struct BlockbenchServer {
    port: u16,
    token: String,
    shared: Arc<ServerShared>,
}

impl BlockbenchServer {
    /// Starts the local loopback server.
    pub fn start(
        web_root: PathBuf,
        initial_project_root: Option<PathBuf>,
        on_save: SaveCallback,
    ) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .context("failed to bind Blockbench loopback server")?;
        let port = listener.local_addr()?.port();
        let token = Uuid::new_v4().simple().to_string();

        let shared = Arc::new(ServerShared {
            web_root,
            token: token.clone(),
            project_root: Mutex::new(initial_project_root),
            pending_commands: Mutex::new(VecDeque::new()),
            last_open_model: Mutex::new(None),
            on_save,
            shutdown: AtomicBool::new(false),
        });

        let accept_shared = Arc::clone(&shared);
        thread::spawn(move || {
            for stream_res in listener.incoming() {
                if accept_shared.shutdown.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(stream) = stream_res {
                    let conn_shared = Arc::clone(&accept_shared);
                    thread::spawn(move || {
                        let _ = handle_connection(stream, &conn_shared);
                    });
                }
            }
        });

        Ok(Self {
            port,
            token,
            shared,
        })
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    #[must_use]
    pub fn launch_url(&self) -> String {
        format!(
            "http://127.0.0.1:{}/index.html?studio_port={}&token={}",
            self.port, self.port, self.token
        )
    }

    pub fn set_project_root(&self, root: Option<PathBuf>) {
        if let Ok(mut guard) = self.shared.project_root.lock() {
            *guard = root;
        }
    }

    pub fn push_command(&self, cmd: StudioToBlockbenchCommand) {
        if matches!(cmd, StudioToBlockbenchCommand::OpenModel { .. })
            && let Ok(mut last) = self.shared.last_open_model.lock()
        {
            *last = Some(cmd.clone());
        }
        if let Ok(mut guard) = self.shared.pending_commands.lock() {
            guard.push_back(cmd);
        }
    }
}

impl Drop for BlockbenchServer {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// Host handle stored inside `StudioState`.
pub struct BlockbenchHost {
    pub server: BlockbenchServer,
    pub sha256: String,
    pub extracted_now: bool,
}

impl BlockbenchHost {
    /// Lazily extracts the bundle (0 ms if `.version` matches) and boots the loopback bridge server.
    pub fn start_with_app(
        app: AppHandle,
        active_pack: Arc<Mutex<Option<ActivePackState>>>,
        initial_root: Option<PathBuf>,
    ) -> Result<Self> {
        let cache_dir = default_cache_dir();
        let outcome = ensure_default_bundle_extracted(&cache_dir)?;

        let on_save: SaveCallback = Arc::new(move |saved_files| {
            let mut saved_names = Vec::with_capacity(saved_files.len());
            if let Ok(mut guard) = active_pack.lock()
                && let Some(pack) = guard.as_mut()
            {
                for (rel, bytes) in &saved_files {
                    let _ = pack.container.insert(rel, bytes.clone());
                    saved_names.push(rel.clone());
                }
            } else {
                for (rel, _) in &saved_files {
                    saved_names.push(rel.clone());
                }
            }
            let _ = app.emit(
                "blockbench://saved",
                BlockbenchSavedEvent {
                    saved_files: saved_names,
                },
            );
        });

        let server = BlockbenchServer::start(outcome.cache_dir, initial_root, on_save)?;
        Ok(Self {
            server,
            sha256: outcome.sha256,
            extracted_now: outcome.extracted,
        })
    }
}

/// Tauri Command: Ensure the local Blockbench web bundle + 2-way bridge server is running.
#[tauri::command]
pub fn blockbench_start(
    app: AppHandle,
    state: tauri::State<'_, StudioState>,
) -> Result<BlockbenchLaunchInfo, String> {
    let project_root = state
        .active_pack
        .lock()
        .ok()
        .and_then(|g| {
            g.as_ref()
                .filter(|p| p.is_directory)
                .and_then(|p| p.source_path.clone())
        });

    let mut guard = state
        .blockbench
        .lock()
        .map_err(|_| String::from("blockbench mutex poisoned"))?;

    if let Some(host) = guard.as_ref() {
        host.server.set_project_root(project_root);
        return Ok(BlockbenchLaunchInfo {
            url: host.server.launch_url(),
            port: host.server.port(),
            sha256: host.sha256.clone(),
            extracted_now: false,
        });
    }

    let host = BlockbenchHost::start_with_app(
        app,
        Arc::clone(&state.active_pack),
        project_root,
    )
    .map_err(|e| format!("{e:#}"))?;

    let info = BlockbenchLaunchInfo {
        url: host.server.launch_url(),
        port: host.server.port(),
        sha256: host.sha256.clone(),
        extracted_now: host.extracted_now,
    };
    *guard = Some(host);
    Ok(info)
}

/// Tauri Command: Open a pack asset (`.geo.json`, `.json`, `.bbmodel`, `.png`) inside Blockbench.
#[tauri::command]
pub fn blockbench_open_asset(
    app: AppHandle,
    state: tauri::State<'_, StudioState>,
    rel_path: String,
) -> Result<BlockbenchLaunchInfo, String> {
    let info = blockbench_start(app, state.clone())?;

    let cmd = {
        let guard = state
            .active_pack
            .lock()
            .map_err(|_| String::from("active pack mutex poisoned"))?;
        let pack = guard
            .as_ref()
            .ok_or_else(|| String::from("No resource pack currently open"))?;

        let lower = rel_path.to_ascii_lowercase();
        if lower.ends_with(".png") {
            let mut tex_list = Vec::new();
            if let Some(bytes) = pack.container.get(&rel_path) {
                tex_list.push(BridgeTexturePayload {
                    path: rel_path.clone(),
                    name: rel_path.rsplit('/').next().map(ToString::to_string),
                    data_url: Some(format!(
                        "data:image/png;base64,{}",
                        BASE64_STANDARD.encode(bytes)
                    )),
                    png_base64: None,
                });
            }
            StudioToBlockbenchCommand::OpenModel {
                path: rel_path,
                format_hint: String::from("image"),
                content: None,
                textures: tex_list,
            }
        } else {
            let hint = if lower.ends_with(".bbmodel") {
                "bbmodel"
            } else if lower.ends_with(".geo.json") {
                "bedrock"
            } else {
                "java_block"
            };
            let content = pack
                .container
                .get(&rel_path)
                .map(|b| rp_core::decode_minecraft_text(b).into_owned());
            let textures = collect_associated_textures(&pack.container, &rel_path);
            StudioToBlockbenchCommand::OpenModel {
                path: rel_path,
                format_hint: String::from(hint),
                content,
                textures,
            }
        }
    };

    if let Ok(guard) = state.blockbench.lock()
        && let Some(host) = guard.as_ref()
    {
        host.server.push_command(cmd);
    }
    Ok(info)
}

/// Tauri Command: Create a brand-new Bedrock `.geo.json` model directly inside Blockbench.
#[tauri::command]
pub fn blockbench_new_bedrock_geo(
    app: AppHandle,
    state: tauri::State<'_, StudioState>,
    rel_path: String,
    identifier: String,
) -> Result<BlockbenchLaunchInfo, String> {
    let info = blockbench_start(app, state.clone())?;
    let clean_path = if rel_path.trim().is_empty() {
        String::from("models/entity/custom_model.geo.json")
    } else {
        rel_path
    };
    let clean_id = if identifier.trim().is_empty() {
        String::from("geometry.custom_model")
    } else {
        identifier
    };

    if let Ok(guard) = state.blockbench.lock()
        && let Some(host) = guard.as_ref()
    {
        host.server
            .push_command(StudioToBlockbenchCommand::NewBedrockGeo {
                path: clean_path,
                identifier: clean_id,
            });
    }
    Ok(info)
}

fn collect_associated_textures(
    container: &PackContainer,
    model_path: &str,
) -> Vec<BridgeTexturePayload> {
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

fn handle_connection(mut stream: TcpStream, shared: &ServerShared) -> Result<()> {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(15)));
    let mut reader = BufReader::new(stream.try_clone()?);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }
    let method = parts[0];
    let raw_target = parts[1];

    let mut content_length: usize = 0;
    let mut ws_key: Option<String> = None;
    let mut is_ws_upgrade = false;

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            let kl = k.trim().to_ascii_lowercase();
            let val = v.trim();
            if kl == "content-length" {
                content_length = val.parse().unwrap_or(0);
            } else if kl == "sec-websocket-key" {
                ws_key = Some(val.to_string());
            } else if kl == "upgrade" && val.eq_ignore_ascii_case("websocket") {
                is_ws_upgrade = true;
            }
        }
    }

    let (path_only, query) = raw_target
        .split_once('?')
        .map_or((raw_target, ""), |(p, q)| (p, q));

    if method == "OPTIONS" {
        write_http_response(&mut stream, 204, "text/plain", b"")?;
        return Ok(());
    }

    if is_ws_upgrade && path_only == "/ws" {
        if !query_has_valid_token(query, &shared.token) {
            write_http_response(
                &mut stream,
                403,
                "application/json",
                br#"{"error":"invalid token"}"#,
            )?;
            return Ok(());
        }
        if let Some(key) = ws_key {
            handle_websocket_session(stream, &key, shared)?;
        }
        return Ok(());
    }

    if method == "GET" && path_only == "/api/commands" {
        if !query_has_valid_token(query, &shared.token) {
            write_http_response(
                &mut stream,
                403,
                "application/json",
                br#"{"error":"invalid token"}"#,
            )?;
            return Ok(());
        }
        let cmds: Vec<StudioToBlockbenchCommand> = shared
            .pending_commands
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default();
        let body = serde_json::to_vec(&cmds)?;
        write_http_response(&mut stream, 200, "application/json", &body)?;
        return Ok(());
    }

    if method == "POST" && path_only == "/api/save" {
        if !query_has_valid_token(query, &shared.token) {
            write_http_response(
                &mut stream,
                403,
                "application/json",
                br#"{"error":"invalid token"}"#,
            )?;
            return Ok(());
        }
        if content_length > 64 * 1024 * 1024 {
            write_http_response(
                &mut stream,
                413,
                "application/json",
                br#"{"error":"payload too large"}"#,
            )?;
            return Ok(());
        }
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body)?;

        let payload: BridgeSavePayload = match serde_json::from_slice(&body) {
            Ok(p) => p,
            Err(err) => {
                let err_json = serde_json::to_vec(&serde_json::json!({
                    "ok": false,
                    "error": format!("invalid JSON save payload: {err}")
                }))?;
                write_http_response(&mut stream, 400, "application/json", &err_json)?;
                return Ok(());
            }
        };

        let root_opt = shared.project_root.lock().ok().and_then(|g| g.clone());
        match apply_bridge_save_payload(root_opt.as_deref(), &payload) {
            Ok(saved_files) => {
                let names: Vec<String> = saved_files.iter().map(|(p, _)| p.clone()).collect();
                let resp = serde_json::to_vec(&serde_json::json!({
                    "ok": true,
                    "saved_files": names
                }))?;
                (shared.on_save)(saved_files);
                write_http_response(&mut stream, 200, "application/json", &resp)?;
            }
            Err(err) => {
                let resp = serde_json::to_vec(&serde_json::json!({
                    "ok": false,
                    "error": format!("{err:#}")
                }))?;
                write_http_response(&mut stream, 400, "application/json", &resp)?;
            }
        }
        return Ok(());
    }

    if method == "GET" || method == "HEAD" {
        serve_static_file(&mut stream, &shared.web_root, path_only, method == "HEAD")?;
        return Ok(());
    }

    write_http_response(&mut stream, 405, "text/plain", b"Method Not Allowed")?;
    Ok(())
}

fn handle_websocket_session(
    mut stream: TcpStream,
    client_key: &str,
    shared: &ServerShared,
) -> Result<()> {
    let accept_key = compute_ws_accept_key(client_key);
    let handshake = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept_key}\r\n\r\n"
    );
    stream.write_all(handshake.as_bytes())?;
    stream.flush()?;

    let _ = stream.set_nonblocking(true);

    // If a fresh WebSocket connects while no commands are queued, replay the last
    // OpenModel command so an iframe reload never lands empty on the Home screen.
    let replay_cmd = {
        let queue_empty = shared
            .pending_commands
            .lock()
            .map(|q| q.is_empty())
            .unwrap_or(true);
        if queue_empty {
            shared.last_open_model.lock().ok().and_then(|g| g.clone())
        } else {
            None
        }
    };
    if let Some(cmd) = replay_cmd
        && let Ok(json) = serde_json::to_string(&cmd)
        && write_ws_text_frame(&mut stream, json.as_bytes()).is_err()
    {
        return Ok(());
    }

    let mut ping_buf = [0u8; 64];
    while !shared.shutdown.load(Ordering::Relaxed) {
        let mut cmds: VecDeque<StudioToBlockbenchCommand> = shared
            .pending_commands
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default();

        while let Some(cmd) = cmds.pop_front() {
            if let Ok(json) = serde_json::to_string(&cmd)
                && write_ws_text_frame(&mut stream, json.as_bytes()).is_err()
            {
                // Re-queue unsent commands in FIFO order so a dying/stale socket never drops them.
                if let Ok(mut q) = shared.pending_commands.lock() {
                    for remaining in cmds.into_iter().rev() {
                        q.push_front(remaining);
                    }
                    q.push_front(cmd);
                }
                return Ok(());
            }
        }

        match stream.read(&mut ping_buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => break,
        }

        thread::sleep(std::time::Duration::from_millis(80));
    }
    Ok(())
}

fn write_ws_text_frame(stream: &mut TcpStream, payload: &[u8]) -> Result<()> {
    let mut header = Vec::with_capacity(10);
    header.push(0x81);
    let len = payload.len();
    if len <= 125 {
        header.push(len as u8);
    } else if len <= 65_535 {
        header.push(126);
        header.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        header.push(127);
        header.extend_from_slice(&(len as u64).to_be_bytes());
    }
    stream.write_all(&header)?;
    stream.write_all(payload)?;
    stream.flush()?;
    Ok(())
}

fn serve_static_file(
    stream: &mut TcpStream,
    web_root: &Path,
    url_path: &str,
    head_only: bool,
) -> Result<()> {
    let decoded = percent_decode(url_path);
    let rel = decoded.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };

    let mut clean = PathBuf::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(seg) => clean.push(seg),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                write_http_response(stream, 403, "text/plain", b"Forbidden")?;
                return Ok(());
            }
        }
    }

    let is_bridge_js = clean.to_string_lossy() == "studio_bridge.js";
    let (bytes, mime, cache_control) = if is_bridge_js {
        (
            EMBEDDED_STUDIO_BRIDGE_JS.as_bytes().to_vec(),
            "application/javascript; charset=utf-8",
            "no-store",
        )
    } else {
        let full = web_root.join(&clean);
        if !full.is_file() {
            write_http_response(stream, 404, "text/plain", b"Not Found")?;
            return Ok(());
        }

        if clean.to_string_lossy() == "index.html" {
            let raw = fs::read(&full)?;
            let html = String::from_utf8_lossy(&raw);
            let injected = inject_compact_overrides(&html);
            (injected.into_bytes(), "text/html; charset=utf-8", "no-store")
        } else {
            (fs::read(&full)?, mime_for_path(&full), "public, max-age=3600")
        }
    };
    let header = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {mime}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: {cache_control}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Connection: close\r\n\r\n",
        bytes.len()
    );
    stream.write_all(header.as_bytes())?;
    if !head_only {
        stream.write_all(&bytes)?;
    }
    stream.flush()?;
    Ok(())
}

fn write_http_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: Content-Type\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

fn query_has_valid_token(query: &str, expected: &str) -> bool {
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=')
            && k == "token"
            && percent_decode(v) == expected
        {
            return true;
        }
    }
    false
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

const fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[must_use]
pub fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("json" | "bbtheme" | "bbkeymap" | "webmanifest") => "application/json; charset=utf-8",
        Some("lang" | "txt" | "md") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Injects `BLOCKBENCH_COMPACT_INJECTION` just before the first `</head>` in `html`.
/// Returns the modified string; if `</head>` is absent the original is returned unchanged.
fn inject_compact_overrides(html: &str) -> String {
    html.replacen(
        "</head>",
        &format!("{}\n</head>", BLOCKBENCH_COMPACT_INJECTION),
        1,
    )
}

fn compute_ws_accept_key(client_key: &str) -> String {
    const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut msg = Vec::with_capacity(client_key.len() + GUID.len());
    msg.extend_from_slice(client_key.trim().as_bytes());
    msg.extend_from_slice(GUID.as_bytes());
    let digest = sha1_digest(&msg);
    BASE64_STANDARD.encode(digest)
}

fn sha1_digest(input: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xEFCD_AB89;
    let mut h2: u32 = 0x98BA_DCFE;
    let mut h3: u32 = 0x1032_5476;
    let mut h4: u32 = 0xC3D2_E1F0;

    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut padded = input.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            let off = i * 4;
            *word = u32::from_be_bytes([
                chunk[off],
                chunk[off + 1],
                chunk[off + 2],
                chunk[off + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_compact_css() {
        let fake_html = "<html><head><title>Blockbench</title></head><body></body></html>";
        let result = inject_compact_overrides(fake_html);
        // Should have injected before </head>
        assert!(
            result.contains("ps_set_zoom"),
            "injection should contain ps_set_zoom listener"
        );
        assert!(
            result.contains("zoom: 0.84 !important"),
            "injection should contain zoom: 0.84 !important CSS"
        );
        assert!(
            result.contains("0.84"),
            "injection should reference 0.84 zoom value in JS"
        );
        // Should still have </head> after injection
        assert!(result.contains("</head>"), "output should still contain </head>");
        // The injection should appear BEFORE </head>
        let inj_pos = result.find("ps_set_zoom").unwrap();
        let head_pos = result.find("</head>").unwrap();
        assert!(inj_pos < head_pos, "injection should come before </head>");
    }

    #[test]
    fn test_inject_compact_css_no_head() {
        // If there's no </head>, the string is returned unchanged.
        let no_head = "<html><body>no head here</body></html>";
        let result = inject_compact_overrides(no_head);
        assert_eq!(result, no_head, "without </head> the HTML should be unchanged");
    }
}
