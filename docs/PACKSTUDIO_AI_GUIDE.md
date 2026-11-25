# PackStudio AI Coding Agent Guide

> Complete architectural, Tauri IPC, security, and workflow reference for AI coding agents (**Claude Code**, **Antigravity**, **OpenCode**, and compatible LLM tools) working on PackStudio or inside PackStudio Minecraft workspaces.

---

## 1. Architecture Overview

PackStudio (`apps/pack-studio/`) is a single-binary **Tauri v2 + Rust + React/TypeScript + Monaco** desktop IDE for Minecraft Java Edition Resource Packs, Bedrock Edition Resource Packs, and Bedrock Add-ons (`BP/` + `RP/` + `.pstdr` DSL).

### Layers & Responsibilities

| Layer | Location | Responsibility |
| --- | --- | --- |
| **Core Conversion Crates** | `crates/rp-core`, `crates/rp-io`, `crates/rp-transforms`, `crates/rp-java`, `crates/rp-bedrock` | Pack container I/O (dir + zip/mcpack/mcaddon), JSONC/UTF-16/section-sign sanitizer, Java <-> Bedrock texture/model/locale/PBR/sound IR & semantic auditor. |
| **Tauri Rust Backend** | `apps/pack-studio/src-tauri/src/` | Workspace filesystem & watcher (`workspace.rs`), Converter/Audit/Export/PSTDR build (`converter.rs`), Hybrid LSP (`lsp.rs`), Verified PSTDR sync (`pstdr_sync.rs`), GitHub Auth (`github_auth.rs`), AI Agent Bridge (`ai_agents.rs`), Embedded Blockbench HTTP/WS bridge (`blockbench.rs`), PTY shell (`pty.rs`), 2D Texture ops (`texture.rs`), Extensions/VSIX (`extensions.rs`). |
| **React Workbench Frontend** | `apps/pack-studio/src/` | VS Code-style Activity Bar, Explorer, Monaco Editor (`MonacoCodeTab.tsx`), Texture Paint Canvas (`TexturePaintTab.tsx`), Converter & Preflight (`ConverterTab.tsx`), Metadata & Packaging (`MetadataPackagingTab.tsx`), Extensions & LSP (`ExtensionsTab.tsx`), VS Code-style Settings (`SettingsView.tsx`), PSTDR Update Approval Modal (`PstdrUpdateModal.tsx`), Embedded xterm.js PTY (`TerminalPanel.tsx`), and React `ErrorBoundary`. |
| **PSTDR Compiler & Spec** | `pstdr-lang` (Git dependency `kaito65452/pstdr-dsl`) | In-process parser, validator (`PSTDR-E001`..`PSTDR-E018`), partitioned JS/TS codegen, `.mcfunction` emission, custom component wiring, and Bedrock API spec. |
| **Embedded Blockbench 3D** | `assets/vendor/blockbench-web.tar.zst` | Offline Blockbench web runtime extracted on demand into cache and bridged over loopback WebSocket/HTTP with strict relative-path traversal guards. |

### 3-Mode Package Manager (`PackType`)

1. **`JavaResourcePack`**: `pack.mcmeta` + `assets/<namespace>/...` -> exports `.zip`.
2. **`BedrockResourcePack`**: root `manifest.json` (`resources` module, `min_engine_version` enforced `[1, x, x]`) -> exports `.mcpack`.
3. **`BedrockAddon`**: `BP/` + `RP/` (+ optional `pstdr.toml` & `src/*.pstdr`) -> auto-compiles `.pstdr` (`BP/scripts/generated/`, `BP/functions/*.mcfunction`, `_registry.js`, `main.js`, custom component JSON wiring) and audits before bundling into `.mcaddon`.

---

## 2. VS Code-Style Settings & Update Controls

PackStudio exposes a unified VS Code-style **Settings** workbench tab (`apps/pack-studio/src/components/SettingsView.tsx`), opened via:
- The **bottom-left Activity Bar gear icon** (`codicon-settings-gear`)
- Keyboard shortcut **`Ctrl+,`** (Linux/Windows) or **`Cmd+,`** (macOS)
- Command Palette (`Ctrl+Shift+P` -> **`Preferences: Open Settings (Ctrl+,)`**)

### Settings Categories & Live Search
The top search input filters across all four categories:
1. **`Updates & PSTDR LSP`**
   - **PackStudio App Update**: Displays installed PackStudio version (e.g. `0.3.1-beta+<commit>`) and triggers `studio_check_update` against `https://api.github.com/repos/kaito65452/rust-java-bedrock/releases/latest`.
   - **PSTDR LSP & Spec Sync**: Displays installed PSTDR version (`builtin` vs `synced`), active Bedrock spec version, hardcoded Ed25519 signing key ID & SHA-256 fingerprint, **`Check & Sync PSTDR LSP`** (launches `PstdrUpdateModal`), and **`Revert to Built-in`** (`pstdr_sync_revert_builtin`).
2. **`GitHub Account`**
   - Connects a GitHub account via Personal Access Token (`read:user`), displaying the user's **profile avatar (`avatar_url`)** with offline fallback, display name, `@login`, profile link, and **`Disconnect`** button.
3. **`AI Coding Agents`**
   - Detects and configures **Claude Code** (`claude`), **Antigravity** (`antigravity` / `agy`), and **OpenCode** (`opencode`), selects the active agent, and builds/copies bounded Markdown context packets for the current workspace and active file.
4. **`Workbench & Files`**
   - Preferred file manager (`auto` / `yazi` / `xdg-open` / etc.), import filename sanitization (`[a-z0-9_-]`), automatic JPG/WebP -> PNG conversion, and default 3D UV-Atlas Guard mode.

---

## 3. Cryptographically Verified PSTDR Update Sync Security Model

Implemented in `apps/pack-studio/src-tauri/src/pstdr_sync.rs` and `apps/pack-studio/src/components/PstdrUpdateModal.tsx`.

### Zero-Trust Invariants
- **Fixed Release Endpoints**: Only fetches `/releases/latest/download/pstdr-release.json` and `.sig` (ignoring `main` commits, Drafts, and Pre-releases).
- **Hardcoded Ed25519 Public Key**: Verified via `ed25519_dalek::VerifyingKey::verify_strict` against `PSTDR_SIGNING_PUBKEY_HEX` (`key_id = "saryu-pstdr-root-2026"`). Public keys are **never** fetched from the internet.
- **Strict HTTPS & Domain Pinning**: Every initial URL and every HTTP redirect hop is validated against `https://` + allowlisted hosts:
  - `github.com` (strictly path prefix `/kaito65452/pstdr-dsl/`, no `..` or `\`)
  - `objects.githubusercontent.com`
  - `release-assets.githubusercontent.com`
- **SemVer Anti-Downgrade**: `remote_version > installed_version` enforced via `semver::Version`. Equal or older releases are rejected.
- **Quarantine + SHA-256 Verification**: Downloaded artifacts are written to `~/.local/share/pack-studio/pstdr-quarantine/<version>/` (`0700` dir, `0600` non-executable files) and verified byte-for-byte against manifest SHA-256 digests. Any mismatch deletes the quarantine folder immediately.
- **Manual Approval Gate**: Passive checks and quarantine downloads **never** overwrite active files or execute binaries. Only explicit user invocation of `pstdr_sync_approve_and_apply` re-verifies both Ed25519 and SHA-256 (defense-in-depth), extracts with **Anti-Zip-Slip** checks (rejecting `..`, leading `/`, Windows drive prefixes, NUL bytes, symlinks, hardlinks, device nodes, and >128 MiB archives), atomically swaps into `~/.local/share/pack-studio/pstdr-runtime/`, and reloads `lsp.rs` (with automatic fallback to `builtin.pstdr` if a synced `pstdr-lsp` fails to start).

### Storage Paths
- Active runtime: `~/.local/share/pack-studio/pstdr-runtime/` (`active.json`, `spec/`, optional `bin/`)
- Quarantine: `~/.local/share/pack-studio/pstdr-quarantine/<version>/`

---

## 4. GitHub Account Integration (`github_auth.rs`)

### Tauri Commands
- `github_auth_status() -> GithubAuthStatusDto`
- `github_connect_pat(token: String) -> Result<GithubAuthStatusDto, String>`
- `github_disconnect() -> Result<GithubAuthStatusDto, String>`

### Profile Fields (`GithubAuthStatusDto`)
- `connected: bool`
- `token_stored: bool` (the raw PAT is **never** returned to the frontend WebView)
- `login: Option<String>`
- `name: Option<String>`
- `avatar_url: Option<String>` (HTTPS-only validated)
- `html_url: Option<String>`
- `storage_path: String`

### Secure Storage
- Path: `~/.config/pack-studio/github-auth.json` (or OS `config_dir`)
- Unix permissions: directory `0700`, file `0600`
- Validates tokens against `https://api.github.com/user` with `User-Agent: PackStudio`.

---

## 5. AI Coding Agent Bridge (`ai_agents.rs`)

### Supported Agents
| Agent ID | Display Name | Default Executable | Fallback Probe |
| --- | --- | --- | --- |
| `claude-code` | Claude Code | `claude` | — |
| `antigravity` | Antigravity | `antigravity` | `agy` |
| `opencode` | OpenCode | `opencode` | — |

### Tauri Commands
- `ai_agents_status() -> AiAgentsStatusDto`: Probes `PATH` (or custom path) and returns installation status, resolved binary paths, and `active_agent_id`.
- `ai_set_active_agent(agent_id: String) -> Result<AiAgentsStatusDto, String>`: Persists active agent in `~/.config/pack-studio/ai-agents.json` (`0600`).
- `ai_configure_agent(agent_id: String, command: String, args: Vec<String>) -> Result<AiAgentsStatusDto, String>`: Persists custom command/args override.
- `ai_build_context_packet(workspace_path?, active_file?, diagnostics_summary?) -> AiContextPacketDto`: Produces a bounded Markdown context packet (capped at 16 KiB / 300 lines per excerpt) including:
  - PackStudio version & selected AI agent
  - Workspace path, pack name, pack type, edition/target
  - Active file path & live unsaved buffer content (syntax-fenced for `.pstdr`, `.mcfunction`, `.json`, `.toml`, `.js`, `.ts`)
  - Active LSP/Audit diagnostics
  - Ready-to-run shell command (`cd "<workspace>" && <agent> [args]`)

### How an AI Agent Should Work Inside a PackStudio Workspace
1. Read `pstdr.toml` (for Bedrock Add-ons) or `manifest.json` / `pack.mcmeta` first to confirm edition and target versions.
2. Remember Bedrock's `min_engine_version` rule: the first element is **always `1`** (e.g. `[1, 26, 60]` or `[1, 21, 50]`, never `[26, 60, 0]`).
3. For `.pstdr` files:
   - Use `mut` (not `let`/`var`), `func` (not `function`), `null` (not `nil`), `!=` (not `~=`).
   - Use time units `ms`, `s`, `m` (e.g. `50ms`, `3s`, `2m`) — never `20t`.
   - Wave 3A/3B supports `mcfunction`, `command`, `run_fn`, `on script_event`, `on entity_trigger`, `on cmd`, `ui action`/`ui message`/`ui modal`, `scoreboard show`/`hide`, `score[...]`, and the 10 survival events (`place`, `dimension_change`, `health_change`, `interact_entity`, `projectile_hit_block`, `entity_hit_block`, `item_release`, `explosion`, `entity_load`, `entity_remove`).
   - `on chat` requires `-beta` Script API (`PSTDR-E018` on stable APIs).
4. PackStudio automatically detects filesystem changes via `notify` (`fs://changed`) and reloads the active workspace snapshot.

---

## 6. Event Cleanup & Diagnostics Tracing

- **Safe Event Cleanup (`apps/pack-studio/src/utils/tauriEvents.ts`)**: All Tauri event subscriptions MUST use `safeListen` / `safeUnlisten` to prevent `window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener` unhandled promise rejections during React StrictMode unmount races.
- **Terminal Crash & Trace Logging**:
  - Frontend `error` / `warn` logs, `window.error`, `unhandledrejection`, and React `ErrorBoundary` stack traces automatically print to `stderr` via `lsp::frontend_trace_log`.
  - Run `PACKSTUDIO_TRACE=1 pack-studio` for full frontend + Rust LSP trace breadcrumbs.

---

## 7. Build & Verification Commands

```bash
# Frontend TypeScript check + Vite production bundle
cd apps/pack-studio && npm run build

# Core Rust workspace tests
cargo test -p rp-io -p rp-core -p rp-bedrock -p rp-java -p rp-transforms

# Full workspace build/test (requires GTK3 + WebKitGTK 4.1 dev packages on Linux)
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# ARM64 .deb packaging
bash scripts/build_deb_arm64.sh target/release/pack-studio-tauri dist-deb
```
