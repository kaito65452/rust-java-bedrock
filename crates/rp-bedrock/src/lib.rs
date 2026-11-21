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
}
