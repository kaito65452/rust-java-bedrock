<div align="center">
  <img src="apps/pack-studio/src-tauri/icons/128x128@2x.png" alt="PackStudio Voxel Tesseract Logo" width="128" height="128" />

  <h1>PackStudio (<code>rust-java-bedrock</code>)</h1>

  <p><strong>Offline-first Minecraft Java &amp; Bedrock Resource Pack, Add-on &amp; PSTDR-DSL Workbench</strong></p>

  <p>
    <a href="https://github.com/kaito65452/rust-java-bedrock/releases"><img src="https://img.shields.io/badge/release-v0.3.1--beta-blue" alt="Release" /></a>
    <a href="https://github.com/kaito65452/rust-java-bedrock/actions"><img src="https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux%20(x86__64%2FARM64)-blueviolet" alt="Platforms" /></a>
    <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-Edition%202024-orange?logo=rust" alt="Rust" /></a>
    <a href="https://v2.tauri.app/"><img src="https://img.shields.io/badge/Tauri-v2-24C8DB?logo=tauri" alt="Tauri v2" /></a>
    <a href="https://github.com/kaito65452/pstdr-dsl"><img src="https://img.shields.io/badge/PSTDR--DSL-Wave%203B%20In--Process-3fb950" alt="PSTDR-DSL" /></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--3.0--or--later-blue.svg" alt="License: GPL-3.0-or-later" /></a>
  </p>
</div>

**PackStudio** is a fast, offline-first **VS Code-style Minecraft Resource Pack & Bedrock Add-on Workbench** built with **Rust**, **Tauri v2**, **React**, **Monaco Editor**, **Embedded Blockbench 3D**, and the **in-process `pstdr-dsl` compiler & Language Server**.

Create, paint, model, script, audit, convert (`Java Edition ↔ Bedrock Edition`), and export `.zip`, `.mcpack`, and `.mcaddon` packages from a single lightweight desktop binary — with zero Electron overhead and zero-trust cryptographic updates.

---

## Key Capabilities

- **3-Mode Package Manager (`Java RP`, `Bedrock RP`, `Bedrock Add-on`)**:
  - Dedicated templates and live metadata management for Java Resource Packs (`pack.mcmeta`), Bedrock Resource Packs (`manifest.json`), and full Bedrock Add-on workspaces (`BP/` + `RP/` + `pstdr.toml` + `src/*.pstdr`).
  - Automatic RFC 4122 UUID v4 generation, BP → RP dependency linking, and strict `[1, x, x]` Bedrock `min_engine_version` enforcement.
- **Bidirectional Java ↔ Bedrock Converter + Semantic Audit Gate**:
  - Converts packs from Minecraft `1.6.1` through `26.3` / `26.60`: textures, `.png.mcmeta` ↔ `flipbook_textures.json`, LabPBR ↔ Bedrock `.texture_set.json`, `SE(3)` 3D block/entity geometry & UVs, `.json` ↔ `.lang` locales, and `.ogg` audio.
  - Built-in **Anti-False-Positive Semantic Audit Gate** catches broken atlas/texture references, orphan textures, invalid manifests, missing `.mcfunction` assets (`PSTDR-E017`), and Stable-vs-Beta Script API mismatches (`PSTDR-E018`).
- **Monaco Code Editor + In-Process `pstdr-dsl` LSP (Wave 3B)**:
  - Syntax highlighting, live diagnostics (`PSTDR-E001`..`PSTDR-E018`), contextual autocomplete (vanilla effects, sounds, `snake_case` helpers, predicates, Wave 1–3B events/UI/scoreboards), hover transpilation previews, and quick-fix code actions (`let → mut`, `const → mut`, `function → func`, `nil → null`).
  - First-class `.mcfunction`, `.molang`, `.lang`, and JSON/JSONC schema validation.
  - One-click **Build `.pstdr`** compiles into partitioned `BP/scripts/generated/`, `_registry.js`, `main.js`, inline `BP/functions/*.mcfunction`, and custom component JSON wiring.
- **Embedded Offline Blockbench 3D + Pixel-Accurate 2D Paint Canvas**:
  - Offline Blockbench Web Studio embedded directly inside the workbench over a path-traversal-guarded loopback bridge with 2-way live sync.
  - Built-in 2D pixel paint tab with `fast_image_resize` nearest-neighbor downscaling and opaque `grass_side.tga` compositing.
- **Zero-Trust Verified PSTDR Sync (Ed25519 + SHA-256 + Manual Approval)**:
  - Verifies official GitHub Releases (`/releases/latest`) with a **hardcoded Ed25519 public key**, pinned HTTPS domains, SemVer anti-downgrade guard, `0600` quarantine SHA-256 verification, Anti-Zip-Slip `.tar.gz` extraction, and a manual approval modal displaying full cryptographic fingerprints before anything is applied.
- **Interactive Quick Tutorial Walkthrough & Voxel Tesseract Branding**:
  - Live 3D/4D **Voxel Tesseract** logo rendered with `three` (with automatic crisp SVG fallback) and regenerated multi-platform icons (`npm run icons:generate` via `@resvg/resvg-js`).
  - Balanced 2-column VS Code-style Start Screen with persisted **Recent Packs** and a 5-step interactive **Quick Tutorial** (Project Wizard, Java ↔ Bedrock Converter, guided `.pstdr` Wave 3B starter, Embedded Blockbench 3D, and Verified Sync / AI Settings).
- **VS Code-Style Settings, GitHub Profile Connect & AI Coding Agent Bridge**:
  - Bottom-left Activity Bar with **GitHub Profile Avatar** + floating Accounts menu and **Manage Gear** popover (`Ctrl+,` opens full searchable Settings).
  - Connect your GitHub account via PAT (`0600` local storage, never exposed to WebView).
  - Built-in **Claude Code**, **Antigravity**, and **OpenCode** CLI detection, custom command configuration, and bounded Markdown workspace/diagnostics context packet export.

---

## Quick Start & Installation

### 1. Download Cross-Platform Installers (`0.3.1-beta+<commit>`)

Pre-built binaries and installers are built automatically via GitHub Actions ([`release-packstudio.yml`](.github/workflows/release-packstudio.yml)) for every push and attached to [GitHub Releases](https://github.com/kaito65452/rust-java-bedrock/releases) on `v*` tags:

| Platform | Formats | Notes |
| --- | --- | --- |
| **Linux ARM64** | `.deb` + `.sha256` | Native `aarch64-unknown-linux-gnu` build (Raspberry Pi, ARM laptops, Termux/proot) |
| **Linux x86_64** | `.deb` & `.AppImage` | Standard Ubuntu/Debian/desktop Linux bundles |
| **Windows x86_64** | `.exe` (NSIS) & `.msi` | Native WebView2 installer with Voxel Tesseract `.ico` |
| **macOS Universal** | `.dmg` & `.app.tar.gz` | Universal Apple Silicon (`arm64`) + Intel (`x86_64`) with `.icns` |

#### Install on Debian / Ubuntu (`.deb` — ARM64 or x86_64)

```bash
sudo apt-get update
sudo apt-get install -y ./pack-studio_0.3.1-beta+*_arm64.deb

# Launch PackStudio
pack-studio

# Optional: launch with verbose frontend + Rust LSP terminal trace logging
PACKSTUDIO_TRACE=1 pack-studio
```

### 2. Build From Source

**Prerequisites:** Rust stable (`edition = "2024"`), Node.js 22+, and on Linux: GTK3 + WebKitGTK 4.1 (`libgtk-3-dev`, `libwebkit2gtk-4.1-dev`, `pkg-config`, `zstd`).

```bash
# 1. Build the PackStudio frontend & (optionally) regenerate Voxel Tesseract app icons
cd apps/pack-studio
npm install
npm run icons:generate
npm run build
cd ../..

# 2. (Optional) Build the offline Blockbench web bundle
bash scripts/build_blockbench.sh

# 3. Build PackStudio release binary
cargo build --release -p pack-studio-tauri --features "embed-blockbench,custom-protocol"

# 4. Run core Rust workspace tests
cargo test -p rp-io -p rp-core -p rp-bedrock -p rp-java -p rp-transforms
```

You can also build the standalone CLI (`rp-cli`), Linux TUI (`rp-tui`), and lightweight egui tool (`rp-gui`):

```bash
cargo build --release -p rp-cli -p rp-tui -p rp-gui
./target/release/rp-cli convert input_java.zip -o output_bedrock.mcpack --to bedrock --target-version 26.2
```

### 3. First Project Wizard

1. Click **New** in the title bar (or press `Ctrl+Shift+P` → `File: New Resource Pack / Add-On Wizard...`).
2. Choose **Java Resource Pack**, **Bedrock Resource Pack**, or **Bedrock Add-on Workspace** (enable `.pstdr` DSL scaffolding for instant `pstdr.toml` + `src/main.pstdr`).
3. Edit scripts/textures/models, press **Build `.pstdr`** (or **Audit**), and export `.zip` / `.mcpack` / `.mcaddon` from **Metadata & Packaging**.

---

## PSTDR-DSL Quick Example (Wave 3B)

PackStudio includes the `pstdr-lang` compiler and `builtin.pstdr` LSP out of the box. Write clean, safe add-on logic targeting Minecraft Bedrock `26.60` (`@minecraft/server 2.11.0` + `@minecraft/server-ui 2.0.0`):

```pstdr
@addon(target = "26.60", api = "2.11.0")

on join of player {
    scoreboard show "coins" title "§eCoins" slot sidebar
    score["coins"][player] += 10
    send "§aWelcome! Run §b/my_pack:shop§a to open the menu." to player
}

on cmd "my_pack:shop" description "Open the starter shop menu" permission any () by player {
    ui action "Starter Shop" body "Balance: {score[\"coins\"][player]} coins" to player {
        button "Speed Boost (5 coins)" {
            if score["coins"][player] >= 5 {
                score["coins"][player] -= 5
                apply speed 2 to player for 30s
                player.play_sound("random.orb")
                send "§aSpeed II applied for 30s!" to player
            } else {
                send "§cNot enough coins." to player
            }
        }
    }
}
```

---

## Documentation & Wiki

| Resource | Description |
| --- | --- |
| **[Wiki Home](docs/wiki/Home.md)** | Full PackStudio & PSTDR-DSL Wiki hub (GitHub Wiki ready) |
| **[PackStudio Getting Started](docs/wiki/PackStudio-Getting-Started.md)** | Installation, Workbench layout, Activity Bar, Settings (`Ctrl+,`), and GitHub Connect |
| **[Converter & Blockbench 3D](docs/wiki/PackStudio-Converter-and-Blockbench.md)** | Java ↔ Bedrock conversion pipeline, Semantic Audit Gate, and Embedded Blockbench |
| **[Verified Sync & AI Agents](docs/wiki/PackStudio-Verified-Sync-and-AI.md)** | Ed25519 + SHA-256 PSTDR update security model & Claude Code / Antigravity / OpenCode bridge |
| **[PSTDR-DSL Overview](docs/wiki/pstdr-01-Overview.md)** | Full `.pstdr` language, Wave 1–3C events, UI forms, scoreboards, CLI & LSP setup |
| **[AI Developer Guide](docs/PACKSTUDIO_AI_GUIDE.md)** & **[`PACKSTUDIO_LLMS.md`](PACKSTUDIO_LLMS.md)** | Complete architecture, Tauri command map, and rules for AI coding agents |

---

## Workspace Crates

| Path | Description |
| --- | --- |
| `apps/pack-studio/` | PackStudio desktop workbench (Tauri v2 Rust backend + React/Monaco frontend) |
| `crates/rp-core` | Canonical `ResourcePackIR`, mappings, JSONC/UTF-16/section-sign sanitizer |
| `crates/rp-io` | Safe `PackContainer` I/O (dir, `.zip`, `.mcpack`, `.mcaddon`) with Zip-Slip guards |
| `crates/rp-transforms` | Textures, flipbooks, LabPBR ↔ MER, `SE(3)` 3D geometry/UVs, locales, audio |
| `crates/rp-java` / `crates/rp-bedrock` | Decoupled Java & Bedrock parsers, emitters, and semantic auditor |
| `crates/rp-cli` / `crates/rp-tui` / `crates/rp-gui` | Standalone CLI, Linux `ratatui` TUI, and `egui` utility |

---

## Disclaimer & License

Licensed under the **GNU General Public License v3.0 or later (`GPL-3.0-or-later`)** — see [`LICENSE`](LICENSE) and third-party notices in [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md) (including in-process `pstdr-lang` `GPL-3.0-or-later` and bundled Blockbench Web `GPL-3.0` licensing).

> **Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft.**
