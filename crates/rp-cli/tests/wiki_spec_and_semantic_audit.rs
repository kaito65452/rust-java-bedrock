use std::io::Cursor;

use image::{ImageFormat, Rgba, RgbaImage};
use tempfile::TempDir;

use rp_bedrock::audit_bedrock_container;
use rp_bedrock::schema::BedrockManifest;
use rp_cli::manager::audit_pack_roundtrip;
use rp_cli::{
    ConvertOptions, VersionSpecificOptions, convert_container_with_audit, convert_pack,
};
use rp_core::{AuditStatus, ConversionReport, Edition, MappingTable, PipelineStage};
use rp_io::{IoLimits, PackContainer};
use rp_transforms::pbr::BedrockTextureSetFile;
use rp_transforms::texture::{
    AtlasTexturePath, AtlasVariant, ItemTextureJson, TerrainTextureJson,
};

fn encode_png(width: u32, height: u32, color: Rgba<u8>) -> Vec<u8> {
    let img = RgbaImage::from_pixel(width, height, color);
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .expect("encode png");
    buf
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

/// Builds a raw ZIP on disk including OS junk files (`desktop.ini`, `.DS_Store`, `Thumbs.db`, `__MACOSX/`).
fn write_raw_zip(path: &std::path::Path, entries: &[(&str, Vec<u8>)]) {
    let file = std::fs::File::create(path).expect("create zip");
    let mut zw = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in entries {
        zw.start_file(*name, opts).expect("start_file");
        std::io::Write::write_all(&mut zw, bytes).expect("write_all");
    }
    zw.finish().expect("finish zip");
}

/// Fixture 1: Standard Vanilla Pack (English) based purely on `minecraft.wiki`.
/// Verifies 1.21.x, Java 26.3, and Bedrock 26.60 always normalize `min_engine_version` to `[1, x, x]`.
#[test]
fn fixture_1_standard_vanilla_pack_versions_and_mob_effects() {
    let mappings = MappingTable::default_embedded().expect("mappings");

    // 1. Verify version resolver produces [1, minor, patch] for 1.21.x, 26.3, 26.60, and 1.26.60
    assert_eq!(
        mappings
            .resolve_target_version(Edition::Bedrock, "1.21.4")
            .expect("1.21.4"),
        (Some(46), Some([1, 21, 50]))
    );
    assert_eq!(
        mappings
            .resolve_target_version(Edition::Bedrock, "1.21.11")
            .expect("1.21.11"),
        (Some(75), Some([1, 21, 130]))
    );
    let (fmt_26_3, eng_26_3) = mappings
        .resolve_target_version(Edition::Java, "26.3")
        .expect("26.3");
    assert_eq!(fmt_26_3, Some(97));
    assert_eq!(eng_26_3, Some([1, 26, 50]));
    assert_eq!(eng_26_3.expect("eng")[0], 1);
    assert_ne!(eng_26_3.expect("eng")[0], 26);

    let (fmt_26_60, eng_26_60) = mappings
        .resolve_target_version(Edition::Bedrock, "26.60")
        .expect("26.60");
    assert_eq!(fmt_26_60, Some(97));
    assert_eq!(eng_26_60, Some([1, 26, 60]));
    assert_eq!(eng_26_60.expect("eng")[0], 1);
    assert_ne!(eng_26_60.expect("eng")[0], 26);

    // 2. Standard English Vanilla Java 26.3 Pack -> Bedrock 26.60 & 26.3
    let mut j_in = PackContainer::new();
    j_in.insert(
        "pack.mcmeta",
        br#"{"pack":{"description":"Standard Vanilla English Pack","min_format":[97,0],"max_format":[97,1]}}"#
            .to_vec(),
    )
    .expect("mcmeta");
    j_in.insert(
        "assets/minecraft/textures/block/stone.png",
        encode_png(16, 16, Rgba([128, 128, 128, 255])),
    )
    .expect("stone");
    j_in.insert(
        "assets/minecraft/textures/block/oak_planks.png",
        encode_png(16, 16, Rgba([170, 130, 80, 255])),
    )
    .expect("oak_planks");
    j_in.insert(
        "assets/minecraft/textures/item/apple.png",
        encode_png(16, 16, Rgba([220, 30, 30, 255])),
    )
    .expect("apple");
    j_in.insert(
        "assets/minecraft/textures/item/diamond_sword.png",
        encode_png(16, 16, Rgba([60, 210, 230, 255])),
    )
    .expect("diamond_sword");
    j_in.insert(
        "assets/minecraft/textures/mob_effect/speed.png",
        encode_png(18, 18, Rgba([120, 210, 255, 255])),
    )
    .expect("speed");
    j_in.insert(
        "assets/minecraft/textures/mob_effect/hero_of_the_village.png",
        encode_png(18, 18, Rgba([60, 220, 90, 255])),
    )
    .expect("hero");

    // Verify both "26.60" and "26.3" emit manifest.header.min_engine_version[0] == 1 (never 26!)
    for (ver_input, expected_engine) in [("26.60", [1, 26, 60]), ("26.3", [1, 26, 50])] {
        let (b_v2, _, _, report_v2) = convert_container_with_audit(
            &j_in,
            Some(Edition::Bedrock),
            Some(ver_input),
            &mappings,
            &VersionSpecificOptions {
                bedrock_manifest_version: Some(2),
                ..VersionSpecificOptions::default()
            },
        )
        .expect("convert v2");

        let manifest_v2: BedrockManifest =
            serde_json::from_slice(b_v2.get("manifest.json").expect("manifest v2"))
                .expect("parse v2");
        assert_eq!(manifest_v2.format_version, 2);
        assert_eq!(manifest_v2.header.min_engine_version, expected_engine);
        assert_eq!(manifest_v2.header.min_engine_version[0], 1);
        assert_ne!(manifest_v2.header.min_engine_version[0], 26);
        assert_ne!(manifest_v2.header.uuid, manifest_v2.modules[0].uuid);

        assert!(b_v2.contains("textures/blocks/stone.png"));
        assert!(b_v2.contains("textures/blocks/planks_oak.png"));
        assert!(b_v2.contains("textures/items/apple.png"));
        assert!(b_v2.contains("textures/items/diamond_sword.png"));
        assert!(b_v2.contains("textures/ui/speed_effect.png"));
        assert!(b_v2.contains("textures/ui/village_hero_effect.png"));
        assert!(!b_v2.files().keys().any(|k| k.starts_with("textures/mob_effect/")));
        assert!(!b_v2.contains("texts/languages.json"));

        let audit_v2 = report_v2.semantic_audit.expect("audit v2");
        assert_eq!(audit_v2.status, AuditStatus::Passed);
        assert_eq!(audit_v2.semantically_mapped_textures, 6);
        assert_eq!(audit_v2.orphan_textures.len(), 0);
        assert_eq!(audit_v2.forbidden_entries.len(), 0);
    }

    // Emit manifest v3 for Bedrock 26.60 (SemVer string "1.26.60" + metadata.authors)
    let (b_v3, _, _, report_v3) = convert_container_with_audit(
        &j_in,
        Some(Edition::Bedrock),
        Some("26.60"),
        &mappings,
        &VersionSpecificOptions {
            bedrock_manifest_version: Some(3),
            ..VersionSpecificOptions::default()
        },
    )
    .expect("convert v3");
    let manifest_v3_val: serde_json::Value =
        serde_json::from_slice(b_v3.get("manifest.json").expect("manifest v3")).expect("parse v3");
    assert_eq!(manifest_v3_val["format_version"], 3);
    assert_eq!(manifest_v3_val["header"]["min_engine_version"], "1.26.60");
    assert!(
        manifest_v3_val["metadata"]["authors"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
    );
    assert_eq!(
        report_v3.semantic_audit.expect("audit v3").status,
        AuditStatus::Passed
    );
}

/// Fixture 2: Generic Synthetic Pack with arbitrary non-vanilla texture names (`custom_tex_a.png`,
/// `custom_tex_b.png`, `custom_sub/tex_c.png`) resolved purely via the generic JSON model graph,
/// plus 3D vanilla block geometry, LabPBR maps, and OS/Java junk cleanup.
#[test]
fn fixture_2_generic_synthetic_model_graph_labpbr_and_junk_cleanup() {
    let temp = TempDir::new().expect("tempdir");
    let input_zip = temp.path().join("generic_synthetic_pack.zip");

    let entries: Vec<(&str, Vec<u8>)> = vec![
        (
            "pack.mcmeta",
            br#"{"pack":{"pack_format":34,"description":"Generic Synthetic Model Graph Pack"}}"#
                .to_vec(),
        ),
        // Arbitrary custom texture names referenced ONLY via vanilla models / blockstates!
        (
            "assets/minecraft/textures/block/custom_tex_a.png",
            encode_png(16, 16, Rgba([220, 30, 30, 255])),
        ),
        (
            "assets/minecraft/textures/block/custom_tex_b.png",
            encode_png(16, 16, Rgba([70, 180, 60, 255])),
        ),
        (
            "assets/minecraft/textures/block/custom_sub/tex_c.png",
            encode_png(16, 16, Rgba([90, 200, 75, 255])),
        ),
        // LabPBR sidecars for custom_tex_b
        (
            "assets/minecraft/textures/block/custom_tex_b_n.png",
            encode_png(16, 16, Rgba([128, 128, 255, 255])),
        ),
        (
            "assets/minecraft/textures/block/custom_tex_b_s.png",
            encode_png(16, 16, Rgba([180, 240, 0, 50])),
        ),
        // 3D vanilla cactus block textures + 3D block model
        (
            "assets/minecraft/textures/block/custom_cactus_side.png",
            encode_png(16, 16, Rgba([40, 140, 50, 255])),
        ),
        (
            "assets/minecraft/textures/block/custom_cactus_top.png",
            encode_png(16, 16, Rgba([55, 160, 65, 255])),
        ),
        (
            "assets/minecraft/models/item/apple.json",
            br#"{
                "parent": "minecraft:item/generated",
                "textures": {
                    "layer0": "minecraft:block/custom_tex_a"
                }
            }"#
            .to_vec(),
        ),
        (
            "assets/minecraft/models/block/bamboo_parent.json",
            br##"{
                "textures": {
                    "all": "#stem"
                }
            }"##
            .to_vec(),
        ),
        (
            "assets/minecraft/models/block/bamboo.json",
            br#"{
                "parent": "minecraft:block/bamboo_parent",
                "textures": {
                    "stem": "minecraft:block/custom_tex_b"
                }
            }"#
            .to_vec(),
        ),
        (
            "assets/minecraft/blockstates/short_grass.json",
            br#"{
                "variants": {
                    "": { "model": "minecraft:block/custom_grass_variant" }
                }
            }"#
            .to_vec(),
        ),
        (
            "assets/minecraft/models/block/custom_grass_variant.json",
            br#"{
                "parent": "minecraft:block/cross",
                "textures": {
                    "cross": "minecraft:block/custom_sub/tex_c"
                }
            }"#
            .to_vec(),
        ),
        (
            "assets/minecraft/models/block/cactus.json",
            br##"{
                "textures": {
                    "side": "minecraft:block/custom_cactus_side",
                    "top": "minecraft:block/custom_cactus_top",
                    "bottom": "minecraft:block/custom_cactus_top"
                },
                "elements": [
                    {
                        "from": [1, 0, 1],
                        "to": [15, 16, 15],
                        "faces": {
                            "up": { "uv": [0, 0, 16, 16], "texture": "#top" },
                            "down": { "uv": [0, 0, 16, 16], "texture": "#bottom" },
                            "north": { "uv": [0, 0, 16, 16], "texture": "#side" },
                            "south": { "uv": [0, 0, 16, 16], "texture": "#side" },
                            "east": { "uv": [0, 0, 16, 16], "texture": "#side" },
                            "west": { "uv": [0, 0, 16, 16], "texture": "#side" }
                        }
                    }
                ]
            }"##
            .to_vec(),
        ),
        // Status effect icon
        (
            "assets/minecraft/textures/mob_effect/speed.png",
            encode_png(18, 18, Rgba([100, 220, 255, 255])),
        ),
        // Junk & Java-only text files that MUST NOT leak
        ("desktop.ini", b"[.ShellClassInfo]\r\nIconResource=x\r\n".to_vec()),
        ("assets/minecraft/textures/desktop.ini", b"junk".to_vec()),
        (".DS_Store", b"macjunk".to_vec()),
        ("Thumbs.db", b"winjunk".to_vec()),
        ("__MACOSX/._pack.mcmeta", b"appledouble".to_vec()),
        (
            "assets/minecraft/texts/splashes.txt",
            b"Custom Splash One!\nCustom Splash Two!\n".to_vec(),
        ),
        ("assets/minecraft/texts/credits.json", b"[]".to_vec()),
    ];
    write_raw_zip(&input_zip, &entries);

    // --- Mode A: Default PBR-disabled conversion ---
    let out_mcpack = temp.path().join("generic_out.mcpack");
    let (_src, _dst, report) = convert_pack(&ConvertOptions {
        input: input_zip.clone(),
        output: out_mcpack.clone(),
        target: Some(Edition::Bedrock),
        target_version: Some(String::from("26.60")),
        mapping_file: None,
        strict: false,
        report_json: None,
        limits: IoLimits::default(),
        version_options: VersionSpecificOptions::default(),
    })
    .expect("convert generic synthetic pack (PBR off)");

    let out = PackContainer::read_from_path(&out_mcpack, IoLimits::default()).expect("read mcpack");
    let mappings = MappingTable::default_embedded().expect("mappings");
    let audit = audit_bedrock_container(&out, 6, &mappings);

    assert_eq!(
        audit.forbidden_entries.len(),
        0,
        "unexpected forbidden entries: {:?}",
        audit.forbidden_entries
    );
    assert_eq!(
        audit.broken_references.len(),
        0,
        "unexpected broken refs: {:?}",
        audit.broken_references
    );
    assert_eq!(
        audit.orphan_textures.len(),
        0,
        "unexpected orphan textures: {:?}",
        audit.orphan_textures
    );
    assert_eq!(audit.semantically_mapped_textures, 6);
    assert_eq!(audit.status, AuditStatus::Passed);

    let out_manifest: BedrockManifest =
        serde_json::from_slice(out.get("manifest.json").expect("manifest")).expect("parse");
    assert_eq!(out_manifest.header.min_engine_version, [1, 26, 60]);
    assert_eq!(out_manifest.header.min_engine_version[0], 1);

    assert!(!out.files().keys().any(|k| k.starts_with("assets/")));
    assert!(!out.files().keys().any(|k| k.to_ascii_lowercase().contains("desktop.ini")));
    assert!(!out.files().keys().any(|k| k.starts_with("textures/mob_effect/")));
    assert!(!out.files().keys().any(|k| k.starts_with("models/entity/block/")));
    assert!(!out.contains("textures/blocks/custom_tex_b_n.png"));
    assert!(!out.contains("textures/blocks/custom_tex_b_s.png"));
    assert!(!out.contains("texts/languages.json"));
    assert!(out.contains("textures/ui/speed_effect.png"));
    assert!(out.contains("splashes.json"));

    let item_tex: ItemTextureJson = serde_json::from_slice(
        out.get("textures/item_texture.json")
            .expect("item_texture.json"),
    )
    .expect("parse item_texture.json");
    assert!(
        item_tex.texture_data.contains_key("apple"),
        "expected 'apple' key in item_texture.json"
    );
    for entry in item_tex.texture_data.values() {
        for p in collect_atlas_paths(&entry.textures) {
            assert!(out.contains(&format!("{p}.png")), "missing item PNG '{p}.png'");
        }
    }

    let terrain_tex: TerrainTextureJson = serde_json::from_slice(
        out.get("textures/terrain_texture.json")
            .expect("terrain_texture.json"),
    )
    .expect("parse terrain_texture.json");
    for required_key in ["bamboo_stem", "short_grass", "cactus_side", "cactus_top"] {
        assert!(
            terrain_tex.texture_data.contains_key(required_key),
            "expected '{required_key}' in terrain_texture.json, got keys: {:?}",
            terrain_tex.texture_data.keys().collect::<Vec<_>>()
        );
    }
    for entry in terrain_tex.texture_data.values() {
        for p in collect_atlas_paths(&entry.textures) {
            assert!(
                out.contains(&format!("{p}.png")),
                "missing terrain PNG '{p}.png'"
            );
        }
    }

    assert!(!report.warnings.is_empty());
    for w in &report.warnings {
        assert!(!w.source_path.is_empty(), "missing source_path on {w:?}");
        assert!(!w.reason.is_empty(), "missing reason on {w:?}");
        assert!(!w.actionable_fix.is_empty(), "missing actionable_fix on {w:?}");
    }
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.source_path.contains("cactus.json") && w.stage == PipelineStage::Emit),
        "expected structured warning for omitted 3D vanilla cactus block geo"
    );
    assert!(
        report.warnings.iter().any(|w| {
            w.source_path.contains("custom_tex_b_n.png") && w.stage == PipelineStage::Transform
        }),
        "expected structured warning for omitted LabPBR normal map"
    );

    // --- Mode B: `--convert-pbr` enabled conversion ---
    let out_pbr = temp.path().join("generic_pbr.mcpack");
    let (_, _, pbr_report) = convert_pack(&ConvertOptions {
        input: input_zip,
        output: out_pbr.clone(),
        target: Some(Edition::Bedrock),
        target_version: Some(String::from("26.60")),
        mapping_file: None,
        strict: false,
        report_json: None,
        limits: IoLimits::default(),
        version_options: VersionSpecificOptions {
            convert_pbr: true,
            ..VersionSpecificOptions::default()
        },
    })
    .expect("convert with --convert-pbr");

    let pbr_out = PackContainer::read_from_path(&out_pbr, IoLimits::default()).expect("read pbr");
    assert!(pbr_out.contains("textures/blocks/custom_tex_b_mer.png"));
    assert!(pbr_out.contains("textures/blocks/custom_tex_b_normal.png"));
    let ts: BedrockTextureSetFile = serde_json::from_slice(
        pbr_out
            .get("textures/blocks/custom_tex_b.texture_set.json")
            .expect("custom_tex_b.texture_set.json"),
    )
    .expect("parse texture_set");
    assert_eq!(ts.texture_set.color, "custom_tex_b");
    assert_eq!(
        ts.texture_set.metalness_emissive_roughness.as_deref(),
        Some("custom_tex_b_mer")
    );
    assert_eq!(
        ts.texture_set.normal.as_deref(),
        Some("custom_tex_b_normal")
    );
    assert!(ts.texture_set.heightmap.is_none());

    let pbr_manifest: BedrockManifest =
        serde_json::from_slice(pbr_out.get("manifest.json").expect("manifest")).expect("parse");
    assert!(pbr_manifest.capabilities.contains(&String::from("pbr")));
    assert_eq!(pbr_manifest.header.min_engine_version[0], 1);

    let pbr_audit = pbr_report.semantic_audit.expect("pbr audit");
    assert_eq!(pbr_audit.status, AuditStatus::Passed);
    assert_eq!(pbr_audit.forbidden_entries.len(), 0);
    assert_eq!(pbr_audit.broken_references.len(), 0);
    assert_eq!(pbr_audit.orphan_textures.len(), 0);
}

/// Negative Test: Bedrock `manifest.json` with `min_engine_version: [26, 60, 0]` (major != 1)
/// MUST be rejected with `AuditStatus::Failed` by the Semantic Auditor!
#[test]
fn negative_test_manifest_min_engine_major_26_is_rejected_by_auditor() {
    let mappings = MappingTable::default_embedded().expect("mappings");

    // v2 array form: [26, 60, 0]
    let mut bad_v2 = PackContainer::new();
    let bad_manifest = BedrockManifest::new("Bad Major Pack", "26.60.0", [1, 0, 0], [26, 60, 0]);
    bad_v2
        .insert(
            "manifest.json",
            bad_manifest.to_json_vec_versioned(2).expect("v2"),
        )
        .expect("manifest");
    bad_v2
        .insert(
            "textures/blocks/stone.png",
            encode_png(16, 16, Rgba([128, 128, 128, 255])),
        )
        .expect("stone");

    let audit_v2 = audit_bedrock_container(&bad_v2, 1, &mappings);
    assert_eq!(audit_v2.status, AuditStatus::Failed);
    assert!(!audit_v2.manifest_valid);
    assert!(
        audit_v2
            .forbidden_entries
            .iter()
            .any(|f| f.rule_id == "invalid_min_engine_major_version"),
        "expected invalid_min_engine_major_version violation, got {:?}",
        audit_v2.forbidden_entries
    );

    // v3 SemVer string form: "26.60.0"
    let mut bad_v3 = PackContainer::new();
    bad_v3
        .insert(
            "manifest.json",
            bad_manifest.to_json_vec_versioned(3).expect("v3"),
        )
        .expect("manifest");
    bad_v3
        .insert(
            "textures/blocks/stone.png",
            encode_png(16, 16, Rgba([128, 128, 128, 255])),
        )
        .expect("stone");

    let audit_v3 = audit_bedrock_container(&bad_v3, 1, &mappings);
    assert_eq!(audit_v3.status, AuditStatus::Failed);
    assert!(!audit_v3.manifest_valid);
    assert!(
        audit_v3
            .forbidden_entries
            .iter()
            .any(|f| f.rule_id == "invalid_min_engine_major_version")
    );
}

#[test]
fn negative_test_unmappable_pack_fails_cli_and_audit_gate_without_false_positive() {
    let temp = TempDir::new().expect("tempdir");
    let bad_zip = temp.path().join("unmappable_java_pack.zip");
    let out_mcpack = temp.path().join("should_fail.mcpack");
    let report_json = temp.path().join("failed_report.json");

    write_raw_zip(
        &bad_zip,
        &[
            (
                "pack.mcmeta",
                br#"{"pack":{"pack_format":34,"description":"Zero Valid Assets Pack"}}"#.to_vec(),
            ),
            (
                "assets/minecraft/textures/block/totally_unknown_custom_1.png",
                encode_png(16, 16, Rgba([200, 10, 10, 255])),
            ),
            (
                "assets/minecraft/textures/block/totally_unknown_custom_2.png",
                encode_png(16, 16, Rgba([10, 200, 10, 255])),
            ),
        ],
    );

    let exe = env!("CARGO_BIN_EXE_rp-cli");
    let output = std::process::Command::new(exe)
        .arg("convert")
        .arg(&bad_zip)
        .arg("-o")
        .arg(&out_mcpack)
        .arg("--to")
        .arg("bedrock")
        .arg("--target-version")
        .arg("26.60")
        .arg("--report-json")
        .arg(&report_json)
        .output()
        .expect("run rp-cli convert");

    assert!(
        !output.status.success(),
        "CLI MUST return non-zero exit code when 0 textures map semantically!"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("FAILED"),
        "expected FAILED message on stderr, got: {stderr}"
    );
    assert!(
        report_json.exists(),
        "report JSON should still be written on audit failure"
    );
    let parsed_report: ConversionReport =
        serde_json::from_slice(&std::fs::read(&report_json).expect("read report"))
            .expect("parse report");
    let audit = parsed_report.semantic_audit.expect("semantic audit");
    assert_eq!(audit.status, AuditStatus::Failed);
    assert_eq!(audit.total_input_textures, 2);
    assert_eq!(audit.semantically_mapped_textures, 0);

    let mappings = MappingTable::default_embedded().expect("mappings");
    let outcome = audit_pack_roundtrip(
        &bad_zip,
        Some("26.60"),
        &VersionSpecificOptions::default(),
        &mappings,
        IoLimits::default(),
    );
    assert!(
        !outcome.passed,
        "audit_pack_roundtrip MUST NOT report passed=true!"
    );
    assert_eq!(outcome.status, AuditStatus::Failed);
}

#[test]
fn semantic_auditor_catches_all_forbidden_bedrock_violations() {
    let mappings = MappingTable::default_embedded().expect("mappings");
    let mut bad = PackContainer::new();
    let manifest = BedrockManifest::new("Bad Pack", "Violations", [1, 0, 0], [1, 26, 60]);
    bad.insert(
        "manifest.json",
        manifest.to_json_vec_versioned(2).expect("manifest"),
    )
    .expect("manifest");
    bad.insert("assets/minecraft/texts/credits.json", b"[]".to_vec())
        .expect("leaked assets");
    bad.insert("desktop.ini", b"junk".to_vec())
        .expect("desktop.ini");
    bad.insert(
        "textures/mob_effect/speed.png",
        encode_png(18, 18, Rgba([1, 2, 3, 255])),
    )
    .expect("bad mob_effect");
    bad.insert("models/entity/block/cactus.geo.json", b"{}".to_vec())
        .expect("dead block geo");
    bad.insert(
        "textures/blocks/custom_tex_a_n.png",
        encode_png(16, 16, Rgba([128, 128, 255, 255])),
    )
    .expect("raw labpbr");
    bad.insert("texts/languages.json", b"[]".to_vec())
        .expect("empty lang stub");
    bad.insert(
        "textures/blocks/unreferenced_orphan.png",
        encode_png(16, 16, Rgba([5, 6, 7, 255])),
    )
    .expect("orphan");

    let audit = audit_bedrock_container(&bad, 2, &mappings);
    assert_eq!(audit.status, AuditStatus::Failed);
    let rule_ids: Vec<&str> = audit
        .forbidden_entries
        .iter()
        .map(|f| f.rule_id.as_str())
        .collect();
    for expected in [
        "leaked_java_assets",
        "os_junk_files",
        "invalid_bedrock_mob_effect_dir",
        "dead_vanilla_block_geo",
        "raw_unconverted_labpbr",
        "empty_stub_languages_json",
    ] {
        assert!(
            rule_ids.contains(&expected),
            "expected auditor to catch rule '{expected}', got {rule_ids:?}"
        );
    }
    assert_eq!(
        audit.orphan_textures,
        vec![String::from("textures/blocks/unreferenced_orphan.png")]
    );
}
