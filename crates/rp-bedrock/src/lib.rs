#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

//! Bedrock Edition resource pack parser and emitter (`manifest.json`,
//! `terrain_texture.json`, `item_texture.json`, `flipbook_textures.json`,
//! `*.geo.json`, `sound_definitions.json`, and `texts/*.lang`).

pub mod emitter;
pub mod error;
pub mod parser;
pub mod schema;

pub use emitter::emit_bedrock_pack;
pub use error::BedrockPackError;
pub use parser::{parse_bedrock_pack, parse_bedrock_pack_with_options};

#[cfg(test)]
mod tests {
    use super::*;
    use rp_core::{Edition, MappingTable, PackMetadata, ResourcePackIR};
    use schema::BedrockManifest;
    use uuid::Version;

    #[test]
    fn generates_unique_rfc4122_v4_uuids_and_roundtrips_manifest() {
        let mappings = MappingTable::default_embedded().expect("mappings");
        let ir = ResourcePackIR {
            metadata: PackMetadata {
                name: String::from("Test Pack"),
                description: String::from("Bedrock manifest test"),
                source_edition: Edition::Java,
                java_pack_format: Some(15),
                bedrock_min_engine_version: Some([1, 20, 10]),
                version: [1, 2, 3],
                ..PackMetadata::default()
            },
            ..ResourcePackIR::default()
        };

        let (c1, _) = emit_bedrock_pack(&ir, &mappings).expect("emit 1");
        let (c2, _) = emit_bedrock_pack(&ir, &mappings).expect("emit 2");

        let m1: BedrockManifest =
            serde_json::from_slice(c1.get("manifest.json").expect("m1")).expect("json 1");
        let m2: BedrockManifest =
            serde_json::from_slice(c2.get("manifest.json").expect("m2")).expect("json 2");

        assert_eq!(m1.format_version, 2);
        assert_eq!(m1.header.uuid.get_version(), Some(Version::Random));
        assert_eq!(m1.modules[0].uuid.get_version(), Some(Version::Random));
        assert_ne!(m1.header.uuid, m1.modules[0].uuid);
        assert_ne!(m1.header.uuid, m2.header.uuid);
        assert_ne!(m1.modules[0].uuid, m2.modules[0].uuid);

        let (parsed, _) = parse_bedrock_pack(&c1, &mappings).expect("parse bedrock");
        assert_eq!(parsed.metadata.name, "Test Pack");
        assert_eq!(parsed.metadata.description, "Bedrock manifest test");
        assert_eq!(parsed.metadata.version, [1, 2, 3]);
        assert_eq!(
            parsed.metadata.bedrock_min_engine_version,
            Some([1, 20, 10])
        );
    }

    #[test]
    fn parses_bedrock_utf16_manifest_control_chars_and_latin1_lang() {
        let mappings = MappingTable::default_embedded().expect("mappings");
        let manifest_text = "{\r\n  \"format_version\": 2,\r\n  \"header\": {\r\n    \"name\": \"§b小困包\",\r\n    \"description\": \"Line1\r\nLine2\tTab\x00\",\r\n    \"uuid\": \"11111111-1111-4111-8111-111111111111\",\r\n    \"version\": [1, 0, 0],\r\n    \"min_engine_version\": [1, 20, 10],\r\n  },\r\n  \"modules\": [\r\n    {\r\n      \"type\": \"resources\",\r\n      \"uuid\": \"22222222-2222-4222-8222-222222222222\",\r\n      \"version\": [1, 0, 0],\r\n    },\r\n  ],\r\n}";
        let mut manifest_utf16_be = vec![0xFE, 0xFF];
        for u in manifest_text.encode_utf16() {
            manifest_utf16_be.extend_from_slice(&u.to_be_bytes());
        }

        let mut container = rp_io::PackContainer::new();
        container
            .insert("manifest.json", manifest_utf16_be)
            .expect("insert manifest");
        container
            .insert(
                "texts/en_US.lang",
                b"tile.stone.stone.name=\xA76Golden Stone\r\n".to_vec(),
            )
            .expect("insert lang");

        let (ir, _) = parse_bedrock_pack(&container, &mappings).expect("parse bedrock");
        assert_eq!(ir.metadata.name, "§b小困包");
        assert_eq!(ir.metadata.description, "Line1\nLine2\tTab");
        assert_eq!(
            ir.locales["en_us"].entries["block.minecraft.stone"],
            "§6Golden Stone"
        );
    }
}
