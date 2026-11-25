# Third-Party Licenses & Attributions

This project (`rust-java-bedrock` / `pack-studio`) integrates or builds upon the following third-party open-source software:

---

## Blockbench

- **Project:** [JannisX11/blockbench](https://github.com/JannisX11/blockbench)
- **Author:** JannisX11 (Jannis Tobias Petersen) and Blockbench Contributors
- **License:** GNU General Public License v3.0 (GPL-3.0)
- **Usage in PackStudio:** Built from upstream source via `scripts/build_blockbench.sh` as a stripped web-only runtime bundle (`assets/vendor/blockbench-web.tar.zst`), served strictly on local loopback (`127.0.0.1`) and bridged to `pack-studio` via `scripts/studio_bridge.js`.
- **License Text:** The complete upstream `LICENSE.MD` (GPL-3.0) is bundled verbatim inside every generated `blockbench-web.tar.zst` archive and extracted alongside the runtime web assets.
- **Source Availability:** Official upstream source code is available at `https://github.com/JannisX11/blockbench`. The exact release tag and SHA-256 digest used for any build are recorded in `assets/vendor/blockbench-web.sha256`.

---

## PSTDR-DSL (`pstdr-lang`)

- **Project:** [kaito65452/pstdr-dsl](https://github.com/kaito65452/pstdr-dsl)
- **License:** GNU General Public License v3.0 or later (`GPL-3.0-or-later`)
- **Usage in PackStudio:** Linked in-process in `apps/pack-studio/src-tauri` for native `.pstdr` parsing, semantic validation, diagnostics, LSP, and Bedrock Script API transpilation.

---

## Core Rust Dependencies

Key Rust crates used by `pack-studio` (subject to their respective MIT / Apache-2.0 / CC0-1.0 licenses):

- `eframe`, `egui`, `egui_extras` — MIT / Apache-2.0
- `egui_dock` — MIT
- `image`, `fast_image_resize`, `glam` — MIT / Apache-2.0
- `notify` — CC0-1.0
- `rfd` — MIT
- `tar`, `zstd`, `sha2`, `dirs`, `base64`, `zip` — MIT / Apache-2.0
- `wry` (optional `native-webview` feature) — MIT / Apache-2.0
