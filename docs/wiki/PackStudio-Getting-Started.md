# PackStudio — Getting Started

## 1. Installation (`v0.3.1-beta+<commit>`)

PackStudio uses SemVer 2.0 versioning (`0.3.1-beta`) with build-time short Git commit metadata (`0.3.1-beta+<commit>`) shown in the Welcome Screen, Settings, and release artifacts.

### Cross-Platform Installers (GitHub Releases & CI)
Download the installer for your OS from [GitHub Releases](https://github.com/kaito65452/rust-java-bedrock/releases):

- **Windows (x86_64)**: Run the `.exe` (NSIS) or `.msi` installer.
- **macOS (Universal — Apple Silicon & Intel)**: Open the `.dmg` image (or extract `.app.tar.gz`) and drag **PackStudio.app** to `/Applications`.
- **Linux x86_64**: Install the `.deb` via `apt` or run the portable `.AppImage`.
- **Linux ARM64 (`.deb`)**: Install via `apt`:

```bash
sudo apt-get update
sudo apt-get install -y ./pack-studio_0.3.1-beta+*_arm64.deb
pack-studio
```

The `/usr/bin/pack-studio` launcher automatically sets safe WebKitGTK defaults for X11, Wayland, containers, and Termux:X11/proot environments while preserving your `stdout`/`stderr`. For full frontend + Rust LSP debugging in a terminal, run:

```bash
PACKSTUDIO_TRACE=1 pack-studio
```

### Building From Source & Regenerating Voxel Tesseract Icons
```bash
cd apps/pack-studio
npm install
npm run icons:generate   # Renders 1024x1024 PNG + .ico + .icns via @resvg/resvg-js
npm run build
cd ../..
cargo build --release -p pack-studio-tauri --features "embed-blockbench,custom-protocol"
```

---

## 2. Welcome Screen & Interactive Quick Tutorial Walkthrough

When you launch PackStudio (or open **Manage Gear → Quick Tutorial** / Command Palette → `Help: Open Quick Tutorial`), you are greeted by the **Voxel Tesseract** hero emblem (interactive 3D Three.js hypercube with automatic SVG fallback), current build version pill (`v0.3.1-beta+<commit>`), and a balanced 2-column VS Code-style Start layout:

- **Left Column (`Start` & `Recent Packs`)**:
  - One-click **Open Pack Folder** (`Ctrl+O`), **Open Archive (`.zip`/`.mcpack`/`.mcaddon`)**, **New Project Wizard** (`Ctrl+Shift+N`), and **Launch Blockbench 3D**.
  - Persisted **Recent Packs** list with pack type badges, timestamps, and quick open/remove controls.
- **Right Column (`Quick Tutorial — Interactive Walkthrough`)**:
  1. **Create or Open a Resource/Behavior Pack** — Launches the New Project Wizard.
  2. **Convert Java ↔ Bedrock with Semantic Audit Gate** — Opens Converter & Preflight Studio.
  3. **Write Add-ons with PSTDR-DSL (`2.11.0` & `1.x` LTS)** — Opens or creates a guided `.pstdr` Wave 3B starter (`on cmd`, `ui action`, `scoreboard`, `50ms` timer) with live in-process LSP diagnostics.
  4. **Edit 3D Models in Embedded Blockbench** — Opens the embedded Blockbench 3D tab.
  5. **Connect GitHub, Verified Updates & AI Agents** — Opens Settings (`Updates & PSTDR LSP`, `GitHub Account`, and `AI Coding Agents`).

Completed walkthrough steps persist locally and can be toggled or reset at any time.

---

## 3. Workbench Layout & Shortcuts

PackStudio mirrors a VS Code desktop layout tailored for Minecraft creators:

- **Title Command Bar**: Top-left **Voxel Tesseract** badge (clicks open **Quick Tutorial**), quick access to **Open**, **New**, **Build `.pstdr`** (visible in Bedrock Add-on workspaces with `.pstdr` sources), **Audit**, **Blockbench**, and Command Palette (`Ctrl+Shift+P`).
- **Left Activity Bar**:
  - **Top stack**: Explorer (`Ctrl+Shift+E`), Search & Resource IDs (`Ctrl+Shift+F`), Java ↔ Bedrock Converter, Pack Metadata & Packaging, and Extensions & LSP (`Ctrl+Shift+X`).
  - **Bottom stack**:
    - **Accounts / GitHub Avatar**: Displays your circular GitHub profile photo with a green online dot when connected, opening a floating Accounts popover (`Manage GitHub Account` / `Disconnect`) positioned cleanly above the status bar.
    - **Manage Gear**: Opens a floating popover (`Command Palette...`, `Quick Tutorial`, `Settings`, `Updates & PSTDR LSP...`, `AI Coding Agents...`).
- **Editor Tabs**: Quick Tutorial, Monaco Editor (`.pstdr`, `.mcfunction`, `.json`, `.mcmeta`, `.lang`, `.molang`, `.toml`, `.js`, `.ts`), Pixel Paint Canvas (`.png`, `.tga`), Converter Studio, Metadata & Packaging, Extensions & LSP, Settings, and persistent Embedded Blockbench 3D.
- **Bottom Panel (`Ctrl+J` / `` Ctrl+` ``)**: Unified **Problems** (LSP + Semantic Auditor), **Output** logs, and embedded **Terminal** (`xterm.js` + PTY).

### Essential Keyboard Shortcuts
| Shortcut | Action |
| --- | --- |
| `Ctrl+Shift+P` / `F1` | Command Palette (includes `Help: Open Quick Tutorial`) |
| `Ctrl+O` | Open Pack Folder |
| `Ctrl+Shift+N` | New Project Wizard |
| `Ctrl+P` | Quick Open Asset File |
| `Ctrl+,` / `Cmd+,` | Open VS Code-style Settings directly |
| `Ctrl+S` | Save active file & trigger LSP diagnostics |
| `Ctrl+B` | Toggle Primary Sidebar |
| `Ctrl+J` / `` Ctrl+` `` | Toggle Bottom Panel (Problems / Output / Terminal) |

---

## 4. Creating Your First Project

Open **New** → **New Project Wizard** and select one of three first-class modes:

1. **Java Resource Pack**: Generates `pack.mcmeta` (`pack_format` + `supported_formats`) and `assets/<namespace>/textures/...`.
2. **Bedrock Resource Pack**: Generates `manifest.json` with valid RFC 4122 v4 UUIDs and `[1, x, x]` `min_engine_version`.
3. **Bedrock Add-on Workspace**: Generates `BP/` + `RP/` with linked UUIDs, `BP/scripts/main.js`, and optional `pstdr.toml` + `src/main.pstdr` ready for one-click **Build `.pstdr`** and `.mcaddon` export.

---

## 5. VS Code-Style Settings & GitHub Account Connect

Press **`Ctrl+,`** (or click **Manage Gear → Settings**) to open the searchable **Settings** view:

- **Updates & PSTDR LSP**: Check for new PackStudio desktop releases and check/sync verified PSTDR compiler/LSP updates.
- **GitHub Account**: Paste a GitHub Personal Access Token (`read:user`) to connect your profile. PackStudio validates it against `https://api.github.com/user`, stores the token locally with `0600` Unix permissions (never exposed back to the WebView), and displays your avatar, `@username`, and profile link in Settings and the bottom-left Activity Bar.
- **AI Coding Agents**: Configure Claude Code, Antigravity, and OpenCode CLI bridges.
- **Workbench & Files**: Select preferred native file manager (`yazi`, `xdg-open`, etc.), filename sanitization, automatic PNG conversion, and default 3D UV-Atlas Guard mode.

> [← Back to Home](./Home.md) | [Next: Converter & Blockbench 3D →](./PackStudio-Converter-and-Blockbench.md)
