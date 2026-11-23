use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rp_core::{ConversionWarning, PipelineStage, WarningSeverity, normalize_bedrock_engine_triplet, parse_minecraft_json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::extensions::{
    BUILTIN_MINECRAFT_EXT_ID, LspContributionDto, default_extensions_dir,
    load_persisted_state, save_persisted_state, scan_extensions_snapshot,
};
use crate::state::StudioState;
use crate::workspace::find_in_path;

/// Live diagnostic item pushed to Monaco & bottom Problems view (`lsp://diagnostics`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnosticDto {
    pub server_id: String,
    pub rel_path: String,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub severity: WarningSeverity,
    pub message: String,
    pub source: String,
}

/// Event emitted on `"lsp://diagnostics"`.
#[derive(Debug, Clone, Serialize)]
pub struct LspDiagnosticsEvent {
    pub server_id: String,
    pub rel_path: String,
    pub diagnostics: Vec<LspDiagnosticDto>,
}

/// Completion item returned by `lsp_completion`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspCompletionItemDto {
    pub label: String,
    pub detail: String,
    pub documentation: String,
    pub insert_text: String,
    pub kind: String,
}

/// Hover payload returned by `lsp_hover`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspHoverDto {
    pub contents: String,
    pub source: String,
}

/// Status row returned by `lsp_list_servers`.
#[derive(Debug, Clone, Serialize)]
pub struct LspServerStatusDto {
    pub server_id: String,
    pub display_name: String,
    pub command: String,
    pub args: Vec<String>,
    pub language_ids: Vec<String>,
    pub builtin: bool,
    pub binary_found: bool,
    pub running: bool,
}

type PendingRequests = Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>;

/// Active external stdio JSON-RPC LSP process managed by `StudioState`.
pub struct LspClientHandle {
    pub server_id: String,
    pub command: String,
    pub language_ids: Vec<String>,
    stdin: Arc<Mutex<ChildStdin>>,
    child: Arc<Mutex<Child>>,
    pending: PendingRequests,
    next_id: AtomicU64,
    alive: Arc<AtomicBool>,
}

/// Encodes a JSON-RPC 2.0 message with standard `Content-Length` header framing.
#[must_use]
pub fn frame_lsp_message(payload: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(&body);
    out
}

/// Reads a single `Content-Length` framed JSON-RPC 2.0 payload from an LSP stdout reader.
pub fn read_lsp_message<R: BufRead>(reader: &mut R) -> Result<Option<Value>> {
    let mut content_len: Option<usize> = None;
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if content_len.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, val)) = trimmed.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            content_len = val.trim().parse::<usize>().ok();
        }
    }

    let len = content_len.context("LSP frame missing Content-Length header")?;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    let parsed: Value = serde_json::from_slice(&body)?;
    Ok(Some(parsed))
}

impl LspClientHandle {
    fn send_notification(&self, method: &str, params: Value) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let framed = frame_lsp_message(&msg);
        let mut guard = self
            .stdin
            .lock()
            .map_err(|_| anyhow::anyhow!("LSP stdin mutex poisoned"))?;
        guard.write_all(&framed)?;
        guard.flush()?;
        Ok(())
    }

    fn send_request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        {
            let mut pguard = self
                .pending
                .lock()
                .map_err(|_| anyhow::anyhow!("LSP pending mutex poisoned"))?;
            pguard.insert(id, tx);
        }

        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let framed = frame_lsp_message(&msg);
        {
            let mut sguard = self
                .stdin
                .lock()
                .map_err(|_| anyhow::anyhow!("LSP stdin mutex poisoned"))?;
            sguard.write_all(&framed)?;
            sguard.flush()?;
        }

        match rx.recv_timeout(timeout) {
            Ok(resp) => Ok(resp),
            Err(err) => {
                if let Ok(mut pguard) = self.pending.lock() {
                    pguard.remove(&id);
                }
                bail!("LSP request '{method}' timed out or failed: {err}")
            }
        }
    }

    /// Gracefully shuts down and terminates the child LSP process.
    pub fn shutdown_and_kill(&self) {
        self.alive.store(false, Ordering::Relaxed);
        let _ = self.send_request("shutdown", Value::Null, Duration::from_millis(250));
        let _ = self.send_notification("exit", Value::Null);
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn rel_path_to_lsp_uri(root: Option<&str>, rel_path: &str) -> String {
    let base = root.unwrap_or("/pack-studio-workspace").trim_end_matches('/');
    let rel = rel_path.trim_start_matches('/').replace('\\', "/");
    let joined = format!("{base}/{rel}");
    if joined.starts_with('/') {
        format!("file://{joined}")
    } else {
        format!("file:///{}", joined.replace('\\', "/"))
    }
}

fn lsp_uri_to_rel_path(uri: &str, root: Option<&str>) -> String {
    let clean = uri.strip_prefix("file://").unwrap_or(uri).replace('\\', "/");
    if let Some(r) = root {
        let norm_root = format!("{}/", r.trim_end_matches('/').replace('\\', "/"));
        if let Some(stripped) = clean.strip_prefix(&norm_root) {
            return stripped.to_string();
        }
    }
    clean.trim_start_matches('/').to_string()
}

/// Built-in Minecraft Resource Pack LSP diagnostics analyzer (pure Rust, unit-testable).
#[must_use]
pub fn builtin_minecraft_diagnostics(
    rel_path: &str,
    content: &str,
    known_pack_files: &[String],
) -> Vec<LspDiagnosticDto> {
    let mut out = Vec::new();
    let lower = rel_path.to_ascii_lowercase();

    if lower.ends_with(".json") || lower.ends_with(".mcmeta") {
        match parse_minecraft_json::<Value>(content.as_bytes()) {
            Err(err) => {
                out.push(LspDiagnosticDto {
                    server_id: String::from("builtin.minecraft-rp"),
                    rel_path: rel_path.to_string(),
                    line: 1,
                    column: 1,
                    end_line: 1,
                    end_column: 80,
                    severity: WarningSeverity::Error,
                    message: format!("Invalid JSON syntax: {err}"),
                    source: String::from("minecraft-lsp"),
                });
                return out;
            }
            Ok(val) => {
                if lower == "manifest.json" {
                    if let Some(triplet) = val
                        .pointer("/header/min_engine_version")
                        .and_then(Value::as_array)
                        && triplet.len() == 3
                        && let (Some(a), Some(b), Some(c)) = (
                            triplet[0].as_u64(),
                            triplet[1].as_u64(),
                            triplet[2].as_u64(),
                        )
                    {
                        let norm = normalize_bedrock_engine_triplet([a as u32, b as u32, c as u32]);
                        if a != 1 {
                            out.push(LspDiagnosticDto {
                                server_id: String::from("builtin.minecraft-rp"),
                                rel_path: rel_path.to_string(),
                                line: 1,
                                column: 1,
                                end_line: 1,
                                end_column: 40,
                                severity: WarningSeverity::Error,
                                message: format!(
                                    "Bedrock min_engine_version major version must stay [1, x, x], normalized suggestion: {:?}",
                                    norm
                                ),
                                source: String::from("minecraft-lsp"),
                            });
                        }
                    }
                    if val.pointer("/modules").and_then(Value::as_array).is_none_or(Vec::is_empty) {
                        out.push(LspDiagnosticDto {
                            server_id: String::from("builtin.minecraft-rp"),
                            rel_path: rel_path.to_string(),
                            line: 1,
                            column: 1,
                            end_line: 1,
                            end_column: 20,
                            severity: WarningSeverity::Error,
                            message: String::from("Bedrock manifest.json must define at least one entry in 'modules'"),
                            source: String::from("minecraft-lsp"),
                        });
                    }
                } else if lower == "pack.mcmeta" {
                    if val.pointer("/pack/pack_format").and_then(Value::as_u64).is_none() {
                        out.push(LspDiagnosticDto {
                            server_id: String::from("builtin.minecraft-rp"),
                            rel_path: rel_path.to_string(),
                            line: 1,
                            column: 1,
                            end_line: 1,
                            end_column: 20,
                            severity: WarningSeverity::Error,
                            message: String::from("Java pack.mcmeta missing required integer '/pack/pack_format'"),
                            source: String::from("minecraft-lsp"),
                        });
                    }
                } else if (lower == "textures/terrain_texture.json" || lower == "textures/item_texture.json")
                    && let Some(tex_data) = val.pointer("/texture_data").and_then(Value::as_object)
                    && !known_pack_files.is_empty()
                {
                    for (key, entry) in tex_data {
                        let tex_ref = entry
                            .get("textures")
                            .and_then(|t| t.as_str())
                            .or_else(|| {
                                entry
                                    .get("textures")
                                    .and_then(|t| t.get("path"))
                                    .and_then(Value::as_str)
                            });
                        if let Some(p) = tex_ref {
                            let norm = p.trim_start_matches('/').replace('\\', "/");
                            let has_match = known_pack_files.iter().any(|f| {
                                let fl = f.to_ascii_lowercase();
                                fl == format!("{norm}.png").to_ascii_lowercase()
                                    || fl == format!("{norm}.tga").to_ascii_lowercase()
                                    || fl == norm.to_ascii_lowercase()
                            });
                            if !has_match {
                                out.push(LspDiagnosticDto {
                                    server_id: String::from("builtin.minecraft-rp"),
                                    rel_path: rel_path.to_string(),
                                    line: 1,
                                    column: 1,
                                    end_line: 1,
                                    end_column: 40,
                                    severity: WarningSeverity::Warning,
                                    message: format!("Atlas entry '{key}' references '{norm}' which is missing in active pack"),
                                    source: String::from("minecraft-lsp"),
                                });
                            }
                        }
                    }
                }
            }
        }
    } else if lower.ends_with(".lang") {
        for (idx, line) in content.lines().enumerate() {
            let t = line.trim();
            if t.is_empty() || t.starts_with("##") || t.starts_with('#') {
                continue;
            }
            if !t.contains('=') {
                out.push(LspDiagnosticDto {
                    server_id: String::from("builtin.minecraft-rp"),
                    rel_path: rel_path.to_string(),
                    line: (idx + 1) as u32,
                    column: 1,
                    end_line: (idx + 1) as u32,
                    end_column: (line.len().max(1)) as u32,
                    severity: WarningSeverity::Warning,
                    message: String::from("Expected 'key=value' syntax in .lang file"),
                    source: String::from("minecraft-lsp"),
                });
            }
        }
    }

    out
}

/// Built-in Minecraft Resource Pack context-aware autocomplete (pure Rust).
#[must_use]
pub fn builtin_minecraft_completions(
    rel_path: &str,
    language_id: &str,
    prefix: &str,
    known_pack_files: &[String],
) -> Vec<LspCompletionItemDto> {
    let mut items = Vec::new();
    let lower_path = rel_path.to_ascii_lowercase();
    let q = prefix.to_ascii_lowercase();

    if language_id == "molang" || lower_path.ends_with(".molang") || lower_path.ends_with(".json") {
        for (label, detail, insert) in [
            ("query.anim_time", "MoLang Query (f32)", "query.anim_time"),
            ("query.life_time", "MoLang Query (f32)", "query.life_time"),
            ("query.modified_move_speed", "MoLang Query (f32)", "query.modified_move_speed"),
            ("query.is_sneaking", "MoLang Query (bool)", "query.is_sneaking"),
            ("math.sin(angle)", "MoLang Math (deg)", "math.sin(${1:query.anim_time * 90})"),
            ("math.cos(angle)", "MoLang Math (deg)", "math.cos(${1:query.anim_time * 90})"),
            ("math.clamp(val, min, max)", "MoLang Math", "math.clamp(${1:val}, ${2:0}, ${3:1})"),
        ] {
            if q.is_empty() || label.to_ascii_lowercase().contains(&q) {
                items.push(LspCompletionItemDto {
                    label: label.to_string(),
                    detail: detail.to_string(),
                    documentation: format!("Built-in Bedrock MoLang symbol `{label}`"),
                    insert_text: insert.to_string(),
                    kind: String::from("Function"),
                });
            }
        }
    }

    if lower_path == "manifest.json" {
        for (label, insert) in [
            ("min_engine_version", "\"min_engine_version\": [1, 21, 50]"),
            ("format_version", "\"format_version\": 2"),
            ("capabilities (pbr)", "\"capabilities\": [\"pbr\"]"),
        ] {
            items.push(LspCompletionItemDto {
                label: label.to_string(),
                detail: String::from("Bedrock Manifest Property"),
                documentation: String::from("Enforces Bedrock [1, x, x] engine compatibility"),
                insert_text: insert.to_string(),
                kind: String::from("Property"),
            });
        }
    } else if lower_path.ends_with(".lang") {
        for (label, insert, desc) in [
            ("pack.name", "pack.name=", "Localized resource pack title"),
            ("pack.description", "pack.description=", "Localized resource pack subtitle"),
            ("§a (Green)", "§a", "Minecraft Bright Green formatting code"),
            ("§b (Aqua)", "§b", "Minecraft Aqua formatting code"),
            ("§l (Bold)", "§l", "Minecraft Bold formatting code"),
            ("§r (Reset)", "§r", "Minecraft Reset formatting code"),
        ] {
            items.push(LspCompletionItemDto {
                label: label.to_string(),
                detail: desc.to_string(),
                documentation: desc.to_string(),
                insert_text: insert.to_string(),
                kind: String::from("Keyword"),
            });
        }
    }

    // Dynamic active-pack texture path suggestions inside JSON manifests/models
    if language_id == "json" {
        for file in known_pack_files {
            let fl = file.to_ascii_lowercase();
            if (fl.ends_with(".png") || fl.ends_with(".tga"))
                && (q.is_empty() || fl.contains(&q))
            {
                let stem = file
                    .strip_suffix(".png")
                    .or_else(|| file.strip_suffix(".tga"))
                    .unwrap_or(file);
                items.push(LspCompletionItemDto {
                    label: stem.to_string(),
                    detail: String::from("Workspace Texture Path"),
                    documentation: format!("Pack asset `{file}`"),
                    insert_text: stem.to_string(),
                    kind: String::from("File"),
                });
                if items.len() >= 60 {
                    break;
                }
            }
        }
    }

    items
}

/// Built-in hover documentation helper.
#[must_use]
pub fn builtin_minecraft_hover(rel_path: &str, word: &str) -> Option<LspHoverDto> {
    let w = word.trim().trim_matches('"');
    let content = match w {
        "min_engine_version" => "**`min_engine_version`** (`[1, minor, patch]`)\n\nMinimum Minecraft Bedrock Edition version required. PackStudio guards major version `1` cleanly.",
        "pack_format" => "**`pack_format`** (Java Edition integer)\n\n- `22` -> 1.20.3–1.20.4\n- `34` -> 1.21–1.21.1\n- `42` -> 1.21.2–1.21.3\n- `46` -> 1.21.4+",
        "texture_data" => "**`texture_data`**\n\nMaps short texture keys (referenced by `blocks.json` or geometry) to pack-relative texture paths inside `textures/`.",
        "query.anim_time" => "**`query.anim_time`** (MoLang)\n\nElapsed time in seconds since the current animation started playing.",
        "math.sin" | "sin" => "**`math.sin(degrees)`** (MoLang)\n\nSine function accepting **degrees** (not radians).",
        _ if rel_path.eq_ignore_ascii_case("manifest.json") && w == "raytraced" => "**`raytraced`**\n\nDeclares Bedrock RTX PBR texture_set compatibility.",
        _ => return None,
    };
    Some(LspHoverDto {
        contents: String::from(content),
        source: String::from("Minecraft Pack Intelligence (Built-in)"),
    })
}

fn collect_configured_servers(extensions_root: &std::path::Path) -> Vec<LspContributionDto> {
    let snap = scan_extensions_snapshot(extensions_root);
    let persisted = load_persisted_state(extensions_root);
    let mut map: BTreeMap<String, LspContributionDto> = BTreeMap::new();

    for ext in snap.extensions {
        if !ext.enabled && ext.id != BUILTIN_MINECRAFT_EXT_ID {
            continue;
        }
        for lsp in ext.lsp_contributions {
            map.insert(lsp.server_id.clone(), lsp);
        }
    }
    for custom in persisted.custom_lsp_servers {
        map.insert(custom.server_id.clone(), custom);
    }

    map.into_values().collect()
}

/// Tauri Command: List all built-in, extension-provided, and user-configured LSP servers.
#[tauri::command]
pub fn lsp_list_servers(state: tauri::State<'_, StudioState>) -> Vec<LspServerStatusDto> {
    let root = default_extensions_dir();
    let configured = collect_configured_servers(&root);
    let builtin_on = state.builtin_lsp_enabled.load(Ordering::Relaxed);
    let running_guard = state.lsp_clients.lock().ok();

    configured
        .into_iter()
        .map(|cfg| {
            let running = if cfg.builtin {
                builtin_on
            } else {
                running_guard
                    .as_ref()
                    .and_then(|m| m.get(&cfg.server_id))
                    .is_some_and(|h| h.alive.load(Ordering::Relaxed))
            };
            let binary_found = if cfg.builtin {
                true
            } else {
                find_in_path(&cfg.command).is_some()
                    || std::path::Path::new(&cfg.command).is_file()
            };
            LspServerStatusDto {
                server_id: cfg.server_id,
                display_name: cfg.display_name,
                command: cfg.command,
                args: cfg.args,
                language_ids: cfg.language_ids,
                builtin: cfg.builtin,
                binary_found,
                running,
            }
        })
        .collect()
}

/// Tauri Command: Register a custom external stdio LSP binary into extension state.
#[tauri::command]
pub fn lsp_register_custom_server(
    server_id: String,
    display_name: String,
    command: String,
    args: Vec<String>,
    language_ids: Vec<String>,
) -> Result<(), String> {
    let sid = server_id.trim().to_string();
    let cmd = command.trim().to_string();
    if sid.is_empty() || cmd.is_empty() {
        return Err(String::from("server_id and command must not be empty"));
    }
    let root = default_extensions_dir();
    let mut persisted = load_persisted_state(&root);
    persisted.custom_lsp_servers.retain(|s| s.server_id != sid);
    persisted.custom_lsp_servers.push(LspContributionDto {
        server_id: sid,
        display_name: if display_name.trim().is_empty() {
            cmd.clone()
        } else {
            display_name.trim().to_string()
        },
        command: cmd,
        args,
        language_ids: if language_ids.is_empty() {
            vec![String::from("json")]
        } else {
            language_ids
        },
        builtin: false,
    });
    save_persisted_state(&root, &persisted).map_err(|e| format!("{e:#}"))
}

/// Tauri Command: Remove a user-registered custom stdio LSP server.
#[tauri::command]
pub fn lsp_unregister_custom_server(
    state: tauri::State<'_, StudioState>,
    server_id: String,
) -> Result<(), String> {
    if let Ok(mut clients) = state.lsp_clients.lock()
        && let Some(old) = clients.remove(&server_id)
    {
        old.shutdown_and_kill();
    }
    let root = default_extensions_dir();
    let mut persisted = load_persisted_state(&root);
    persisted.custom_lsp_servers.retain(|s| s.server_id != server_id);
    save_persisted_state(&root, &persisted).map_err(|e| format!("{e:#}"))
}

/// Tauri Command: Spawn and `initialize` an external stdio JSON-RPC LSP server.
#[tauri::command]
pub fn lsp_start_server(
    app: AppHandle,
    state: tauri::State<'_, StudioState>,
    server_id: String,
    command: String,
    args: Vec<String>,
    language_ids: Vec<String>,
) -> Result<(), String> {
    let sid = server_id.trim().to_string();
    if sid == "builtin.minecraft-rp" || command.trim() == "builtin" {
        state.builtin_lsp_enabled.store(true, Ordering::Relaxed);
        return Ok(());
    }

    let workspace_root = state
        .active_pack
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|p| p.source_path.clone()));

    let mut cmd = Command::new(command.trim());
    cmd.args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(ref dir) = workspace_root
        && dir.is_dir()
    {
        cmd.current_dir(dir);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed spawning LSP '{command}': {e}"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| String::from("LSP stdin unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| String::from("LSP stdout unavailable"))?;

    let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
    let alive = Arc::new(AtomicBool::new(true));

    let pending_clone = Arc::clone(&pending);
    let alive_clone = Arc::clone(&alive);
    let sid_clone = sid.clone();
    let root_arc = state.active_pack.clone();

    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(msg)) = read_lsp_message(&mut reader) {
            if let Some(id_val) = msg.get("id").and_then(Value::as_u64) {
                if let Ok(mut pguard) = pending_clone.lock()
                    && let Some(tx) = pguard.remove(&id_val)
                {
                    let _ = tx.send(msg);
                }
                continue;
            }
            if msg.get("method").and_then(Value::as_str)
                == Some("textDocument/publishDiagnostics")
                && let Some(params) = msg.get("params")
            {
                let uri = params
                    .get("uri")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let root_str = root_arc
                    .lock()
                    .ok()
                    .and_then(|g| {
                        g.as_ref()
                            .and_then(|p| p.source_path.as_ref().map(|pb| pb.display().to_string()))
                    });
                let rel_path = lsp_uri_to_rel_path(uri, root_str.as_deref());
                let mut diags = Vec::new();
                if let Some(arr) = params.get("diagnostics").and_then(Value::as_array) {
                    for d in arr {
                        let start_line = d
                            .pointer("/range/start/line")
                            .and_then(Value::as_u64)
                            .unwrap_or(0) as u32
                            + 1;
                        let start_col = d
                            .pointer("/range/start/character")
                            .and_then(Value::as_u64)
                            .unwrap_or(0) as u32
                            + 1;
                        let end_line = d
                            .pointer("/range/end/line")
                            .and_then(Value::as_u64)
                            .unwrap_or(0) as u32
                            + 1;
                        let end_col = d
                            .pointer("/range/end/character")
                            .and_then(Value::as_u64)
                            .unwrap_or(1) as u32
                            + 1;
                        let sev = match d.get("severity").and_then(Value::as_u64).unwrap_or(1) {
                            1 => WarningSeverity::Error,
                            2 => WarningSeverity::Warning,
                            _ => WarningSeverity::Info,
                        };
                        diags.push(LspDiagnosticDto {
                            server_id: sid_clone.clone(),
                            rel_path: rel_path.clone(),
                            line: start_line,
                            column: start_col,
                            end_line,
                            end_column: end_col,
                            severity: sev,
                            message: d
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("LSP diagnostic")
                                .to_string(),
                            source: d
                                .get("source")
                                .and_then(Value::as_str)
                                .unwrap_or(&sid_clone)
                                .to_string(),
                        });
                    }
                }
                let _ = app.emit(
                    "lsp://diagnostics",
                    LspDiagnosticsEvent {
                        server_id: sid_clone.clone(),
                        rel_path,
                        diagnostics: diags,
                    },
                );
            }
        }
        alive_clone.store(false, Ordering::Relaxed);
    });

    let handle = LspClientHandle {
        server_id: sid.clone(),
        command,
        language_ids,
        stdin: Arc::new(Mutex::new(stdin)),
        child: Arc::new(Mutex::new(child)),
        pending,
        next_id: AtomicU64::new(1),
        alive,
    };

    let root_uri = workspace_root
        .as_ref()
        .map(|p| rel_path_to_lsp_uri(Some(&p.display().to_string()), ""));
    let _ = handle.send_request(
        "initialize",
        json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "synchronization": { "didSave": true },
                    "completion": { "completionItem": { "snippetSupport": true } },
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "publishDiagnostics": {}
                }
            }
        }),
        Duration::from_secs(3),
    );
    let _ = handle.send_notification("initialized", json!({}));

    let mut clients = state
        .lsp_clients
        .lock()
        .map_err(|_| String::from("LSP clients mutex poisoned"))?;
    if let Some(old) = clients.insert(sid, handle) {
        old.shutdown_and_kill();
    }
    Ok(())
}

/// Tauri Command: Stop an active external stdio LSP server or disable Built-in LSP.
#[tauri::command]
pub fn lsp_stop_server(
    state: tauri::State<'_, StudioState>,
    server_id: String,
) -> Result<(), String> {
    if server_id == "builtin.minecraft-rp" {
        state.builtin_lsp_enabled.store(false, Ordering::Relaxed);
        return Ok(());
    }
    let mut clients = state
        .lsp_clients
        .lock()
        .map_err(|_| String::from("LSP clients mutex poisoned"))?;
    if let Some(old) = clients.remove(&server_id) {
        old.shutdown_and_kill();
    }
    Ok(())
}

/// Tauri Command: Sync open/changed/saved document to matching stdio LSPs & Built-in Minecraft LSP.
#[tauri::command]
pub fn lsp_sync_document(
    app: AppHandle,
    state: tauri::State<'_, StudioState>,
    rel_path: String,
    language_id: String,
    content: String,
    version: i32,
    event_kind: String,
) -> Vec<LspDiagnosticDto> {
    let (root_str, known_files) = state
        .active_pack
        .lock()
        .ok()
        .map(|g| match g.as_ref() {
            Some(pack) => (
                pack.source_path.as_ref().map(|p| p.display().to_string()),
                pack.container.files().keys().cloned().collect::<Vec<_>>(),
            ),
            None => (None, Vec::new()),
        })
        .unwrap_or((None, Vec::new()));

    let uri = rel_path_to_lsp_uri(root_str.as_deref(), &rel_path);

    if let Ok(clients) = state.lsp_clients.lock() {
        for client in clients.values() {
            if !client.alive.load(Ordering::Relaxed)
                || (!client.language_ids.is_empty()
                    && !client.language_ids.iter().any(|l| l == &language_id))
            {
                continue;
            }
            match event_kind.as_str() {
                "open" => {
                    let _ = client.send_notification(
                        "textDocument/didOpen",
                        json!({
                            "textDocument": {
                                "uri": uri,
                                "languageId": language_id,
                                "version": version,
                                "text": content
                            }
                        }),
                    );
                }
                "save" => {
                    let _ = client.send_notification(
                        "textDocument/didSave",
                        json!({
                            "textDocument": { "uri": uri },
                            "text": content
                        }),
                    );
                }
                _ => {
                    let _ = client.send_notification(
                        "textDocument/didChange",
                        json!({
                            "textDocument": { "uri": uri, "version": version },
                            "contentChanges": [{ "text": content }]
                        }),
                    );
                }
            }
        }
    }

    let diags = if state.builtin_lsp_enabled.load(Ordering::Relaxed) {
        builtin_minecraft_diagnostics(&rel_path, &content, &known_files)
    } else {
        Vec::new()
    };

    let _ = app.emit(
        "lsp://diagnostics",
        LspDiagnosticsEvent {
            server_id: String::from("builtin.minecraft-rp"),
            rel_path,
            diagnostics: diags.clone(),
        },
    );

    diags
}

/// Tauri Command: Query completions from Built-in Minecraft LSP & matching external stdio LSPs.
#[tauri::command]
pub fn lsp_completion(
    state: tauri::State<'_, StudioState>,
    rel_path: String,
    language_id: String,
    line: u32,
    character: u32,
    prefix: String,
) -> Vec<LspCompletionItemDto> {
    let (root_str, known_files) = state
        .active_pack
        .lock()
        .ok()
        .map(|g| match g.as_ref() {
            Some(pack) => (
                pack.source_path.as_ref().map(|p| p.display().to_string()),
                pack.container.files().keys().cloned().collect::<Vec<_>>(),
            ),
            None => (None, Vec::new()),
        })
        .unwrap_or((None, Vec::new()));

    let mut out = if state.builtin_lsp_enabled.load(Ordering::Relaxed) {
        builtin_minecraft_completions(&rel_path, &language_id, &prefix, &known_files)
    } else {
        Vec::new()
    };

    let uri = rel_path_to_lsp_uri(root_str.as_deref(), &rel_path);
    if let Ok(clients) = state.lsp_clients.lock() {
        for client in clients.values() {
            if !client.alive.load(Ordering::Relaxed)
                || (!client.language_ids.is_empty()
                    && !client.language_ids.iter().any(|l| l == &language_id))
            {
                continue;
            }
            if let Ok(resp) = client.send_request(
                "textDocument/completion",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": line.saturating_sub(1), "character": character.saturating_sub(1) }
                }),
                Duration::from_millis(600),
            ) {
                let list = resp
                    .pointer("/result/items")
                    .or_else(|| resp.get("result"))
                    .and_then(Value::as_array);
                if let Some(arr) = list {
                    for entry in arr.iter().take(60) {
                        if let Some(label) = entry.get("label").and_then(Value::as_str) {
                            out.push(LspCompletionItemDto {
                                label: label.to_string(),
                                detail: entry
                                    .get("detail")
                                    .and_then(Value::as_str)
                                    .unwrap_or(&client.server_id)
                                    .to_string(),
                                documentation: entry
                                    .get("documentation")
                                    .and_then(|d| {
                                        d.as_str()
                                            .or_else(|| d.get("value").and_then(Value::as_str))
                                    })
                                    .unwrap_or("")
                                    .to_string(),
                                insert_text: entry
                                    .get("insertText")
                                    .and_then(Value::as_str)
                                    .unwrap_or(label)
                                    .to_string(),
                                kind: String::from("Property"),
                            });
                        }
                    }
                }
            }
        }
    }

    out
}

/// Tauri Command: Query hover info from Built-in Minecraft LSP & matching external stdio LSPs.
#[tauri::command]
pub fn lsp_hover(
    state: tauri::State<'_, StudioState>,
    rel_path: String,
    language_id: String,
    line: u32,
    character: u32,
    word: String,
) -> Option<LspHoverDto> {
    let root_str = state.active_pack.lock().ok().and_then(|g| {
        g.as_ref()
            .and_then(|p| p.source_path.as_ref().map(|pb| pb.display().to_string()))
    });
    let uri = rel_path_to_lsp_uri(root_str.as_deref(), &rel_path);

    if let Ok(clients) = state.lsp_clients.lock() {
        for client in clients.values() {
            if !client.alive.load(Ordering::Relaxed)
                || (!client.language_ids.is_empty()
                    && !client.language_ids.iter().any(|l| l == &language_id))
            {
                continue;
            }
            if let Ok(resp) = client.send_request(
                "textDocument/hover",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": line.saturating_sub(1), "character": character.saturating_sub(1) }
                }),
                Duration::from_millis(450),
            ) && let Some(contents) = resp.pointer("/result/contents")
            {
                let text = contents
                    .as_str()
                    .or_else(|| contents.get("value").and_then(Value::as_str))
                    .map(str::to_string);
                if let Some(c) = text
                    && !c.trim().is_empty()
                {
                    return Some(LspHoverDto {
                        contents: c,
                        source: client.server_id.clone(),
                    });
                }
            }
        }
    }

    if state.builtin_lsp_enabled.load(Ordering::Relaxed) {
        return builtin_minecraft_hover(&rel_path, &word);
    }
    None
}

/// Converts an `LspDiagnosticDto` into `rp_core::ConversionWarning` for unified Problems display.
#[must_use]
pub fn diagnostic_to_problem(diag: &LspDiagnosticDto) -> ConversionWarning {
    ConversionWarning {
        source_path: diag.rel_path.clone(),
        stage: PipelineStage::Audit,
        severity: diag.severity,
        reason: format!("{} (L{}:{})", diag.message, diag.line, diag.column),
        actionable_fix: format!("Reported by {}", diag.source),
        asset: diag.rel_path.clone(),
        message: diag.message.clone(),
    }
}
