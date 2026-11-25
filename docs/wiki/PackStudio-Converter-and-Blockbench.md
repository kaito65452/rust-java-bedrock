# Converter, Semantic Audit Gate & Embedded Blockbench 3D

## 1. Bidirectional Java ↔ Bedrock Conversion

PackStudio's converter (`crates/rp-core`, `rp-io`, `rp-transforms`, `rp-java`, `rp-bedrock`) parses source packs into a canonical Intermediate Representation (`ResourcePackIR`) before emitting the target format:

- **Version Coverage**: Minecraft Java `1.6.1`–`26.3` (`pack_format 1` through `97.1`) and Bedrock `1.10.0`–`26.60` (`manifest.json` `format_version` 1, 2, and 3).
- **Textures & Animations**: Lazy PNG header inspection, vertical strip split/stitch between `.png.mcmeta` and `flipbook_textures.json`, nearest-neighbor auto-downscaling (`fast_image_resize`), and opaque Bedrock `grass_side.tga` compositing.
- **PBR / Vibrant Visuals**: Converts LabPBR `_n` / `_s` textures into Bedrock `.texture_set.json` + MER maps.
- **3D Geometry & UVs**: Exact `SE(3)` hierarchical bone baking, `inflate`, Box UV + `mirror`, 180° top/bottom UV orientation matching Blockbench, and Java `CustomHeadLayer` wearable preservation.
- **Locales & Audio**: UTF-8 / UTF-16 BOM & ISO-8859-1 section-sign (`§`) handling between Java `.json` and Bedrock `.lang`, plus `.ogg` validation.

### Preflight Before Converting
In the **Converter & Preflight** tab, run **Preflight Scan** first to inspect every asset (`ready`, `composite`, `lossy`, `skipped`) and preview semantic audit status before writing to disk.

---

## 2. Anti-False-Positive Semantic Audit Gate

Before packaging `.zip`, `.mcpack`, or `.mcaddon` archives (and whenever you click **Audit**), PackStudio runs strict semantic checks:

- **Manifest Guard**: Verifies `pack.mcmeta` (`pack_format`) and Bedrock `manifest.json` (`modules`, unique BP/RP UUIDs, and `[1, x, x]` `min_engine_version`).
- **3D UV-Atlas Guard**: Flags oversized or invalid UV atlas assumptions with configurable modes (`Safe Fallback 2D`, `Skip`, `Warn`).
- **Reference Integrity**: Detects broken atlas paths in `terrain_texture.json` / `item_texture.json` and reports orphan output textures.
- **PSTDR Workspace Audit**: Runs `pstdr-lang::audit_workspace`, surfacing:
  - `PSTDR-E001`–`E016` syntax, immutability, scope, and vanilla ID errors.
  - `PSTDR-E017`: Missing RP texture aliases/images or missing `BP/functions/<path>.mcfunction` files referenced by `run_fn`.
  - `PSTDR-E018`: Beta-only `@minecraft/server` APIs (such as `on chat`) used under a stable Script API version.

When packaging a `BedrockAddon` containing `.pstdr` sources into `.mcaddon`, PackStudio automatically compiles `.pstdr` sources, merges `BP/scripts/`, `BP/functions/`, `BP/items/`, `BP/blocks/`, and `BP/manifest.json`, and blocks export on fatal errors.

---

## 3. Embedded Offline Blockbench 3D Studio

PackStudio embeds a verified offline build of **Blockbench Web Studio** (`assets/vendor/blockbench-web.tar.zst`) with a local loopback HTTP/WebSocket bridge (`blockbench.rs`):

- Click **Blockbench** in the title bar or double-click any `.geo.json`, `.bbmodel`, or model JSON in Explorer to open it directly inside the workbench.
- **2-Way Live Sync**: Saving inside Blockbench immediately writes back to the active pack (guarded against path traversal, absolute paths, and Windows drive prefixes) and triggers an automatic Explorer refresh (`blockbench://saved`).
- **Viewport Scaling**: Use the built-in zoom controls (`60%`–`120%`) in the Blockbench tab toolbar for comfortable modeling on high-DPI or compact Linux screens.
- **New `.geo.json`**: Generate valid Bedrock entity geometry scaffolds directly into `models/entity/` (or `RP/models/entity/` in Add-on workspaces).

> [← Previous: Getting Started](./PackStudio-Getting-Started.md) | [Next: Verified Sync & AI Agents →](./PackStudio-Verified-Sync-and-AI.md)
