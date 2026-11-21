use std::collections::BTreeMap;
use std::io::Cursor;

use image::{ImageFormat, ImageReader, RgbaImage, imageops};
use serde::{Deserialize, Serialize};

use rp_core::{
    AnimationFrame, AnimationSpec, ConversionReport, CoreError, MappingTable, TextureAsset,
    TextureCategory, WarningSeverity,
};

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Bedrock `textures/flipbook_textures.json` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlipbookEntry {
    /// Relative texture path without `.png` (e.g. `"textures/blocks/water_still"`).
    pub flipbook_texture: String,
    /// Atlas tile name registered in `terrain_texture.json`.
    pub atlas_tile: String,
    /// Game ticks per frame (`1 tick = 0.05s`).
    #[serde(default = "default_ticks_per_frame")]
    pub ticks_per_frame: u32,
    /// Explicit playback frame indices.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<Vec<u32>>,
    /// Whether to smoothly blend between adjacent frames.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blend_frames: Option<bool>,
    /// Optional frame replication factor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replicate: Option<u32>,
}

fn default_ticks_per_frame() -> u32 {
    1
}

/// Bedrock `terrain_texture.json` or `item_texture.json` texture reference entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtlasTextureData {
    /// Texture path(s) relative to pack root without `.png`.
    pub textures: AtlasTexturePath,
}

/// Single path or weighted variants in Bedrock texture atlas JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AtlasTexturePath {
    /// Single texture path string.
    Single(String),
    /// Multiple texture variants.
    List(Vec<AtlasVariant>),
}

/// Either a plain string or object variant inside `textures` array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AtlasVariant {
    /// Plain texture path.
    Path(String),
    /// Object with `path` field.
    Object {
        /// Relative texture path.
        path: String,
    },
}

/// Bedrock `textures/terrain_texture.json` schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainTextureJson {
    /// Resource pack identifier.
    #[serde(default)]
    pub resource_pack_name: String,
    /// Atlas name (`"atlas.terrain"`).
    #[serde(default)]
    pub texture_name: String,
    /// Optional atlas padding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<u32>,
    /// Optional mip levels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_mip_levels: Option<u32>,
    /// Deterministic map of tile key -> texture reference.
    pub texture_data: BTreeMap<String, AtlasTextureData>,
}

/// Bedrock `textures/item_texture.json` schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemTextureJson {
    /// Resource pack identifier.
    #[serde(default)]
    pub resource_pack_name: String,
    /// Atlas name (`"atlas.items"`).
    #[serde(default)]
    pub texture_name: String,
    /// Deterministic map of item key -> texture reference.
    pub texture_data: BTreeMap<String, AtlasTextureData>,
}

/// Reads PNG dimensions `(width, height)` from header bytes without decoding pixel buffers.
pub fn inspect_png_dimensions(bytes: &[u8]) -> Result<(u32, u32), CoreError> {
    if bytes.len() < PNG_SIGNATURE.len() || bytes[..8] != PNG_SIGNATURE {
        return Err(CoreError::InvalidFormat(String::from(
            "invalid PNG header signature",
        )));
    }
    let reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let (width, height) = reader.into_dimensions()?;
    if width == 0 || height == 0 {
        return Err(CoreError::InvalidDimensions(format!(
            "PNG has zero dimension ({width}x{height})"
        )));
    }
    Ok((width, height))
}

/// Computes `(frame_width, frame_height, frame_count)` for a vertical animation strip.
pub fn compute_animation_strip_geometry(
    width: u32,
    height: u32,
    spec: &AnimationSpec,
) -> Result<(u32, u32, u32), CoreError> {
    if width == 0 || height == 0 {
        return Err(CoreError::InvalidDimensions(format!(
            "animation texture has zero dimensions ({width}x{height})"
        )));
    }
    let frame_w = spec.frame_width.unwrap_or(width);
    let frame_h = spec.frame_height.unwrap_or(frame_w);
    if frame_w == 0 || frame_h == 0 {
        return Err(CoreError::InvalidDimensions(format!(
            "animation frame dimensions must be positive, got {frame_w}x{frame_h}"
        )));
    }
    if !width.is_multiple_of(frame_w) {
        return Err(CoreError::InvalidDimensions(format!(
            "texture width {width} is not divisible by frame width {frame_w}"
        )));
    }
    if !height.is_multiple_of(frame_h) {
        return Err(CoreError::InvalidDimensions(format!(
            "texture height {height} is not divisible by frame height {frame_h}"
        )));
    }
    let frame_count = height / frame_h;
    Ok((frame_w, frame_h, frame_count))
}

/// Splits a vertical sprite strip (`Wx(N*H)`) into `N` individual frame images.
pub fn split_vertical_strip(
    img: &RgbaImage,
    frame_height: u32,
) -> Result<Vec<RgbaImage>, CoreError> {
    let (width, height) = img.dimensions();
    if frame_height == 0 || width == 0 || height == 0 {
        return Err(CoreError::InvalidDimensions(String::from(
            "frame_height and image dimensions must be positive",
        )));
    }
    if !height.is_multiple_of(frame_height) {
        return Err(CoreError::InvalidDimensions(format!(
            "image height {height} is not a multiple of frame_height {frame_height}"
        )));
    }
    let count = height / frame_height;
    let mut frames = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    for idx in 0..count {
        let y = idx.saturating_mul(frame_height);
        let cropped = imageops::crop_imm(img, 0, y, width, frame_height).to_image();
        frames.push(cropped);
    }
    Ok(frames)
}

/// Stitches uniform frame images into a vertical animation strip (`Wx(N*H)`).
pub fn stitch_vertical_strip(frames: &[RgbaImage]) -> Result<RgbaImage, CoreError> {
    let Some(first) = frames.first() else {
        return Err(CoreError::InvalidDimensions(String::from(
            "cannot stitch empty frame list",
        )));
    };
    let (width, frame_h) = first.dimensions();
    if width == 0 || frame_h == 0 {
        return Err(CoreError::InvalidDimensions(String::from(
            "frame dimensions must be positive",
        )));
    }
    for (idx, frame) in frames.iter().enumerate() {
        if frame.dimensions() != (width, frame_h) {
            return Err(CoreError::InvalidDimensions(format!(
                "frame {idx} dimensions {:?} do not match expected {:?}",
                frame.dimensions(),
                (width, frame_h)
            )));
        }
    }

    let total_h = frame_h
        .checked_mul(u32::try_from(frames.len()).map_err(|_| {
            CoreError::InvalidDimensions(String::from("too many frames in animation strip"))
        })?)
        .ok_or_else(|| {
            CoreError::InvalidDimensions(String::from("animation strip height overflow"))
        })?;

    let mut out = RgbaImage::new(width, total_h);
    for (idx, frame) in frames.iter().enumerate() {
        let y = u32::try_from(idx).unwrap_or(0).saturating_mul(frame_h);
        imageops::replace(&mut out, frame, 0, i64::from(y));
    }
    Ok(out)
}

/// Converts a Java `.png.mcmeta` `AnimationSpec` into a Bedrock `FlipbookEntry`.
///
/// Java `frametime` and Bedrock `ticks_per_frame` both express duration in Minecraft game ticks
/// (`1 tick = 0.05s`). Per-frame Java duration overrides are expanded via GCD when non-uniform.
pub fn java_animation_to_flipbook(
    atlas_tile: &str,
    bedrock_texture_path: &str,
    spec: &AnimationSpec,
    report: &mut ConversionReport,
) -> FlipbookEntry {
    let base_time = spec.frametime.max(1);

    if let (Some(w), Some(h)) = (spec.frame_width, spec.frame_height)
        && w != h
    {
        report.record_warning(
            bedrock_texture_path,
            WarningSeverity::Warning,
            format!(
                "Non-square animation frame size ({w}x{h}) may render unexpectedly in Bedrock flipbooks"
            ),
        );
    }

    let frames = if spec.frames.is_empty() {
        (base_time, None)
    } else {
        let has_custom_times = spec
            .frames
            .iter()
            .any(|f| f.time.is_some_and(|t| t.max(1) != base_time));

        if !has_custom_times {
            (
                base_time,
                Some(spec.frames.iter().map(|f| f.index).collect()),
            )
        } else {
            let mut common_gcd = 0u32;
            for frame in &spec.frames {
                let t = frame.time.unwrap_or(base_time).max(1);
                common_gcd = gcd(common_gcd, t);
            }
            let tick_unit = common_gcd.max(1);
            let mut expanded = Vec::new();
            for frame in &spec.frames {
                let t = frame.time.unwrap_or(base_time).max(1);
                let repeat = (t / tick_unit).max(1);
                for _ in 0..repeat {
                    expanded.push(frame.index);
                }
            }
            report.record_warning(
                bedrock_texture_path,
                WarningSeverity::Info,
                "Expanded per-frame Java animation timings using frame index repetition",
            );
            (tick_unit, Some(expanded))
        }
    };

    FlipbookEntry {
        flipbook_texture: bedrock_texture_path.to_string(),
        atlas_tile: atlas_tile.to_string(),
        ticks_per_frame: frames.0,
        frames: frames.1,
        blend_frames: if spec.interpolate { Some(true) } else { None },
        replicate: None,
    }
}

/// Converts a Bedrock `FlipbookEntry` back into a canonical `AnimationSpec`.
#[must_use]
pub fn flipbook_to_java_animation(entry: &FlipbookEntry) -> AnimationSpec {
    let frames = entry
        .frames
        .as_ref()
        .map(|indices| {
            indices
                .iter()
                .map(|&index| AnimationFrame { index, time: None })
                .collect()
        })
        .unwrap_or_default();

    AnimationSpec {
        frametime: entry.ticks_per_frame.max(1),
        interpolate: entry.blend_frames.unwrap_or(false),
        frames,
        frame_width: None,
        frame_height: None,
    }
}

/// Generates Bedrock `terrain_texture.json` index from canonical textures.
#[must_use]
pub fn build_terrain_texture_json(
    pack_name: &str,
    textures: &BTreeMap<String, TextureAsset>,
    mappings: &MappingTable,
) -> TerrainTextureJson {
    let mut texture_data = BTreeMap::new();

    for (id, tex) in textures {
        if tex.category == TextureCategory::Block {
            let target = mappings.java_texture_to_bedrock(id);
            let atlas_key = tex.atlas_key.clone().unwrap_or(target.atlas_key);
            texture_data.insert(
                atlas_key,
                AtlasTextureData {
                    textures: AtlasTexturePath::Single(target.path),
                },
            );
        }
    }

    TerrainTextureJson {
        resource_pack_name: pack_name.to_string(),
        texture_name: String::from("atlas.terrain"),
        padding: Some(8),
        num_mip_levels: Some(4),
        texture_data,
    }
}

/// Generates Bedrock `item_texture.json` index from canonical textures.
#[must_use]
pub fn build_item_texture_json(
    pack_name: &str,
    textures: &BTreeMap<String, TextureAsset>,
    mappings: &MappingTable,
) -> ItemTextureJson {
    let mut texture_data = BTreeMap::new();

    for (id, tex) in textures {
        if tex.category == TextureCategory::Item {
            let target = mappings.java_texture_to_bedrock(id);
            let atlas_key = tex.atlas_key.clone().unwrap_or(target.atlas_key);
            texture_data.insert(
                atlas_key,
                AtlasTextureData {
                    textures: AtlasTexturePath::Single(target.path),
                },
            );
        }
    }

    ItemTextureJson {
        resource_pack_name: pack_name.to_string(),
        texture_name: String::from("atlas.items"),
        texture_data,
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn strip_split_stitch_and_dimension_math() {
        let mut f0 = RgbaImage::new(16, 16);
        let mut f1 = RgbaImage::new(16, 16);
        f0.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        f1.put_pixel(0, 0, Rgba([0, 255, 0, 255]));

        let strip = stitch_vertical_strip(&[f0.clone(), f1.clone()]).expect("stitch");
        assert_eq!(strip.dimensions(), (16, 32));

        let spec = AnimationSpec {
            frametime: 4,
            interpolate: true,
            frames: vec![
                AnimationFrame {
                    index: 0,
                    time: None,
                },
                AnimationFrame {
                    index: 1,
                    time: None,
                },
            ],
            frame_width: None,
            frame_height: None,
        };
        let (fw, fh, count) = compute_animation_strip_geometry(16, 32, &spec).expect("strip geom");
        assert_eq!((fw, fh, count), (16, 16, 2));

        let split = split_vertical_strip(&strip, fh).expect("split");
        assert_eq!(split.len(), 2);
        assert_eq!(split[0], f0);
        assert_eq!(split[1], f1);
    }

    #[test]
    fn animation_mcmeta_to_flipbook_and_back() {
        let spec = AnimationSpec {
            frametime: 5,
            interpolate: true,
            frames: vec![
                AnimationFrame {
                    index: 0,
                    time: None,
                },
                AnimationFrame {
                    index: 2,
                    time: None,
                },
                AnimationFrame {
                    index: 1,
                    time: None,
                },
            ],
            frame_width: None,
            frame_height: None,
        };
        let mut report = ConversionReport::new();
        let entry = java_animation_to_flipbook(
            "water_still",
            "textures/blocks/water_still",
            &spec,
            &mut report,
        );
        assert_eq!(entry.ticks_per_frame, 5);
        assert_eq!(entry.frames, Some(vec![0, 2, 1]));
        assert_eq!(entry.blend_frames, Some(true));

        let back = flipbook_to_java_animation(&entry);
        assert_eq!(back, spec);
    }

    #[test]
    fn expands_non_uniform_frame_timings_via_gcd() {
        let spec = AnimationSpec {
            frametime: 2,
            interpolate: false,
            frames: vec![
                AnimationFrame {
                    index: 0,
                    time: Some(4),
                },
                AnimationFrame {
                    index: 1,
                    time: Some(2),
                },
            ],
            frame_width: None,
            frame_height: None,
        };
        let mut report = ConversionReport::new();
        let entry = java_animation_to_flipbook(
            "custom_anim",
            "textures/blocks/custom_anim",
            &spec,
            &mut report,
        );
        assert_eq!(entry.ticks_per_frame, 2);
        assert_eq!(entry.frames, Some(vec![0, 0, 1]));
    }

    #[test]
    fn header_dimension_inspection_rejects_corrupt_png() {
        assert!(inspect_png_dimensions(b"not a png").is_err());
        let mut img = RgbaImage::new(32, 64);
        img.put_pixel(1, 1, Rgba([10, 20, 30, 255]));
        let mut buf = Vec::new();
        img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .expect("encode png");
        assert_eq!(inspect_png_dimensions(&buf).expect("inspect"), (32, 64));
    }
}
