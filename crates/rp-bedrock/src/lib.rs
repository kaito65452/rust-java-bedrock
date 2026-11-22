#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

//! Bedrock Edition resource pack parser and emitter (`manifest.json`,
//! `terrain_texture.json`, `item_texture.json`, `flipbook_textures.json`,
//! `*.geo.json`, `sound_definitions.json`, and `texts/*.lang`).

pub mod audit;
pub mod emitter;
pub mod error;
pub mod parser;
pub mod schema;

pub use audit::audit_bedrock_container;
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
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
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
    fn bedrock_flipbook_224_by_40_mismatch_succeeds_with_warning_and_lossy_report() {
        let mappings = MappingTable::default_embedded().expect("mappings");
        let mut container = rp_io::PackContainer::new();
        container
            .insert(
                "manifest.json",
                br#"{
                  "format_version": 2,
                  "header": {
                    "name": "256x Pack",
                    "description": "Converted 256x pack",
                    "uuid": "11111111-1111-4111-8111-111111111111",
                    "version": [1, 0, 0],
                    "min_engine_version": [1, 20, 10]
                  },
                  "modules": [{
                    "type": "resources",
                    "uuid": "22222222-2222-4222-8222-222222222222",
                    "version": [1, 0, 0]
                  }]
                }"#
                .to_vec(),
            )
            .expect("insert manifest");
        container
            .insert(
                "textures/blocks/custom_flipbook.png",
                make_header_png(40, 224),
            )
            .expect("insert 40x224 png");
        container
            .insert("textures/blocks/stone.png", make_header_png(256, 256))
            .expect("insert 256x256 png");
        container
            .insert("textures/blocks/ultra_hd.png", make_header_png(2048, 2048))
            .expect("insert 2048x2048 png");
        container
            .insert(
                "textures/flipbook_textures.json",
                br#"[{
                  "flipbook_texture": "textures/blocks/custom_flipbook",
                  "atlas_tile": "custom_flipbook",
                  "ticks_per_frame": 2
                }]"#
                .to_vec(),
            )
            .expect("insert flipbook");

        let (ir, report) =
            parse_bedrock_pack(&container, &mappings).expect("non-fatal flipbook mismatch");

        assert_eq!(ir.textures.len(), 3);
        let flip_tex = &ir.textures["block/custom_flipbook"];
        assert_eq!(flip_tex.width, Some(40));
        assert_eq!(flip_tex.height, Some(224));
        assert!(flip_tex.animation.is_none());

        let stone = &ir.textures["block/stone"];
        assert_eq!(stone.width, Some(256));
        assert_eq!(stone.height, Some(256));
        assert_eq!(stone.resolution_tier, Some(rp_core::ResolutionTier::R256));
        assert!(!stone.payload.is_decoded());

        let ultra = &ir.textures["block/ultra_hd"];
        assert_eq!(ultra.width, Some(2048));
        assert_eq!(ultra.height, Some(2048));
        assert_eq!(ultra.resolution_tier, Some(rp_core::ResolutionTier::R2048));
        assert!(!ultra.payload.is_decoded());

        assert_eq!(report.lossy_count, 1);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.severity == rp_core::WarningSeverity::Warning
                    && w.message.contains("40x224")
                    && w.message.contains("40")),
            "expected structured warning for 40x224 mismatch, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn bedrock_roundtrip_infers_non_square_flipbook_frame_height_from_emitted_frames() {
        let mappings = MappingTable::default_embedded().expect("mappings");
        let mut ir = ResourcePackIR {
            metadata: PackMetadata {
                name: String::from("NonSquareAnim"),
                description: String::from("Test"),
                source_edition: Edition::Java,
                java_pack_format: Some(15),
                bedrock_min_engine_version: Some([1, 20, 10]),
                version: [1, 0, 0],
                ..PackMetadata::default()
            },
            ..ResourcePackIR::default()
        };
        ir.textures.insert(
            String::from("block/custom_strip"),
            rp_core::TextureAsset {
                id: String::from("block/custom_strip"),
                category: rp_core::TextureCategory::Block,
                atlas_key: Some(String::from("custom_strip")),
                width: Some(40),
                height: Some(224),
                resolution_tier: Some(rp_core::ResolutionTier::from_dimensions(40, 224)),
                payload: rp_core::TexturePayload::from_raw(make_header_png(40, 224)),
                animation: Some(rp_core::AnimationSpec {
                    frametime: 3,
                    interpolate: false,
                    frames: Vec::new(),
                    frame_width: Some(40),
                    frame_height: Some(32),
                }),
            },
        );

        let (bedrock_pack, _) = emit_bedrock_pack(&ir, &mappings).expect("emit bedrock");
        let (reparsed, reparse_report) =
            parse_bedrock_pack(&bedrock_pack, &mappings).expect("reparse bedrock");
        assert_eq!(reparse_report.lossy_count, 0);
        let anim = reparsed.textures["block/custom_strip"]
            .animation
            .as_ref()
            .expect("preserved animation");
        assert_eq!(anim.frame_height, Some(32));
        assert_eq!(anim.frames.len(), 7);
    }
}
