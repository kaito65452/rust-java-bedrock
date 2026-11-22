# rust-java-bedrock (`rp-cli`)

A bidirectional **Minecraft Resource Pack Converter** (`Java Edition <-> Bedrock Edition`) written in safe, zero-panic Rust (`edition = "2024"`, MIT licensed).

It parses `.zip`, `.mcpack`, and unpacked directory packs into a canonical Intermediate Representation (`ResourcePackIR`), transforms textures, animations, 3D geometry, sounds, localizations, and attachables across versions, and emits valid target-edition resource packs with structured conversion diagnostics.

---

## Key Features

- **Wide Version Coverage (`1.6.1` to `26.3` / `26.50`)**:
  - **Java Edition**: Supports legacy pre-flattening packs (`pack_format 1..=3`, `textures/blocks/`, `textures/items/`, `.lang`), `supported_formats` (`1.20.2+`), `1.19.3+` `assets/minecraft/atlases/blocks.json` & `1.21.4+` `atlases/items.json` stitching, `1.21.4+` `assets/minecraft/items/*.json` client item definitions + `light_emission`, `1.21.11+` / `26.x` multi-axis Euler model rotations (`x`, `y`, `z`, `ZYX` order) without `[-45, 45]` limits, `1.21.9+` / `26.x` `min_format` / `max_format` (`84` for `26.1`, `88` for `26.2`, `97.1` for `26.3`), and `26.3` `shade_direction_override`.
  - **Bedrock Edition**: Supports `manifest.json` `format_version` `1`, `2`, and `3` (SemVer `min_engine_version` such as `26.10.0`, `26.30.0`, `26.50.0`), automatic RFC 4122 UUID v4 generation, `terrain_texture.json`, `item_texture.json`, `flipbook_textures.json`, `.geo.json` (`1.10.0` legacy, `1.12.0`, `1.16.0`, `1.21.0`), `attachables/*.json`, `sound_definitions.json`, and `texts/*.lang` + `texts/languages.json`.
- **Safe & Deterministic Container I/O**:
  - Reads `.zip`, `.mcpack`, and directories with `rayon` parallel file ingestion, Zip Slip / symlink traversal rejection, configurable file-count and uncompressed byte limits (`IoLimits`), deterministic `BTreeMap` ordering, and automatic unwrapping of nested single-pack wrapper folders.
- **Lazy Image & Audio Handling**:
  - Inspects PNG headers without eagerly decoding every pixel buffer into memory, converts vertical animation strips between `.png.mcmeta` and `flipbook_textures.json` (including non-uniform frame-time expansion), and validates `.ogg` headers.
- **3D Model Geometry & UV Conversion (Blockbench-Verified)**:
  - Bidirectional coordinate, pivot, `inflate`, `uv_rotation`, 180° `up`/`down` per-face UV flip, Box UV + `mirror`, relative auto-UV, and exact `SE(3)` `ZYX` hierarchical bone-chain baking between Java `[0..16]` block space and Bedrock `.geo.json`.
  - Preserves exact player-head wearable offsets relative to Bedrock's `head` bone (`[0, 24, 0]`), emits 1:1 Java `CustomHeadLayer` `display.head` transforms (`scale: [1.6, 1.6, 1.6]`), normalizes oversized geometry into Java's `[-16, 32]` element bounds, and rasterizes 3D-rotated helmet armor sheets (`128×64`).
- **Zero-Copy JSONC Sanitization & Strict Mode**:
  - Handles UTF-8 BOM, `//` / `/* ... */` comments, and trailing commas commonly found in community packs, and supports `--strict` mode for failing on skipped, lossy, or warning diagnostics.

---

## Workspace Architecture (6 Crates)

The converter decouples format parsing/emission from a shared canonical IR so `rp-java` and `rp-bedrock` never depend on each other:

| Crate | Purpose |
|---|---|
| [`crates/rp-core`](crates/rp-core) | Canonical `ResourcePackIR`, lazy `TexturePayload`, `AnimationSpec`, `ModelAsset`, `SoundRegistry`, `LocaleTable`, `AttachableAsset`, `ConversionReport`, `MappingTable` ([`data/default_mappings.json`](crates/rp-core/data/default_mappings.json)), and zero-copy JSONC/BOM sanitizer ([`jsonc.rs`](crates/rp-core/src/jsonc.rs)). |
| [`crates/rp-io`](crates/rp-io) | `PackContainer` abstraction for `.zip`, `.mcpack`, and directories with Zip Slip / symlink checks, `IoLimits`, `rayon` parallel reads, deterministic output ordering, and root-folder unwrapping. |
| [`crates/rp-transforms`](crates/rp-transforms) | Stateless format transformers: PNG header inspection, strip split/stitch, `.png.mcmeta` <-> `flipbook_textures.json`, `terrain_texture.json` / `item_texture.json` builders, Java <-> Bedrock 3D geometry & UV transforms, `.json` <-> `.lang` locales, and `.ogg` validation. |
| [`crates/rp-java`](crates/rp-java) | Decoupled Java Edition parser & emitter (`pack.mcmeta`, multi-namespace `lang/`, `models/`, `items/`, `atlases/blocks.json`, `sounds.json`, `overlays`, and skipping Java-only `shaders/` / `optifine/` / `mcpatcher/`). |
| [`crates/rp-bedrock`](crates/rp-bedrock) | Decoupled Bedrock Edition parser & emitter (`manifest.json` v1/v2/v3, UUID v4 generation, atlases, flipbooks, `.geo.json` `1.10.0`–`1.21.0`, `attachables/*.json`, `sound_definitions.json`, and `texts/*.lang`). |
| [`crates/rp-cli`](crates/rp-cli) | `clap` CLI binary (`convert`, `inspect`, `versions`) with `indicatif` progress spinner, diagnostic tables, JSON report export, integration tests, and `proptest` fuzz/round-trip suites. |

---

## Build & Test

### Build CLI Binary
```bash
cargo build --release -p rp-cli
# Binary output: target/release/rp-cli
```

Or debug build:
```bash
cargo build -p rp-cli
# Binary output: target/debug/rp-cli
```

### Run Full Workspace Test Suite
```bash
cargo test --workspace
```

---

## CLI Usage

### 1. Inspect a Resource Pack
Prints detected edition (`Java` or `Bedrock`), format/engine version, asset counts, and compatibility warnings:
```bash
rp-cli inspect path/to/pack.zip
```

### 2. List Supported Minecraft Versions
Displays supported Java `pack_format`, Bedrock `min_engine_version`, default `.geo.json` version, and format features from `1.6.1` up through `26.3` / `26.50`:
```bash
rp-cli versions
```

### 3. Convert Resource Packs

**Bedrock `.mcpack` / `.zip` -> Java `26.2` / `26.3` (`pack_format 88` / `97.1`, multi-axis `{x, y, z}` rotations, `items/*.json`, `atlases/`):**
```bash
rp-cli convert input_bedrock.mcpack -o output_java.zip --to java --target-version 26.2
# Or for Java 26.3 (pack_format 97.1 + shade_direction_override):
rp-cli convert input_bedrock.mcpack -o output_java_26_3.zip --to java --target-version 26.3
```

**Java `.zip` -> Bedrock `26.2` (`manifest.json` v3 SemVer + `.geo.json` `1.21.0`):**
```bash
rp-cli convert input_java.zip -o output_bedrock.mcpack \
  --to bedrock \
  --target-version 26.2 \
  --bedrock-manifest-version 3 \
  --bedrock-geo-version 1.21.0
```

**Legacy Pre-Flattening Java (`1.8.9`):**
```bash
rp-cli convert input_bedrock.mcpack -o output_java_1_8.zip \
  --to java \
  --target-version 1.8.9 \
  --rotation-mode snap
```

### `convert` Options
| Flag | Description |
|---|---|
| `-o, --output <PATH>` | Output `.zip`, `.mcpack`, or directory path (required). |
| `--to <java\|bedrock>` | Target edition (defaults to opposite of detected source edition). |
| `--target-version <VER>` | Target Minecraft version or Java `pack_format` (e.g., `1.8.9`, `1.20.1`, `1.21.11`, `26.1`, `26.2`, `26.3`, `97`). |
| `--supported-formats <MIN,MAX>` | Multi-version Java compatibility range (e.g., `15,88`). |
| `--rotation-mode <preserve\|snap\|clamp>` | How Bedrock angles outside Java 22.5-degree increments are handled (default: `preserve`). |
| `--bedrock-geo-version <1.10.0\|1.12.0\|1.16.0\|1.21.0>` | Override target Bedrock `.geo.json` `format_version`. |
| `--bedrock-manifest-version <1\|2\|3>` | Override Bedrock `manifest.json` `format_version` (`3` uses SemVer strings + `metadata.authors`). |
| `--items-model-defs` / `--no-items-model-defs` | Force-enable or disable emitting Java `1.21.4+` `assets/minecraft/items/*.json`. |
| `--mapping <FILE>` | Load a custom JSON mapping override file on top of embedded vanilla mappings. |
| `--strict` | Fail with non-zero exit code if any skipped, lossy, or warning diagnostic occurs. |
| `--report-json <FILE>` | Write the structured `ConversionReport` JSON to a file. |

---

## License

This project is licensed under the MIT License (see [Cargo.toml](Cargo.toml)).
