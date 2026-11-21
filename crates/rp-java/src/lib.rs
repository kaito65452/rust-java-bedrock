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
}
