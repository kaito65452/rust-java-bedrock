# Verified PSTDR Update Sync & AI Coding Agent Bridge

## 1. Zero-Trust PSTDR Update Sync (Ed25519 + SHA-256)

PackStudio ships with the compiled `pstdr-lang` engine and `builtin.pstdr` LSP out of the box, and lets you safely sync newer official `pstdr-dsl` specs and runtimes from GitHub Releases without rebuilding the entire desktop app.

### Security Architecture
1. **Official Releases Only**: Queries only `/releases/latest/download/pstdr-release.json` and `.sig` — daily `main` commits, Drafts, and Pre-releases are ignored.
2. **Hardcoded Ed25519 Public Key**: Every manifest is verified against the raw bytes using `ed25519_dalek::VerifyingKey::verify_strict` with the hardcoded signing key (`key_id: saryu-pstdr-root-2026`, fingerprint `0b:fb:5a:7a:03:86:6d:c8:31:f6:f1:a8:95:88:36:a6:25:8f:8b:95:e9:75:d0:d4:82:73:38:d0:4a:f6:a4:92`). Public keys are **never** fetched over the network.
3. **Strict HTTPS Domain Pinning**: Every download URL and redirect hop is restricted to `https://` on `github.com/kaito65452/pstdr-dsl/`, `objects.githubusercontent.com`, and `release-assets.githubusercontent.com`.
4. **SemVer Anti-Downgrade Guard**: Only versions strictly greater than the installed version (`remote > installed`) are offered.
5. **Quarantine & SHA-256 Verification**: Downloaded `.tar.gz` artifacts are saved non-executable (`0600`) inside `~/.local/share/pack-studio/pstdr-quarantine/<version>/` (`0700`) and hashed byte-for-byte against the signed manifest. Any mismatch immediately deletes the quarantine folder.
6. **Manual Approval & Anti-Zip-Slip Extraction**: Nothing is installed or executed until you explicitly click **Approve & Apply Update** in the approval modal. PackStudio re-verifies Ed25519 + SHA-256, rejects unsafe tar entries (`..`, leading `/`, Windows drives, NUL bytes, symlinks, hardlinks, device nodes, or >128 MiB bombs), atomically swaps into `~/.local/share/pack-studio/pstdr-runtime/`, and reloads `lsp.rs` (falling back automatically to `builtin.pstdr` if a synced binary ever fails to launch).
7. **Instant Rollback**: Click **Revert to Built-in PSTDR** in Settings or the update modal at any time to remove the synced override and immediately return to the built-in compiled engine.

---

## 2. AI Coding Agents (Claude Code, Antigravity & OpenCode)

Open **Settings (`Ctrl+,`) → AI Coding Agents** (or **Manage Gear → AI Coding Agents...**) to connect PackStudio with your local CLI coding agents:

| Agent | ID | Default Binary | Notes |
| --- | --- | --- | --- |
| **Claude Code** | `claude-code` | `claude` | Anthropic CLI coding agent |
| **Antigravity** | `antigravity` | `antigravity` (fallback: `agy`) | Google DeepMind Antigravity CLI/IDE |
| **OpenCode** | `opencode` | `opencode` | Open-source terminal coding agent |

### Features
- **Automatic `PATH` Detection**: Displays `Installed` / `Not found in PATH` badges and exact resolved binary paths.
- **Custom Invocation**: Override command path and default flags per agent (stored `0600` in `~/.config/pack-studio/ai-agents.json`).
- **Workspace & `.pstdr` Context Handoff**: Click **Build & Copy Context Packet** to generate a bounded Markdown context block (capped at 16 KiB / 300 lines) containing:
  - Active workspace path, pack name, `PackType`, and target Minecraft version
  - Active file path and live unsaved editor content (syntax-fenced for `.pstdr`, `.mcfunction`, `.json`, etc.)
  - Current LSP and Semantic Auditor diagnostics
  - Copyable terminal launch command (`cd "<workspace>" && <agent> [args]`)

For agent developers and LLM prompts, also see [`PACKSTUDIO_LLMS.md`](../../PACKSTUDIO_LLMS.md) and [`docs/PACKSTUDIO_AI_GUIDE.md`](../PACKSTUDIO_AI_GUIDE.md).

> [← Previous: Converter & Blockbench 3D](./PackStudio-Converter-and-Blockbench.md) | [Next: PSTDR-DSL Overview →](./pstdr-01-Overview.md)
