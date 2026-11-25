# PackStudio (`rust-java-bedrock`) — LLM Quick Reference (`PACKSTUDIO_LLMS.md`)

> Concise, machine-oriented map of PackStudio's architecture, Tauri commands, security boundaries, and coding conventions for AI agents (**Claude Code**, **Antigravity**, **OpenCode**). See `docs/PACKSTUDIO_AI_GUIDE.md` for full details.

## Repository Layout
- `crates/rp-core`, `crates/rp-io`, `crates/rp-transforms`, `crates/rp-java`, `crates/rp-bedrock`: Pure-Rust Java <-> Bedrock resource pack engine, JSONC sanitizer, model/texture/PBR/locale transforms, and semantic auditor.
- `apps/pack-studio/src-tauri/src/`:
  - `lib.rs`: Tauri v2 command registry (`tauri::generate_handler!`) & `is_trace_enabled()`.
  - `state.rs`: `StudioState` (`active_pack`, `watcher`, `blockbench`, `pty_sessions`, `lsp_clients`, `open_documents`) and `PackType` (`JavaResourcePack`, `BedrockResourcePack`, `BedrockAddon`).
  - `workspace.rs`: Project open/create templates, CRUD, asset import/sanitize, file managers (`yazi`, `xdg-open`), pack metadata.
  - `converter.rs`: Preflight, Java <-> Bedrock conversion, semantic audit (`audit_pstdr_workspace`), `.zip`/`.mcpack`/`.mcaddon` packaging, `pstdr_check_workspace`, `pstdr_convert_workspace`.
  - `lsp.rs`: Hybrid LSP (`builtin.minecraft-rp`, `builtin.pstdr` Wave 3B, `synced.pstdr-lsp`, optional `external.pstdr-lsp`), diagnostics, completions, hovers, quick-fix code actions, `reload_pstdr_runtime`, `frontend_trace_log`.
  - `pstdr_sync.rs`: Zero-trust Ed25519 + SHA-256 + domain-pinned + anti-downgrade + anti-Zip-Slip quarantine & manual approval sync + `studio_check_update`.
  - `github_auth.rs`: GitHub PAT validation (`/user`), `0600` local credential storage, safe profile status DTO (`login`, `name`, `avatar_url`, `html_url`; never returns token).
  - `ai_agents.rs`: PATH detection & config for `claude-code` (`claude`), `antigravity` (`antigravity`/`agy`), and `opencode` (`opencode`) + bounded Markdown context packet builder.
  - `blockbench.rs`, `pty.rs`, `texture.rs`, `extensions.rs`: Embedded Blockbench web bridge, PTY terminal, 2D texture tools, VSIX declarative extension loader.
- `apps/pack-studio/src/`:
  - `App.tsx`: Workbench shell, Activity Bar, Editor Tabs, Bottom Panel (`Problems`/`Output`/`Terminal`), Status Bar, Command Palette (`Ctrl+Shift+P`), Settings shortcut (`Ctrl+,`).
  - `components/SettingsView.tsx`: VS Code-style Settings (`Updates & PSTDR LSP`, `GitHub Account`, `AI Coding Agents`, `Workbench & Files`) with live search.
  - `components/PstdrUpdateModal.tsx`: Cryptographic fingerprint review + 2-step quarantine & manual approval UI.
  - `components/MonacoCodeTab.tsx`: Monaco editor with `mc-lang`, `molang`, `pstdr` (Wave 3B), and `mcfunction` Monarch tokenizers + LSP completion/hover/code-action providers.
  - `components/ErrorBoundary.tsx`: Dark fallback crash UI forwarding stack traces to `stderr`.
  - `utils/tauriEvents.ts`: `safeListen` & `safeUnlisten` guarding `window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener`.

## Key Tauri Commands (Grouped)
- **Workspace**: `open_project`, `create_project`, `fs_create_file`, `fs_create_dir`, `fs_rename`, `fs_delete`, `fs_import_assets`, `fs_read_text`, `fs_write_text`, `fs_read_binary_base64`, `fs_write_binary_base64`, `get_pack_metadata`, `save_pack_metadata`
- **Converter / PSTDR Build**: `run_preflight`, `execute_convert`, `run_audit`, `package_pack`, `pstdr_check_workspace`, `pstdr_convert_workspace`
- **LSP & Tracing**: `lsp_list_servers`, `lsp_register_custom_server`, `lsp_unregister_custom_server`, `lsp_start_server`, `lsp_stop_server`, `lsp_sync_document`, `lsp_completion`, `lsp_hover`, `lsp_code_actions`, `frontend_trace_log`
- **Updates & Verified PSTDR Sync**: `studio_check_update`, `pstdr_sync_status`, `pstdr_sync_check`, `pstdr_sync_download_quarantine`, `pstdr_sync_approve_and_apply`, `pstdr_sync_reject_quarantine`, `pstdr_sync_revert_builtin`
- **GitHub Account**: `github_auth_status`, `github_connect_pat`, `github_disconnect`
- **AI Coding Agents**: `ai_agents_status`, `ai_set_active_agent`, `ai_configure_agent`, `ai_build_context_packet`

## Non-Negotiable Security & Technical Rules
1. **Never bypass `pstdr_sync` security**: Manifest verified with hardcoded `PSTDR_SIGNING_PUBKEY_HEX` (`ed25519_dalek::VerifyingKey::verify_strict`), HTTPS domain pinning (`github.com/kaito65452/pstdr-dsl/`, `objects.githubusercontent.com`, `release-assets.githubusercontent.com`), SemVer anti-downgrade, `0600` quarantine + SHA-256 check, Anti-Zip-Slip tar extraction, explicit user approval required.
2. **Never expose GitHub tokens**: Stored `0600` in `~/.config/pack-studio/github-auth.json`; `GithubAuthStatusDto` omits the raw token.
3. **Never put `#[tauri::command] pub fn` directly in `lib.rs`**: Tauri v2 causes `E0255` duplicate macro errors at crate root; define commands inside submodules.
4. **Always use `safeListen` / `safeUnlisten`** from `src/utils/tauriEvents.ts` for all Tauri event listeners.
5. **Monarch regex safety**: Always escape literal `@` as `[@]` inside Monarch tokenizer regexes so Monaco never interprets `@word` as a Monarch attribute macro.
6. **PSTDR durations**: Valid time units are `ms`, `s`, `m` (e.g. `50ms`, `3s`, `2m`); do NOT use `t` / `20t`.
7. **Bedrock `min_engine_version`**: First integer MUST always be `1` (e.g. `[1, 26, 60]` or `[1, 21, 50]`).
8. **CI target**: Single ARM64 `.deb` on stable `aarch64-unknown-linux-gnu` using pure-Rust crypto/TLS (`ed25519-dalek`, `sha2`, `reqwest` `rustls-tls`, zero OpenSSL).

## Verification Workflow
- Frontend: `cd apps/pack-studio && npm run build`
- Core crates: `cargo test -p rp-io -p rp-core -p rp-bedrock -p rp-java -p rp-transforms`
- Runtime trace debug: `PACKSTUDIO_TRACE=1 pack-studio`
- Knowledge graph: `graphify update .`
