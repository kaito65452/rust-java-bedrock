use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use rp_core::{SoundEntry, SoundEventDef};
use rp_transforms::sound::{bedrock_sound_ref_to_canonical, canonical_sound_to_bedrock_ref};

use crate::BedrockPackError;

/// Bedrock `manifest.json` schema (`format_version: 1`, `2`, or `3`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BedrockManifest {
    /// Manifest schema version (`1`, `2`, or `3`).
    pub format_version: u32,
    /// Pack header metadata.
    pub header: BedrockManifestHeader,
    /// Pack module declarations (must include `"resources"`).
    pub modules: Vec<BedrockManifestModule>,
    /// Optional metadata section (required in `format_version: 3`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BedrockManifestMetadata>,
}

/// Optional `metadata` block in Bedrock `manifest.json` (`format_version: 3`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BedrockManifestMetadata {
    /// Pack authors list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
}

/// `header` object in Bedrock `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BedrockManifestHeader {
    /// Pack display name.
    pub name: String,
    /// Pack description.
    pub description: String,
    /// RFC-4122 UUID identifying the pack header.
    pub uuid: Uuid,
    /// Pack version `[major, minor, patch]` (accepts array or SemVer string).
    #[serde(deserialize_with = "deserialize_version_triplet")]
    pub version: [u32; 3],
    /// Minimum required Bedrock engine version `[major, minor, patch]`.
    #[serde(
        default = "default_min_engine_version",
        deserialize_with = "deserialize_version_triplet"
    )]
    pub min_engine_version: [u32; 3],
}

fn default_min_engine_version() -> [u32; 3] {
    [1, 20, 0]
}

/// Single entry in `modules` array of Bedrock `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BedrockManifestModule {
    /// Optional module description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Module type (`"resources"`).
    #[serde(rename = "type")]
    pub kind: String,
    /// RFC-4122 UUID identifying the module (must differ from `header.uuid`).
    pub uuid: Uuid,
    /// Module version `[major, minor, patch]`.
    #[serde(deserialize_with = "deserialize_version_triplet")]
    pub version: [u32; 3],
}

fn deserialize_version_triplet<'de, D>(deserializer: D) -> Result<[u32; 3], D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum VersionInput {
        Triplet([u32; 3]),
        SemVer(String),
    }

    match VersionInput::deserialize(deserializer)? {
        VersionInput::Triplet(v) => Ok(v),
        VersionInput::SemVer(s) => {
            let parts: Vec<&str> = s.trim().split('.').collect();
            if parts.len() != 3 {
                return Err(serde::de::Error::custom(format!(
                    "expected SemVer 'X.Y.Z', got '{s}'"
                )));
            }
            let a = parts[0]
                .parse::<u32>()
                .map_err(|_| serde::de::Error::custom(format!("invalid SemVer '{s}'")))?;
            let b = parts[1]
                .parse::<u32>()
                .map_err(|_| serde::de::Error::custom(format!("invalid SemVer '{s}'")))?;
            let c = parts[2]
                .parse::<u32>()
                .map_err(|_| serde::de::Error::custom(format!("invalid SemVer '{s}'")))?;
            Ok([a, b, c])
        }
    }
}

impl BedrockManifest {
    /// Constructs a new Bedrock `manifest.json` (`format_version: 2`) with fresh RFC-4122 UUID v4
    /// identifiers for both `header.uuid` and `modules[0].uuid`.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        version: [u32; 3],
        min_engine_version: [u32; 3],
    ) -> Self {
        let description_str = description.into();
        let header_uuid = Uuid::new_v4();
        let mut module_uuid = Uuid::new_v4();
        while module_uuid == header_uuid {
            module_uuid = Uuid::new_v4();
        }

        Self {
            format_version: 2,
            header: BedrockManifestHeader {
                name: name.into(),
                description: description_str.clone(),
                uuid: header_uuid,
                version,
                min_engine_version,
            },
            modules: vec![BedrockManifestModule {
                description: Some(description_str),
                kind: String::from("resources"),
                uuid: module_uuid,
                version,
            }],
            metadata: None,
        }
    }

    /// Serializes `manifest.json` using `format_version` `1`, `2` (arrays), or `3` (SemVer strings).
    pub fn to_json_vec_versioned(&self, format_version: u32) -> Result<Vec<u8>, serde_json::Error> {
        match format_version {
            1 => {
                let val = serde_json::json!({
                    "format_version": 1,
                    "header": {
                        "name": self.header.name,
                        "description": self.header.description,
                        "uuid": self.header.uuid,
                        "version": self.header.version,
                    },
                    "modules": self.modules,
                });
                serde_json::to_vec_pretty(&val)
            }
            3 => {
                let ver_str = format!(
                    "{}.{}.{}",
                    self.header.version[0], self.header.version[1], self.header.version[2]
                );
                let engine_str = format!(
                    "{}.{}.{}",
                    self.header.min_engine_version[0],
                    self.header.min_engine_version[1],
                    self.header.min_engine_version[2]
                );
                let modules_v3: Vec<serde_json::Value> = self
                    .modules
                    .iter()
                    .map(|m| {
                        let m_ver = format!("{}.{}.{}", m.version[0], m.version[1], m.version[2]);
                        serde_json::json!({
                            "description": m.description,
                            "type": m.kind,
                            "uuid": m.uuid,
                            "version": m_ver,
                        })
                    })
                    .collect();
                let authors = self
                    .metadata
                    .as_ref()
                    .map(|m| m.authors.clone())
                    .filter(|a| !a.is_empty())
                    .unwrap_or_else(|| vec![String::from("mc-rp-converter")]);
                let val = serde_json::json!({
                    "format_version": 3,
                    "header": {
                        "name": self.header.name,
                        "description": self.header.description,
                        "uuid": self.header.uuid,
                        "version": ver_str,
                        "min_engine_version": engine_str,
                    },
                    "modules": modules_v3,
                    "metadata": {
                        "authors": authors
                    }
                });
                serde_json::to_vec_pretty(&val)
            }
            _ => serde_json::to_vec_pretty(self),
        }
    }

    /// Validates required manifest fields and `format_version`.
    pub fn validate(&self) -> Result<(), BedrockPackError> {
        if self.format_version == 0 || self.format_version > 3 {
            return Err(BedrockPackError::InvalidManifest(format!(
                "unsupported manifest format_version {} (expected 1, 2, or 3)",
                self.format_version
            )));
        }
        if self.header.name.trim().is_empty() {
            return Err(BedrockPackError::InvalidManifest(String::from(
                "header.name must not be empty",
            )));
        }
        if self.modules.is_empty() {
            return Err(BedrockPackError::InvalidManifest(String::from(
                "manifest.modules must contain at least one module",
            )));
        }
        Ok(())
    }
}

/// Bedrock `attachables/*.json` definition linking item IDs, textures, and geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BedrockAttachableFile {
    /// Attachable format version (e.g. `"1.8.0"` or `"1.10.0"`).
    pub format_version: String,
    /// Attachable definition body.
    #[serde(rename = "minecraft:attachable")]
    pub attachable: BedrockAttachableBody,
}

/// Inner `minecraft:attachable` object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BedrockAttachableBody {
    /// Attachable description.
    pub description: BedrockAttachableDescription,
}

/// Attachable `description` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BedrockAttachableDescription {
    /// Item identifier (e.g. `"minecraft:netherite_helmet"`).
    pub identifier: String,
    /// Optional conditional item binding map/string (`1.10.0`+ attachables).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<serde_json::Value>,
    /// Material bindings.
    #[serde(default)]
    pub materials: BTreeMap<String, String>,
    /// Texture path bindings (e.g. `"default": "textures/models/armor/kitsune_black"`).
    #[serde(default)]
    pub textures: BTreeMap<String, String>,
    /// Geometry bindings (e.g. `"default": "geometry.reyzz_mask"`).
    #[serde(default)]
    pub geometry: BTreeMap<String, String>,
    /// Render controllers list (strings or conditional controller objects).
    #[serde(default)]
    pub render_controllers: Vec<serde_json::Value>,
}

/// Bedrock `sounds/sound_definitions.json` (`1.14.0` format).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BedrockSoundDefinitionsFile {
    /// Format version (`"1.14.0"`).
    pub format_version: String,
    /// Map of Bedrock sound event ID -> sound definition.
    pub sound_definitions: BTreeMap<String, BedrockSoundDefJson>,
}

/// Either modern `{ format_version, sound_definitions }` or legacy flat map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BedrockSoundDefinitionsInput {
    /// Modern `1.14.0`+ wrapper.
    Modern(BedrockSoundDefinitionsFile),
    /// Legacy flat map at JSON root.
    Legacy(BTreeMap<String, BedrockSoundDefJson>),
}

/// Single Bedrock sound event definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BedrockSoundDefJson {
    /// Sound category (`"block"`, `"neutral"`, `"hostile"`, `"player"`, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Sound variants.
    #[serde(default)]
    pub sounds: Vec<BedrockSoundVariantJson>,
}

/// Sound variant inside Bedrock `sound_definitions.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BedrockSoundVariantJson {
    /// Simple `"sounds/..."` path string.
    Simple(String),
    /// Detailed object with volume/pitch/weight/stream.
    Detailed {
        /// Sound path relative to pack root without `.ogg`.
        name: String,
        /// Volume multiplier.
        #[serde(skip_serializing_if = "Option::is_none")]
        volume: Option<f32>,
        /// Pitch multiplier.
        #[serde(skip_serializing_if = "Option::is_none")]
        pitch: Option<f32>,
        /// Selection weight.
        #[serde(skip_serializing_if = "Option::is_none")]
        weight: Option<u32>,
        /// Stream from disk flag.
        #[serde(skip_serializing_if = "Option::is_none")]
        stream: Option<bool>,
    },
}

impl From<&BedrockSoundDefJson> for SoundEventDef {
    fn from(def: &BedrockSoundDefJson) -> Self {
        let sounds = def
            .sounds
            .iter()
            .map(|s| match s {
                BedrockSoundVariantJson::Simple(path) => SoundEntry {
                    name: bedrock_sound_ref_to_canonical(path),
                    ..SoundEntry::default()
                },
                BedrockSoundVariantJson::Detailed {
                    name,
                    volume,
                    pitch,
                    weight,
                    stream,
                } => SoundEntry {
                    name: bedrock_sound_ref_to_canonical(name),
                    volume: volume.unwrap_or(1.0),
                    pitch: pitch.unwrap_or(1.0),
                    weight: weight.unwrap_or(1),
                    stream: stream.unwrap_or(false),
                },
            })
            .collect();

        Self {
            category: def.category.clone(),
            subtitle: None,
            replace: false,
            sounds,
        }
    }
}

impl From<&SoundEventDef> for BedrockSoundDefJson {
    fn from(def: &SoundEventDef) -> Self {
        let sounds = def
            .sounds
            .iter()
            .map(|s| {
                let path = canonical_sound_to_bedrock_ref(&s.name);
                let is_default = (s.volume - 1.0).abs() < 1e-6
                    && (s.pitch - 1.0).abs() < 1e-6
                    && s.weight == 1
                    && !s.stream;
                if is_default {
                    BedrockSoundVariantJson::Simple(path)
                } else {
                    BedrockSoundVariantJson::Detailed {
                        name: path,
                        volume: if (s.volume - 1.0).abs() < 1e-6 {
                            None
                        } else {
                            Some(s.volume)
                        },
                        pitch: if (s.pitch - 1.0).abs() < 1e-6 {
                            None
                        } else {
                            Some(s.pitch)
                        },
                        weight: if s.weight == 1 { None } else { Some(s.weight) },
                        stream: if s.stream { Some(true) } else { None },
                    }
                }
            })
            .collect();

        Self {
            category: def.category.clone(),
            sounds,
        }
    }
}
