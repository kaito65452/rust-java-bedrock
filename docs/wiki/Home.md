# PackStudio Wiki

Welcome to the **PackStudio (`v0.3.1-beta+<commit>`)** & **PSTDR-DSL** documentation hub!

PackStudio is an offline-first, cross-platform (**Windows `.exe`/`.msi`**, **macOS `.dmg`/`.app`**, **Linux ARM64 & x86_64 `.deb`/`.AppImage`**) **VS Code-style Minecraft Resource Pack & Bedrock Add-on Workbench** powered by Rust, Tauri v2, Monaco Editor, Embedded Blockbench 3D, and the in-process `pstdr-lang` compiler and LSP.

---

## Quick Navigation

### PackStudio Workbench
| Page | What You'll Learn |
| --- | --- |
| **[Getting Started](./PackStudio-Getting-Started.md)** | Cross-platform installers (`.exe`, `.dmg`, `.deb`, `.AppImage`), Interactive Quick Tutorial Walkthrough, Voxel Tesseract icons, 3-mode New Project Wizard, Activity Bar, VS Code-style Settings (`Ctrl+,`), and GitHub Account Connect |
| **[Converter & Blockbench 3D](./PackStudio-Converter-and-Blockbench.md)** | Bidirectional Java ↔ Bedrock conversion, Semantic Audit Gate, 2D Texture Paint, and Embedded Offline Blockbench 3D |
| **[Verified PSTDR Sync & AI Agents](./PackStudio-Verified-Sync-and-AI.md)** | Zero-trust Ed25519 + SHA-256 update verification, quarantine & manual approval modal, and Claude Code / Antigravity / OpenCode integration |

### PSTDR-DSL Language Reference
| Page | What You'll Learn |
| --- | --- |
| **[1. Overview](./pstdr-01-Overview.md)** | Target Bedrock `26.60` (`2.11.0`) & Legacy `1.x` LTS profiles, compilation pipeline, and toolchain architecture |
| **[2. Installation](./pstdr-02-Installation.md)** | Standalone `pstdr-c` / `pstdr-lsp` CLI installation (optional — PackStudio already embeds `builtin.pstdr`) |
| **[3. Quick Start](./pstdr-03-Quick-Start.md)** | Workspace structure (`pstdr.toml`, `src/`, `BP/`, `RP/`) and end-to-end build walkthrough |
| **[4. Language Guide](./pstdr-04-Language-Guide.md)** | `@addon`, `const`/`mut`, persistent `state`, duration/distance units (`ms`, `s`, `m`, `blocks`), control flow, `func`, `mcfunction`, and `unsafe` |
| **[5. Events & Actions](./pstdr-05-Events-and-Actions.md)** | All Wave 1–3C events (`on cmd`, 10 survival events, `script_event`, `entity_trigger`), native `@minecraft/server-ui` forms, scoreboards, timers, and `snake_case` helpers |
| **[6. CLI & Diagnostics](./pstdr-06-CLI-and-Diagnostics.md)** | `pstdr-c` commands, Asset Guard, and `PSTDR-E001`–`E018` diagnostic reference |
| **[7. Editor Setup](./pstdr-07-Editor-Setup.md)** | PackStudio `builtin.pstdr` + `pstdr-lsp` configuration for Helix, Neovim, and VS Code |

---

## AI Coding Agent References
- **[`PACKSTUDIO_LLMS.md`](../../PACKSTUDIO_LLMS.md)** — Compact machine-readable architecture & Tauri IPC reference
- **[`docs/PACKSTUDIO_AI_GUIDE.md`](../PACKSTUDIO_AI_GUIDE.md)** — Detailed guide for Claude Code, Antigravity, and OpenCode

> *Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft.*
