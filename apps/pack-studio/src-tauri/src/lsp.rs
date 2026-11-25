use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use pstdr_lang::diagnostics::{Diagnostic as PstdrDiagnostic, Severity as PstdrSeverity};
use pstdr_lang::spec::{
    BedrockSpec, map_item_property, map_method, map_property, map_world_method,
};
use pstdr_lang::{ValidationContext, parse_source, validate_program};
use rp_core::{ConversionWarning, PipelineStage, WarningSeverity, normalize_bedrock_engine_triplet, parse_minecraft_json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::extensions::{
    BUILTIN_MINECRAFT_EXT_ID, BUILTIN_PSTDR_EXT_ID, LspContributionDto, default_extensions_dir,
    load_persisted_state, save_persisted_state, scan_extensions_snapshot,
};
use crate::state::StudioState;
use crate::workspace::find_in_path;

pub const BUILTIN_MINECRAFT_SERVER_ID: &str = "builtin.minecraft-rp";
pub const BUILTIN_PSTDR_SERVER_ID: &str = "builtin.pstdr";
pub const EXTERNAL_PSTDR_SERVER_ID: &str = "external.pstdr-lsp";

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
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

/// Single 1-based text replacement returned inside `LspCodeActionDto`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspTextEditDto {
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub new_text: String,
}

/// Quick-fix / code action returned by `lsp_code_actions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspCodeActionDto {
    pub title: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_code: Option<String>,
    pub is_preferred: bool,
    pub edits: Vec<LspTextEditDto>,
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
                    server_id: String::from(BUILTIN_MINECRAFT_SERVER_ID),
                    rel_path: rel_path.to_string(),
                    line: 1,
                    column: 1,
                    end_line: 1,
                    end_column: 80,
                    severity: WarningSeverity::Error,
                    code: None,
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
                                server_id: String::from(BUILTIN_MINECRAFT_SERVER_ID),
                                rel_path: rel_path.to_string(),
                                line: 1,
                                column: 1,
                                end_line: 1,
                                end_column: 40,
                                severity: WarningSeverity::Error,
                                code: None,
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
                            server_id: String::from(BUILTIN_MINECRAFT_SERVER_ID),
                            rel_path: rel_path.to_string(),
                            line: 1,
                            column: 1,
                            end_line: 1,
                            end_column: 20,
                            severity: WarningSeverity::Error,
                            code: None,
                            message: String::from("Bedrock manifest.json must define at least one entry in 'modules'"),
                            source: String::from("minecraft-lsp"),
                        });
                    }
                } else if lower == "pack.mcmeta" {
                    if val.pointer("/pack/pack_format").and_then(Value::as_u64).is_none() {
                        out.push(LspDiagnosticDto {
                            server_id: String::from(BUILTIN_MINECRAFT_SERVER_ID),
                            rel_path: rel_path.to_string(),
                            line: 1,
                            column: 1,
                            end_line: 1,
                            end_column: 20,
                            severity: WarningSeverity::Error,
                            code: None,
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
                                    server_id: String::from(BUILTIN_MINECRAFT_SERVER_ID),
                                    rel_path: rel_path.to_string(),
                                    line: 1,
                                    column: 1,
                                    end_line: 1,
                                    end_column: 40,
                                    severity: WarningSeverity::Warning,
                                    code: None,
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
                    server_id: String::from(BUILTIN_MINECRAFT_SERVER_ID),
                    rel_path: rel_path.to_string(),
                    line: (idx + 1) as u32,
                    column: 1,
                    end_line: (idx + 1) as u32,
                    end_column: (line.len().max(1)) as u32,
                    severity: WarningSeverity::Warning,
                    code: None,
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

// ─── In-Process `builtin.pstdr` Language Server (powered by `pstdr-lang`) ───

const PSTDR_MEMBER_PROPERTIES: &[(&str, &str)] = &[
    ("name_tag", "nameTag"),
    ("type_id", "typeId"),
    ("hp", "getComponent(\"minecraft:health\")?.currentValue"),
    ("max_hp", "getComponent(\"minecraft:health\")?.effectiveMax"),
    ("selected_slot", "selectedSlotIndex"),
    ("head_location", "getHeadLocation()"),
    ("view_direction", "getViewDirection()"),
    (
        "holding",
        "getComponent(\"minecraft:equippable\")?.getEquipment(EquipmentSlot.Mainhand)",
    ),
    (
        "mainhand",
        "getComponent(\"minecraft:equippable\")?.getEquipment(EquipmentSlot.Mainhand)",
    ),
    ("location", "location"),
    ("dimension", "dimension"),
    ("is_sneaking", "isSneaking"),
    ("is_sprinting", "isSprinting"),
    ("is_swimming", "isSwimming"),
    ("is_flying", "isFlying"),
    ("is_sleeping", "isSleeping"),
    ("is_on_ground", "isOnGround"),
    ("is_in_water", "isInWater"),
    ("is_climbing", "isClimbing"),
    ("is_falling", "isFalling"),
    ("is_valid", "isValid"),
];

const PSTDR_MEMBER_METHODS: &[(&str, &str, &str)] = &[
    ("actionbar", "actionbar(${1:\"msg\"})", "onScreenDisplay.setActionBar(msg)"),
    ("title", "title(${1:\"title\"})", "onScreenDisplay.setTitle(msg)"),
    ("subtitle", "subtitle(${1:\"subtitle\"})", "onScreenDisplay.updateSubtitle(msg)"),
    ("send_message", "send_message(${1:\"msg\"})", "sendMessage(msg)"),
    ("play_sound", "play_sound(\"${1:random.orb}\")", "playSound(id)"),
    ("add_tag", "add_tag(\"${1:tag}\")", "addTag(tag)"),
    ("remove_tag", "remove_tag(\"${1:tag}\")", "removeTag(tag)"),
    ("has_tag", "has_tag(\"${1:tag}\")", "hasTag(tag)"),
    ("get_tags", "get_tags()", "getTags()"),
    ("apply_damage", "apply_damage(${1:4})", "applyDamage(amount)"),
    (
        "apply_impulse",
        "apply_impulse(${1:{ x = 0, y = 1, z = 0 }})",
        "applyImpulse(vec)",
    ),
    (
        "apply_knockback",
        "apply_knockback(${1:0}, ${2:0}, ${3:1}, ${4:0.5})",
        "applyKnockback(x, z, h, v)",
    ),
    ("run_command", "run_command(\"${1:say hi}\")", "runCommand(cmd)"),
    ("add_effect", "add_effect(\"${1:speed}\", ${2:100})", "addEffect(id, ticks, opts)"),
    ("remove_effect", "remove_effect(\"${1:speed}\")", "removeEffect(id)"),
    ("set_on_fire", "set_on_fire(${1:3}, true)", "setOnFire(seconds, useEffects)"),
    ("extinguish_fire", "extinguish_fire()", "extinguishFire()"),
    (
        "spawn_particle",
        "spawn_particle(\"${1:minecraft:totem_particle}\", ${2:loc})",
        "spawnParticle(id, loc)",
    ),
    (
        "spawn_entity",
        "spawn_entity(\"${1:minecraft:zombie}\", ${2:loc})",
        "spawnEntity(id, loc)",
    ),
    (
        "spawn_item",
        "spawn_item(\"${1:minecraft:diamond}\", ${2:loc}, ${3:1})",
        "__pstdr_spawn_item(dim, itemId, loc, amount?)",
    ),
    (
        "give_item",
        "give_item(\"${1:minecraft:diamond}\", ${2:1})",
        "__pstdr_give_item(entity, itemId, amount?, nameTag?)",
    ),
    (
        "get_score",
        "get_score(\"${1:objective}\")",
        "__pstdr_get_score(target, objective)",
    ),
    (
        "set_score",
        "set_score(\"${1:objective}\", ${2:0})",
        "__pstdr_set_score(target, objective, value)",
    ),
    (
        "add_score",
        "add_score(\"${1:objective}\", ${2:1})",
        "__pstdr_add_score(target, objective, delta)",
    ),
    (
        "get_entities",
        "get_entities(${1:{ maxDistance = 8 }})",
        "getEntities(opts)",
    ),
    (
        "get_players",
        "get_players(${1:{ maxDistance = 8 }})",
        "getPlayers(opts)",
    ),
    ("teleport", "teleport(${1:loc})", "teleport(loc)"),
    ("kill", "kill()", "kill()"),
    ("players", "players()", "world.getAllPlayers()"),
];

const PSTDR_PREDICATES: &[(&str, &str)] = &[
    ("sneaking", "subject.isSneaking"),
    ("sprinting", "subject.isSprinting"),
    ("swimming", "subject.isSwimming"),
    ("flying", "subject.isFlying"),
    ("sleeping", "subject.isSleeping"),
    ("on_ground", "subject.isOnGround"),
    ("in_water", "subject.isInWater"),
    ("holding", "holding \"namespace:item\""),
];

const PSTDR_TOP_LEVEL_SNIPPETS: &[(&str, &str, &str)] = &[
    (
        "@addon",
        "@addon(target = \"26.60\", api = \"2.11.0\")",
        "Target Bedrock & Script API directive",
    ),
    (
        "bind_target",
        "bind_target(\"BP/items/${1:item}.json\")",
        "Bind a BP item or block JSON file",
    ),
    ("const", "const ${1:NAME} = ${2:value}", "Immutable binding"),
    ("mut", "mut ${1:counter} = ${2:0}", "Mutable binding"),
    (
        "state",
        "state ${1:mana}[player] = ${2:100}",
        "Persistent DynamicProperty state",
    ),
    (
        "func",
        "func ${1:helper}(${2:player}) {\n    $0\n}",
        "Safe .pstdr function",
    ),
    (
        "unsafe",
        "unsafe ${1:raw_fn}(${2:arg}) {\n    $0\n}",
        "Isolated raw JavaScript function",
    ),
    (
        "mcfunction",
        "mcfunction \"${1:folder/my_func}\" {\n    $0\n}",
        "Inline .mcfunction block emitted to BP/functions/<path>.mcfunction",
    ),
    (
        "on join",
        "on join of ${1:player} {\n    $0\n}",
        "Player initial join event",
    ),
    (
        "on leave",
        "on leave of ${1:player} {\n    $0\n}",
        "Player leave event (beforeEvents, auto system.run)",
    ),
    (
        "on chat",
        "on chat ${1:msg} by ${2:player} {\n    $0\n}",
        "[Beta API only — requires api = \"...-beta\"] Chat send event (cancellable beforeEvents, auto system.run)",
    ),
    (
        "on right click",
        "on right click with ${1:\"example:wand\"} by ${2:player} {\n    $0\n}",
        "Item right-click / use event",
    ),
    (
        "on hit",
        "on hit ${1:target} with ${2:\"example:wand\"} by ${3:player} {\n    $0\n}",
        "Entity hit with item event",
    ),
    (
        "on break",
        "on break of ${1:\"minecraft:stone\"} by ${2:player} {\n    $0\n}",
        "Block break event (cancellable)",
    ),
    (
        "on hurt",
        "on hurt ${1:target} by ${2:attacker} {\n    $0\n}",
        "Entity hurt event",
    ),
    (
        "on die",
        "on die of ${1:victim} by ${2:killer} {\n    $0\n}",
        "Entity death event",
    ),
    (
        "on consume",
        "on consume ${1:\"example:potion\"} by ${2:player} {\n    $0\n}",
        "Item consume completion event",
    ),
    (
        "on interact",
        "on interact of ${1:\"example:altar\"} by ${2:player} {\n    $0\n}",
        "Block interact event (cancellable or custom component)",
    ),
    (
        "on tick",
        "on tick of ${1:\"example:altar\"} {\n    $0\n}",
        "Custom block tick event",
    ),
    (
        "on projectile hit",
        "on projectile hit ${1:victim} with ${2:\"example:spear\"} by ${3:shooter} {\n    $0\n}",
        "Projectile hit entity event",
    ),
    (
        "on spawn",
        "on spawn of ${1:entity} type ${2:\"minecraft:zombie\"} {\n    $0\n}",
        "Entity spawn event",
    ),
    (
        "on script_event",
        "on script_event \"${1:my_pack:my_event}\" {\n    $0\n}",
        "Script event receiver (system.afterEvents.scriptEventReceive)",
    ),
    (
        "on entity_trigger",
        "on entity_trigger \"${1:my_pack:on_trigger}\" for \"${2:my_pack:boss}\" {\n    $0\n}",
        "Data-driven entity trigger event (world.afterEvents.dataDrivenEntityTrigger)",
    ),
    (
        "after",
        "after ${1:2s} {\n    $0\n}",
        "One-shot delayed timer (system.runTimeout)",
    ),
    (
        "every",
        "every ${1:1s} {\n    $0\n}",
        "Repeating interval timer (system.runInterval)",
    ),
    (
        "command",
        "command \"${1:say Hello}\"",
        "Run a Bedrock command on the overworld dimension (sync runCommand)",
    ),
    (
        "command on",
        "command on ${1:player} \"${2:particle minecraft:totem_particle ~ ~1 ~}\"",
        "Run a Bedrock command on a specific entity or dimension",
    ),
    (
        "run_fn",
        "run_fn \"${1:folder/my_func}\"",
        "Execute a BP/functions/<path>.mcfunction (validated by PSTDR-E017)",
    ),
    (
        "run_fn on",
        "run_fn on ${1:player} \"${2:folder/my_func}\"",
        "Execute a BP/functions/<path>.mcfunction as a target entity or dimension",
    ),
    (
        "spawn particle",
        "spawn particle \"${1:minecraft:totem_particle}\" at ${2:player}",
        "Spawn particle at entity or coordinates",
    ),
    (
        "apply",
        "apply ${1:speed} ${2:1} to ${3:player} for ${4:5s}",
        "Apply 1-based status effect",
    ),
    ("damage", "damage ${1:target} by ${2:4}", "Deal damage to target entity"),
    ("ignite", "ignite ${1:target} for ${2:3s}", "Set target entity on fire"),
    (
        "send",
        "send \"${1:message}\" to ${2:player}",
        "Send chat message to player",
    ),
    (
        "cancel event",
        "cancel event",
        "Cancel a cancellable beforeEvent",
    ),
];

fn is_pstdr_document(rel_path: &str, language_id: &str) -> bool {
    language_id.eq_ignore_ascii_case("pstdr")
        || rel_path.to_ascii_lowercase().ends_with(".pstdr")
}

/// Converts a `pstdr-lang` `Diagnostic` into a PackStudio `LspDiagnosticDto` with 1-based ranges.
#[must_use]
pub fn pstdr_diag_to_lsp_dto(d: &PstdrDiagnostic, fallback_rel_path: &str) -> LspDiagnosticDto {
    let rel_path = if d.file.trim().is_empty() {
        fallback_rel_path.to_string()
    } else {
        d.file.replace('\\', "/")
    };
    let line = d.line.max(1) as u32;
    let col = d.column.max(1) as u32;
    let end_col = (d.end_column.max(d.column) + 1).max(col as usize + 1) as u32;
    let severity = match d.severity {
        PstdrSeverity::Error => WarningSeverity::Error,
        PstdrSeverity::Warning => WarningSeverity::Warning,
    };
    let message = if d.solution.trim().is_empty() {
        format!("[{}] {}", d.code, d.message)
    } else {
        format!("[{}] {} — Help: {}", d.code, d.message, d.solution)
    };
    LspDiagnosticDto {
        server_id: String::from(BUILTIN_PSTDR_SERVER_ID),
        rel_path,
        line,
        column: col,
        end_line: line,
        end_column: end_col,
        severity,
        code: Some(d.code.clone()),
        message,
        source: String::from("pstdr"),
    }
}

/// Converts a `pstdr-lang` `Diagnostic` into `rp_core::ConversionWarning` for Audit / Problems.
#[must_use]
pub fn pstdr_diag_to_conversion_warning(d: &PstdrDiagnostic) -> ConversionWarning {
    let rel = d.file.replace('\\', "/");
    let severity = match d.severity {
        PstdrSeverity::Error => WarningSeverity::Error,
        PstdrSeverity::Warning => WarningSeverity::Warning,
    };
    ConversionWarning {
        source_path: rel.clone(),
        stage: PipelineStage::Audit,
        severity,
        reason: format!("[{}] {} (L{}:{})", d.code, d.message, d.line.max(1), d.column.max(1)),
        actionable_fix: d.solution.clone(),
        asset: rel,
        message: format!("[{}] {}", d.code, d.message),
    }
}

/// In-process `builtin.pstdr` diagnostics analyzer powered by `pstdr-lang`.
#[must_use]
pub fn builtin_pstdr_diagnostics(
    rel_path: &str,
    content: &str,
    workspace_root: Option<&Path>,
) -> Vec<LspDiagnosticDto> {
    let spec = BedrockSpec::bundled();
    let (program, mut diags) = parse_source(content, rel_path);
    if let Some(prog) = &program {
        let ctx = ValidationContext {
            workspace_root,
            spec: &spec,
            strict_vanilla_ids: true,
        };
        let (_info, val_diags) = validate_program(prog, &ctx);
        diags.extend(val_diags);
    }
    diags.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then_with(|| a.column.cmp(&b.column))
            .then_with(|| a.code.cmp(&b.code))
    });
    diags.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.code == b.code);
    let out: Vec<LspDiagnosticDto> = diags
        .iter()
        .map(|d| pstdr_diag_to_lsp_dto(d, rel_path))
        .collect();
    if crate::is_trace_enabled() {
        eprintln!(
            "[pack-studio:rust:trace] [builtin.pstdr] diagnostics for '{}' -> {} item(s)",
            rel_path,
            out.len()
        );
    }
    out
}

fn char_col_1based_to_byte_offset(line_text: &str, col_1based: u32) -> usize {
    let target_chars = col_1based.saturating_sub(1) as usize;
    line_text
        .char_indices()
        .nth(target_chars)
        .map_or(line_text.len(), |(idx, _)| idx)
}

fn byte_offset_to_char_col_1based(line_text: &str, byte_offset: usize) -> u32 {
    let clamped = byte_offset.min(line_text.len());
    (line_text[..clamped].chars().count() + 1) as u32
}

/// In-process `builtin.pstdr` contextual completion powered by `pstdr-lang`.
#[must_use]
pub fn builtin_pstdr_completion(
    content: &str,
    line: u32,
    character: u32,
    prefix: &str,
) -> Vec<LspCompletionItemDto> {
    let spec = BedrockSpec::bundled();
    let line_text = content.lines().nth(line.saturating_sub(1) as usize).unwrap_or("");
    let byte_idx = char_col_1based_to_byte_offset(line_text, character);
    let line_prefix = &line_text[..byte_idx];
    let q = prefix.to_ascii_lowercase();

    // 1. Inside play_sound("...
    if pstdr_is_inside_play_sound(line_prefix) {
        let mut sounds: Vec<&String> = spec.sounds.iter().collect();
        sounds.sort();
        return sounds
            .into_iter()
            .filter(|s| q.is_empty() || s.to_ascii_lowercase().contains(&q))
            .take(120)
            .map(|s| LspCompletionItemDto {
                label: s.clone(),
                detail: String::from("Vanilla Bedrock sound ID"),
                documentation: format!("Vanilla sound `{s}` from Bedrock spec {}", spec.bedrock_version),
                insert_text: s.clone(),
                kind: String::from("Property"),
            })
            .collect();
    }

    // 2. After `apply `
    if pstdr_is_after_apply(line_prefix) {
        let mut effects: Vec<&String> = spec.effects.iter().collect();
        effects.sort();
        return effects
            .into_iter()
            .filter(|e| q.is_empty() || e.to_ascii_lowercase().contains(&q))
            .map(|e| LspCompletionItemDto {
                label: e.clone(),
                detail: String::from("Vanilla Bedrock status effect"),
                documentation: format!("Status effect `{e}` (1-based level in .pstdr)"),
                insert_text: e.clone(),
                kind: String::from("Property"),
            })
            .collect();
    }

    // 3. After `is ` or `is not `
    if pstdr_is_after_predicate(line_prefix) {
        return PSTDR_PREDICATES
            .iter()
            .filter(|(label, _)| q.is_empty() || label.contains(&q))
            .map(|(label, detail)| LspCompletionItemDto {
                label: (*label).to_string(),
                detail: (*detail).to_string(),
                documentation: format!("Predicate `{label}` → `{detail}`"),
                insert_text: (*label).to_string(),
                kind: String::from("Keyword"),
            })
            .collect();
    }

    // 4. After `.` or `?.`
    if pstdr_is_after_member_dot(line_prefix) {
        let mut items = Vec::with_capacity(PSTDR_MEMBER_PROPERTIES.len() + PSTDR_MEMBER_METHODS.len());
        for (prop, api) in PSTDR_MEMBER_PROPERTIES {
            if q.is_empty() || prop.contains(&q) {
                items.push(LspCompletionItemDto {
                    label: (*prop).to_string(),
                    detail: format!("-> {api}"),
                    documentation: format!("Bedrock snake_case property `.{prop}` → `.{api}`"),
                    insert_text: (*prop).to_string(),
                    kind: String::from("Property"),
                });
            }
        }
        for (name, snippet, api) in PSTDR_MEMBER_METHODS {
            if q.is_empty() || name.contains(&q) {
                items.push(LspCompletionItemDto {
                    label: (*name).to_string(),
                    detail: format!("-> {api}"),
                    documentation: format!("Bedrock snake_case method `.{name}(...)` → `{api}`"),
                    insert_text: (*snippet).to_string(),
                    kind: String::from("Function"),
                });
            }
        }
        return items;
    }

    // 5. Top-level / general keywords, events, and actions
    PSTDR_TOP_LEVEL_SNIPPETS
        .iter()
        .filter(|(label, _, _)| q.is_empty() || label.to_ascii_lowercase().contains(&q))
        .map(|(label, snippet, detail)| LspCompletionItemDto {
            label: (*label).to_string(),
            detail: (*detail).to_string(),
            documentation: (*detail).to_string(),
            insert_text: (*snippet).to_string(),
            kind: String::from("Keyword"),
        })
        .collect()
}

fn pstdr_is_inside_play_sound(prefix: &str) -> bool {
    for needle in ["play_sound(\"", "playSound(\""] {
        if let Some(pos) = prefix.rfind(needle) {
            let after = &prefix[pos + needle.len()..];
            if !after.contains('"') {
                return true;
            }
        }
    }
    false
}

fn pstdr_is_after_apply(prefix: &str) -> bool {
    let trimmed_start = prefix.trim_start();
    if let Some(rest) = trimmed_start.strip_prefix("apply ") {
        return !rest.trim_start().contains(char::is_whitespace);
    }
    false
}

fn pstdr_is_after_predicate(prefix: &str) -> bool {
    let trimmed = prefix.trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_');
    trimmed.ends_with(" is ") || trimmed.ends_with(" is not ")
}

fn pstdr_is_after_member_dot(prefix: &str) -> bool {
    let trimmed = prefix.trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_');
    trimmed.ends_with('.')
}

fn pstdr_describe_duration(token: &str) -> Option<String> {
    let (num_str, unit) = if let Some(n) = token.strip_suffix("ms") {
        (n, "ms")
    } else if let Some(n) = token.strip_suffix('s') {
        (n, "s")
    } else if let Some(n) = token.strip_suffix('m') {
        (n, "m")
    } else {
        let n = token.strip_suffix('t')?;
        (n, "t")
    };
    if num_str.is_empty() || !num_str.chars().next()?.is_ascii_digit() {
        return None;
    }
    let val: f64 = num_str.parse().ok()?;
    let ms = match unit {
        "ms" => val,
        "s" => val * 1000.0,
        "m" => val * 60_000.0,
        "t" => val * 50.0,
        _ => return None,
    };
    let ticks = (ms / 50.0).round() as u64;
    Some(format!(
        "**Duration Literal**\n\n`{token}` = **{ticks} ticks** ({} ms)",
        if ms.fract() == 0.0 {
            format!("{}", ms as i64)
        } else {
            format!("{ms}")
        }
    ))
}

fn pstdr_extract_token_at(line: &str, byte_idx: usize) -> Option<(&str, usize)> {
    if line.is_empty() {
        return None;
    }
    let mut idx = byte_idx.min(line.len());
    if idx == line.len() && idx > 0 {
        idx -= 1;
    }
    while idx > 0 && !line.is_char_boundary(idx) {
        idx -= 1;
    }

    let is_dur_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.';
    let mut dur_start = idx;
    while dur_start > 0 {
        let prev = line[..dur_start].chars().next_back()?;
        if is_dur_char(prev) {
            dur_start -= prev.len_utf8();
        } else {
            break;
        }
    }
    let mut dur_end = idx;
    while dur_end < line.len() {
        let next = line[dur_end..].chars().next()?;
        if is_dur_char(next) {
            dur_end += next.len_utf8();
        } else {
            break;
        }
    }
    let dur_cand = &line[dur_start..dur_end];
    if pstdr_describe_duration(dur_cand).is_some() {
        return Some((dur_cand, dur_start));
    }

    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut start = idx;
    if !line[start..].chars().next().is_some_and(is_word) {
        if start == 0 {
            return None;
        }
        let prev = line[..start].chars().next_back()?;
        if !is_word(prev) {
            return None;
        }
        start -= prev.len_utf8();
    }
    while start > 0 {
        let prev = line[..start].chars().next_back()?;
        if is_word(prev) {
            start -= prev.len_utf8();
        } else {
            break;
        }
    }
    let mut end = idx;
    while end < line.len() {
        let next = line[end..].chars().next()?;
        if is_word(next) {
            end += next.len_utf8();
        } else {
            break;
        }
    }
    if start < end {
        Some((&line[start..end], start))
    } else {
        None
    }
}

/// In-process `builtin.pstdr` hover documentation powered by `pstdr-lang`.
#[must_use]
pub fn builtin_pstdr_hover(
    content: &str,
    line: u32,
    character: u32,
    fallback_word: &str,
) -> Option<LspHoverDto> {
    let mk = |contents: String| LspHoverDto {
        contents,
        source: String::from("PSTDR Language Server (Built-in)"),
    };

    let line_text = content.lines().nth(line.saturating_sub(1) as usize).unwrap_or("");
    let byte_idx = char_col_1based_to_byte_offset(line_text, character);
    let (word, start_byte) = pstdr_extract_token_at(line_text, byte_idx)
        .unwrap_or((fallback_word.trim(), 0));
    if word.is_empty() {
        return None;
    }

    // 1. Duration literal (e.g. `3s`, `500ms`, `2m`, `10t`)
    if let Some(dur_md) = pstdr_describe_duration(word) {
        return Some(mk(dur_md));
    }

    // 2. Member property or method after `.` / `?.`
    let before = line_text.get(..start_byte).unwrap_or("").trim_end();
    if before.ends_with('.') {
        let is_world = before
            .trim_end_matches('.')
            .trim_end_matches('?')
            .trim_end()
            .ends_with("world");
        if is_world
            && let Some(mapped) = map_world_method(word)
        {
            return Some(mk(format!(
                "**World Helper**\n\n`world.{word}(...)` → `world.{mapped}(...)`"
            )));
        }
        match word {
            "give_item" => {
                return Some(mk(String::from(
                    "**Safe ItemStack Helper**\n\n`.give_item(item_id, amount?, name_tag?)` → `__pstdr_give_item(entity, itemId, amount = 1, nameTag = null)`\n\nConstructs `new ItemStack(...)` safely and inserts it into `minecraft:inventory`.",
                )));
            }
            "spawn_item" => {
                return Some(mk(String::from(
                    "**Safe ItemStack Helper**\n\n`.spawn_item(item_id, loc_or_entity, amount?)` → `__pstdr_spawn_item(dim, itemId, loc, amount = 1)`\n\nConstructs `new ItemStack(...)` when given a string ID and spawns it in the dimension.",
                )));
            }
            "get_score" => {
                return Some(mk(String::from(
                    "**Scoreboard Helper**\n\n`.get_score(objective)` → `__pstdr_get_score(target, objective)`\n\nReturns the participant's score as a `number`, defaulting to `0` if missing.",
                )));
            }
            "set_score" => {
                return Some(mk(String::from(
                    "**Scoreboard Helper**\n\n`.set_score(objective, value)` → `__pstdr_set_score(target, objective, value)`\n\nSets the participant's score, creating the objective if missing.",
                )));
            }
            "add_score" => {
                return Some(mk(String::from(
                    "**Scoreboard Helper**\n\n`.add_score(objective, delta)` → `__pstdr_add_score(target, objective, delta)`\n\nAdds `delta` to the participant's score, creating the objective if missing.",
                )));
            }
            _ => {}
        }
        if let Some(mapped) = map_method(word) {
            return Some(mk(format!(
                "**Bedrock Method Helper**\n\n`.{word}(...)` → `.{mapped}(...)`"
            )));
        }
        if let Some(mapped) = map_property(word) {
            return Some(mk(format!(
                "**Bedrock Property Helper**\n\n`.{word}` → `.{mapped}`"
            )));
        }
        if let Some(mapped) = map_item_property(word) {
            return Some(mk(format!(
                "**ItemStack Property Helper**\n\n`.{word}` → `.{mapped}`"
            )));
        }
    }

    // 3. Declared `state` variable or `state` keyword
    let state_decl_prefix = format!("state {word}[");
    let is_declared_state = content
        .lines()
        .any(|l| l.trim_start().starts_with(&state_decl_prefix));
    if word == "state" || is_declared_state {
        let name = if word == "state" { "<name>" } else { word };
        return Some(mk(format!(
            "**Persistent State (`DynamicProperty`)**\n\nReads compile to `owner.getDynamicProperty(\"pstdr:{name}\")`.\n\nWrites compile to `owner.setDynamicProperty(\"pstdr:{name}\", value)`."
        )));
    }

    // 4. Core DSL keywords
    let kw_doc = match word {
        "apply" => Some(
            "**`apply <effect> <level> to <target> for <duration>`**\n\nApplies a status effect using 1-based levels (automatically converted to Bedrock 0-based `amplifier`).",
        ),
        "cancel" => Some(
            "**`cancel event`**\n\nValid inside `on right click`, `on break`, `on interact`, and `on chat` *(Beta API only)*. Automatically switches to `beforeEvents` and schedules mutations in `system.run(...)`.",
        ),
        "chat" => Some(
            "**`on chat <msg> by <player> { ... }`** `[Beta API only — requires api = \"...-beta\"]`\n\nCompiles to `world.beforeEvents.chatSend` (cancellable, auto `system.run(...)` for mutations). Requires `@addon(target = \"26.60\", api = \"2.11.0-beta\")` and the **Beta APIs** world experiment (`PSTDR-E018` on stable APIs).",
        ),
        "every" => Some(
            "**`every <duration> { ... }`**\n\nCompiles to `system.runInterval(..., ticks)`. Minimum interval is `50ms` (`1` tick).",
        ),
        "after" => Some(
            "**`after <duration> { ... }`**\n\nCompiles to `system.runTimeout(..., ticks)`.",
        ),
        "unsafe" => Some(
            "**`unsafe`**\n\nIsolates raw JavaScript inside a `.pstdr` file without DSL keyword restrictions.",
        ),
        "bind_target" => Some(
            "**`bind_target(\"BP/...\")`**\n\nExplicitly binds a Behavior Pack item or block JSON file for automatic custom component wiring.",
        ),
        "command" => Some(
            "**`command [on <target>] \"<cmd>\"`**\n\nExecutes a synchronous Bedrock command via `.runCommand(...)` (on `world.getDimension(\"overworld\")` by default, or on `<target>`). Automatically deferred via `system.run(...)` inside `beforeEvents` handlers.",
        ),
        "run_fn" => Some(
            "**`run_fn [on <target>] \"<path>\"`**\n\nCompiles to `.runCommand(\"function <path>\")`. Validated against `BP/functions/<path>.mcfunction` and inline `mcfunction` blocks (`PSTDR-E017`). Automatically deferred via `system.run(...)` inside `beforeEvents` handlers.",
        ),
        "mcfunction" => Some(
            "**`mcfunction \"<path>\" { ... }`**\n\nTop-level inline `.mcfunction` declaration emitted to `BP/functions/<path>.mcfunction` during workspace conversion (stripping `//` comments and blank lines).",
        ),
        "script_event" => Some(
            "**`on script_event \"<ns>:<id>\" { ... }`**\n\nCompiles to `system.afterEvents.scriptEventReceive.subscribe(..., { namespaces: [\"<ns>\"] })`. Exposes `event.id`, `event.message`, `source` (`event.sourceEntity`), and `source_block` (`event.sourceBlock`).",
        ),
        "entity_trigger" => Some(
            "**`on entity_trigger \"<event_id>\" [for \"<entity_type>\"] { ... }`**\n\nCompiles to `world.afterEvents.dataDrivenEntityTrigger.subscribe(..., { eventTypes: [\"<event_id>\"], ... })`. Exposes `entity` (`event.entity`) and `event_id` (`event.eventId`).",
        ),
        "const" => Some(
            "**`const <name> = <expr>`**\n\nDeclares an immutable binding (`PSTDR-E002` if reassigned).",
        ),
        "mut" => Some(
            "**`mut <name> = <expr>`**\n\nDeclares a mutable binding (compiles to JavaScript `let`).",
        ),
        "func" => Some(
            "**`func <name>(<params>) { ... }`**\n\nDeclares a safe `.pstdr` helper function.",
        ),
        "addon" => Some(
            "**`@addon(target = \"26.60\", api = \"2.11.0\")`**\n\nDeclares the target Minecraft Bedrock version and `@minecraft/server` Script API version.",
        ),
        _ => None,
    };

    kw_doc.map(|doc| mk(String::from(doc)))
}

/// In-process `builtin.pstdr` quick-fix code actions (`PSTDR-E001` & `PSTDR-E002`).
#[must_use]
pub fn builtin_pstdr_code_actions(
    rel_path: &str,
    content: &str,
    start_line: u32,
    end_line: u32,
    workspace_root: Option<&Path>,
) -> Vec<LspCodeActionDto> {
    let diags = builtin_pstdr_diagnostics(rel_path, content, workspace_root);
    let mut actions = Vec::new();

    for diag in &diags {
        if diag.end_line < start_line || diag.line > end_line {
            continue;
        }
        let Some(code) = diag.code.as_deref() else {
            continue;
        };
        match code {
            "PSTDR-E001" => {
                if let Some(action) = pstdr_fix_e001(content, diag) {
                    actions.push(action);
                }
            }
            "PSTDR-E002" => {
                if let Some(action) = pstdr_fix_e002(content, diag) {
                    actions.push(action);
                }
            }
            _ => {}
        }
    }

    actions
}

fn pstdr_fix_e001(content: &str, diag: &LspDiagnosticDto) -> Option<LspCodeActionDto> {
    let line_text = content.lines().nth(diag.line.saturating_sub(1) as usize)?;
    let start_b = char_col_1based_to_byte_offset(line_text, diag.column);
    let end_b = char_col_1based_to_byte_offset(line_text, diag.end_column);
    let snippet = line_text.get(start_b..end_b)?.trim();

    let (replacement, title) = match snippet {
        "let" => ("mut", "Replace 'let' with 'mut'"),
        "var" => ("mut", "Replace 'var' with 'mut'"),
        "function" => ("func", "Replace 'function' with 'func'"),
        "nil" => ("null", "Replace 'nil' with 'null'"),
        "~=" => ("!=", "Replace '~=' with '!='"),
        _ => return None,
    };

    Some(LspCodeActionDto {
        title: String::from(title),
        kind: String::from("quickfix"),
        diagnostic_code: Some(String::from("PSTDR-E001")),
        is_preferred: true,
        edits: vec![LspTextEditDto {
            line: diag.line,
            column: diag.column,
            end_line: diag.end_line,
            end_column: diag.end_column,
            new_text: String::from(replacement),
        }],
    })
}

fn pstdr_fix_e002(content: &str, diag: &LspDiagnosticDto) -> Option<LspCodeActionDto> {
    let marker = "on line ";
    let idx = diag.message.find(marker)?;
    let rest = &diag.message[idx + marker.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let decl_line: u32 = digits.parse().ok()?;
    let line_text = content.lines().nth(decl_line.saturating_sub(1) as usize)?;

    let trimmed = line_text.trim_start();
    if !(trimmed.starts_with("const ") || trimmed.starts_with("const\t")) {
        return None;
    }
    let const_byte = line_text.len() - trimmed.len();
    let start_col = byte_offset_to_char_col_1based(line_text, const_byte);
    let end_col = byte_offset_to_char_col_1based(line_text, const_byte + "const".len());

    Some(LspCodeActionDto {
        title: format!("Change 'const' to 'mut' on line {decl_line}"),
        kind: String::from("quickfix"),
        diagnostic_code: Some(String::from("PSTDR-E002")),
        is_preferred: true,
        edits: vec![LspTextEditDto {
            line: decl_line,
            column: start_col,
            end_line: decl_line,
            end_column: end_col,
            new_text: String::from("mut"),
        }],
    })
}

fn collect_configured_servers(extensions_root: &std::path::Path) -> Vec<LspContributionDto> {
    let snap = scan_extensions_snapshot(extensions_root);
    let persisted = load_persisted_state(extensions_root);
    let mut map: BTreeMap<String, LspContributionDto> = BTreeMap::new();

    for ext in snap.extensions {
        if !ext.enabled
            && ext.id != BUILTIN_MINECRAFT_EXT_ID
            && ext.id != BUILTIN_PSTDR_EXT_ID
        {
            continue;
        }
        for lsp in ext.lsp_contributions {
            map.insert(lsp.server_id.clone(), lsp);
        }
    }
    for custom in persisted.custom_lsp_servers {
        map.insert(custom.server_id.clone(), custom);
    }

    // Optional external `pstdr-lsp` binary: auto-detected if present on PATH.
    if find_in_path("pstdr-lsp").is_some() && !map.contains_key(EXTERNAL_PSTDR_SERVER_ID) {
        map.insert(
            String::from(EXTERNAL_PSTDR_SERVER_ID),
            LspContributionDto {
                server_id: String::from(EXTERNAL_PSTDR_SERVER_ID),
                display_name: String::from("PSTDR External LSP (pstdr-lsp on PATH)"),
                command: String::from("pstdr-lsp"),
                args: Vec::new(),
                language_ids: vec![String::from("pstdr")],
                builtin: false,
            },
        );
    }

    map.into_values().collect()
}

/// Tauri Command: List all built-in, extension-provided, and user-configured LSP servers.
#[tauri::command]
pub fn lsp_list_servers(state: tauri::State<'_, StudioState>) -> Vec<LspServerStatusDto> {
    let root = default_extensions_dir();
    let configured = collect_configured_servers(&root);
    let builtin_mc_on = state.builtin_lsp_enabled.load(Ordering::Relaxed);
    let builtin_pstdr_on = state.builtin_pstdr_lsp_enabled.load(Ordering::Relaxed);
    let running_guard = state.lsp_clients.lock().ok();

    configured
        .into_iter()
        .map(|cfg| {
            let running = if cfg.builtin {
                if cfg.server_id == BUILTIN_PSTDR_SERVER_ID {
                    builtin_pstdr_on
                } else {
                    builtin_mc_on
                }
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
    if sid == BUILTIN_PSTDR_SERVER_ID {
        state
            .builtin_pstdr_lsp_enabled
            .store(true, Ordering::Relaxed);
        return Ok(());
    }
    if sid == BUILTIN_MINECRAFT_SERVER_ID || command.trim() == "builtin" {
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
                        let code = d.get("code").and_then(|c| {
                            c.as_str()
                                .map(str::to_string)
                                .or_else(|| c.as_i64().map(|n| n.to_string()))
                        });
                        diags.push(LspDiagnosticDto {
                            server_id: sid_clone.clone(),
                            rel_path: rel_path.clone(),
                            line: start_line,
                            column: start_col,
                            end_line,
                            end_column: end_col,
                            severity: sev,
                            code,
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
                    "codeAction": {},
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
    if server_id == BUILTIN_MINECRAFT_SERVER_ID {
        state.builtin_lsp_enabled.store(false, Ordering::Relaxed);
        return Ok(());
    }
    if server_id == BUILTIN_PSTDR_SERVER_ID {
        state
            .builtin_pstdr_lsp_enabled
            .store(false, Ordering::Relaxed);
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

fn read_live_document_content(state: &StudioState, rel_path: &str) -> String {
    if let Ok(docs) = state.open_documents.lock()
        && let Some(text) = docs.get(rel_path)
    {
        return text.clone();
    }
    if let Ok(guard) = state.active_pack.lock()
        && let Some(pack) = guard.as_ref()
        && let Some(bytes) = pack.container.get(rel_path)
        && let Ok(text) = std::str::from_utf8(bytes)
    {
        return text.to_string();
    }
    String::new()
}

/// Tauri Command: Sync open/changed/saved document to matching stdio LSPs & Built-in LSPs.
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
    if let Ok(mut docs) = state.open_documents.lock() {
        docs.insert(rel_path.clone(), content.clone());
    }
    if crate::is_trace_enabled() {
        eprintln!(
            "[pack-studio:rust:trace] [lsp_sync_document] event='{}' lang='{}' path='{}' v={} ({} bytes)",
            event_kind,
            language_id,
            rel_path,
            version,
            content.len()
        );
    }

    let (root_path, root_str, known_files) = state
        .active_pack
        .lock()
        .ok()
        .map(|g| match g.as_ref() {
            Some(pack) => (
                pack.source_path.clone(),
                pack.source_path.as_ref().map(|p| p.display().to_string()),
                pack.container.files().keys().cloned().collect::<Vec<_>>(),
            ),
            None => (None, None, Vec::new()),
        })
        .unwrap_or((None, None, Vec::new()));

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

    if is_pstdr_document(&rel_path, &language_id) {
        let diags = if state.builtin_pstdr_lsp_enabled.load(Ordering::Relaxed) {
            let ws_root = root_path.as_deref().filter(|p| p.is_dir());
            builtin_pstdr_diagnostics(&rel_path, &content, ws_root)
        } else {
            Vec::new()
        };
        let _ = app.emit(
            "lsp://diagnostics",
            LspDiagnosticsEvent {
                server_id: String::from(BUILTIN_PSTDR_SERVER_ID),
                rel_path,
                diagnostics: diags.clone(),
            },
        );
        return diags;
    }

    let diags = if state.builtin_lsp_enabled.load(Ordering::Relaxed) {
        builtin_minecraft_diagnostics(&rel_path, &content, &known_files)
    } else {
        Vec::new()
    };

    let _ = app.emit(
        "lsp://diagnostics",
        LspDiagnosticsEvent {
            server_id: String::from(BUILTIN_MINECRAFT_SERVER_ID),
            rel_path,
            diagnostics: diags.clone(),
        },
    );

    diags
}

/// Tauri Command: Query completions from Built-in LSPs & matching external stdio LSPs.
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

    let mut out = if is_pstdr_document(&rel_path, &language_id) {
        if state.builtin_pstdr_lsp_enabled.load(Ordering::Relaxed) {
            let content = read_live_document_content(&state, &rel_path);
            builtin_pstdr_completion(&content, line, character, &prefix)
        } else {
            Vec::new()
        }
    } else if state.builtin_lsp_enabled.load(Ordering::Relaxed) {
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

/// Tauri Command: Query hover info from Built-in LSPs & matching external stdio LSPs.
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

    if is_pstdr_document(&rel_path, &language_id) {
        if state.builtin_pstdr_lsp_enabled.load(Ordering::Relaxed) {
            let content = read_live_document_content(&state, &rel_path);
            return builtin_pstdr_hover(&content, line, character, &word);
        }
        return None;
    }

    if state.builtin_lsp_enabled.load(Ordering::Relaxed) {
        return builtin_minecraft_hover(&rel_path, &word);
    }
    None
}

/// Tauri Command: Query quick-fix code actions from `builtin.pstdr` & external stdio LSPs.
#[tauri::command]
pub fn lsp_code_actions(
    state: tauri::State<'_, StudioState>,
    rel_path: String,
    language_id: String,
    start_line: u32,
    start_column: u32,
    end_line: u32,
    end_column: u32,
) -> Vec<LspCodeActionDto> {
    let (root_path, root_str) = state
        .active_pack
        .lock()
        .ok()
        .map(|g| match g.as_ref() {
            Some(pack) => (
                pack.source_path.clone(),
                pack.source_path.as_ref().map(|p| p.display().to_string()),
            ),
            None => (None, None),
        })
        .unwrap_or((None, None));

    let mut out = if is_pstdr_document(&rel_path, &language_id)
        && state.builtin_pstdr_lsp_enabled.load(Ordering::Relaxed)
    {
        let content = read_live_document_content(&state, &rel_path);
        let ws_root = root_path.as_deref().filter(|p| p.is_dir());
        builtin_pstdr_code_actions(&rel_path, &content, start_line, end_line, ws_root)
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
                "textDocument/codeAction",
                json!({
                    "textDocument": { "uri": uri },
                    "range": {
                        "start": { "line": start_line.saturating_sub(1), "character": start_column.saturating_sub(1) },
                        "end": { "line": end_line.saturating_sub(1), "character": end_column.saturating_sub(1) }
                    },
                    "context": { "diagnostics": [] }
                }),
                Duration::from_millis(500),
            ) && let Some(arr) = resp.get("result").and_then(Value::as_array)
            {
                for entry in arr {
                    let Some(title) = entry.get("title").and_then(Value::as_str) else {
                        continue;
                    };
                    let mut edits = Vec::new();
                    if let Some(changes) = entry
                        .pointer("/edit/changes")
                        .and_then(Value::as_object)
                    {
                        for (_doc_uri, edit_arr) in changes {
                            if let Some(list) = edit_arr.as_array() {
                                for te in list {
                                    let sl = te
                                        .pointer("/range/start/line")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0) as u32
                                        + 1;
                                    let sc = te
                                        .pointer("/range/start/character")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0) as u32
                                        + 1;
                                    let el = te
                                        .pointer("/range/end/line")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0) as u32
                                        + 1;
                                    let ec = te
                                        .pointer("/range/end/character")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0) as u32
                                        + 1;
                                    let new_text = te
                                        .get("newText")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .to_string();
                                    edits.push(LspTextEditDto {
                                        line: sl,
                                        column: sc,
                                        end_line: el,
                                        end_column: ec,
                                        new_text,
                                    });
                                }
                            }
                        }
                    }
                    if !edits.is_empty() {
                        out.push(LspCodeActionDto {
                            title: title.to_string(),
                            kind: entry
                                .get("kind")
                                .and_then(Value::as_str)
                                .unwrap_or("quickfix")
                                .to_string(),
                            diagnostic_code: None,
                            is_preferred: entry
                                .get("isPreferred")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                            edits,
                        });
                    }
                }
            }
        }
    }

    out
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
