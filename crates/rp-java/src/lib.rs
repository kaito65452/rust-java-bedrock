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
    }
}
