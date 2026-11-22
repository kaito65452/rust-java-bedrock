# Minecraft Java ↔ Bedrock Resource Pack Reference Specification (`minecraft.wiki`)

> **Verification Policy**: Every path, version rule, JSON schema, and engine limitation in this document was collected directly from `minecraft.wiki` (via MediaWiki API) and cross-verified against Mojang's official [`bedrock-samples`](https://github.com/Mojang/bedrock-samples) (`v26.60`).

## Primary Reference Sources
- Pack Structure: https://minecraft.wiki/w/Resource_pack
- Java Pack Format Table: https://minecraft.wiki/w/Pack_format & https://minecraft.wiki/w/Template:Resource_pack_format
- Java `pack.mcmeta` Schema: https://minecraft.wiki/w/Pack.mcmeta
- Java Model & Blockstates: https://minecraft.wiki/w/Model & https://minecraft.wiki/w/Items_model_definition
- Bedrock `manifest.json` Schema: https://minecraft.wiki/w/Manifest.json
- Bedrock Version History & 26.x Numbering: https://minecraft.wiki/w/Bedrock_Edition_version_history & https://minecraft.wiki/w/Bedrock_Edition_26.60
- Textures & GUI/Effect Sprites: https://minecraft.wiki/w/Textures & https://minecraft.wiki/w/Bedrock_Edition_GUI,_UI,_and_Ore_UI_textures
- Bedrock PBR & Texture Sets: https://minecraft.wiki/w/Vibrant_Visuals#Texture_sets
- Bedrock Custom Block Components: https://minecraft.wiki/w/Block_components
- Official Vanilla Bedrock Schema Cross-Check: https://github.com/Mojang/bedrock-samples/tree/main/resource_pack

---

## 1. Pack Format & Version Mapping

### 1.1 Java Edition `pack.mcmeta` Rules (`minecraft.wiki/w/Pack.mcmeta` & `Pack_format`)
| Minecraft Java Version | Resource `pack_format` | Metadata & Folder Structure Milestones |
|---|---|---|
| `1.6.1` – `1.8.9` | `1` | Pre-flattening: `textures/blocks/`, `textures/items/`, `lang/en_US.lang` |
| `1.9` – `1.10.2` | `2` | Multipart blockstates, item predicate overrides, updated display slots |
| `1.11` – `1.12.2` | `3` | All filenames lowercase (`lang/en_us.lang`), still `blocks/` & `items/` |
| `1.13` – `1.14.4` | `4` | **The Flattening**: `textures/block/`, `textures/item/`, `lang/*.json`; 1.14 splits `mob_effect/`, `painting/`, `particle/` into individual PNGs |
| `1.15` – `1.16.1` | `5` | Texture mechanic changes |
| `1.16.2` – `1.16.5` | `6` | Wall block state updates |
| `1.17` – `1.17.1` | `7` | `credits.json` replaces `credits.txt` |
| `1.18` – `1.18.2` | `8` | `inventory.png` thin layout, `regional_compliancies.json` |
| `1.19` – `1.19.2` | `9` | `filter` section in `pack.mcmeta` |
| `1.19.3` | `12` | `assets/minecraft/atlases/blocks.json` required for non-standard texture directories |
| `1.19.4` | `13` | Enchantment glint split |
| `1.20` – `1.20.1` | `15` | Font & credits updates |
| `1.20.2` | `18` | `supported_formats` & `overlays` added to `pack.mcmeta` |
| `1.20.3` – `1.20.4` | `22` | PNG is the only supported texture format |
| `1.20.5` – `1.20.6` | `32` | Menu GUI & map icon updates |
| `1.21` – `1.21.1` | `34` | Tricky Trials |
| `1.21.2` – `1.21.3` | `42` | `light_emission` field added to model elements |
| `1.21.4` | `46` | **`assets/minecraft/items/*.json`** (`Items model definition`) replaces `overrides`; block & item model formats unified |
| `1.21.5` | `55` | Spring to Life |
| `1.21.6` | `63` | Arbitrary rotation angles in `[-45, 45]` (no longer restricted to 22.5° steps); `mob_effect` folded into GUI atlas |
| `1.21.7` – `1.21.8` | `64` | New textures |
| `1.21.9` – `1.21.10` | `69.0` | **`min_format` / `max_format`** (`int` or `[major, minor]`) introduced at `65.0`; `pack_format` & `supported_formats` omitted unless supporting `< 65` |
| `1.21.11` | `75.0` | Multi-axis Euler element rotations `{x, y, z}` without `[-45, 45]` restriction |
| `26.1` – `26.1.2` | `84.0` | Year-based versioning; `textures` map supports `{ "sprite": "...", "force_translucent": bool }` |
| `26.2` | `88.0` | Java 26.2 resource pack format |
| `26.3` | `97.1` (`[97, 1]`) | `shade_direction_override` replaces element `shade: false` |

### 1.2 Bedrock Edition `manifest.json` & `26.x` Versioning Rules (`minecraft.wiki/w/Manifest.json` & `Bedrock_Edition_26.60`)
- **`format_version`**:
  - `1`: Legacy integer-array `[major, minor, patch]` versions; `min_engine_version` optional.
  - `2`: Standard stable format; `version` and `min_engine_version` are 3-element integer arrays `[major, minor, patch]`.
  - `3`: Modern format; `version` and `min_engine_version` are SemVer strings `"major.minor.patch"`, and `metadata.authors` is **mandatory**.
- **UUID Uniqueness**: `header.uuid` and each `modules[i].uuid` **must** be distinct valid RFC-4122 UUIDs.
- **Module Type**: Resource packs must declare `"type": "resources"` (v2/v3).
- **`1.x.x` vs `26.x` Engine Version Compatibility**:
  - Starting with Bedrock `26.0` (Feb 2026), public display versions follow `YY.MH` (`26.0`, `26.10`, `26.20`, `26.30`, `26.40`, `26.50`, `26.60`).
  - Internally—and inside `manifest.json` `min_engine_version`—Bedrock represents `26.60` as **`[1, 26, 60]`** (v2) or **`"1.26.60"`** (v3), as confirmed by `minecraft.wiki/w/Bedrock_Edition_26.60` ("`26.60`, also known as `1.26.60`") and Mojang's official `bedrock-samples/resource_pack/manifest.json` (`"min_engine_version": [1, 26, 60]`).
  - Converters **must** accept `1.21.x`, `1.26.x`, and `26.x` (e.g. `26.60`, `26.50`, `26.3`) without hardcoded rejection and emit valid `min_engine_version` triplets (`[1, 26, 60]` for Bedrock `26.60`).

---

## 2. Texture Path & Atlas Mapping (Java ↔ Bedrock)

| Java Edition Path (`assets/<ns>/...`) | Bedrock Edition Path | Notes |
|---|---|---|
| `assets/minecraft/textures/block/<id>.png` | `textures/blocks/<bedrock_id>.png` | Registered in `textures/terrain_texture.json` (`atlas.terrain`) |
| `assets/minecraft/textures/item/<id>.png` | `textures/items/<bedrock_id>.png` | Registered in `textures/item_texture.json` (`atlas.items`) |
| `assets/minecraft/textures/mob_effect/<id>.png` | `textures/ui/<id>_effect.png` | **Critical**: Bedrock does NOT read `textures/mob_effect/`. Exception: `hero_of_the_village` ↔ `textures/ui/village_hero_effect.png`. |
| `assets/minecraft/textures/gui/...` | `textures/ui/...` | Note: Bedrock Ore UI (`hbui`) cannot be modified by resource packs. |
| `assets/minecraft/textures/colormap/...` | `textures/colormap/...` | 256×256 biome colormaps (`grass.png`, `foliage.png`). |
| `assets/minecraft/textures/entity/...` | `textures/entity/...` | Mob/entity textures. |
| `assets/minecraft/textures/painting/...` | `textures/painting/...` | Painting textures. |
| `assets/minecraft/textures/particle/...` | `textures/particle/...` | Particle textures (Bedrock also uses `textures/particles/`). |
| `assets/minecraft/textures/environment/...` | `textures/environment/...` | Sun, moon, clouds, rain/snow. |
| `assets/minecraft/textures/models/armor/...` | `textures/models/armor/...` | Armor layer sheets. |
| `assets/minecraft/textures/**/*.png.mcmeta` | `textures/flipbook_textures.json` | Links `flipbook_texture` path to `atlas_tile` in `terrain_texture.json`. |
| `assets/minecraft/lang/<lc>_<cc>.json` | `texts/<lc>_<CC>.lang` + `texts/languages.json` | Do not emit empty `texts/languages.json` (`[]`) when no `.lang` files exist. |

### Bedrock Atlas & Block Registration Rules
- `textures/terrain_texture.json`: Maps Bedrock terrain shortnames (e.g. `bamboo_stem`, `cactus_side`, `short_grass`, `oak_planks`, `stone`) to relative texture paths without `.png`.
- `textures/item_texture.json`: Maps Bedrock item shortnames (e.g. `apple`, `diamond_sword`, `bow_standby`) to relative texture paths without `.png`.
- `blocks.json`: Maps vanilla block identifiers (`bamboo`, `cactus`, `short_grass`, etc.) to their `terrain_texture.json` face keys (`up`, `down`, `side`, `north`, `south`, `east`, `west`).
- **Custom/non-vanilla texture names in Java packs** (e.g. `block/custom_tex_a.png` referenced by `models/item/apple.json`, or `block/custom_tex_b.png` referenced by `models/block/bamboo.json`) **will NOT be read by Bedrock** if copied raw as `textures/blocks/custom_tex_a.png` without being mapped to the target vanilla Bedrock texture path and/or registered under the exact vanilla Bedrock key (`apple` in `item_texture.json`, `bamboo_stem` in `terrain_texture.json`).

---

## 3. Models & Geometry (`minecraft.wiki/w/Model`, `Items_model_definition`, `Block_components`)

### 3.1 Java Model Resolution Graph
1. **Blocks**: `assets/<ns>/blockstates/<block_id>.json` (`variants` or `multipart`) → references model(s) in `assets/<ns>/models/<model_path>.json`.
2. **Items**:
   - Pre-1.21.4: `assets/<ns>/models/item/<item_id>.json`.
   - 1.21.4+ (`pack_format >= 46`): `assets/<ns>/items/<item_id>.json` (`model.model`, `cases`, `fallback`, `entries`, `on_true`/`on_false`) → references model(s) in `assets/<ns>/models/<model_path>.json`.
3. **Model Inheritance & Texture Variables**:
   - Follow `parent` chains (detecting cycles).
   - Resolve `#var` references recursively across the model's `textures` map (supporting both string values and Java 26.1+ `{ "sprite": "..." }` objects) and `elements[].faces.*.texture`.

### 3.2 Hard Bedrock Pure Resource Pack Geometry Limitation
- **Pure Bedrock Resource Packs CANNOT override vanilla 3D block geometry using `models/entity/block/*.geo.json`.**
- Valid uses of `models/entity/*.geo.json` in Bedrock Resource Packs:
  1. Entity models (`models/entity/<entity>.geo.json` bound by `entity/*.entity.json`).
  2. Item **attachables** (`attachables/*.json` binding an equipped/held item to a `.geo.json` geometry + texture).
  3. **Custom blocks** (requires a companion **Behavior Pack** defining `minecraft:geometry` in `bp/blocks/*.json` referencing `models/blocks/*.geo.json`).
- Therefore, when converting Java 3D block models to a pure Bedrock Resource Pack:
  - **NEVER** emit dead `models/entity/block/*.geo.json` files.
  - Extract the primary resolved texture(s) to the vanilla Bedrock block/item texture path & atlas so the texture still renders in-game.
  - Emit a structured diagnostic warning explaining this engine limitation and the actionable alternatives (Behavior Pack custom block or item attachable).

---

## 4. PBR / Texture Sets & Texts (`minecraft.wiki/w/Vibrant_Visuals#Texture_sets`)

### 4.1 Java LabPBR vs Bedrock PBR (`*.texture_set.json`)
- **Java LabPBR**: Sidecar textures `<name>_n.png` (normal XY + AO + height) and `<name>_s.png` (R = smoothness, G = F0/metalness `>= 230`, B = porosity/SSS, A = emissive `0..254`, `255 = 0`).
- **Bedrock PBR (`*.texture_set.json`)**:
  - `format_version`: `"1.16.100"` (MER) or `"1.21.30"` (MERS / Vibrant Visuals).
  - `minecraft:texture_set.color` (required): base texture stem in same directory.
  - `minecraft:texture_set.metalness_emissive_roughness`: `*_mer` PNG (R = metalness, G = emissive, B = roughness) — mutually exclusive with `metalness_emissive_roughness_subsurface`.
  - `minecraft:texture_set.normal`: `*_normal` PNG — **mutually exclusive** with `heightmap`.
  - `manifest.json` must declare `"capabilities": ["pbr"]` (or `"raytraced"`).
  - **Rule**: When PBR conversion is disabled, raw Java `*_n.png` and `*_s.png` files **must NOT be copied** into the Bedrock `.mcpack` (otherwise they become dead orphan textures bloating archive size by tens of megabytes).

### 4.2 Texts & Non-Java Leakage Rules
- Java `assets/minecraft/texts/` (`splashes.txt`, `end.txt`, `credits.json`, `postcredits.txt`) **must NEVER leak** under `assets/minecraft/...` into a Bedrock `.mcpack`.
  - `assets/minecraft/texts/splashes.txt` may be converted to Bedrock root `splashes.json` (`{ "splashes": [...] }`) or explicitly skipped with a warning.
  - `credits.json`, `end.txt`, `postcredits.txt` have no pure Bedrock RP equivalent and must be skipped with an actionable warning.
- OS junk files (`desktop.ini`, `.DS_Store`, `Thumbs.db`, `__MACOSX/`) must be filtered at scan time.

---

## 5. Machine-Readable Artifacts
- [data/spec/pack_versions.json](../../data/spec/pack_versions.json)
- [data/spec/asset_mappings.json](../../data/spec/asset_mappings.json)
- [data/spec/bedrock_acceptance_rules.json](../../data/spec/bedrock_acceptance_rules.json)
