use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Texture attachment pushed from PackStudio to Blockbench or saved back from Blockbench.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeTexturePayload {
    /// Pack-relative path (e.g. `"textures/entity/custom.png"`).
    pub path: String,
    /// Optional filename label.
    #[serde(default)]
    pub name: Option<String>,
    /// Data URL (`"data:image/png;base64,..."`) when sending Studio -> Blockbench.
    #[serde(default)]
    pub data_url: Option<String>,
    /// Raw base64 PNG payload when receiving Blockbench -> Studio.
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

/// Payload POSTed by `studio_bridge.js` when the artist saves or clicks "Sync to PackStudio".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BridgeSavePayload {
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub model_content: Option<String>,
    #[serde(default)]
    pub textures: Vec<BridgeTexturePayload>,
}

/// Event delivered to the `PackStudioApp` UI thread after a validated bridge save completes.
#[derive(Debug, Clone)]
pub struct BridgeSaveEvent {
    pub saved_files: Vec<(String, Vec<u8>)>,
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
    // Reject Windows drive letter prefixes like C:/...
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
    // Cross-verify via std::path::Path components
    let path_check = Path::new(&joined);
    for comp in path_check.components() {
        if !matches!(comp, Component::Normal(_)) {
            bail!("unsafe path component rejected by traversal guard: '{raw}'");
        }
    }

    // Re-check via rp_io::sanitize_archive_path for consistency with engine container rules
    let verified = rp_io::sanitize_archive_path(&joined)
        .with_context(|| format!("path rejected by archive sanitizer: '{raw}'"))?;
    Ok(verified)
}

/// Validates and writes a `BridgeSavePayload` (model JSON + base64 PNG textures) into
/// `project_root` (if `Some`), returning all validated `(relative_path, raw_bytes)` pairs.
///
/// Enforces the path traversal guard BEFORE writing any byte to disk.
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
            fs::write(&target, bytes)
                .with_context(|| format!("failed writing bridge file '{}'", target.display()))?;
        }
    }

    Ok(files_to_write)
}

/// Shared state accessed by the loopback HTTP/WebSocket worker threads.
struct ServerShared {
    web_root: PathBuf,
    token: String,
    project_root: Mutex<Option<PathBuf>>,
    pending_commands: Mutex<VecDeque<StudioToBlockbenchCommand>>,
    save_tx: Sender<BridgeSaveEvent>,
    shutdown: AtomicBool,
}

/// Loopback-only (`127.0.0.1:0`) static file server & 2-way Blockbench JSON/WebSocket bridge.
pub struct BlockbenchServer {
    port: u16,
    token: String,
    shared: Arc<ServerShared>,
}

impl BlockbenchServer {
    /// Starts the local server bound strictly to `127.0.0.1:0` (random free ephemeral port).
    pub fn start(
        web_root: PathBuf,
        initial_project_root: Option<PathBuf>,
        save_tx: Sender<BridgeSaveEvent>,
    ) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .context("failed to bind local Blockbench loopback server on 127.0.0.1:0")?;
        let port = listener
            .local_addr()
            .context("failed to read local Blockbench server port")?
            .port();
        let token = Uuid::new_v4().simple().to_string();

        let shared = Arc::new(ServerShared {
            web_root,
            token: token.clone(),
            project_root: Mutex::new(initial_project_root),
            pending_commands: Mutex::new(VecDeque::new()),
            save_tx,
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

    /// Returns the assigned loopback port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Returns the random session security token.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Returns the full authenticated URL to load in WebView or the local browser.
    #[must_use]
    pub fn launch_url(&self) -> String {
        format!(
            "http://127.0.0.1:{}/index.html?studio_port={}&token={}",
            self.port, self.port, self.token
        )
    }

    /// Updates the active project directory root written to when Blockbench saves files.
    pub fn set_project_root(&self, root: Option<PathBuf>) {
        if let Ok(mut guard) = self.shared.project_root.lock() {
            *guard = root;
        }
    }

    /// Enqueues a Studio -> Blockbench command (`open_model` or `new_bedrock_geo`).
    pub fn push_command(&self, cmd: StudioToBlockbenchCommand) {
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

    // CORS preflight support for local browser requests
    if method == "OPTIONS" {
        write_http_response(&mut stream, 204, "text/plain", b"")?;
        return Ok(());
    }

    // WebSocket upgrade endpoint (`/ws?token=...`)
    if is_ws_upgrade && path_only == "/ws" {
        if !query_has_valid_token(query, &shared.token) {
            write_http_response(&mut stream, 403, "application/json", br#"{"error":"invalid token"}"#)?;
            return Ok(());
        }
        if let Some(key) = ws_key {
            handle_websocket_session(stream, &key, shared)?;
        }
        return Ok(());
    }

    // API: Poll queued commands (`GET /api/commands?token=...`)
    if method == "GET" && path_only == "/api/commands" {
        if !query_has_valid_token(query, &shared.token) {
            write_http_response(&mut stream, 403, "application/json", br#"{"error":"invalid token"}"#)?;
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

    // API: Save model + textures (`POST /api/save?token=...`)
    if method == "POST" && path_only == "/api/save" {
        if !query_has_valid_token(query, &shared.token) {
            write_http_response(&mut stream, 403, "application/json", br#"{"error":"invalid token"}"#)?;
            return Ok(());
        }
        // Cap single save payload at 64 MiB
        if content_length > 64 * 1024 * 1024 {
            write_http_response(&mut stream, 413, "application/json", br#"{"error":"payload too large"}"#)?;
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
                let names: Vec<&str> = saved_files.iter().map(|(p, _)| p.as_str()).collect();
                let resp = serde_json::to_vec(&serde_json::json!({
                    "ok": true,
                    "saved_files": names
                }))?;
                let _ = shared.save_tx.send(BridgeSaveEvent { saved_files });
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

    // Static web bundle serving (`GET /...`)
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

    // Push queued commands over WebSocket until shutdown or disconnect.
    let _ = stream.set_nonblocking(true);
    let mut ping_buf = [0u8; 64];
    while !shared.shutdown.load(Ordering::Relaxed) {
        let cmds: Vec<StudioToBlockbenchCommand> = shared
            .pending_commands
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default();

        for cmd in cmds {
            if let Ok(json) = serde_json::to_string(&cmd)
                && write_ws_text_frame(&mut stream, json.as_bytes()).is_err()
            {
                return Ok(());
            }
        }

        // Detect peer close
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
    header.push(0x81); // FIN + Text opcode
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

    // Strict traversal guard on static root
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

    let full = web_root.join(&clean);
    if !full.is_file() {
        write_http_response(stream, 404, "text/plain", b"Not Found")?;
        return Ok(());
    }

    let bytes = fs::read(&full)?;
    let mime = mime_for_path(&full);
    let header = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {mime}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: public, max-age=3600\r\n\
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

/// Accurate MIME types for Blockbench web runtime assets.
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

/// Minimal RFC-3174 SHA-1 implementation used solely for the RFC-6455 WebSocket handshake.
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
