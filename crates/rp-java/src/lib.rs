#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

//! Java Edition resource pack parser and emitter (`pack.mcmeta`, textures,
//! `.png.mcmeta`, JSON models, `sounds.json`, and `lang/*.json`).

pub mod emitter;
pub mod error;
pub mod parser;
pub mod schema;

pub use emitter::emit_java_pack;
pub use error::JavaPackError;
pub use parser::parse_java_pack;

#[cfg(test)]
mod tests {
    use super::*;
    use rp_core::MappingTable;
    use rp_io::PackContainer;

    #[test]
    fn pack_mcmeta_parses_text_components_and_rejects_invalid_pack_format() {
        let mappings = MappingTable::default_embedded().expect("mappings");

        let mut container = PackContainer::new();
        container
            .insert(
                "pack.mcmeta",
                br#"{"pack":{"pack_format":15,"description":{"text":"Hello ","extra":[{"text":"World"}]}}}"#.to_vec(),
            )
            .expect("insert");

        let (ir, _report) = parse_java_pack(&container, &mappings).expect("parse java");
        assert_eq!(ir.metadata.java_pack_format, Some(15));
        assert_eq!(ir.metadata.description, "Hello World");

        let (re_emitted, _) = emit_java_pack(&ir, &mappings).expect("emit java");
        let (re_parsed, _) = parse_java_pack(&re_emitted, &mappings).expect("reparse java");
        assert_eq!(re_parsed.metadata.java_pack_format, Some(15));
        assert_eq!(re_parsed.metadata.description, "Hello World");

        for bad_fmt in [0u32, 201, 9999] {
            let mut bad = PackContainer::new();
            bad.insert(
                "pack.mcmeta",
                format!(r#"{{"pack":{{"pack_format":{bad_fmt},"description":"Bad"}}}}"#)
                    .into_bytes(),
            )
            .expect("insert");
            assert!(matches!(
                parse_java_pack(&bad, &mappings),
                Err(JavaPackError::InvalidPackFormat(v)) if v == bad_fmt
            ));
        }
    }

    #[test]
    fn pack_mcmeta_handles_chinese_utf8_supported_formats_array_and_raw_control_chars() {
        let mappings = MappingTable::default_embedded().expect("mappings");

        // 1. Clean Chinese pack.mcmeta with supported_formats: [15, 1024]
        let clean_mcmeta = r#"{
  "pack": {
    "pack_format": 15,
    "supported_formats": [15, 1024],
    "description": " §b小困超高清材质包§6_128x§e v2.8 §d【植物包】 §e1.20.1+"
  }
}"#;
        let mut c1 = PackContainer::new();
        c1.insert("pack.mcmeta", clean_mcmeta.as_bytes().to_vec())
            .expect("insert clean");
        let (ir1, _) = parse_java_pack(&c1, &mappings).expect("parse clean chinese mcmeta");
        assert_eq!(ir1.metadata.java_pack_format, Some(15));
        assert_eq!(ir1.metadata.java_supported_formats, Some([15, 1024]));
        assert_eq!(
            ir1.metadata.description,
            " §b小困超高清材质包§6_128x§e v2.8 §d【植物包】 §e1.20.1+"
        );

        // 2. Same Chinese pack.mcmeta with UTF-8 BOM, raw CR/LF newlines, tabs, NUL, and 0x07 inside string + trailing comma
        let mut dirty_mcmeta = vec![0xEF, 0xBB, 0xBF];
        dirty_mcmeta.extend_from_slice(
            "{\r\n  \"pack\": {\r\n    \"pack_format\": 15,\r\n    \"supported_formats\": [15, 1024],\r\n    \"description\": \" §b小困超高清材质包§6_128x\r\n\t§e v2.8 \x00§d【植物包】\x07 §e1.20.1+\",\r\n  },\r\n}"
                .as_bytes(),
        );
        let mut c2 = PackContainer::new();
        c2.insert("pack.mcmeta", dirty_mcmeta)
            .expect("insert dirty");
        // Also include UTF-16 LE lang JSON and raw ISO-8859-1 0xA7 legacy .lang file
        let zh_json = "{\n  \"block.minecraft.stone\": \"§a石头\n高清\",\n}";
        let mut zh_utf16_le = vec![0xFF, 0xFE];
        for u in zh_json.encode_utf16() {
            zh_utf16_le.extend_from_slice(&u.to_le_bytes());
        }
        c2.insert("assets/minecraft/lang/zh_cn.json", zh_utf16_le)
            .expect("insert zh_cn.json");
        c2.insert(
            "assets/minecraft/lang/en_us.lang",
            b"tile.stone.stone.name=\xA7bCool Stone\r\n".to_vec(),
        )
        .expect("insert en_us.lang");

        let (ir2, _) = parse_java_pack(&c2, &mappings).expect("parse dirty chinese pack");
        assert_eq!(ir2.metadata.java_pack_format, Some(15));
        assert_eq!(ir2.metadata.java_supported_formats, Some([15, 1024]));
        assert_eq!(
            ir2.metadata.description,
            " §b小困超高清材质包§6_128x\n\t§e v2.8 §d【植物包】\u{0007} §e1.20.1+"
        );
        assert_eq!(
            ir2.locales["zh_cn"].entries["block.minecraft.stone"],
            "§a石头\n高清"
        );
        assert_eq!(
            ir2.locales["en_us"].entries["block.minecraft.stone"],
            "§bCool Stone"
        );

        // 3. UTF-16 LE BOM pack.mcmeta with \u0001 control char and raw \r\n inside description
        let utf16_text = "{\r\n  \"pack\": {\r\n    \"pack_format\": 15,\r\n    \"supported_formats\": [15, 1024],\r\n    \"description\": \" §b小困超高清材质包\u{0001}\r\n§6_128x§e v2.8 §d【植物包】 §e1.20.1+\"\r\n  }\r\n}";
        let mut utf16_bytes = vec![0xFF, 0xFE];
        for u in utf16_text.encode_utf16() {
            utf16_bytes.extend_from_slice(&u.to_le_bytes());
        }
        let mut c3 = PackContainer::new();
        c3.insert("pack.mcmeta", utf16_bytes)
            .expect("insert utf16 mcmeta");
        let (ir3, _) = parse_java_pack(&c3, &mappings).expect("parse utf16 chinese mcmeta");
        assert_eq!(ir3.metadata.java_pack_format, Some(15));
        assert_eq!(ir3.metadata.java_supported_formats, Some([15, 1024]));
        assert_eq!(
            ir3.metadata.description,
            " §b小困超高清材质包\u{0001}\n§6_128x§e v2.8 §d【植物包】 §e1.20.1+"
        );
    }

    fn make_header_png(width: u32, height: u32) -> Vec<u8> {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFFu32;
            for &b in data {
                crc ^= u32::from(b);
                for _ in 0..8 {
                    if crc & 1 != 0 {
                        crc = (crc >> 1) ^ 0xEDB8_8320;
                    } else {
                        crc >>= 1;
                    }
                }
            }
            !crc
        }
        fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
            out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(payload);
            let mut crc_buf = Vec::with_capacity(4 + payload.len());
            crc_buf.extend_from_slice(kind);
            crc_buf.extend_from_slice(payload);
            out.extend_from_slice(&crc32(&crc_buf).to_be_bytes());
        }

        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
        push_chunk(&mut out, b"IHDR", &ihdr);
        push_chunk(
            &mut out,
            b"IDAT",
            &[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
        );
        push_chunk(&mut out, b"IEND", &[]);
        out
    }

    #[test]
    fn java_animation_dimension_mismatch_224_by_40_falls_back_with_warning() {
        let mappings = MappingTable::default_embedded().expect("mappings");
        let mut container = PackContainer::new();
        container
            .insert(
                "pack.mcmeta",
                br#"{"pack":{"pack_format":15,"description":"Anim mismatch test"}}"#.to_vec(),
            )
            .expect("insert mcmeta");
        container
            .insert(
                "assets/minecraft/textures/block/custom_anim.png",
                make_header_png(40, 224),
            )
            .expect("insert anim png");
        container
            .insert(
                "assets/minecraft/textures/block/custom_anim.png.mcmeta",
                br#"{"animation":{"frametime":2}}"#.to_vec(),
            )
            .expect("insert anim mcmeta");
        container
            .insert(
                "assets/minecraft/textures/block/stone.png",
                make_header_png(256, 256),
            )
            .expect("insert stone");

        let (ir, report) =
            parse_java_pack(&container, &mappings).expect("non-fatal animation mismatch");
        assert_eq!(ir.textures.len(), 2);
        let anim_tex = &ir.textures["block/custom_anim"];
        assert_eq!(anim_tex.width, Some(40));
        assert_eq!(anim_tex.height, Some(224));
        assert!(anim_tex.animation.is_none());
        assert_eq!(report.lossy_count, 1);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.severity == rp_core::WarningSeverity::Warning
                    && w.message.contains("40x224")),
            "expected mismatch warning, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn high_resolution_textures_up_to_2048_preserve_raw_bytes_and_resolution_tier() {
        let mappings = MappingTable::default_embedded().expect("mappings");
        let mut container = PackContainer::new();
        container
            .insert(
                "pack.mcmeta",
                br#"{"pack":{"pack_format":15,"description":"Ultra-HD Pack"}}"#.to_vec(),
            )
            .expect("insert mcmeta");

        let cases = [
            ("block/t128", 128u32, 128u32, rp_core::ResolutionTier::R128),
            ("block/t256", 256, 256, rp_core::ResolutionTier::R256),
            ("block/t1024", 1024, 1024, rp_core::ResolutionTier::R1024),
            ("block/t2048", 2048, 2048, rp_core::ResolutionTier::R2048),
        ];
        let mut raw_bytes = std::collections::BTreeMap::new();
        for (id, w, h, _) in cases {
            let png = make_header_png(w, h);
            container
                .insert(format!("assets/minecraft/textures/{id}.png"), png.clone())
                .expect("insert hi-res png");
            raw_bytes.insert(id, png);
        }

        let (ir, _report) = parse_java_pack(&container, &mappings).expect("parse hi-res pack");
        for (id, w, h, tier) in cases {
            let tex = &ir.textures[id];
            assert_eq!(tex.width, Some(w));
            assert_eq!(tex.height, Some(h));
            assert_eq!(tex.resolution_tier, Some(tier));
            assert!(
                !tex.payload.is_decoded(),
                "high-res texture {id} should remain raw pass-through"
            );
        }

        let (emitted, emit_report) = emit_java_pack(&ir, &mappings).expect("emit hi-res pack");
        assert!(emit_report.passthrough_count >= 4);
        for (id, expected_bytes) in raw_bytes {
            let path = format!("assets/minecraft/textures/{id}.png");
            assert_eq!(emitted.get(&path), Some(expected_bytes.as_slice()));
        }
    }
}
