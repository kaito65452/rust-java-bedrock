# PackStudio — Getting Started

## 1. Installation

### Debian / Ubuntu (`.deb` — ARM64 & x86_64)
Download the latest release artifact (`pack-studio_0.3.0-beta.1_arm64.deb`) from [GitHub Releases](https://github.com/kaito65452/rust-java-bedrock/releases) and install via `apt`:

```bash
sudo apt-get update
sudo apt-get install -y ./pack-studio_0.3.0-beta.1_arm64.deb
pack-studio
```

The `/usr/bin/pack-studio` launcher automatically sets safe WebKitGTK defaults for X11, Wayland, containers, and Termux:X11/proot environments while preserving your `stdout`/`stderr`. For full frontend + Rust LSP debugging in a terminal, run:

```bash
PACKSTUDIO_TRACE=1 pack-studio
```

### Building From Source
```bash
cd apps/pack-studio && npm install && npm run build && cd ../..
cargo build --release -p pack-studio-tauri --features "embed-blockbench,custom-protocol"
```

---

## 2. Workbench Layout & Shortcuts

PackStudio mirrors a VS Code desktop layout tailored for Minecraft creators:

- **Title Command Bar**: Quick access to **Open**, **New**, **Build `.pstdr`** (visible in Bedrock Add-on workspaces with `.pstdr` sources), **Audit**, **Blockbench**, and Command Palette (`Ctrl+Shift+P`).
- **Left Activity Bar**:
  - **Top stack**: Explorer (`Ctrl+Shift+E`), Search & Resource IDs (`Ctrl+Shift+F`), Java ↔ Bedrock Converter, Pack Metadata & Packaging, and Extensions & LSP (`Ctrl+Shift+X`).
  - **Bottom stack**:
    - **Accounts / GitHub Avatar**: Displays your circular GitHub profile photo with a green online dot when connected, opening a floating Accounts popover (`Manage GitHub Account` / `Disconnect`).
    - **Manage Gear**: Opens a floating popover (`Command Palette...`, `Settings`, `Updates & PSTDR LSP...`, `AI Coding Agents...`).
- **Editor Tabs**: Monaco Editor (`.pstdr`, `.mcfunction`, `.json`, `.mcmeta`, `.lang`, `.molang`, `.toml`, `.js`, `.ts`), Pixel Paint Canvas (`.png`, `.tga`), Converter Studio, Metadata & Packaging, Extensions & LSP, Settings, and persistent Embedded Blockbench 3D.
- **Bottom Panel (`Ctrl+J` / `` Ctrl+` ``)**: Unified **Problems** (LSP + Semantic Auditor), **Output** logs, and embedded **Terminal** (`xterm.js` + PTY).

### Essential Keyboard Shortcuts
| Shortcut | Action |
| --- | --- |
| `Ctrl+Shift+P` / `F1` | Command Palette |
| `Ctrl+P` | Quick Open Asset File |
| `Ctrl+,` / `Cmd+,` | Open VS Code-style Settings directly |
| `Ctrl+S` | Save active file & trigger LSP diagnostics |
| `Ctrl+B` | Toggle Primary Sidebar |
| `Ctrl+J` / `` Ctrl+` `` | Toggle Bottom Panel (Problems / Output / Terminal) |

---

## 3. Creating Your First Project

Open **New** → **New Project Wizard** and select one of three first-class modes:

1. **Java Resource Pack**: Generates `pack.mcmeta` (`pack_format` + `supported_formats`) and `assets/<namespace>/textures/...`.
2. **Bedrock Resource Pack**: Generates `manifest.json` with valid RFC 4122 v4 UUIDs and `[1, x, x]` `min_engine_version`.
3. **Bedrock Add-on Workspace**: Generates `BP/` + `RP/` with linked UUIDs, `BP/scripts/main.js`, and optional `pstdr.toml` + `src/main.pstdr` ready for one-click **Build `.pstdr`** and `.mcaddon` export.

---

## 4. VS Code-Style Settings & GitHub Account Connect

Press **`Ctrl+,`** (or click **Manage Gear → Settings**) to open the searchable **Settings** view:

- **Updates & PSTDR LSP**: Check for new PackStudio desktop releases and check/sync verified PSTDR compiler/LSP updates.
- **GitHub Account**: Paste a GitHub Personal Access Token (`read:user`) to connect your profile. PackStudio validates it against `https://api.github.com/user`, stores the token locally with `0600` Unix permissions (never exposed back to the WebView), and displays your avatar, `@username`, and profile link in Settings and the bottom-left Activity Bar.
- **AI Coding Agents**: Configure Claude Code, Antigravity, and OpenCode CLI bridges.
- **Workbench & Files**: Select preferred native file manager (`yazi`, `xdg-open`, etc.), filename sanitization, automatic PNG conversion, and default 3D UV-Atlas Guard mode.

> [← Back to Home](./Home.md) | [Next: Converter & Blockbench 3D →](./PackStudio-Converter-and-Blockbench.md)
