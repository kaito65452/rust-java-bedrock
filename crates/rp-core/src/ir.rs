use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::Cursor;
use std::sync::Arc;

use image::{ImageFormat, ImageReader, RgbaImage};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::CoreError;

/// Target or source Minecraft edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edition {
    /// Minecraft: Java Edition.
    Java,
    /// Minecraft: Bedrock Edition.
    Bedrock,
}

impl fmt::Display for Edition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Java => write!(f, "Java"),
            Self::Bedrock => write!(f, "Bedrock"),
        }
    }
}

/// Logical texture category used for atlas placement and path mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureCategory {
    /// Block / terrain texture.
    Block,
    /// Item / held icon texture.
    Item,
    /// Entity / mob texture.
    Entity,
    /// Graphical user interface texture.
    Gui,
    /// Particle sprite sheet or particle texture.
    Particle,
    /// Sky, sun, moon, clouds, or rain/snow environment texture.
    Environment,
    /// Armor layer texture.
    Armor,
    /// Painting texture.
    Painting,
    /// Any other texture category.
    Other,
}

impl TextureCategory {
    /// Infers a category from a canonical texture ID (e.g. `"block/stone"`).
    #[must_use]
    pub fn infer_from_id(id: &str) -> Self {
        let lower = id.to_ascii_lowercase();
        if lower.starts_with("block/") || lower.starts_with("blocks/") {
            Self::Block
        } else if lower.starts_with("item/") || lower.starts_with("items/") {
            Self::Item
        } else if lower.starts_with("entity/") {
            Self::Entity
        } else if lower.starts_with("gui/") {
            Self::Gui
        } else if lower.starts_with("particle/") {
            Self::Particle
        } else if lower.starts_with("environment/") {
            Self::Environment
        } else if lower.starts_with("models/armor/") {
            Self::Armor
        } else if lower.starts_with("painting/") {
            Self::Painting
        } else {
            Self::Other
        }
    }
}

/// Lazy texture storage avoiding eager PNG decoding on high-resolution packs.
#[derive(Debug, Clone)]
pub enum TexturePayload {
    /// Raw encoded PNG bytes (zero-copy pass-through when pixel manipulation is unnecessary).
    Raw(Arc<Vec<u8>>),
    /// Decoded RGBA8 pixel buffer produced only when cropping/stitching/pixel work is required.
    Decoded(Arc<RgbaImage>),
}

impl PartialEq for TexturePayload {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Raw(a), Self::Raw(b)) => a == b,
            (Self::Decoded(a), Self::Decoded(b)) => {
                a.dimensions() == b.dimensions() && a.as_raw() == b.as_raw()
            }
            _ => match (self.to_png_bytes(), other.to_png_bytes()) {
                (Ok(a), Ok(b)) => a.as_ref() == b.as_ref(),
                _ => false,
            },
        }
    }
}

impl TexturePayload {
    /// Wraps raw PNG bytes without decoding pixel data.
    #[must_use]
    pub fn from_raw(bytes: Vec<u8>) -> Self {
        Self::Raw(Arc::new(bytes))
    }

    /// Wraps an already-decoded RGBA image.
    #[must_use]
    pub fn from_decoded(image: RgbaImage) -> Self {
        Self::Decoded(Arc::new(image))
    }

    /// Returns `true` if the texture has already been decoded into RGBA pixels.
    #[must_use]
    pub fn is_decoded(&self) -> bool {
        matches!(self, Self::Decoded(_))
    }

    /// Reads `(width, height)` from the PNG header without decoding pixel data when `Raw`.
    pub fn dimensions(&self) -> Result<(u32, u32), CoreError> {
        match self {
            Self::Decoded(img) => Ok(img.dimensions()),
            Self::Raw(bytes) => {
                let reader =
                    ImageReader::with_format(Cursor::new(bytes.as_slice()), ImageFormat::Png);
                Ok(reader.into_dimensions()?)
            }
        }
    }

    /// Decodes PNG bytes lazily into an `RgbaImage`, or borrows the already-decoded image.
    pub fn decode(&self) -> Result<Cow<'_, RgbaImage>, CoreError> {
        match self {
            Self::Decoded(img) => Ok(Cow::Borrowed(img.as_ref())),
            Self::Raw(bytes) => {
                let dyn_img =
                    image::load_from_memory_with_format(bytes.as_slice(), ImageFormat::Png)?;
                Ok(Cow::Owned(dyn_img.into_rgba8()))
            }
        }
    }

    /// Returns raw PNG bytes directly when `Raw`, or encodes `Decoded` into PNG bytes.
    pub fn to_png_bytes(&self) -> Result<Cow<'_, [u8]>, CoreError> {
        match self {
            Self::Raw(bytes) => Ok(Cow::Borrowed(bytes.as_slice())),
            Self::Decoded(img) => {
                let mut buf = Vec::new();
                img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)?;
                Ok(Cow::Owned(buf))
            }
        }
    }
}

/// Single frame entry in a texture animation sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimationFrame {
    /// Zero-based vertical strip frame index.
    pub index: u32,
    /// Optional per-frame duration override in game ticks (`1 tick = 0.05s`).
    pub time: Option<u32>,
}

/// Edition-agnostic animated texture specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimationSpec {
    /// Default ticks per frame (`1 tick = 0.05s`).
    pub frametime: u32,
    /// Whether adjacent frames interpolate/blend smoothly.
    pub interpolate: bool,
    /// Explicit frame playback sequence; empty means sequential `0..N`.
    pub frames: Vec<AnimationFrame>,
    /// Optional explicit frame width hint (Java `width`).
    pub frame_width: Option<u32>,
    /// Optional explicit frame height hint (Java `height`).
    pub frame_height: Option<u32>,
}

impl Default for AnimationSpec {
    fn default() -> Self {
        Self {
            frametime: 1,
            interpolate: false,
            frames: Vec::new(),
            frame_width: None,
            frame_height: None,
        }
    }
}

/// Texture asset stored in the canonical intermediate representation.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureAsset {
    /// Canonical texture identifier without `.png` extension (e.g. `"block/stone"`).
    pub id: String,
    /// Logical texture category.
    pub category: TextureCategory,
    /// Optional Bedrock atlas short key preserved across round-trips (e.g. `"grass_side"`).
    pub atlas_key: Option<String>,
    /// Lazy PNG byte payload or decoded RGBA image.
    pub payload: TexturePayload,
    /// Optional animation metadata for vertical flipbook strips.
    pub animation: Option<AnimationSpec>,
}

/// Strategy for handling Bedrock rotation angles outside Java's 22.5° increments.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationSnapMode {
    /// Preserve exact float angles (supported natively in Java 1.21.6+ / 26.x).
    #[default]
    Preserve,
    /// Snap angles to the nearest multiple of 22.5° within `[-45.0, 45.0]`.
    Snap22_5,
    /// Clamp angles to `[-45.0, 45.0]` without snapping increments.
    Clamp45,
}

/// Supported Bedrock geometry schema `format_version` targets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BedrockGeoVersion {
    /// Legacy top-level `"geometry.<id>"` format (`1.10.0`).
    V1_10_0,
    /// Standard `"minecraft:geometry"` format (`1.12.0`).
    #[default]
    V1_12_0,
    /// Nether-era `"minecraft:geometry"` format (`1.16.0`).
    V1_16_0,
    /// Modern `"minecraft:geometry"` format (`1.21.0`).
    V1_21_0,
}

impl BedrockGeoVersion {
    /// Returns the string representation used in Bedrock geometry JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1_10_0 => "1.10.0",
            Self::V1_12_0 => "1.12.0",
            Self::V1_16_0 => "1.16.0",
            Self::V1_21_0 => "1.21.0",
        }
    }

    /// Chooses an appropriate default Bedrock geometry version from `min_engine_version`.
    #[must_use]
    pub const fn for_engine_version(ver: [u32; 3]) -> Self {
        if ver[0] >= 26 || (ver[0] == 1 && ver[1] >= 21) {
            Self::V1_21_0
        } else if ver[0] == 1 && ver[1] >= 16 {
            Self::V1_16_0
        } else if ver[0] == 1 && ver[1] >= 12 {
            Self::V1_12_0
        } else {
            Self::V1_10_0
        }
    }
}

/// Bedrock attachable definition (`attachables/*.json`) linking an item, texture, and geometry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachableAsset {
    /// Attachable ID (file stem without `.json`, e.g. `"kitsune_black"`).
    pub id: String,
    /// Item identifier (e.g. `"minecraft:netherite_helmet"`).
    pub item_identifier: String,
    /// Canonical default texture ID if specified (e.g. `"models/armor/kitsune_black"`).
    pub default_texture: Option<String>,
    /// Default geometry identifier if specified (e.g. `"geometry.reyzz_mask"`).
    pub default_geometry: Option<String>,
}

/// Pack-level metadata shared across Java `pack.mcmeta` and Bedrock `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackMetadata {
    /// Human-readable pack name.
    pub name: String,
    /// Pack description text.
    pub description: String,
    /// Edition of the parsed source pack.
    pub source_edition: Edition,
    /// Java `pack_format` integer if known or requested.
    pub java_pack_format: Option<u32>,
    /// Optional Java `supported_formats` / `min_format..max_format` range `[min, max]`.
    pub java_supported_formats: Option<[u32; 2]>,
    /// Bedrock `min_engine_version` `[major, minor, patch]` if known or requested.
    pub bedrock_min_engine_version: Option<[u32; 3]>,
    /// Semantic pack version `[major, minor, patch]`.
    pub version: [u32; 3],
    /// Optional Bedrock `header.uuid`.
    pub header_uuid: Option<Uuid>,
    /// Optional Bedrock `modules[0].uuid`.
    pub module_uuid: Option<Uuid>,
    /// Raw PNG bytes for `pack.png` / `pack_icon.png`.
    pub icon_png: Option<Arc<Vec<u8>>>,
    /// Strategy for handling non-standard rotation angles when targeting Java.
    pub rotation_mode: RotationSnapMode,
    /// Optional target Bedrock geometry schema override (`1.10.0` .. `1.21.0`).
    pub bedrock_geo_version: Option<BedrockGeoVersion>,
    /// Optional Bedrock `manifest.json` `format_version` (`1`, `2`, or `3`).
    pub bedrock_manifest_version: Option<u32>,
    /// Override for generating Java `1.21.4+` (`pack_format >= 46`) `assets/minecraft/items/*.json`.
    pub emit_java_items_definitions: Option<bool>,
}

impl Default for PackMetadata {
    fn default() -> Self {
        Self {
            name: String::from("Converted Resource Pack"),
            description: String::from("Converted by mc-rp-converter"),
            source_edition: Edition::Java,
            java_pack_format: Some(15),
            java_supported_formats: None,
            bedrock_min_engine_version: Some([1, 20, 0]),
            version: [1, 0, 0],
            header_uuid: None,
            module_uuid: None,
            icon_png: None,
            rotation_mode: RotationSnapMode::Preserve,
            bedrock_geo_version: None,
            bedrock_manifest_version: None,
            emit_java_items_definitions: None,
        }
    }
}

/// Model category distinguishing block, item, and entity models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCategory {
    /// Block model (`models/block/*`).
    Block,
    /// Item model (`models/item/*`).
    Item,
    /// Entity geometry (`models/entity/*.geo.json`).
    Entity,
}

/// Cardinal rotation axis supported natively by Java Edition models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RotationAxis {
    /// X axis.
    X,
    /// Y axis.
    Y,
    /// Z axis.
    Z,
}

/// Cube face directions in Java/Bedrock models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FaceDirection {
    /// -Z in Java.
    North,
    /// +Z in Java.
    South,
    /// +X in Java.
    East,
    /// -X in Java.
    West,
    /// +Y.
    Up,
    /// -Y.
    Down,
}

impl FaceDirection {
    /// All six cube face directions in deterministic order.
    pub const ALL: [Self; 6] = [
        Self::North,
        Self::South,
        Self::East,
        Self::West,
        Self::Up,
        Self::Down,
    ];

    /// Canonical lowercase name of the face.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::North => "north",
            Self::South => "south",
            Self::East => "east",
            Self::West => "west",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// Rotation metadata on a model element/cube.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementRotation {
    /// Pivot point in Java `[0..16]` block coordinate space.
    pub origin: [f32; 3],
    /// Primary rotation axis.
    pub axis: RotationAxis,
    /// Rotation angle in degrees.
    pub angle: f32,
    /// Java `rescale` flag.
    pub rescale: bool,
    /// Preserved Bedrock multi-axis Euler rotation if the source cube rotated on multiple axes.
    pub extra_bedrock_euler: Option<[f32; 3]>,
}

/// UV mapping and texture binding for a single cube face.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementFace {
    /// `[u1, v1, u2, v2]` UV coordinates.
    pub uv: Option<[f32; 4]>,
    /// Texture variable (e.g. `"#all"`) or canonical texture path.
    pub texture: String,
    /// Optional cullface direction.
    pub cullface: Option<FaceDirection>,
    /// Texture UV rotation in degrees (`0`, `90`, `180`, `270`).
    pub rotation: u16,
    /// Optional tint index (`-1` / `0` etc.).
    pub tintindex: Option<i32>,
}

/// Single cuboid element in canonical Java `[0..16]` block coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelElement {
    /// Optional group/bone name (used when mapping to Bedrock `bones`).
    pub group: Option<String>,
    /// Minimum corner `[x, y, z]` in Java `[0..16]` space.
    pub from: [f32; 3],
    /// Maximum corner `[x, y, z]` in Java `[0..16]` space.
    pub to: [f32; 3],
    /// Optional element rotation.
    pub rotation: Option<ElementRotation>,
    /// Per-face UV and texture assignments.
    pub faces: BTreeMap<FaceDirection, ElementFace>,
    /// Whether directional face shading is enabled.
    pub shade: bool,
}

/// Display transform slot for Java item/block rendering contexts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayTransform {
    /// Euler rotation `[x, y, z]` in degrees.
    pub rotation: [f32; 3],
    /// Translation `[x, y, z]`.
    pub translation: [f32; 3],
    /// Scale `[x, y, z]`.
    pub scale: [f32; 3],
}

impl Default for DisplayTransform {
    fn default() -> Self {
        Self {
            rotation: [0.0, 0.0, 0.0],
            translation: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }
    }
}

/// Canonical 3D model / geometry asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelAsset {
    /// Canonical model ID without extension (e.g. `"block/custom_pillar"`).
    pub id: String,
    /// Model category.
    pub category: ModelCategory,
    /// Optional parent model identifier (e.g. `"minecraft:block/cube_all"`).
    pub parent: Option<String>,
    /// Texture variable map (`"all" -> "block/stone"`).
    pub textures: BTreeMap<String, String>,
    /// Texture resolution `[width, height]`, typically `[16, 16]` or `[64, 64]`.
    pub texture_size: [u32; 2],
    /// Ambient occlusion flag.
    pub ambient_occlusion: bool,
    /// Cuboid elements making up the model.
    pub elements: Vec<ModelElement>,
    /// Java display transforms keyed by context name (`"gui"`, `"ground"`, etc.).
    pub display: BTreeMap<String, DisplayTransform>,
    /// Whether Java `overrides` predicates were present (flagged as unsupported in Bedrock).
    pub has_overrides: bool,
    /// Number of Java item override predicates found.
    pub overrides_count: usize,
}

/// Raw audio file asset (`.ogg`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundAsset {
    /// Canonical relative sound path without `.ogg` extension (e.g. `"mob/zombie/say1"`).
    pub id: String,
    /// Raw `.ogg` bytes.
    pub data: Arc<Vec<u8>>,
}

/// Single sound variant reference inside a sound event definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundEntry {
    /// Canonical relative sound path without `.ogg` extension (e.g. `"mob/zombie/say1"`).
    pub name: String,
    /// Playback volume multiplier.
    pub volume: f32,
    /// Playback pitch multiplier.
    pub pitch: f32,
    /// Weighted random selection weight.
    pub weight: u32,
    /// Whether audio should be streamed from disk.
    pub stream: bool,
}

impl Default for SoundEntry {
    fn default() -> Self {
        Self {
            name: String::new(),
            volume: 1.0,
            pitch: 1.0,
            weight: 1,
            stream: false,
        }
    }
}

/// Sound event definition mapping an event ID to one or more `.ogg` sound variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SoundEventDef {
    /// Sound category (e.g. `"block"`, `"hostile"`, `"player"`).
    pub category: Option<String>,
    /// Optional subtitle key.
    pub subtitle: Option<String>,
    /// Whether vanilla sounds for this event should be replaced.
    pub replace: bool,
    /// Candidate sound variants.
    pub sounds: Vec<SoundEntry>,
}

/// Collection of sound event definitions and audio payloads.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SoundRegistry {
    /// Sound event map keyed by canonical event name (e.g. `"entity.zombie.ambient"`).
    pub events: BTreeMap<String, SoundEventDef>,
    /// Raw `.ogg` sound files keyed by canonical relative path without `.ogg`.
    pub files: BTreeMap<String, SoundAsset>,
}

/// Localization key-value table for a single language.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocaleTable {
    /// Translation entries (`key -> localized string`).
    pub entries: HashMap<String, String>,
    /// Preserved header/file comments when converting `.lang` files.
    pub comments: Vec<String>,
}

/// Canonical Intermediate Representation decoupling source parsing from target emission.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourcePackIR {
    /// Pack metadata (`pack.mcmeta` / `manifest.json`).
    pub metadata: PackMetadata,
    /// Textures keyed by canonical texture ID without `.png` (e.g. `"block/stone"`).
    pub textures: BTreeMap<String, TextureAsset>,
    /// 3D models keyed by canonical model ID without extension.
    pub models: BTreeMap<String, ModelAsset>,
    /// Sound events and `.ogg` audio files.
    pub sounds: SoundRegistry,
    /// Language tables keyed by lowercase locale code (e.g. `"en_us"`).
    pub locales: BTreeMap<String, LocaleTable>,
    /// Bedrock attachable bindings (`attachables/*.json`).
    pub attachables: BTreeMap<String, AttachableAsset>,
    /// Unmodified extra files (e.g. pack license/README) preserved as pass-through.
    pub passthrough_files: BTreeMap<String, Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn lazy_texture_payload_reads_dimensions_without_decoding() {
        let mut img = RgbaImage::new(256, 512);
        img.put_pixel(0, 0, Rgba([12, 34, 56, 255]));
        let mut png_bytes = Vec::new();
        img.write_to(&mut Cursor::new(&mut png_bytes), ImageFormat::Png)
            .expect("encode png");

        let payload = TexturePayload::from_raw(png_bytes.clone());
        assert!(!payload.is_decoded());
        assert_eq!(payload.dimensions().expect("dimensions"), (256, 512));
        assert!(!payload.is_decoded());

        let borrowed = payload.to_png_bytes().expect("to_png_bytes");
        assert!(matches!(borrowed, Cow::Borrowed(_)));
        assert_eq!(borrowed.as_ref(), png_bytes.as_slice());

        let decoded = payload.decode().expect("decode");
        assert_eq!(decoded.dimensions(), (256, 512));
    }
}
