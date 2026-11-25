use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use anyhow::Context;
use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::pstdr_sync::PACKSTUDIO_DISPLAY_VERSION;
use crate::state::StudioState;
use crate::workspace::find_in_path;

pub const AGENT_CLAUDE_CODE: &str = "claude-code";
pub const AGENT_ANTIGRAVITY: &str = "antigravity";
pub const AGENT_OPENCODE: &str = "opencode";

/// Maximum active file bytes embedded inside an AI context packet.
const MAX_CONTEXT_FILE_BYTES: usize = 16 * 1024;
/// Maximum active file lines embedded inside an AI context packet.
const MAX_CONTEXT_FILE_LINES: usize = 300;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomAgentConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedAiAgentsConfig {
    #[serde(default = "default_active_agent")]
    active_agent_id: String,
    #[serde(default)]
    custom_configs: BTreeMap<String, CustomAgentConfig>,
}

fn default_active_agent() -> String {
    String::from(AGENT_CLAUDE_CODE)
}

impl Default for PersistedAiAgentsConfig {
    fn default() -> Self {
        Self {
            active_agent_id: default_active_agent(),
            custom_configs: BTreeMap::new(),
        }
    }
}

/// Detection and configuration row returned inside `AiAgentsStatusDto`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AiAgentEntryDto {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub installed: bool,
    pub resolved_path: Option<String>,
    pub description: String,
}

/// Aggregate status returned by `ai_agents_status`, `ai_set_active_agent`, and `ai_configure_agent`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AiAgentsStatusDto {
    pub active_agent_id: String,
    pub agents: Vec<AiAgentEntryDto>,
}

/// Structured context packet returned by `ai_build_context_packet`.
#[derive(Debug, Clone, Serialize)]
pub struct AiContextPacketDto {
    pub agent_id: String,
    pub agent_name: String,
    pub suggested_command: String,
    pub truncated: bool,
    pub markdown: String,
}

struct BuiltinAgentDef {
    id: &'static str,
    name: &'static str,
    default_cmd: &'static str,
    fallback_cmds: &'static [&'static str],
    description: &'static str,
}

const BUILTIN_AGENTS: &[BuiltinAgentDef] = &[
    BuiltinAgentDef {
        id: AGENT_CLAUDE_CODE,
        name: "Claude Code",
        default_cmd: "claude",
        fallback_cmds: &[],
        description: "Anthropic CLI coding agent for editing .pstdr scripts, Bedrock JSON, and resource packs.",
    },
    BuiltinAgentDef {
        id: AGENT_ANTIGRAVITY,
        name: "Antigravity",
        default_cmd: "antigravity",
        fallback_cmds: &["agy"],
        description: "Google DeepMind Antigravity agentic coding CLI / IDE bridge.",
    },
    BuiltinAgentDef {
        id: AGENT_OPENCODE,
        name: "OpenCode",
        default_cmd: "opencode",
        fallback_cmds: &[],
        description: "Open-source terminal AI coding agent.",
    },
];

/// Returns `<config_dir>/pack-studio/ai-agents.json`.
#[must_use]
pub fn default_ai_agents_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("pack-studio")
        .join("ai-agents.json")
}

fn set_unix_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .with_context(|| format!("failed setting permissions {mode:o} on '{}'", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

fn load_persisted_config(path: &Path) -> PersistedAiAgentsConfig {
    fs::read_to_string(path)
        .ok()
        .and_then(|txt| serde_json::from_str::<PersistedAiAgentsConfig>(&txt).ok())
        .unwrap_or_default()
}

fn save_persisted_config(path: &Path, cfg: &PersistedAiAgentsConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        set_unix_mode(parent, 0o700)?;
    }
    let bytes = serde_json::to_vec_pretty(cfg)?;
    let mut f = File::create(path)?;
    f.write_all(&bytes)?;
    f.sync_all().ok();
    set_unix_mode(path, 0o600)?;
    Ok(())
}

fn validate_agent_id(agent_id: &str) -> Result<&'static BuiltinAgentDef> {
    let trimmed = agent_id.trim();
    BUILTIN_AGENTS
        .iter()
        .find(|def| def.id == trimmed)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Unsupported AI agent id '{trimmed}'. Expected one of: claude-code, antigravity, opencode."
            )
        })
}

fn resolve_executable(cmd: &str, fallbacks: &[&str]) -> Option<PathBuf> {
    let trimmed = cmd.trim();
    if !trimmed.is_empty() {
        let p = Path::new(trimmed);
        if p.is_file() {
            return Some(p.to_path_buf());
        }
        if let Some(found) = find_in_path(trimmed) {
            return Some(found);
        }
    }
    for fb in fallbacks {
        if let Some(found) = find_in_path(fb) {
            return Some(found);
        }
    }
    None
}

/// Scans installed AI agents and merges persisted preferences from `config_path`.
#[must_use]
pub fn build_ai_agents_status(config_path: &Path) -> AiAgentsStatusDto {
    let cfg = load_persisted_config(config_path);
    let active_agent_id = if BUILTIN_AGENTS.iter().any(|d| d.id == cfg.active_agent_id) {
        cfg.active_agent_id
    } else {
        default_active_agent()
    };

    let agents = BUILTIN_AGENTS
        .iter()
        .map(|def| {
            let custom = cfg.custom_configs.get(def.id);
            let cmd = custom
                .map(|c| c.command.trim())
                .filter(|c| !c.is_empty())
                .unwrap_or(def.default_cmd)
                .to_string();
            let args = custom.map(|c| c.args.clone()).unwrap_or_default();
            let fallbacks = if custom.is_some() {
                &[][..]
            } else {
                def.fallback_cmds
            };
            let resolved = resolve_executable(&cmd, fallbacks);
            AiAgentEntryDto {
                id: def.id.to_string(),
                name: def.name.to_string(),
                command: cmd,
                args,
                installed: resolved.is_some(),
                resolved_path: resolved.map(|p| p.display().to_string()),
                description: def.description.to_string(),
            }
        })
        .collect();

    AiAgentsStatusDto {
        active_agent_id,
        agents,
    }
}

/// Persists active AI coding agent ID in `config_path`.
pub fn set_active_agent_at_path(config_path: &Path, agent_id: &str) -> Result<AiAgentsStatusDto> {
    let def = validate_agent_id(agent_id)?;
    let mut cfg = load_persisted_config(config_path);
    cfg.active_agent_id = def.id.to_string();
    save_persisted_config(config_path, &cfg)?;
    Ok(build_ai_agents_status(config_path))
}

/// Persists custom command/args override for an AI coding agent in `config_path`.
pub fn configure_agent_at_path(
    config_path: &Path,
    agent_id: &str,
    command: &str,
    args: Vec<String>,
) -> Result<AiAgentsStatusDto> {
    let def = validate_agent_id(agent_id)?;
    let mut cfg = load_persisted_config(config_path);
    let clean_cmd = command.trim();
    if clean_cmd.is_empty() && args.is_empty() {
        cfg.custom_configs.remove(def.id);
    } else {
        cfg.custom_configs.insert(
            def.id.to_string(),
            CustomAgentConfig {
                command: if clean_cmd.is_empty() {
                    def.default_cmd.to_string()
                } else {
                    clean_cmd.to_string()
                },
                args,
            },
        );
    }
    save_persisted_config(config_path, &cfg)?;
    Ok(build_ai_agents_status(config_path))
}

fn bound_file_excerpt(raw: &str) -> (String, bool) {
    let mut out = String::new();
    let mut truncated = false;
    for (idx, line) in raw.lines().enumerate() {
        if idx >= MAX_CONTEXT_FILE_LINES {
            truncated = true;
            break;
        }
        if out.len().saturating_add(line.len() + 1) > MAX_CONTEXT_FILE_BYTES {
            truncated = true;
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    if truncated {
        out.push_str("// ... [truncated to keep AI context packet bounded]\n");
    }
    (out, truncated)
}

fn fence_language_for_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".pstdr") {
        "pstdr"
    } else if lower.ends_with(".mcfunction") {
        "mcfunction"
    } else if lower.ends_with(".json") || lower.ends_with(".mcmeta") {
        "json"
    } else if lower.ends_with(".ts") {
        "typescript"
    } else if lower.ends_with(".js") {
        "javascript"
    } else if lower.ends_with(".toml") {
        "toml"
    } else {
        "text"
    }
}

/// Core pure builder for the structured Markdown AI context packet.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn build_context_packet_core(
    status: &AiAgentsStatusDto,
    workspace_path: Option<&str>,
    pack_name: Option<&str>,
    pack_type_label: Option<&str>,
    edition_label: Option<&str>,
    active_file: Option<&str>,
    active_file_content: Option<&str>,
    diagnostics_summary: Option<&str>,
) -> AiContextPacketDto {
    let active_agent = status
        .agents
        .iter()
        .find(|a| a.id == status.active_agent_id)
        .unwrap_or(&status.agents[0]);

    let cmd_base = if active_agent.args.is_empty() {
        active_agent.command.clone()
    } else {
        format!("{} {}", active_agent.command, active_agent.args.join(" "))
    };
    let suggested_command = match workspace_path.map(str::trim).filter(|s| !s.is_empty()) {
        Some(ws) => format!("cd \"{ws}\" && {cmd_base}"),
        None => cmd_base,
    };

    let mut md = String::new();
    md.push_str("# PackStudio Workspace Context Packet\n\n");
    md.push_str(&format!("- **PackStudio Version:** `{PACKSTUDIO_DISPLAY_VERSION}`\n"));
    md.push_str(&format!(
        "- **Selected AI Agent:** {} (`{}`)\n",
        active_agent.name, active_agent.id
    ));
    md.push_str(&format!(
        "- **Workspace Path:** `{}`\n",
        workspace_path.unwrap_or("(no folder open)")
    ));
    if let Some(name) = pack_name {
        md.push_str(&format!("- **Pack Name:** `{name}`\n"));
    }
    if let Some(pt) = pack_type_label {
        md.push_str(&format!("- **Pack Type:** `{pt}`\n"));
    }
    if let Some(ed) = edition_label {
        md.push_str(&format!("- **Target / Edition:** `{ed}`\n"));
    }
    if let Some(af) = active_file.map(str::trim).filter(|s| !s.is_empty()) {
        md.push_str(&format!("- **Active File:** `{af}`\n"));
    }
    md.push_str("- **Reference Guides:** `PACKSTUDIO_LLMS.md`, `docs/PACKSTUDIO_AI_GUIDE.md`\n");

    let mut truncated = false;
    if let Some(diag) = diagnostics_summary.map(str::trim).filter(|s| !s.is_empty()) {
        md.push_str("\n## Active Diagnostics\n\n```text\n");
        let (bounded_diag, diag_trunc) = bound_file_excerpt(diag);
        truncated |= diag_trunc;
        md.push_str(&bounded_diag);
        md.push_str("```\n");
    }

    if let (Some(af), Some(content)) = (
        active_file.map(str::trim).filter(|s| !s.is_empty()),
        active_file_content,
    ) {
        let lang = fence_language_for_path(af);
        let (excerpt, file_trunc) = bound_file_excerpt(content);
        truncated |= file_trunc;
        md.push_str(&format!("\n## Active File Content (`{af}`)\n\n```{lang}\n"));
        md.push_str(&excerpt);
        if !excerpt.ends_with('\n') {
            md.push('\n');
        }
        md.push_str("```\n");
    }

    AiContextPacketDto {
        agent_id: active_agent.id.clone(),
        agent_name: active_agent.name.clone(),
        suggested_command,
        truncated,
        markdown: md,
    }
}

/// Tauri Command: Detect `claude`, `antigravity`, and `opencode` on `PATH` and return active selection.
#[tauri::command]
pub fn ai_agents_status() -> AiAgentsStatusDto {
    build_ai_agents_status(&default_ai_agents_config_path())
}

/// Tauri Command: Persist preferred AI coding agent (`claude-code` | `antigravity` | `opencode`).
#[tauri::command]
pub fn ai_set_active_agent(agent_id: String) -> Result<AiAgentsStatusDto, String> {
    set_active_agent_at_path(&default_ai_agents_config_path(), &agent_id)
        .map_err(|e| format!("{e:#}"))
}

/// Tauri Command: Persist custom command & arguments for an AI coding agent.
#[tauri::command]
pub fn ai_configure_agent(
    agent_id: String,
    command: String,
    args: Vec<String>,
) -> Result<AiAgentsStatusDto, String> {
    configure_agent_at_path(&default_ai_agents_config_path(), &agent_id, &command, args)
        .map_err(|e| format!("{e:#}"))
}

/// Tauri Command: Build a structured Markdown context packet for the active workspace, file, and diagnostics.
#[tauri::command]
pub fn ai_build_context_packet(
    state: tauri::State<'_, StudioState>,
    workspace_path: Option<String>,
    active_file: Option<String>,
    diagnostics_summary: Option<String>,
) -> AiContextPacketDto {
    let status = build_ai_agents_status(&default_ai_agents_config_path());

    let (pack_ws, pack_name, pack_type, edition_label, container_file_text) = state
        .active_pack
        .lock()
        .ok()
        .map(|guard| match guard.as_ref() {
            Some(pack) => {
                let ws = pack.source_path.as_ref().map(|p| p.display().to_string());
                let pt = format!("{:?}", pack.pack_type);
                let file_text = active_file.as_deref().and_then(|rel| {
                    pack.container
                        .get(rel)
                        .and_then(|b| std::str::from_utf8(b).ok())
                        .map(str::to_string)
                });
                (
                    ws,
                    Some(pack.pack_name.clone()),
                    Some(pt),
                    Some(pack.edition_label.clone()),
                    file_text,
                )
            }
            None => (None, None, None, None, None),
        })
        .unwrap_or((None, None, None, None, None));

    // Prefer live unsaved editor buffer if open in `open_documents`.
    let live_file_text = active_file
        .as_deref()
        .and_then(|rel| {
            state
                .open_documents
                .lock()
                .ok()
                .and_then(|docs| docs.get(rel).cloned())
        })
        .or(container_file_text);

    let effective_ws = workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(pack_ws.as_deref());

    build_context_packet_core(
        &status,
        effective_ws,
        pack_name.as_deref(),
        pack_type.as_deref(),
        edition_label.as_deref(),
        active_file.as_deref(),
        live_file_text.as_deref(),
        diagnostics_summary.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn persists_active_agent_and_custom_command_safely() {
        let tmp = TempDir::new().unwrap();
        let cfg_path = tmp.path().join("pack-studio/ai-agents.json");

        let initial = build_ai_agents_status(&cfg_path);
        assert_eq!(initial.active_agent_id, AGENT_CLAUDE_CODE);
        assert_eq!(initial.agents.len(), 3);

        let updated = set_active_agent_at_path(&cfg_path, AGENT_ANTIGRAVITY).unwrap();
        assert_eq!(updated.active_agent_id, AGENT_ANTIGRAVITY);

        assert!(set_active_agent_at_path(&cfg_path, "unknown-agent").is_err());

        let configured = configure_agent_at_path(
            &cfg_path,
            AGENT_OPENCODE,
            "opencode",
            vec![String::from("--model"), String::from("sonnet")],
        )
        .unwrap();
        let opencode = configured
            .agents
            .iter()
            .find(|a| a.id == AGENT_OPENCODE)
            .unwrap();
        assert_eq!(opencode.args, vec!["--model", "sonnet"]);
    }

    #[test]
    fn builds_bounded_markdown_context_packet() {
        let tmp = TempDir::new().unwrap();
        let cfg_path = tmp.path().join("ai-agents.json");
        let status = set_active_agent_at_path(&cfg_path, AGENT_CLAUDE_CODE).unwrap();

        let huge_src = "on join of player {\n    send \"hi\" to player\n}\n".repeat(400);
        let packet = build_context_packet_core(
            &status,
            Some("/tmp/frost-addon"),
            Some("Frost Addon"),
            Some("BedrockAddon"),
            Some("Bedrock 26.60"),
            Some("src/main.pstdr"),
            Some(&huge_src),
            Some("[PSTDR-E001] Use 'mut' instead of 'let' (L2:5)"),
        );
        assert!(packet.truncated);
        assert!(packet.markdown.contains("# PackStudio Workspace Context Packet"));
        assert!(packet.markdown.contains("`src/main.pstdr`"));
        assert!(packet.markdown.contains("```pstdr"));
        assert!(packet.markdown.contains("PSTDR-E001"));
        assert!(packet.suggested_command.contains("claude"));
    }
}
