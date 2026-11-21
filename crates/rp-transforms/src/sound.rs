use rp_core::{ConversionReport, Edition, MappingTable, WarningSeverity};

const OGG_MAGIC: &[u8; 4] = b"OggS";

/// Validates that raw audio bytes begin with an Ogg Vorbis/Opus `"OggS"` container header.
#[must_use]
pub fn validate_ogg_header(bytes: &[u8]) -> bool {
    bytes.len() >= OGG_MAGIC.len() && &bytes[..4] == OGG_MAGIC
}

/// Validates an audio asset's `.ogg` header and records a warning if invalid.
pub fn validate_sound_asset_ogg(
    asset_id: &str,
    bytes: &[u8],
    report: &mut ConversionReport,
) -> bool {
    let valid = validate_ogg_header(bytes);
    if !valid {
        report.record_warning(
            asset_id,
            WarningSeverity::Warning,
            "Audio file does not start with valid 'OggS' header; copying raw bytes as pass-through",
        );
    }
    valid
}

/// Normalizes a canonical sound asset ID (without `.ogg`) into Bedrock `sounds/<path>` format.
#[must_use]
pub fn canonical_sound_to_bedrock_ref(sound_id: &str) -> String {
    let clean = sound_id
        .trim()
        .replace('\\', "/")
        .strip_prefix("minecraft:")
        .unwrap_or(sound_id.trim())
        .trim_start_matches('/')
        .to_string();
    let without_ext = clean.strip_suffix(".ogg").unwrap_or(&clean);
    if without_ext.starts_with("sounds/") {
        without_ext.to_string()
    } else {
        format!("sounds/{without_ext}")
    }
}

/// Normalizes a Bedrock `sounds/<path>` reference into a canonical sound ID (without `sounds/` or `.ogg`).
#[must_use]
pub fn bedrock_sound_ref_to_canonical(bedrock_ref: &str) -> String {
    let clean = bedrock_ref.trim().replace('\\', "/");
    let without_leading = clean.trim_start_matches('/');
    let without_ext = without_leading
        .strip_suffix(".ogg")
        .unwrap_or(without_leading);
    without_ext
        .strip_prefix("sounds/")
        .unwrap_or(without_ext)
        .to_string()
}

/// Maps a sound event identifier using `MappingTable` for the requested edition.
#[must_use]
pub fn map_sound_event_name(event_name: &str, mappings: &MappingTable, target: Edition) -> String {
    match target {
        Edition::Bedrock => mappings.java_sound_to_bedrock(event_name),
        Edition::Java => mappings.bedrock_sound_to_java(event_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_ogg_magic_and_paths() {
        assert!(validate_ogg_header(b"OggS\x00\x02dummy_vorbis"));
        assert!(!validate_ogg_header(b"RIFF\x00\x00\x00\x00WAVE"));
        assert!(!validate_ogg_header(b"Og"));

        assert_eq!(
            canonical_sound_to_bedrock_ref("mob/zombie/say1"),
            "sounds/mob/zombie/say1"
        );
        assert_eq!(
            bedrock_sound_ref_to_canonical("sounds/mob/zombie/say1.ogg"),
            "mob/zombie/say1"
        );
    }
}
