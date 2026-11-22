use std::collections::BTreeMap;
use std::io::Cursor;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use fast_image_resize::images::Image;
use fast_image_resize::{PixelType, ResizeAlg, ResizeOptions, Resizer};
use glam::Vec3;
use image::{ImageFormat, Rgba, RgbaImage};
use rp_core::{ConversionWarning, PipelineStage, ResolutionTier, WarningSeverity};
use rp_io::PackContainer;
use rp_transforms::texture::inspect_png_metadata;
use serde::Serialize;

use crate::state::{ProjectSnapshot, StudioState};

/// Supported texture image file formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormatKind {
    Png,
    Tga,
}

impl TextureFormatKind {
    /// Infers texture format from path extension.
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

    /// Short uppercase label.
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

/// Result returned after modifying or compositing a texture in the active pack.
#[derive(Debug, Clone, Serialize)]
pub struct TextureTransformResult {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub base64_png: String,
    pub snapshot: ProjectSnapshot,
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

/// Resizes an RGBA8 texture using `fast_image_resize` with strict Nearest-Neighbor sampling.
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

/// Composites a Java `grass_block_side` + optional `grass_block_side_overlay` into a Bedrock-ready
/// 32-bit RGBA `grass_side.tga` texture.
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

/// Ensures `grass_top` texture pixels are fully opaque (`alpha = 255`) for Bedrock compatibility.
pub fn make_texture_opaque(img: &RgbaImage) -> RgbaImage {
    let mut out = img.clone();
    for px in out.pixels_mut() {
        px[3] = 255;
    }
    out
}

/// Scans texture headers in a `PackContainer` to return `(texture_count, dominant_resolution)`.
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

/// Checks 3D model JSON files in `container` for out-of-bounds UV coordinates (`< 0` or `> 16`).
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

/// Generates a blank transparent RGBA PNG buffer (`width` x `height`).
pub fn create_blank_png_bytes(width: u32, height: u32) -> Result<Vec<u8>> {
    let w = width.clamp(1, 2048);
    let h = height.clamp(1, 2048);
    let img = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));
    encode_texture_bytes(&img, TextureFormatKind::Png)
}

/// Tauri Command: Create a blank transparent PNG texture at `path` (`width` x `height`).
#[tauri::command]
pub fn create_blank_texture(
    state: tauri::State<'_, StudioState>,
    path: String,
    width: u32,
    height: u32,
) -> Result<ProjectSnapshot, String> {
    let bytes = create_blank_png_bytes(width, height).map_err(|e| format!("{e:#}"))?;
    let mut guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_mut()
        .ok_or_else(|| String::from("No resource pack currently open"))?;
    pack.write_asset(&path, bytes)
        .map_err(|e| format!("{e:#}"))?;
    let (tex_count, dom_res) = compute_dominant_resolution(&pack.container);
    Ok(pack.to_snapshot(dom_res, tex_count))
}

/// Tauri Command: Downscale a texture at `path` to `target_size` using `fast_image_resize` Nearest-Neighbor.
#[tauri::command]
pub fn downscale_texture(
    state: tauri::State<'_, StudioState>,
    path: String,
    target_size: u32,
) -> Result<TextureTransformResult, String> {
    let mut guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_mut()
        .ok_or_else(|| String::from("No resource pack currently open"))?;

    let raw = pack
        .container
        .get(&path)
        .ok_or_else(|| format!("Texture '{path}' not found in pack"))?;
    let (src_img, fmt) = decode_texture_bytes(raw, &path).map_err(|e| format!("{e:#}"))?;
    let (src_w, src_h) = src_img.dimensions();
    let dst_w = target_size.clamp(1, 2048);
    let dst_h = if src_h > src_w && src_w > 0 {
        ((u64::from(src_h) * u64::from(dst_w)) / u64::from(src_w)).max(1) as u32
    } else {
        dst_w
    };

    let resized = resize_nearest_neighbor(&src_img, dst_w, dst_h).map_err(|e| format!("{e:#}"))?;
    let encoded = encode_texture_bytes(&resized, fmt).map_err(|e| format!("{e:#}"))?;
    let png_preview = if fmt == TextureFormatKind::Png {
        encoded.clone()
    } else {
        encode_texture_bytes(&resized, TextureFormatKind::Png).map_err(|e| format!("{e:#}"))?
    };

    pack.write_asset(&path, encoded)
        .map_err(|e| format!("{e:#}"))?;
    let (tex_count, dom_res) = compute_dominant_resolution(&pack.container);
    let snapshot = pack.to_snapshot(dom_res, tex_count);

    Ok(TextureTransformResult {
        path,
        width: dst_w,
        height: dst_h,
        base64_png: BASE64_STANDARD.encode(&png_preview),
        snapshot,
    })
}

/// Tauri Command: Composite `side_path` + optional `overlay_path` into Bedrock `out_tga` (`grass_side.tga`).
#[tauri::command]
pub fn composite_grass_side(
    state: tauri::State<'_, StudioState>,
    side_path: String,
    overlay_path: Option<String>,
    out_tga: String,
) -> Result<TextureTransformResult, String> {
    let mut guard = state
        .active_pack
        .lock()
        .map_err(|_| String::from("active pack mutex poisoned"))?;
    let pack = guard
        .as_mut()
        .ok_or_else(|| String::from("No resource pack currently open"))?;

    let side_raw = pack
        .container
        .get(&side_path)
        .ok_or_else(|| format!("Base grass side texture '{side_path}' not found"))?;
    let (side_img, _) =
        decode_texture_bytes(side_raw, &side_path).map_err(|e| format!("{e:#}"))?;

    let overlay_img = match overlay_path.as_deref() {
        Some(ov_path) if !ov_path.trim().is_empty() => {
            if let Some(bytes) = pack.container.get(ov_path) {
                Some(
                    decode_texture_bytes(bytes, ov_path)
                        .map_err(|e| format!("{e:#}"))?
                        .0,
                )
            } else {
                None
            }
        }
        _ => {
            let candidates = [
                "assets/minecraft/textures/block/grass_block_side_overlay.png",
                "textures/blocks/grass_side_overlay.png",
            ];
            candidates.iter().find_map(|c| {
                pack.container
                    .get(c)
                    .and_then(|b| decode_texture_bytes(b, c).ok())
                    .map(|(img, _)| img)
            })
        }
    };

    let plains_tint = Vec3::new(0x91 as f32 / 255.0, 0xBD as f32 / 255.0, 0x59 as f32 / 255.0);
    let (composited_img, tga_bytes) =
        composite_grass_side_tga(&side_img, overlay_img.as_ref(), plains_tint, true)
            .map_err(|e| format!("{e:#}"))?;
    let png_preview =
        encode_texture_bytes(&composited_img, TextureFormatKind::Png).map_err(|e| format!("{e:#}"))?;

    let clean_out = if out_tga.trim().is_empty() {
        String::from("textures/blocks/grass_side.tga")
    } else {
        out_tga
    };
    pack.write_asset(&clean_out, tga_bytes)
        .map_err(|e| format!("{e:#}"))?;

    let (tex_count, dom_res) = compute_dominant_resolution(&pack.container);
    let snapshot = pack.to_snapshot(dom_res, tex_count);

    Ok(TextureTransformResult {
        path: clean_out,
        width: composited_img.width(),
        height: composited_img.height(),
        base64_png: BASE64_STANDARD.encode(&png_preview),
        snapshot,
    })
}
