use std::collections::BTreeMap;
use std::io::Cursor;

use anyhow::{Context, Result, bail};
use fast_image_resize::images::Image;
use fast_image_resize::{PixelType, ResizeAlg, ResizeOptions, Resizer};
use glam::Vec3;
use image::{ImageFormat, Rgba, RgbaImage};
use rp_core::{ConversionWarning, PipelineStage, ResolutionTier, WarningSeverity};
use rp_io::PackContainer;
use rp_transforms::texture::inspect_png_metadata;

/// Supported texture image file formats in the Artist Inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormatKind {
    Png,
    Tga,
}

impl TextureFormatKind {
    /// Infers texture format from a file path extension.
    #[must_use]
    pub fn from_path(path: &str) -> Option<Self> {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".png") {
            Some(Self::Png)
        } else if lower.ends_with(".tga") {
            Some(Self::Tga)
        } else {
            None
        }
    }

    /// Short uppercase label for display in the inspector toolbar.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Tga => "TGA",
        }
    }

    /// Corresponding `image::ImageFormat`.
    #[must_use]
    pub const fn image_format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Tga => ImageFormat::Tga,
        }
    }
}

/// Minecraft biome foliage / grass tint presets for previewing grayscale textures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BiomeTintPreset {
    #[default]
    Plains,
    Forest,
    Jungle,
    Swamp,
    Badlands,
    Taiga,
}

impl BiomeTintPreset {
    /// All selectable biome tint presets.
    pub const ALL: [Self; 6] = [
        Self::Plains,
        Self::Forest,
        Self::Jungle,
        Self::Swamp,
        Self::Badlands,
        Self::Taiga,
    ];

    /// Display name for UI dropdowns.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Plains => "Plains (#91BD59)",
            Self::Forest => "Forest (#79C05A)",
            Self::Jungle => "Jungle (#59C93C)",
            Self::Swamp => "Swamp (#6A7039)",
            Self::Badlands => "Badlands (#90814D)",
            Self::Taiga => "Taiga (#86B783)",
        }
    }

    /// Normalized `[0.0..=1.0]` RGB tint multiplier vector (`glam::Vec3`).
    #[must_use]
    pub const fn rgb_multiplier(self) -> Vec3 {
        match self {
            Self::Plains => Vec3::new(0x91 as f32 / 255.0, 0xBD as f32 / 255.0, 0x59 as f32 / 255.0),
            Self::Forest => Vec3::new(0x79 as f32 / 255.0, 0xC0 as f32 / 255.0, 0x5A as f32 / 255.0),
            Self::Jungle => Vec3::new(0x59 as f32 / 255.0, 0xC9 as f32 / 255.0, 0x3C as f32 / 255.0),
            Self::Swamp => Vec3::new(0x6A as f32 / 255.0, 0x70 as f32 / 255.0, 0x39 as f32 / 255.0),
            Self::Badlands => Vec3::new(0x90 as f32 / 255.0, 0x81 as f32 / 255.0, 0x4D as f32 / 255.0),
            Self::Taiga => Vec3::new(0x86 as f32 / 255.0, 0xB7 as f32 / 255.0, 0x83 as f32 / 255.0),
        }
    }
}

/// Summary metadata computed for a decoded texture in the Artist Inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureStats {
    pub width: u32,
    pub height: u32,
    pub format: TextureFormatKind,
    pub byte_len: usize,
    pub resolution_tier: ResolutionTier,
    pub has_transparency: bool,
    pub partial_alpha_pixels: usize,
}

impl TextureStats {
    /// Computes stats from an `RgbaImage`, its format, and raw byte length.
    #[must_use]
    pub fn from_image(img: &RgbaImage, format: TextureFormatKind, byte_len: usize) -> Self {
        let (width, height) = img.dimensions();
        let mut has_transparency = false;
        let mut partial_alpha_pixels = 0usize;
        for px in img.pixels() {
            let a = px[3];
            if a < 255 {
                has_transparency = true;
                if a > 0 {
                    partial_alpha_pixels += 1;
                }
            }
        }
        Self {
            width,
            height,
            format,
            byte_len,
            resolution_tier: ResolutionTier::from_dimensions(width, height),
            has_transparency,
            partial_alpha_pixels,
        }
    }

    /// Formats a concise info badge like `128x128 RGBA PNG`.
    #[must_use]
    pub fn format_badge(&self) -> String {
        format!(
            "{}x{} RGBA {} ({})",
            self.width,
            self.height,
            self.format.label(),
            self.resolution_tier
        )
    }
}

/// Decodes raw PNG or TGA bytes into an `RgbaImage`.
pub fn decode_texture_bytes(bytes: &[u8], path: &str) -> Result<(RgbaImage, TextureFormatKind)> {
    let format = TextureFormatKind::from_path(path).unwrap_or(TextureFormatKind::Png);
    let dyn_img = image::load_from_memory_with_format(bytes, format.image_format())
        .with_context(|| format!("failed to decode {} texture '{}'", format.label(), path))?;
    Ok((dyn_img.into_rgba8(), format))
}

/// Encodes an `RgbaImage` into raw PNG or TGA bytes.
pub fn encode_texture_bytes(img: &RgbaImage, format: TextureFormatKind) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), format.image_format())
        .with_context(|| format!("failed to encode {} texture", format.label()))?;
    Ok(buf)
}

/// Resizes an RGBA8 texture using `fast_image_resize` with strict Nearest-Neighbor sampling
/// so pixel art stays razor-sharp without interpolation blur.
pub fn resize_nearest_neighbor(
    src: &RgbaImage,
    dst_width: u32,
    dst_height: u32,
) -> Result<RgbaImage> {
    let (src_w, src_h) = src.dimensions();
    if src_w == 0 || src_h == 0 || dst_width == 0 || dst_height == 0 {
        bail!(
            "cannot resize texture with zero dimension ({src_w}x{src_h} -> {dst_width}x{dst_height})"
        );
    }

    let src_image = Image::from_vec_u8(src_w, src_h, src.as_raw().clone(), PixelType::U8x4)
        .context("failed to wrap source RGBA8 buffer for fast_image_resize")?;
    let mut dst_image = Image::new(dst_width, dst_height, PixelType::U8x4);

    let mut resizer = Resizer::new();
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Nearest);
    resizer
        .resize(&src_image, &mut dst_image, Some(&options))
        .context("fast_image_resize nearest-neighbor resize failed")?;

    RgbaImage::from_raw(dst_width, dst_height, dst_image.into_vec())
        .context("resized RGBA buffer length mismatch")
}

/// Applies Minecraft biome foliage/grass tinting (`glam::Vec3` RGB multiply) to a texture buffer,
/// preserving the original alpha channel verbatim.
#[must_use]
pub fn apply_biome_tint(src: &RgbaImage, tint_rgb: Vec3) -> RgbaImage {
    let (w, h) = src.dimensions();
    let mut out = RgbaImage::new(w, h);
    let clamped_tint = tint_rgb.clamp(Vec3::ZERO, Vec3::ONE);
    for (x, y, px) in src.enumerate_pixels() {
        let rgb = Vec3::new(f32::from(px[0]), f32::from(px[1]), f32::from(px[2])) * clamped_tint;
        let r = rgb.x.round().clamp(0.0, 255.0) as u8;
        let g = rgb.y.round().clamp(0.0, 255.0) as u8;
        let b = rgb.z.round().clamp(0.0, 255.0) as u8;
        out.put_pixel(x, y, Rgba([r, g, b, px[3]]));
    }
    out
}

/// Generates an Alpha Channel / Cutout Mask visualization buffer:
/// - Opaque pixels (`a == 255`): bright white (`#F2F2F2`)
/// - Fully transparent pixels (`a == 0`): deep charcoal (`#141414`)
/// - Semi-transparent pixels (`1..=254`): highlighted magenta/cyan depending on `cutout_threshold`
///   so artists immediately spot stray soft edges that break Minecraft `cutout` blocks/items.
#[must_use]
pub fn build_alpha_mask_preview(src: &RgbaImage, cutout_threshold: u8) -> RgbaImage {
    let (w, h) = src.dimensions();
    let mut out = RgbaImage::new(w, h);
    for (x, y, px) in src.enumerate_pixels() {
        let a = px[3];
        let color = if a == 255 {
            Rgba([240, 240, 240, 255])
        } else if a == 0 {
            Rgba([20, 20, 20, 255])
        } else if a >= cutout_threshold {
            // Kept by cutout, but semi-transparent (cyan highlight)
            Rgba([64, 210, 235, 255])
        } else {
            // Discarded by cutout, stray faint alpha (magenta warning)
            Rgba([235, 64, 180, 255])
        };
        out.put_pixel(x, y, color);
    }
    out
}

/// Composites a Java `grass_block_side` + optional `grass_block_side_overlay` into a Bedrock-ready
/// 32-bit RGBA `grass_side.tga` texture.
///
/// In Bedrock Edition `textures/blocks/grass_side.tga`:
/// - Dirt pixels remain opaque (`alpha = 255`).
/// - Grass blade overlay pixels are composited with optional biome tinting and stored with
///   `alpha = 0` when `bedrock_tint_mask` is true (Bedrock's native shader tint mask), or
///   pre-tinted opaque pixels when `bedrock_tint_mask` is false.
pub fn composite_grass_side_tga(
    base_side: &RgbaImage,
    overlay: Option<&RgbaImage>,
    biome_tint: Vec3,
    bedrock_tint_mask: bool,
) -> Result<(RgbaImage, Vec<u8>)> {
    let (w, h) = base_side.dimensions();
    if w == 0 || h == 0 {
        bail!("cannot composite empty grass_side image");
    }

    let resized_overlay = match overlay {
        Some(ov) if ov.dimensions() != (w, h) => Some(resize_nearest_neighbor(ov, w, h)?),
        Some(ov) => Some(ov.clone()),
        None => None,
    };

    let mut out = base_side.clone();
    let clamped_tint = biome_tint.clamp(Vec3::ZERO, Vec3::ONE);

    if let Some(ov) = resized_overlay {
        for (x, y, ov_px) in ov.enumerate_pixels() {
            if ov_px[3] >= 128 {
                if bedrock_tint_mask {
                    // Bedrock grass_side.tga convention: grayscale/tinted blade RGB with alpha = 0
                    // signals the Bedrock terrain shader to apply biome grass color.
                    out.put_pixel(x, y, Rgba([ov_px[0], ov_px[1], ov_px[2], 0]));
                } else {
                    let tinted = Vec3::new(
                        f32::from(ov_px[0]),
                        f32::from(ov_px[1]),
                        f32::from(ov_px[2]),
                    ) * clamped_tint;
                    out.put_pixel(
                        x,
                        y,
                        Rgba([
                            tinted.x.round().clamp(0.0, 255.0) as u8,
                            tinted.y.round().clamp(0.0, 255.0) as u8,
                            tinted.z.round().clamp(0.0, 255.0) as u8,
                            255,
                        ]),
                    );
                }
            }
        }
    } else {
        // Fallback when only grass_side is available without a separate overlay:
        // tint top 25% green-ish pixels or mark top grass band.
        let band_h = (h / 4).max(1);
        for y in 0..band_h {
            for x in 0..w {
                let px = out.get_pixel(x, y);
                let is_greenish = px[1] >= px[0] && px[1] >= px[2];
                if is_greenish {
                    let tinted =
                        Vec3::new(f32::from(px[0]), f32::from(px[1]), f32::from(px[2]))
                            * clamped_tint;
                    let alpha = if bedrock_tint_mask { 0 } else { 255 };
                    out.put_pixel(
                        x,
                        y,
                        Rgba([
                            tinted.x.round().clamp(0.0, 255.0) as u8,
                            tinted.y.round().clamp(0.0, 255.0) as u8,
                            tinted.z.round().clamp(0.0, 255.0) as u8,
                            alpha,
                        ]),
                    );
                }
            }
        }
    }

    let tga_bytes = encode_texture_bytes(&out, TextureFormatKind::Tga)?;
    Ok((out, tga_bytes))
}

/// Scans texture headers in a `PackContainer` to return `(texture_count, dominant_resolution)`.
/// Uses zero-decode PNG header inspection where possible so 50 MB+ HD packs remain instant.
#[must_use]
pub fn compute_dominant_resolution(container: &PackContainer) -> (usize, Option<ResolutionTier>) {
    let mut counts: BTreeMap<u32, (ResolutionTier, usize)> = BTreeMap::new();
    let mut texture_count = 0usize;

    for (path, bytes) in container.files() {
        let lower = path.to_ascii_lowercase();
        if lower == "pack.png" || lower == "pack_icon.png" {
            continue;
        }
        let tier = if lower.ends_with(".png") {
            texture_count += 1;
            inspect_png_metadata(bytes).ok().map(|(_, _, t)| t)
        } else if lower.ends_with(".tga") {
            texture_count += 1;
            decode_texture_bytes(bytes, path)
                .ok()
                .map(|(img, _)| ResolutionTier::from_dimensions(img.width(), img.height()))
        } else {
            None
        };

        if let Some(t) = tier {
            let key = tier_sort_key(t);
            let entry = counts.entry(key).or_insert((t, 0));
            entry.1 += 1;
        }
    }

    let dominant = counts
        .into_values()
        .max_by_key(|(tier, count)| (*count, tier_sort_key(*tier)))
        .map(|(tier, _)| tier);

    (texture_count, dominant)
}

const fn tier_sort_key(tier: ResolutionTier) -> u32 {
    match tier {
        ResolutionTier::R16 => 16,
        ResolutionTier::R32 => 32,
        ResolutionTier::R64 => 64,
        ResolutionTier::R128 => 128,
        ResolutionTier::R256 => 256,
        ResolutionTier::R512 => 512,
        ResolutionTier::R1024 => 1024,
        ResolutionTier::R2048 => 2048,
        ResolutionTier::Custom(n) => n,
    }
}

/// Checks 3D model JSON files in `container` for out-of-bounds UV coordinates (`< 0` or `> 16`)
/// and non-power-of-two atlas risks when the **3D UV-Atlas Guard** is enabled.
#[must_use]
pub fn check_uv_atlas_guard(container: &PackContainer) -> Vec<ConversionWarning> {
    let mut warnings = Vec::new();
    for (path, bytes) in container.files() {
        let lower = path.to_ascii_lowercase();
        if !lower.ends_with(".json") || !lower.contains("models/") {
            continue;
        }
        let Ok(val) = rp_core::parse_minecraft_json::<serde_json::Value>(bytes) else {
            continue;
        };
        if let Some(elements) = val.get("elements").and_then(|v| v.as_array()) {
            let mut out_of_bounds_uvs = 0usize;
            for el in elements {
                if let Some(faces) = el.get("faces").and_then(|v| v.as_object()) {
                    for face in faces.values() {
                        if let Some(uv) = face.get("uv").and_then(|v| v.as_array()) {
                            let bad = uv.iter().filter_map(serde_json::Value::as_f64).any(|c| {
                                !c.is_finite() || !(-0.001..=16.001).contains(&c)
                            });
                            if bad {
                                out_of_bounds_uvs += 1;
                            }
                        }
                    }
                }
            }
            if out_of_bounds_uvs > 0 {
                warnings.push(ConversionWarning {
                    source_path: path.clone(),
                    stage: PipelineStage::Resolve,
                    severity: WarningSeverity::Warning,
                    reason: format!(
                        "3D UV-Atlas Guard: {out_of_bounds_uvs} face(s) have UV coordinates outside [0..16], which may bleed across atlas tiles"
                    ),
                    actionable_fix: String::from(
                        "Clamp model face UVs inside [0, 16] in Blockbench or verify texture_size matches the atlas tile.",
                    ),
                    asset: path.clone(),
                    message: format!(
                        "3D UV-Atlas Guard: {out_of_bounds_uvs} face(s) have UV coordinates outside [0..16]"
                    ),
                });
            }
        }
    }
    warnings
}
