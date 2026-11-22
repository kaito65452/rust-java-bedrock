use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;

use image::{ImageFormat, Rgba, RgbaImage};
use proptest::prelude::*;

use rp_bedrock::{emit_bedrock_pack, parse_bedrock_pack};
use rp_cli::convert_container;
use rp_core::{
    AnimationFrame, AnimationSpec, ConversionReport, Edition, ElementFace, ElementRotation,
    FaceDirection, LocaleTable, MappingTable, ModelAsset, ModelCategory, ModelElement,
    RotationAxis,
};
use rp_io::{IoError, IoLimits, PackContainer, sanitize_archive_path};
use rp_java::{JavaPackError, parse_java_pack};
use rp_transforms::locale::{emit_bedrock_lang, parse_bedrock_lang};
use rp_transforms::model::{
    bedrock_geo_to_model_assets, bedrock_to_java_box, bedrock_to_java_pivot,
    bedrock_to_java_rotation, bedrock_to_java_uv, java_to_bedrock_box, java_to_bedrock_pivot,
    java_to_bedrock_rotation, java_to_bedrock_uv, model_asset_to_bedrock_geo,
};
use rp_transforms::texture::{
    flipbook_to_java_animation, inspect_png_dimensions, java_animation_to_flipbook,
};

fn solid_png(w: u32, h: u32) -> Vec<u8> {
    let mut img = RgbaImage::new(w, h);
    for px in img.pixels_mut() {
        *px = Rgba([100, 150, 200, 255]);
    }
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .expect("png");
    buf
}

fn arb_axis() -> impl Strategy<Value = RotationAxis> {
    prop_oneof![
        Just(RotationAxis::X),
        Just(RotationAxis::Y),
        Just(RotationAxis::Z),
    ]
}

fn arb_java_angle() -> impl Strategy<Value = f32> {
    prop_oneof![Just(-45.0f32), Just(-22.5f32), Just(22.5f32), Just(45.0f32)]
}

fn arb_locale_entries() -> impl Strategy<Value = HashMap<String, String>> {
    prop::collection::hash_map(
        "[a-z]{2,6}\\.[a-z_]{2,10}",
        "[A-Za-z0-9 _=\\\\\\n\\t!@#$%^&*()\\-]{0,30}",
        1..12,
    )
}

proptest! {
    #[test]
    fn model_box_pivot_rotation_uv_roundtrip(
        fx in 0.0f32..8.0f32,
        fy in 0.0f32..8.0f32,
        fz in 0.0f32..8.0f32,
        sx in 1.0f32..8.0f32,
        sy in 1.0f32..8.0f32,
        sz in 1.0f32..8.0f32,
        px in 0.0f32..16.0f32,
        py in 0.0f32..16.0f32,
        pz in 0.0f32..16.0f32,
        axis in arb_axis(),
        angle in arb_java_angle(),
        u1 in 0.0f32..8.0f32,
        v1 in 0.0f32..8.0f32,
        du in 1.0f32..8.0f32,
        dv in 1.0f32..8.0f32,
    ) {
        let from = [fx, fy, fz];
        let to = [fx + sx, fy + sy, fz + sz];
        let (b_origin, b_size) = java_to_bedrock_box(from, to);
        let (rt_from, rt_to) = bedrock_to_java_box(b_origin, b_size);
        for i in 0..3 {
            prop_assert!((rt_from[i] - from[i]).abs() < 1e-4);
            prop_assert!((rt_to[i] - to[i]).abs() < 1e-4);
        }

        let b_pivot = java_to_bedrock_pivot([px, py, pz]);
        let rt_pivot = bedrock_to_java_pivot(b_pivot);
        for i in 0..3 {
            prop_assert!((rt_pivot[i] - [px, py, pz][i]).abs() < 1e-4);
        }

        let b_rot = java_to_bedrock_rotation(axis, angle);
        let mut report = ConversionReport::new();
        let (rt_axis, rt_angle, extra) =
            bedrock_to_java_rotation(b_rot, "block/prop", &mut report).expect("rot");
        prop_assert_eq!(rt_axis, axis);
        prop_assert!((rt_angle - angle).abs() < 1e-4);
        prop_assert!(extra.is_none());
        prop_assert_eq!(report.lossy_count, 0);

        let uv = [u1, v1, u1 + du, v1 + dv];
        let (buv, bsize) = java_to_bedrock_uv(uv);
        let rt_uv = bedrock_to_java_uv(buv, bsize);
        for i in 0..4 {
            prop_assert!((rt_uv[i] - uv[i]).abs() < 1e-4);
        }
    }

    #[test]
    fn full_model_asset_geo_roundtrips_cleanly(
        fx in 0u8..8,
        fy in 0u8..8,
        fz in 0u8..8,
        dx in 1u8..8,
        dy in 1u8..8,
        dz in 1u8..8,
        axis in arb_axis(),
        angle in arb_java_angle(),
    ) {
        let mappings = MappingTable::default_embedded().expect("mappings");
        let from = [f32::from(fx), f32::from(fy), f32::from(fz)];
        let to = [f32::from(fx + dx), f32::from(fy + dy), f32::from(fz + dz)];

        let mut faces = BTreeMap::new();
        faces.insert(
            FaceDirection::North,
            ElementFace {
                uv: Some([0.0, 0.0, f32::from(dx), f32::from(dy)]),
                texture: String::from("#all"),
                cullface: None,
                rotation: 0,
                tintindex: None,
            },
        );
        faces.insert(
            FaceDirection::Up,
            ElementFace {
                uv: Some([0.0, 0.0, f32::from(dx), f32::from(dz)]),
                texture: String::from("#all"),
                cullface: None,
                rotation: 90,
                tintindex: None,
            },
        );
        faces.insert(
            FaceDirection::Down,
            ElementFace {
                uv: Some([0.0, 0.0, f32::from(dx), f32::from(dz)]),
                texture: String::from("#all"),
                cullface: None,
                rotation: 180,
                tintindex: None,
            },
        );

        let model = ModelAsset {
            id: String::from("block/prop_cube"),
            category: ModelCategory::Block,
            parent: None,
            textures: BTreeMap::from([(String::from("all"), String::from("block/stone"))]),
            texture_size: [16, 16],
            ambient_occlusion: true,
            elements: vec![ModelElement {
                group: Some(String::from("root")),
                from,
                to,
                rotation: Some(ElementRotation {
                    origin: [8.0, 8.0, 8.0],
                    axis,
                    angle,
                    rescale: false,
                    extra_bedrock_euler: None,
                }),
                faces,
                shade: true,
                shade_direction_override: None,
                light_emission: None,
            }],
            display: BTreeMap::new(),
            has_overrides: false,
            overrides_count: 0,
        };

        let mut report = ConversionReport::new();
        let geo = model_asset_to_bedrock_geo(&model, &mappings, &mut report);
        let back = bedrock_geo_to_model_assets(&geo, "block/prop_cube", ModelCategory::Block, &mut report);
        prop_assert_eq!(back.len(), 1);
        prop_assert_eq!(&back[0].elements, &model.elements);
        prop_assert_eq!(report.lossy_count, 0);
    }

    #[test]
    fn animation_timing_and_sequence_roundtrips(
        frametime in 1u32..40,
        interpolate in any::<bool>(),
        frames in prop::collection::vec(0u32..4, 1..8),
    ) {
        let spec = AnimationSpec {
            frametime,
            interpolate,
            frames: frames
                .iter()
                .map(|&index| AnimationFrame { index, time: None })
                .collect(),
            frame_width: None,
            frame_height: None,
        };
        let mut report = ConversionReport::new();
        let flip = java_animation_to_flipbook("tile", "textures/blocks/tile", &spec, &mut report);
        let back = flipbook_to_java_animation(&flip);
        prop_assert_eq!(back, spec);
    }

    #[test]
    fn locale_table_bedrock_lang_roundtrips(entries in arb_locale_entries()) {
        let table = LocaleTable {
            entries,
            comments: vec![String::from("## Comment")],
        };
        let encoded = emit_bedrock_lang(&table);
        let decoded = parse_bedrock_lang(&encoded).expect("valid emitted lang");
        prop_assert_eq!(decoded, table);
    }

    #[test]
    fn java_bedrock_java_and_bedrock_java_bedrock_pack_roundtrips(
        desc in "[A-Za-z0-9 _.-]{1,40}",
        frametime in 1u32..20,
        interpolate in any::<bool>(),
        locale_val in "[A-Za-z0-9 _.-]{1,30}",
    ) {
        let mappings = MappingTable::default_embedded().expect("mappings");

        // Java -> Bedrock -> Java
        let mut j_in = PackContainer::new();
        let mcmeta = serde_json::json!({
            "pack": { "pack_format": 15, "description": desc }
        });
        j_in.insert("pack.mcmeta", serde_json::to_vec(&mcmeta).expect("mcmeta")).expect("ins");
        j_in.insert("assets/minecraft/textures/block/water_still.png", solid_png(16, 64)).expect("png");
        let anim_meta = serde_json::json!({
            "animation": { "frametime": frametime, "interpolate": interpolate, "frames": [0, 1, 2, 3] }
        });
        j_in.insert("assets/minecraft/textures/block/water_still.png.mcmeta", serde_json::to_vec(&anim_meta).expect("anim")).expect("ins");
        let lang_json = serde_json::json!({ "block.minecraft.water_still": locale_val });
        j_in.insert("assets/minecraft/lang/en_us.json", serde_json::to_vec(&lang_json).expect("lang")).expect("ins");

        let (bedrock_mid, _, _, _) =
            convert_container(&j_in, Some(Edition::Bedrock), None, &mappings, true).expect("J->B");
        let (java_out, _, _, _) =
            convert_container(&bedrock_mid, Some(Edition::Java), None, &mappings, true).expect("B->J");

        let (ir_orig, _) = parse_java_pack(&j_in, &mappings).expect("parse orig");
        let (ir_rt, _) = parse_java_pack(&java_out, &mappings).expect("parse rt");
        prop_assert_eq!(&ir_rt.metadata.description, &ir_orig.metadata.description);
        prop_assert_eq!(ir_rt.metadata.java_pack_format, ir_orig.metadata.java_pack_format);
        prop_assert_eq!(
            &ir_rt.textures["block/water_still"].animation,
            &ir_orig.textures["block/water_still"].animation
        );
        prop_assert_eq!(
            &ir_rt.locales["en_us"].entries,
            &ir_orig.locales["en_us"].entries
        );

        // Bedrock -> Java -> Bedrock
        let (ir_b_orig, _) = parse_bedrock_pack(&bedrock_mid, &mappings).expect("parse b_orig");
        let (bedrock_rt, _) = emit_bedrock_pack(&ir_rt, &mappings).expect("emit b_rt");
        let (ir_b_rt, _) = parse_bedrock_pack(&bedrock_rt, &mappings).expect("parse b_rt");
        prop_assert_eq!(&ir_b_rt.metadata.description, &ir_b_orig.metadata.description);
        prop_assert_eq!(
            &ir_b_rt.textures["block/water_still"].animation,
            &ir_b_orig.textures["block/water_still"].animation
        );
        prop_assert_eq!(
            &ir_b_rt.locales["en_us"].entries,
            &ir_b_orig.locales["en_us"].entries
        );
    }

    #[test]
    fn malformed_json_and_truncated_png_never_panic(
        junk in prop::collection::vec(any::<u8>(), 0..128),
        bad_fmt in prop_oneof![Just(0u32), 201u32..100_000u32],
    ) {
        let mappings = MappingTable::default_embedded().expect("mappings");

        // Truncated PNG header must always return Err without panicking.
        let valid_png = solid_png(16, 16);
        let cut = junk.len() % 20;
        prop_assert!(inspect_png_dimensions(&valid_png[..cut]).is_err());

        // Arbitrary byte payloads as PNG must never panic.
        let _ = inspect_png_dimensions(&junk);

        // Malformed pack.mcmeta / manifest.json must return Err without panicking.
        let mut c = PackContainer::new();
        if serde_json::from_slice::<serde_json::Value>(&junk).is_err() {
            c.insert("pack.mcmeta", junk.clone()).expect("insert junk");
            prop_assert!(parse_java_pack(&c, &mappings).is_err());

            let mut bc = PackContainer::new();
            bc.insert("manifest.json", junk).expect("insert junk");
            prop_assert!(parse_bedrock_pack(&bc, &mappings).is_err());
        }

        // Invalid pack_format must return InvalidPackFormat without panicking.
        let mut bad_fmt_container = PackContainer::new();
        bad_fmt_container
            .insert(
                "pack.mcmeta",
                format!(r#"{{"pack":{{"pack_format":{bad_fmt},"description":"x"}}}}"#).into_bytes(),
            )
            .expect("insert");
        let err = parse_java_pack(&bad_fmt_container, &mappings);
        let is_bad_fmt = matches!(err, Err(JavaPackError::InvalidPackFormat(_)));
        prop_assert!(is_bad_fmt);
    }

    #[test]
    fn traversal_paths_always_rejected(
        prefix in "[a-z]{0,6}",
        suffix in "[a-z.]{1,10}",
    ) {
        let raw = if prefix.is_empty() {
            format!("../{suffix}")
        } else {
            format!("{prefix}/../../{suffix}")
        };
        let res = sanitize_archive_path(&raw);
        let is_zip_slip = matches!(res, Err(IoError::ZipSlipDetected { .. }));
        prop_assert!(is_zip_slip);

        // Verify PackContainer::read_from_zip_bytes also rejects it.
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut cursor);
            let opts = zip::write::SimpleFileOptions::default();
            zw.start_file(&raw, opts).expect("start_file");
            std::io::Write::write_all(&mut zw, b"evil").expect("write");
            zw.finish().expect("finish");
        }
        let zip_res = PackContainer::read_from_zip_bytes(cursor.get_ref(), IoLimits::default());
        prop_assert!(zip_res.is_err());
    }
}
