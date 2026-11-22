use std::collections::BTreeSet;
use std::io::Cursor;

use image::{ImageFormat, Rgba, RgbaImage};
use tempfile::TempDir;

use rp_bedrock::schema::{BedrockManifest, BedrockSoundDefinitionsFile};
use rp_cli::{
    ConvertOptions, VersionSpecificOptions, convert_container_with_options, convert_pack,
    inspect_pack,
};
use rp_core::{BedrockGeoVersion, ConversionReport, Edition, MappingTable, RotationSnapMode};
use rp_io::{IoLimits, PackContainer};
use rp_java::parse_java_pack;
use rp_java::schema::{JavaItemDefinitionJson, JavaPackMcmeta, JavaSoundEventJson};
use rp_transforms::texture::{
    AtlasTexturePath, AtlasVariant, FlipbookEntry, ItemTextureJson, TerrainTextureJson,
};

fn encode_png(width: u32, height: u32, color: Rgba<u8>) -> Vec<u8> {
    let mut img = RgbaImage::new(width, height);
    for px in img.pixels_mut() {
        *px = color;
    }
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .expect("encode png");
    buf
}

fn encode_animated_strip(width: u32, frame_h: u32, frames: u32) -> Vec<u8> {
    let mut img = RgbaImage::new(width, frame_h * frames);
    for f in 0..frames {
        let color = Rgba([(f * 60) as u8, 120, 220, 255]);
        for y in (f * frame_h)..((f + 1) * frame_h) {
            for x in 0..width {
                img.put_pixel(x, y, color);
            }
        }
    }
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .expect("encode strip png");
    buf
}

fn build_realistic_java_pack_zip() -> (Vec<u8>, Vec<u8>) {
    let mut c = PackContainer::new();
    c.insert(
        "pack.mcmeta",
        br#"{
            "pack": {
                "pack_format": 15,
                "description": "Synthetic Java Pack"
            }
        }"#
        .to_vec(),
    )
    .expect("pack.mcmeta");

    let icon = encode_png(64, 64, Rgba([20, 180, 90, 255]));
    c.insert("pack.png", icon).expect("pack.png");

    let stone_png = encode_png(16, 16, Rgba([128, 128, 128, 255]));
    c.insert(
        "assets/minecraft/textures/block/stone.png",
        stone_png.clone(),
    )
    .expect("stone.png");

    let planks_png = encode_png(16, 16, Rgba([180, 130, 70, 255]));
    c.insert("assets/minecraft/textures/block/oak_planks.png", planks_png)
        .expect("oak_planks.png");

    let water_strip = encode_animated_strip(16, 16, 4);
    c.insert(
        "assets/minecraft/textures/block/water_still.png",
        water_strip,
    )
    .expect("water_still.png");
    c.insert(
        "assets/minecraft/textures/block/water_still.png.mcmeta",
        br#"{
            "animation": {
                "frametime": 3,
                "interpolate": true,
                "frames": [0, 1, 2, 3]
            }
        }"#
        .to_vec(),
    )
    .expect("water_still.png.mcmeta");

    let sword_png = encode_png(16, 16, Rgba([60, 220, 230, 255]));
    c.insert(
        "assets/minecraft/textures/item/diamond_sword.png",
        sword_png,
    )
    .expect("diamond_sword.png");

    c.insert(
        "assets/minecraft/models/block/custom_pedestal.json",
        br##"{
            "parent": "minecraft:block/cube_all",
            "texture_size": [16, 16],
            "textures": {
                "all": "block/stone"
            },
            "elements": [
                {
                    "name": "pillar",
                    "from": [4, 0, 4],
                    "to": [12, 16, 12],
                    "rotation": { "origin": [8, 8, 8], "axis": "y", "angle": 45.0 },
                    "faces": {
                        "north": { "uv": [0, 0, 8, 16], "texture": "#all" },
                        "south": { "uv": [0, 0, 8, 16], "texture": "#all" },
                        "east":  { "uv": [0, 0, 8, 16], "texture": "#all" },
                        "west":  { "uv": [0, 0, 8, 16], "texture": "#all" },
                        "up":    { "uv": [0, 0, 8, 8], "texture": "#all" },
                        "down":  { "uv": [0, 0, 8, 8], "texture": "#all" }
                    }
                }
            ]
        }"##
        .to_vec(),
    )
    .expect("custom_pedestal.json");

    c.insert(
        "assets/minecraft/sounds.json",
        br#"{
            "entity.zombie.ambient": {
                "category": "hostile",
                "sounds": [
                    { "name": "mob/zombie/say1", "volume": 0.9, "pitch": 1.0 }
                ]
            }
        }"#
        .to_vec(),
    )
    .expect("sounds.json");

    c.insert(
        "assets/minecraft/sounds/mob/zombie/say1.ogg",
        b"OggS\x00\x02dummy_vorbis_audio_stream".to_vec(),
    )
    .expect("say1.ogg");

    c.insert(
        "assets/minecraft/lang/en_us.json",
        br#"{
            "block.minecraft.stone": "Smooth Stone",
            "item.minecraft.diamond_sword": "Azure Blade",
            "pack.custom.title": "My Pack"
        }"#
        .to_vec(),
    )
    .expect("en_us.json");

    (c.write_to_zip_bytes().expect("zip bytes"), stone_png)
}

fn build_realistic_bedrock_pack_zip() -> Vec<u8> {
    let mut c = PackContainer::new();
    let manifest = BedrockManifest::new(
        "Synthetic Bedrock Pack",
        "Bedrock Fixture Description",
        [1, 0, 0],
        [1, 20, 10],
    );
    c.insert(
        "manifest.json",
        serde_json::to_vec_pretty(&manifest).expect("manifest"),
    )
    .expect("insert manifest");
    c.insert(
        "pack_icon.png",
        encode_png(64, 64, Rgba([220, 140, 40, 255])),
    )
    .expect("pack_icon.png");

    c.insert(
        "textures/blocks/stone.png",
        encode_png(16, 16, Rgba([100, 100, 100, 255])),
    )
    .expect("stone");
    c.insert(
        "textures/blocks/lava_still.png",
        encode_animated_strip(16, 16, 3),
    )
    .expect("lava");
    c.insert(
        "textures/items/ender_pearl.png",
        encode_png(16, 16, Rgba([30, 160, 130, 255])),
    )
    .expect("pearl");

    c.insert(
        "textures/terrain_texture.json",
        br#"{
            "resource_pack_name": "Synthetic Bedrock Pack",
            "texture_name": "atlas.terrain",
            "texture_data": {
                "stone": { "textures": "textures/blocks/stone" },
                "lava_still": { "textures": "textures/blocks/lava_still" }
            }
        }"#
        .to_vec(),
    )
    .expect("terrain_texture.json");

    c.insert(
        "textures/item_texture.json",
        br#"{
            "resource_pack_name": "Synthetic Bedrock Pack",
            "texture_name": "atlas.items",
            "texture_data": {
                "ender_pearl": { "textures": "textures/items/ender_pearl" }
            }
        }"#
        .to_vec(),
    )
    .expect("item_texture.json");

    c.insert(
        "textures/flipbook_textures.json",
        br#"[
            {
                "flipbook_texture": "textures/blocks/lava_still",
                "atlas_tile": "lava_still",
                "ticks_per_frame": 4,
                "frames": [0, 2, 1],
                "blend_frames": true
            }
        ]"#
        .to_vec(),
    )
    .expect("flipbook");

    c.insert(
        "models/entity/custom_totem.geo.json",
        br#"{
            "format_version": "1.12.0",
            "minecraft:geometry": [
                {
                    "description": {
                        "identifier": "geometry.custom_totem",
                        "texture_width": 16,
                        "texture_height": 16
                    },
                    "bones": [
                        {
                            "name": "body",
                            "pivot": [0.0, 0.0, 0.0],
                            "cubes": [
                                {
                                    "origin": [-4.0, 0.0, -4.0],
                                    "size": [8.0, 12.0, 8.0],
                                    "uv": {
                                        "north": { "uv": [0, 0], "uv_size": [8, 12] },
                                        "up": { "uv": [0, 0], "uv_size": [8, 8] }
                                    }
                                }
                            ]
                        }
                    ]
                }
            ]
        }"#
        .to_vec(),
    )
    .expect("geo");

    c.insert(
        "sounds/sound_definitions.json",
        br#"{
            "format_version": "1.14.0",
            "sound_definitions": {
                "dig.stone": {
                    "category": "block",
                    "sounds": ["sounds/dig/stone1"]
                }
            }
        }"#
        .to_vec(),
    )
    .expect("sound_defs");
    c.insert("sounds/dig/stone1.ogg", b"OggS\x00\x02stone_break".to_vec())
        .expect("stone1.ogg");

    c.insert("texts/languages.json", br#"["en_US"]"#.to_vec())
        .expect("languages.json");
    c.insert(
        "texts/en_US.lang",
        b"## Bedrock English\ntile.stone.stone.name=Bedrock Stone\nitem.enderPearl.name=Void Pearl\n"
            .to_vec(),
    )
    .expect("en_US.lang");

    c.write_to_zip_bytes().expect("write bedrock zip")
}

fn collect_atlas_paths(path: &AtlasTexturePath) -> Vec<String> {
    match path {
        AtlasTexturePath::Single(s) => vec![s.clone()],
        AtlasTexturePath::List(items) => items
            .iter()
            .map(|v| match v {
                AtlasVariant::Path(s) => s.clone(),
                AtlasVariant::Object { path } => path.clone(),
            })
            .collect(),
    }
}

#[test]
fn end_to_end_java_to_bedrock_zero_broken_references() {
    let temp = TempDir::new().expect("tempdir");
    let (java_zip_bytes, raw_stone_png) = build_realistic_java_pack_zip();
    let input_zip = temp.path().join("java_pack.zip");
    let output_zip = temp.path().join("bedrock_pack.mcpack");
    let report_path = temp.path().join("report.json");
    std::fs::write(&input_zip, java_zip_bytes).expect("write java zip");

    let summary = inspect_pack(&input_zip, None, IoLimits::default()).expect("inspect java");
    assert_eq!(summary.edition, Edition::Java);
    assert_eq!(summary.java_pack_format, Some(15));
    assert_eq!(summary.texture_count, 4);
    assert_eq!(summary.animated_texture_count, 1);
    assert_eq!(summary.model_count, 1);
    assert_eq!(summary.sound_event_count, 1);
    assert_eq!(summary.locale_count, 1);

    let (src, dst, report) = convert_pack(&ConvertOptions {
        input: input_zip,
        output: output_zip.clone(),
        target: None,
        target_version: Some(String::from("1.20.1")),
        mapping_file: None,
        strict: false,
        report_json: Some(report_path.clone()),
        limits: IoLimits::default(),
        version_options: VersionSpecificOptions::default(),
    })
    .expect("convert java -> bedrock");

    assert_eq!(src, Edition::Java);
    assert_eq!(dst, Edition::Bedrock);
    assert!(report.converted_count > 0);
    assert!(report.passthrough_count > 0);
    assert_eq!(report.skipped_count, 0);

    let written_report: ConversionReport =
        serde_json::from_slice(&std::fs::read(&report_path).expect("read report"))
            .expect("parse report");
    assert_eq!(written_report, report);

    let out = PackContainer::read_from_path(&output_zip, IoLimits::default()).expect("read output");

    // 1. manifest.json valid + RFC-4122 v4 UUIDs
    let manifest: BedrockManifest =
        serde_json::from_slice(out.get("manifest.json").expect("manifest.json")).expect("parse");
    assert_eq!(manifest.format_version, 2);
    assert_eq!(manifest.header.min_engine_version, [1, 20, 10]);
    assert_ne!(manifest.header.uuid, manifest.modules[0].uuid);

    // Raw PNG bytes were passed through identically (no re-encoding).
    assert_eq!(
        out.get("textures/blocks/stone.png").expect("stone.png"),
        raw_stone_png.as_slice()
    );

    // 2. terrain_texture.json & item_texture.json valid + zero broken texture references
    let terrain: TerrainTextureJson = serde_json::from_slice(
        out.get("textures/terrain_texture.json")
            .expect("terrain_texture.json"),
    )
    .expect("parse terrain_texture.json");
    let mut terrain_tiles = BTreeSet::new();
    for (tile_name, entry) in &terrain.texture_data {
        terrain_tiles.insert(tile_name.clone());
        for tex_path in collect_atlas_paths(&entry.textures) {
            let png = format!("{tex_path}.png");
            assert!(
                out.contains(&png),
                "broken terrain_texture.json reference: missing '{png}'"
            );
        }
    }

    let items: ItemTextureJson = serde_json::from_slice(
        out.get("textures/item_texture.json")
            .expect("item_texture.json"),
    )
    .expect("parse item_texture.json");
    for entry in items.texture_data.values() {
        for tex_path in collect_atlas_paths(&entry.textures) {
            let png = format!("{tex_path}.png");
            assert!(
                out.contains(&png),
                "broken item_texture.json reference: missing '{png}'"
            );
        }
    }

    // 3. flipbook_textures.json valid + references existing PNG and terrain tile
    let flipbooks: Vec<FlipbookEntry> = serde_json::from_slice(
        out.get("textures/flipbook_textures.json")
            .expect("flipbook_textures.json"),
    )
    .expect("parse flipbook_textures.json");
    assert_eq!(flipbooks.len(), 1);
    assert_eq!(flipbooks[0].ticks_per_frame, 3);
    assert_eq!(flipbooks[0].blend_frames, Some(true));
    assert_eq!(flipbooks[0].frames, Some(vec![0, 1, 2, 3]));
    assert!(out.contains(&format!("{}.png", flipbooks[0].flipbook_texture)));
    assert!(terrain_tiles.contains(&flipbooks[0].atlas_tile));

    // 4. languages.json valid + every referenced .lang exists
    let langs: Vec<String> =
        serde_json::from_slice(out.get("texts/languages.json").expect("languages.json"))
            .expect("parse languages.json");
    assert_eq!(langs, vec!["en_US"]);
    for code in &langs {
        let lang_path = format!("texts/{code}.lang");
        assert!(out.contains(&lang_path), "missing '{lang_path}'");
    }
    let en_us_lang =
        std::str::from_utf8(out.get("texts/en_US.lang").expect("en_US.lang")).expect("utf8");
    assert!(en_us_lang.contains("tile.stone.stone.name=Smooth Stone"));
    assert!(en_us_lang.contains("item.diamond_sword.name=Azure Blade"));
    assert!(en_us_lang.contains("pack.custom.title=My Pack"));

    // 5. sound_definitions.json valid + every referenced sound exists
    let sound_defs: BedrockSoundDefinitionsFile = serde_json::from_slice(
        out.get("sounds/sound_definitions.json")
            .expect("sound_definitions.json"),
    )
    .expect("parse sound_definitions.json");
    assert!(sound_defs.sound_definitions.contains_key("mob.zombie.say"));
    for def in sound_defs.sound_definitions.values() {
        for variant in &def.sounds {
            let sound_path = match variant {
                rp_bedrock::schema::BedrockSoundVariantJson::Simple(p) => p,
                rp_bedrock::schema::BedrockSoundVariantJson::Detailed { name, .. } => name,
            };
            let ogg = format!("{sound_path}.ogg");
            assert!(
                out.contains(&ogg),
                "broken sound_definitions.json reference: missing '{ogg}'"
            );
        }
    }

    // 6. Pure Bedrock Resource Pack MUST NOT emit dead vanilla block .geo.json,
    //    and MUST emit a structured warning explaining the Bedrock limitation.
    assert!(!out.contains("models/entity/block/custom_pedestal.geo.json"));
    assert!(
        report.warnings.iter().any(|w| {
            w.source_path.contains("custom_pedestal")
                && !w.reason.is_empty()
                && !w.actionable_fix.is_empty()
        }),
        "expected structured warning for omitted vanilla 3D block geometry"
    );
    let audit = report.semantic_audit.expect("semantic audit present");
    assert_eq!(audit.forbidden_entries.len(), 0);
    assert_eq!(audit.broken_references.len(), 0);
    assert_eq!(audit.orphan_textures.len(), 0);
    assert_eq!(audit.semantically_mapped_textures, 4);
}

#[test]
fn end_to_end_bedrock_to_java_zero_broken_references() {
    let temp = TempDir::new().expect("tempdir");
    let bedrock_zip_bytes = build_realistic_bedrock_pack_zip();
    let input_zip = temp.path().join("bedrock.zip");
    let output_zip = temp.path().join("java.zip");
    std::fs::write(&input_zip, bedrock_zip_bytes).expect("write bedrock zip");

    let summary = inspect_pack(&input_zip, None, IoLimits::default()).expect("inspect bedrock");
    assert_eq!(summary.edition, Edition::Bedrock);
    assert_eq!(summary.texture_count, 3);
    assert_eq!(summary.animated_texture_count, 1);
    assert_eq!(summary.model_count, 1);

    let (src, dst, _report) = convert_pack(&ConvertOptions {
        input: input_zip,
        output: output_zip.clone(),
        target: Some(Edition::Java),
        target_version: None,
        mapping_file: None,
        strict: true,
        report_json: None,
        limits: IoLimits::default(),
        version_options: VersionSpecificOptions::default(),
    })
    .expect("convert bedrock -> java");

    assert_eq!(src, Edition::Bedrock);
    assert_eq!(dst, Edition::Java);

    let out = PackContainer::read_from_path(&output_zip, IoLimits::default()).expect("read output");

    let mcmeta: JavaPackMcmeta =
        serde_json::from_slice(out.get("pack.mcmeta").expect("pack.mcmeta"))
            .expect("parse pack.mcmeta");
    assert_eq!(mcmeta.pack.pack_format, Some(15));
    assert!(out.contains("pack.png"));

    assert!(out.contains("assets/minecraft/textures/block/stone.png"));
    assert!(out.contains("assets/minecraft/textures/block/lava_still.png"));
    assert!(out.contains("assets/minecraft/textures/block/lava_still.png.mcmeta"));
    assert!(out.contains("assets/minecraft/textures/item/ender_pearl.png"));
    assert!(out.contains("assets/minecraft/models/entity/custom_totem.json"));

    let sounds_json: std::collections::BTreeMap<String, JavaSoundEventJson> =
        serde_json::from_slice(
            out.get("assets/minecraft/sounds.json")
                .expect("sounds.json"),
        )
        .expect("parse sounds.json");
    assert!(sounds_json.contains_key("block.stone.break"));
    assert!(out.contains("assets/minecraft/sounds/dig/stone1.ogg"));

    let en_us: serde_json::Value = serde_json::from_slice(
        out.get("assets/minecraft/lang/en_us.json")
            .expect("en_us.json"),
    )
    .expect("parse en_us.json");
    assert_eq!(en_us["block.minecraft.stone"], "Bedrock Stone");
    assert_eq!(en_us["item.minecraft.ender_pearl"], "Void Pearl");
}

#[test]
fn strict_mode_rejects_unsupported_shaders_and_mapping_override_applies() {
    let temp = TempDir::new().expect("tempdir");
    let mut c = PackContainer::new();
    c.insert(
        "pack.mcmeta",
        br#"{"pack":{"pack_format":15,"description":"Shaders Pack"}}"#.to_vec(),
    )
    .expect("mcmeta");
    c.insert(
        "assets/minecraft/textures/block/custom_ore.png",
        encode_png(16, 16, Rgba([1, 2, 3, 255])),
    )
    .expect("custom_ore");
    c.insert(
        "assets/minecraft/shaders/core/rendertype_solid.fsh",
        b"void main() {}".to_vec(),
    )
    .expect("shader");

    let input_dir = temp.path().join("java_dir");
    c.write_to_directory(&input_dir).expect("write dir");

    let out_strict = temp.path().join("out_strict.zip");
    assert!(
        convert_pack(&ConvertOptions {
            input: input_dir.clone(),
            output: out_strict,
            target: Some(Edition::Bedrock),
            target_version: None,
            mapping_file: None,
            strict: true,
            report_json: None,
            limits: IoLimits::default(),
            version_options: VersionSpecificOptions::default(),
        })
        .is_err()
    );

    let custom_map_path = temp.path().join("override_map.json");
    std::fs::write(
        &custom_map_path,
        r#"{
            "schema_version": "1.0",
            "textures": [
                { "java": "block/custom_ore", "bedrock": "textures/blocks/my_override_ore", "atlas_key": "override_ore" }
            ]
        }"#,
    )
    .expect("write mapping");

    let out_dir = temp.path().join("bedrock_out_dir");
    let (_s, _t, report) = convert_pack(&ConvertOptions {
        input: input_dir,
        output: out_dir.clone(),
        target: Some(Edition::Bedrock),
        target_version: None,
        mapping_file: Some(custom_map_path),
        strict: false,
        report_json: None,
        limits: IoLimits::default(),
        version_options: VersionSpecificOptions::default(),
    })
    .expect("convert non-strict with override mapping");

    assert_eq!(report.skipped_count, 1);
    let out_container =
        PackContainer::read_from_directory(&out_dir, IoLimits::default()).expect("read out dir");
    assert!(out_container.contains("textures/blocks/my_override_ore.png"));
    let terrain: TerrainTextureJson = serde_json::from_slice(
        out_container
            .get("textures/terrain_texture.json")
            .expect("terrain"),
    )
    .expect("parse terrain");
    assert!(terrain.texture_data.contains_key("override_ore"));
}

#[test]
fn cli_binary_inspect_and_convert_smoke() {
    let temp = TempDir::new().expect("tempdir");
    let (java_zip_bytes, _) = build_realistic_java_pack_zip();
    let input_zip = temp.path().join("cli_input.zip");
    let output_zip = temp.path().join("cli_output.zip");
    let report_json = temp.path().join("cli_report.json");
    std::fs::write(&input_zip, java_zip_bytes).expect("write input");

    let exe = env!("CARGO_BIN_EXE_rp-cli");
    let inspect_out = std::process::Command::new(exe)
        .arg("inspect")
        .arg(&input_zip)
        .output()
        .expect("run inspect");
    assert!(
        inspect_out.status.success(),
        "inspect failed: {}",
        String::from_utf8_lossy(&inspect_out.stderr)
    );
    let inspect_stdout = String::from_utf8_lossy(&inspect_out.stdout);
    assert!(inspect_stdout.contains("Java"));

    let convert_out = std::process::Command::new(exe)
        .arg("convert")
        .arg(&input_zip)
        .arg("-o")
        .arg(&output_zip)
        .arg("--to")
        .arg("bedrock")
        .arg("--target-version")
        .arg("1.20.1")
        .arg("--report-json")
        .arg(&report_json)
        .output()
        .expect("run convert");
    assert!(
        convert_out.status.success(),
        "convert failed: {}",
        String::from_utf8_lossy(&convert_out.stderr)
    );
    assert!(output_zip.exists());
    assert!(report_json.exists());

    let versions_out = std::process::Command::new(exe)
        .arg("versions")
        .output()
        .expect("run versions");
    assert!(versions_out.status.success());
    let versions_stdout = String::from_utf8_lossy(&versions_out.stdout);
    assert!(versions_stdout.contains("26.1"));
    assert!(versions_stdout.contains("26.2"));
    assert!(versions_stdout.contains("88"));
}

#[test]
fn target_java_26_2_and_26_1_emit_min_max_format_and_items_defs() {
    let mappings = MappingTable::default_embedded().expect("mappings");
    let mut b_in = PackContainer::new();
    let manifest = BedrockManifest::new("Mask Pack", "26.2 Test", [1, 0, 0], [26, 2, 0]);
    b_in.insert(
        "manifest.json",
        manifest.to_json_vec_versioned(3).expect("v3 manifest"),
    )
    .expect("insert manifest");
    b_in.insert(
        "textures/models/armor/mask.png",
        encode_png(32, 32, Rgba([200, 40, 40, 255])),
    )
    .expect("mask png");
    b_in.insert(
        "models/entity/custom_mask.json",
        br#"{
            "format_version": "1.21.0",
            "minecraft:geometry": [
                {
                    "description": { "identifier": "geometry.custom_mask", "texture_width": 32, "texture_height": 32 },
                    "bones": [
                        {
                            "name": "mask",
                            "pivot": [0, 8, 0],
                            "cubes": [
                                {
                                    "origin": [-4, 4, -4],
                                    "size": [8, 8, 8],
                                    "pivot": [0, 8, 0],
                                    "rotation": [0, -74.2, 0],
                                    "uv": { "north": { "uv": [0, 0], "uv_size": [16, 16] } }
                                }
                            ]
                        }
                    ]
                }
            ]
        }"#
        .to_vec(),
    )
    .expect("geo");
    b_in.insert(
        "attachables/custom_mask.json",
        br#"{
            "format_version": "1.10.0",
            "minecraft:attachable": {
                "description": {
                    "identifier": "minecraft:netherite_helmet",
                    "textures": { "default": "textures/models/armor/mask" },
                    "geometry": { "default": "geometry.custom_mask" }
                }
            }
        }"#
        .to_vec(),
    )
    .expect("attachable");

    // Target Java 26.2 (format 88.0): arbitrary rotation angle 74.2 deg is allowed without warning,
    // pack.mcmeta uses min_format/max_format [88, 0], and assets/minecraft/items/netherite_helmet.json is generated.
    let (java_26_2, _, _, report_26_2) = convert_container_with_options(
        &b_in,
        Some(Edition::Java),
        Some("26.2"),
        &mappings,
        true,
        &VersionSpecificOptions::default(),
    )
    .expect("convert to Java 26.2");

    assert_eq!(report_26_2.warnings.len(), 0);
    let mcmeta: JavaPackMcmeta =
        serde_json::from_slice(java_26_2.get("pack.mcmeta").expect("mcmeta")).expect("parse");
    assert_eq!(mcmeta.pack.pack_format, None);
    assert_eq!(mcmeta.pack.min_format, Some(serde_json::json!([88, 0])));
    assert_eq!(mcmeta.pack.max_format, Some(serde_json::json!([88, 0])));

    let item_def: JavaItemDefinitionJson = serde_json::from_slice(
        java_26_2
            .get("assets/minecraft/items/netherite_helmet.json")
            .expect("1.21.4+ / 26.2 item definition"),
    )
    .expect("parse item def");
    assert_eq!(item_def.model.kind, "minecraft:model");
    assert_eq!(item_def.model.model, "minecraft:item/netherite_helmet");
    assert!(java_26_2.contains("assets/minecraft/models/item/netherite_helmet.json"));

    let (reparsed_ir, _) = parse_java_pack(&java_26_2, &mappings).expect("reparse 26.2 pack");
    assert_eq!(reparsed_ir.metadata.java_pack_format, Some(88));

    // Target Java 26.1 with supported_formats [84, 88]
    let (java_26_1, _, _, _) = convert_container_with_options(
        &b_in,
        Some(Edition::Java),
        Some("26.1"),
        &mappings,
        true,
        &VersionSpecificOptions {
            supported_formats: Some([84, 88]),
            ..VersionSpecificOptions::default()
        },
    )
    .expect("convert to Java 26.1");
    let mcmeta_26_1: JavaPackMcmeta =
        serde_json::from_slice(java_26_1.get("pack.mcmeta").expect("mcmeta 26.1")).expect("parse");
    assert_eq!(
        mcmeta_26_1.pack.min_format,
        Some(serde_json::json!([84, 0]))
    );
    assert_eq!(
        mcmeta_26_1.pack.max_format,
        Some(serde_json::json!([88, 0]))
    );

    // Target legacy Java 1.20.1 with RotationSnapMode::Snap22_5 (snaps 74.2 -> 45.0)
    let (java_legacy, _, _, report_snap) = convert_container_with_options(
        &b_in,
        Some(Edition::Java),
        Some("1.20.1"),
        &mappings,
        false,
        &VersionSpecificOptions {
            rotation_mode: RotationSnapMode::Snap22_5,
            ..VersionSpecificOptions::default()
        },
    )
    .expect("convert to Java 1.20.1 with snap");
    assert!(report_snap.lossy_count >= 1);
    assert!(report_snap.warnings.is_empty());
    assert!(!java_legacy.contains("assets/minecraft/items/netherite_helmet.json"));
}

#[test]
fn legacy_pre_flattening_java_and_bedrock_v3_and_geo_1_10_0() {
    let mappings = MappingTable::default_embedded().expect("mappings");
    let bedrock_zip_bytes = build_realistic_bedrock_pack_zip();
    let b_in =
        PackContainer::read_from_zip_bytes(&bedrock_zip_bytes, IoLimits::default()).expect("zip");

    // Convert Bedrock -> pre-flattening Java 1.8.9 (pack_format: 1)
    let (java_1_8, _, _, _) = convert_container_with_options(
        &b_in,
        Some(Edition::Java),
        Some("1.8.9"),
        &mappings,
        true,
        &VersionSpecificOptions::default(),
    )
    .expect("convert to 1.8.9");
    let mcmeta: JavaPackMcmeta =
        serde_json::from_slice(java_1_8.get("pack.mcmeta").expect("mcmeta")).expect("parse");
    assert_eq!(mcmeta.pack.pack_format, Some(1));
    assert!(java_1_8.contains("assets/minecraft/textures/blocks/stone.png"));
    assert!(java_1_8.contains("assets/minecraft/textures/items/ender_pearl.png"));
    assert!(java_1_8.contains("assets/minecraft/lang/en_US.lang"));

    // Parse legacy 1.8.9 pack back and convert -> Bedrock 26.2 with manifest v3 & geo 1.10.0
    let (bedrock_26_2, _, _, _) = convert_container_with_options(
        &java_1_8,
        Some(Edition::Bedrock),
        Some("26.2"),
        &mappings,
        true,
        &VersionSpecificOptions {
            bedrock_manifest_version: Some(3),
            bedrock_geo_version: Some(BedrockGeoVersion::V1_10_0),
            ..VersionSpecificOptions::default()
        },
    )
    .expect("convert 1.8.9 -> Bedrock 26.2 v3");

    let manifest_val: serde_json::Value =
        serde_json::from_slice(bedrock_26_2.get("manifest.json").expect("manifest"))
            .expect("parse v3 manifest");
    assert_eq!(manifest_val["format_version"], 3);
    // Per minecraft.wiki/w/Bedrock_Edition_26.60 & Mojang/bedrock-samples:
    // Bedrock 26.2 / 26.30 uses internal min_engine_version "1.26.30".
    assert_eq!(manifest_val["header"]["min_engine_version"], "1.26.30");
    assert_eq!(manifest_val["header"]["version"], "1.0.0");

    let geo_val: serde_json::Value = serde_json::from_slice(
        bedrock_26_2
            .get("models/entity/custom_totem.geo.json")
            .expect("geo"),
    )
    .expect("parse legacy 1.10.0 geo");
    assert_eq!(geo_val["format_version"], "1.10.0");
    assert!(geo_val.get("geometry.entity.custom_totem").is_some());
}

#[test]
fn internet_pvp_pack_nested_jsonc_and_parent_only_models() {
    let mut raw = PackContainer::new();
    raw.insert(
        "Downloads/PvP_Pack_v2/Pack.mcmeta",
        br#"// PvP pack header
{
  "pack": {
    "pack_format": 15,
    "description": "PvP Pack // v2",
  },
}"#
        .to_vec(),
    )
    .expect("mcmeta");
    raw.insert(
        "Downloads/PvP_Pack_v2/assets/minecraft/textures/item/golden_apple.png",
        encode_png(16, 16, Rgba([255, 215, 0, 255])),
    )
    .expect("gapple");
    raw.insert(
        "Downloads/PvP_Pack_v2/assets/minecraft/textures/item/wooden_sword.png",
        encode_png(16, 16, Rgba([160, 110, 60, 255])),
    )
    .expect("wood_sword");
    raw.insert(
        "Downloads/PvP_Pack_v2/assets/minecraft/textures/item/bow.png",
        encode_png(16, 16, Rgba([140, 90, 50, 255])),
    )
    .expect("bow");
    raw.insert(
        "Downloads/PvP_Pack_v2/assets/minecraft/textures/item/water_bucket.png",
        encode_png(16, 16, Rgba([40, 120, 240, 255])),
    )
    .expect("water_bucket");
    raw.insert(
        "Downloads/PvP_Pack_v2/assets/minecraft/models/item/wooden_sword.json",
        br#"/* parent-only handheld item */
{
  "parent": "minecraft:item/handheld",
  "textures": {
    "layer0": "minecraft:item/wooden_sword",
  },
}"#
        .to_vec(),
    )
    .expect("sword model");

    let zip_bytes = raw.write_to_zip_bytes().expect("zip");
    let container =
        PackContainer::read_from_zip_bytes(&zip_bytes, IoLimits::default()).expect("unwrap zip");
    assert!(container.contains("pack.mcmeta"));

    let mappings = MappingTable::default_embedded().expect("mappings");
    let (bedrock, _, _, _) = convert_container_with_options(
        &container,
        Some(Edition::Bedrock),
        Some("26.2"),
        &mappings,
        true,
        &VersionSpecificOptions::default(),
    )
    .expect("convert PvP pack to Bedrock");

    assert!(bedrock.contains("textures/items/apple_golden.png"));
    assert!(bedrock.contains("textures/items/wood_sword.png"));
    assert!(bedrock.contains("textures/items/bow_standby.png"));
    assert!(bedrock.contains("textures/items/bucket_water.png"));
    // Empty parent-only Java model should not produce an empty .geo.json file.
    assert!(!bedrock.contains("models/entity/item/wooden_sword.geo.json"));
}

#[test]
fn bedrock_jsonc_uppercase_paths_and_java_atlas_generation() {
    let mappings = MappingTable::default_embedded().expect("mappings");
    let mut b_in = PackContainer::new();
    b_in.insert(
        "manifest.json",
        br#"/* JSONC manifest */
{
  "format_version": 2,
  "header": {
    "name": "Mixed Case JSONC Pack",
    "description": "Test",
    "uuid": "11111111-1111-4111-8111-111111111111",
    "version": [1, 0, 0],
    "min_engine_version": [1, 21, 0],
  },
  "modules": [
    {
      "type": "resources",
      "uuid": "22222222-2222-4222-8222-222222222222",
      "version": [1, 0, 0],
    },
  ],
}"#
        .to_vec(),
    )
    .expect("manifest");
    b_in.insert(
        "textures/models/armor/My Mask.png",
        encode_png(16, 16, Rgba([220, 220, 220, 255])),
    )
    .expect("mask png");
    b_in.insert(
        "models/entity/My Mask.json",
        br#"// comment in geo
{
  "format_version": "1.12.0",
  "minecraft:geometry": [
    {
      "description": { "identifier": "geometry.my_mask", "texture_width": 16, "texture_height": 16 },
      "bones": [
        {
          "name": "head",
          "pivot": [0, 24, 0],
          "cubes": [
            { "origin": [-4, 20, -4], "size": [8, 12, 8], "uv": { "north": { "uv": [0, 0], "uv_size": [8, 12] } } }
          ]
        }
      ]
    }
  ]
}"#
        .to_vec(),
    )
    .expect("geo");
    b_in.insert(
        "attachables/my_mask.json",
        br#"{
  "format_version": "1.10.0",
  "minecraft:attachable": {
    "description": {
      "identifier": "minecraft:carved_pumpkin",
      "textures": { "default": "textures/models/armor/My Mask" },
      "geometry": { "default": "geometry.my_mask" },
    }
  }
}"#
        .to_vec(),
    )
    .expect("attachable");

    let (java_out, _, _, _) = convert_container_with_options(
        &b_in,
        Some(Edition::Java),
        Some("26.2"),
        &mappings,
        true,
        &VersionSpecificOptions::default(),
    )
    .expect("convert JSONC Bedrock -> Java 26.2");

    // Uppercase + space sanitized for Java ResourceLocation compliance
    assert!(java_out.contains("assets/minecraft/textures/models/armor/my_mask.png"));
    assert!(java_out.contains("assets/minecraft/models/item/carved_pumpkin.json"));
    assert!(java_out.contains("assets/minecraft/items/carved_pumpkin.json"));

    // Particle + display transforms emitted on synthesized attachable item model
    let item_model: serde_json::Value = serde_json::from_slice(
        java_out
            .get("assets/minecraft/models/item/carved_pumpkin.json")
            .expect("item model"),
    )
    .expect("parse item model");
    assert_eq!(item_model["textures"]["all"], "models/armor/my_mask");
    assert_eq!(item_model["textures"]["particle"], "models/armor/my_mask");
    assert!(item_model["display"]["head"].is_object());
    assert!(item_model["display"]["gui"].is_object());

    // Atlas config emitted for non-block/non-item texture directory `models/armor`
    let atlas: serde_json::Value = serde_json::from_slice(
        java_out
            .get("assets/minecraft/atlases/blocks.json")
            .expect("atlases/blocks.json"),
    )
    .expect("parse atlas");
    assert_eq!(atlas["sources"][0]["type"], "minecraft:directory");
    assert_eq!(atlas["sources"][0]["source"], "models/armor");
    assert_eq!(atlas["sources"][0]["prefix"], "models/armor/");
}

#[test]
fn modern_java_euler_min_max_multi_ns_and_bedrock_attachable_item_map() {
    let mappings = MappingTable::default_embedded().expect("mappings");

    // 1. Java 1.21.11+ / 26.x: omitted pack_format with open-ended max_format=999,
    //    Euler element rotation {"x": 180, "y": 45, "z": 180}, multi-namespace lang & optifine.
    let mut j_in = PackContainer::new();
    j_in.insert(
        "pack.mcmeta",
        br#"{"pack":{"description":"Modern 26.x Pack","min_format":97,"max_format":999}}"#.to_vec(),
    )
    .expect("mcmeta");
    j_in.insert(
        "assets/minecraft/textures/item/amethyst_shard.png",
        encode_png(16, 16, Rgba([180, 90, 240, 255])),
    )
    .expect("tex");
    j_in.insert(
        "assets/minecraft/models/item/amethyst_shard.json",
        br##"{
            "textures": { "0": "item/amethyst_shard" },
            "elements": [
                {
                    "from": [6.5, 6, 8],
                    "to": [9.5, 8, 8],
                    "rotation": { "x": 180, "y": 45, "z": 180, "origin": [8, 5, 8] },
                    "faces": {
                        "north": { "uv": [6, 0, 9, 2], "texture": "#0" }
                    }
                }
            ]
        }"##
        .to_vec(),
    )
    .expect("model");
    j_in.insert(
        "assets/fo/lang/en_us.json",
        br#"{"fo.custom.option": "Custom Toggle"}"#.to_vec(),
    )
    .expect("fo lang");
    j_in.insert(
        "assets/create/optifine/ctm/ores/zinc_ore/0.png",
        encode_png(16, 16, Rgba([100, 120, 100, 255])),
    )
    .expect("optifine ctm");

    let (b_out, _, _, report) = convert_container_with_options(
        &j_in,
        Some(Edition::Bedrock),
        Some("26.2"),
        &mappings,
        false,
        &VersionSpecificOptions::default(),
    )
    .expect("convert modern java -> bedrock");

    assert_eq!(report.skipped_count, 1);
    let en_us_lang =
        std::str::from_utf8(b_out.get("texts/en_US.lang").expect("en_US.lang")).expect("utf8 lang");
    assert!(en_us_lang.contains("fo.custom.option=Custom Toggle"));

    let geo_val: serde_json::Value = serde_json::from_slice(
        b_out
            .get("models/entity/item/amethyst_shard.geo.json")
            .expect("geo"),
    )
    .expect("parse geo");
    let rot = &geo_val["minecraft:geometry"][0]["bones"][0]["cubes"][0]["rotation"];
    assert_eq!(rot[0], -180.0);
    assert_eq!(rot[1], -45.0);
    assert_eq!(rot[2], 180.0);

    // 2. Bedrock attachable with conditional `item` object + wrapped repo directory layout.
    let mut raw_b = PackContainer::new();
    raw_b
        .insert(
            "Repo-master/pack/manifest.json",
            BedrockManifest::new("Repo Pack", "Wrapped Bedrock", [1, 0, 0], [1, 21, 0])
                .to_json_vec_versioned(2)
                .expect("manifest"),
        )
        .expect("manifest");
    raw_b
        .insert("Repo-master/README.md", b"# Repo README".to_vec())
        .expect("readme");
    raw_b
        .insert(
            "Repo-master/pack/textures/terrain_texture.json",
            br#"{"texture_data":{"stone":{"textures":"textures/blocks/stone"}}}"#.to_vec(),
        )
        .expect("terrain");
    raw_b
        .insert(
            "Repo-master/pack/textures/models/armor/chain_custom.png",
            encode_png(32, 32, Rgba([160, 160, 160, 255])),
        )
        .expect("chain png");
    raw_b
        .insert(
            "Repo-master/pack/models/entity/armor_stand.chestplate.geo.json",
            br#"{
                "format_version": "1.12.0",
                "minecraft:geometry": [{
                    "description": { "identifier": "geometry.armor_stand.armor.chestplate", "texture_width": 64, "texture_height": 32 },
                    "bones": [{
                        "name": "body",
                        "cubes": [{ "origin": [-4, 12, -2], "size": [8, 12, 4], "uv": { "north": { "uv": [0, 0], "uv_size": [8, 12] } } }]
                    }]
                }]
            }"#
            .to_vec(),
        )
        .expect("chestplate geo");
    raw_b
        .insert(
            "Repo-master/pack/attachables/chainmail_chestplate.armor_stand.json",
            br#"{
                "format_version": "1.10.0",
                "minecraft:attachable": {
                    "description": {
                        "identifier": "minecraft:chainmail_chestplate.armor_stand",
                        "item": { "minecraft:chainmail_chestplate": "query.owner_identifier == 'minecraft:armor_stand'" },
                        "textures": { "default": "textures/models/armor/chain_custom" },
                        "geometry": { "default": "geometry.armor_stand.armor.chestplate" },
                        "render_controllers": [ { "controller.render.armor": "1" } ]
                    }
                }
            }"#
            .to_vec(),
        )
        .expect("attachable");

    let zip_bytes = raw_b.write_to_zip_bytes().expect("zip");
    let unwrapped =
        PackContainer::read_from_zip_bytes(&zip_bytes, IoLimits::default()).expect("unwrap");
    assert!(unwrapped.contains("manifest.json"));

    let (j_out, _, _, _) = convert_container_with_options(
        &unwrapped,
        Some(Edition::Java),
        Some("26.2"),
        &mappings,
        false,
        &VersionSpecificOptions::default(),
    )
    .expect("convert wrapped bedrock -> java 26.2");

    assert!(j_out.contains("assets/minecraft/models/item/chainmail_chestplate.json"));
    assert!(j_out.contains("assets/minecraft/items/chainmail_chestplate.json"));
}

#[test]
fn java_26_3_euler_shade_override_and_bedrock_se3_roundtrip() {
    let mappings = MappingTable::default_embedded().expect("mappings");

    // 1. Bedrock hierarchical parent-rotated rig -> Java 26.3 (`pack_format 97.1`)
    let mut b_in = PackContainer::new();
    b_in.insert(
        "manifest.json",
        BedrockManifest::new("SE3 Rig Pack", "26.3 SE3 test", [1, 0, 0], [26, 50, 0])
            .to_json_vec_versioned(2)
            .expect("manifest"),
    )
    .expect("manifest");
    b_in.insert(
        "textures/models/armor/mask.png",
        encode_png(32, 32, Rgba([240, 40, 60, 255])),
    )
    .expect("mask png");
    b_in.insert(
        "models/entity/kitsune_rig.json",
        br#"{
            "format_version": "1.12.0",
            "minecraft:geometry": [{
                "description": { "identifier": "geometry.kitsune_rig", "texture_width": 32, "texture_height": 32 },
                "bones": [
                    { "name": "head", "pivot": [0, 24, 0] },
                    { "name": "ribbon", "parent": "head", "pivot": [8, 21, 7], "rotation": [-71.32319, -74.20678, 71.98187] },
                    {
                        "name": "mask",
                        "parent": "ribbon",
                        "pivot": [0, 29, 15],
                        "cubes": [{
                            "origin": [-2, 26, 13],
                            "size": [4, 4, 2],
                            "uv": {
                                "north": { "uv": [0, 0], "uv_size": [4, 4], "uv_rotation": 90 },
                                "up": { "uv": [8, 4], "uv_size": [-4, -2] }
                            }
                        }]
                    }
                ]
            }]
        }"#
        .to_vec(),
    )
    .expect("geo");
    b_in.insert(
        "attachables/kitsune_rig.json",
        br#"{
            "format_version": "1.10.0",
            "minecraft:attachable": {
                "description": {
                    "identifier": "minecraft:diamond_helmet",
                    "textures": { "default": "textures/models/armor/mask" },
                    "geometry": { "default": "geometry.kitsune_rig" }
                }
            }
        }"#
        .to_vec(),
    )
    .expect("attachable");

    let (j263, _, _, report) = convert_container_with_options(
        &b_in,
        Some(Edition::Java),
        Some("26.3"),
        &mappings,
        true,
        &VersionSpecificOptions::default(),
    )
    .expect("convert Bedrock rig -> Java 26.3");
    assert_eq!(report.lossy_count, 0);

    let mcmeta: serde_json::Value =
        serde_json::from_slice(j263.get("pack.mcmeta").expect("pack.mcmeta")).expect("mcmeta");
    assert_eq!(mcmeta["pack"]["min_format"], serde_json::json!([97, 0]));
    assert_eq!(mcmeta["pack"]["max_format"], serde_json::json!([97, 1]));

    let model_val: serde_json::Value = serde_json::from_slice(
        j263.get("assets/minecraft/models/item/diamond_helmet.json")
            .expect("diamond_helmet model"),
    )
    .expect("parse model");
    let el = &model_val["elements"][0];
    // Multi-axis Euler emitted with x/y/z (NOT legacy axis/angle!)
    assert!(el["rotation"]["x"].is_number());
    assert!(el["rotation"]["y"].is_number());
    assert!(el["rotation"]["z"].is_number());
    assert!(el["rotation"]["axis"].is_null());
    assert!(el["rotation"]["angle"].is_null());
    // North uv_rotation preserved, Up face UV flipped 180° from Bedrock [8,4]+[-4,-2] => [4,2,8,4] * 0.5 = [2,1,4,2]
    assert_eq!(el["faces"]["north"]["rotation"], 90);
    assert_eq!(
        el["faces"]["up"]["uv"],
        serde_json::json!([2.0, 1.0, 4.0, 2.0])
    );
    // Display head scale matches exact 1:1 CustomHeadLayer (0.625 * 1.6 = 1.0)
    assert_eq!(
        model_val["display"]["head"]["scale"],
        serde_json::json!([1.6, 1.6, 1.6])
    );

    // 2. Java 26.3 model with shade_direction_override + light_emission -> Java 26.3 & 1.21.11
    let mut j_in = PackContainer::new();
    j_in.insert(
        "pack.mcmeta",
        br#"{"pack":{"description":"26.3 Shade Test","min_format":[97,0],"max_format":[97,1]}}"#
            .to_vec(),
    )
    .expect("mcmeta");
    j_in.insert(
        "assets/minecraft/textures/block/glow_tile.png",
        encode_png(16, 16, Rgba([255, 220, 80, 255])),
    )
    .expect("tex");
    j_in.insert(
        "assets/minecraft/models/block/glow_tile.json",
        br##"{
            "textures": { "all": "block/glow_tile" },
            "elements": [{
                "from": [0, 0, 0],
                "to": [16, 16, 16],
                "shade_direction_override": "up",
                "light_emission": 15,
                "rotation": { "x": 15, "y": 60, "z": -30, "origin": [8, 8, 8] },
                "faces": { "up": { "uv": [0, 0, 16, 16], "texture": "#all" } }
            }]
        }"##
        .to_vec(),
    )
    .expect("model");

    // Roundtrip Java 26.3 -> Java 26.3 (preserves shade_direction_override + light_emission + {x,y,z})
    let (j_same, _, _, _) = convert_container_with_options(
        &j_in,
        Some(Edition::Java),
        Some("26.3"),
        &mappings,
        true,
        &VersionSpecificOptions::default(),
    )
    .expect("Java 26.3 -> Java 26.3");
    let m263: serde_json::Value = serde_json::from_slice(
        j_same
            .get("assets/minecraft/models/block/glow_tile.json")
            .expect("glow_tile 26.3"),
    )
    .expect("parse 26.3");
    assert_eq!(m263["elements"][0]["shade_direction_override"], "up");
    assert!(m263["elements"][0]["shade"].is_null());
    assert_eq!(m263["elements"][0]["light_emission"], 15);
    assert_eq!(m263["elements"][0]["rotation"]["y"], 60.0);

    // Downgrade Java 26.3 -> Java 26.2 (maps shade_direction_override back to shade: false)
    let (j_down, _, _, _) = convert_container_with_options(
        &j_in,
        Some(Edition::Java),
        Some("26.2"),
        &mappings,
        true,
        &VersionSpecificOptions::default(),
    )
    .expect("Java 26.3 -> Java 26.2");
    let m262: serde_json::Value = serde_json::from_slice(
        j_down
            .get("assets/minecraft/models/block/glow_tile.json")
            .expect("glow_tile 26.2"),
    )
    .expect("parse 26.2");
    assert_eq!(m262["elements"][0]["shade"], false);
    assert!(m262["elements"][0]["shade_direction_override"].is_null());
}
