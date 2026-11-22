use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};

use image::{ImageFormat, ImageReader, Rgba, RgbaImage, imageops};
use serde::{Deserialize, Serialize};

use rp_core::{
    AnimationFrame, AnimationSpec, ConversionReport, CoreError, FaceDirection, MappingTable,
    ModelAsset, ModelElement, ResolutionTier, RotationAxis, TextureAsset, TextureCategory,
    TexturePayload, WarningSeverity,
};

use crate::model::{auto_java_uv_for_face, euler_zyx_to_mat3};

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Maximum number of concurrent high-resolution (`1024x`/`2048x`) RGBA pixel decodes
/// permitted at once to keep peak memory bounded on ARM64 / mobile / proot environments.
pub const MAX_CONCURRENT_HIGH_RES_DECODES: usize = 2;

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

/// Reads PNG header dimensions and classifies the `ResolutionTier` without decoding pixel buffers.
pub fn inspect_png_metadata(bytes: &[u8]) -> Result<(u32, u32, ResolutionTier), CoreError> {
    let (w, h) = inspect_png_dimensions(bytes)?;
    Ok((w, h, ResolutionTier::from_dimensions(w, h)))
}

/// Resolved animation strip dimensions after applying inference and non-fatal fallbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedAnimationGeometry {
    /// Effective width of a single frame in pixels.
    pub frame_width: u32,
    /// Effective height of a single frame in pixels.
    pub frame_height: u32,
    /// Number of complete frames along the vertical strip.
    pub frame_count: u32,
    /// `true` when the texture height was not divisible by the requested/inferred frame height
    /// and the parser safely degraded the texture to a static single frame (`1` frame).
    pub fell_back_to_static: bool,
}

/// Non-fatally resolves animation strip geometry for `(width, height)` and `spec`.
///
/// Unlike [`compute_animation_strip_geometry`], dimension mismatches (such as a `40x224` strip
/// with inferred square `frame_height = 40`) never return a pack-aborting error: instead, they
/// record structured `Warning` and `Lossy` entries in `report` and fall back safely to a static
/// single-frame geometry (`frame_width = width, frame_height = height, frame_count = 1`).
pub fn resolve_animation_strip_geometry(
    asset_id: &str,
    width: u32,
    height: u32,
    spec: &AnimationSpec,
    report: &mut ConversionReport,
) -> Option<ResolvedAnimationGeometry> {
    if width == 0 || height == 0 {
        let msg = format!(
            "Animation texture has zero dimensions ({width}x{height}); skipping animation metadata"
        );
        report.record_warning(asset_id, WarningSeverity::Warning, msg.clone());
        report.record_lossy(asset_id, msg);
        return None;
    }

    let frame_w = match spec.frame_width {
        Some(fw) if fw > 0 && width.is_multiple_of(fw) => fw,
        Some(fw) => {
            let msg = format!(
                "Animation frame width {fw} does not divide texture width {width}; using full texture width {width}"
            );
            report.record_warning(asset_id, WarningSeverity::Warning, msg.clone());
            report.record_lossy(asset_id, msg);
            width
        }
        None => width,
    };

    // 1. Explicit positive frame_height that evenly divides texture height.
    if let Some(fh) = spec.frame_height
        && fh > 0
        && height.is_multiple_of(fh)
    {
        return Some(ResolvedAnimationGeometry {
            frame_width: frame_w,
            frame_height: fh,
            frame_count: height / fh,
            fell_back_to_static: false,
        });
    }

    // 2. No explicit frame_height: standard square frames (`frame_h == frame_w`).
    if spec.frame_height.is_none() && height.is_multiple_of(frame_w) {
        return Some(ResolvedAnimationGeometry {
            frame_width: frame_w,
            frame_height: frame_w,
            frame_count: height / frame_w,
            fell_back_to_static: false,
        });
    }

    // 3. No explicit frame_height, but explicit frame indices let us infer non-square frame count.
    if spec.frame_height.is_none() && !spec.frames.is_empty() {
        let max_idx = spec.frames.iter().map(|f| f.index).max().unwrap_or(0);
        let inferred_count = max_idx.saturating_add(1);
        if inferred_count > 0 && height.is_multiple_of(inferred_count) {
            let inferred_h = height / inferred_count;
            if inferred_h > 0 {
                report.record_warning(
                    asset_id,
                    WarningSeverity::Info,
                    format!(
                        "Inferred non-square animation frame height {inferred_h} ({inferred_count} frames) for {width}x{height} texture"
                    ),
                );
                return Some(ResolvedAnimationGeometry {
                    frame_width: frame_w,
                    frame_height: inferred_h,
                    frame_count: inferred_count,
                    fell_back_to_static: false,
                });
            }
        }
    }

    // 4. Non-divisible mismatch (e.g., 40x224 with frame_height 40): never fail the whole pack.
    let requested_h = spec.frame_height.unwrap_or(frame_w);
    let msg = format!(
        "Texture dimensions {width}x{height} are not divisible by animation frame height {requested_h}; falling back to static single-frame texture ({width}x{height})"
    );
    report.record_warning(asset_id, WarningSeverity::Warning, msg.clone());
    report.record_lossy(asset_id, msg);
    Some(ResolvedAnimationGeometry {
        frame_width: width,
        frame_height: height,
        frame_count: 1,
        fell_back_to_static: true,
    })
}

/// Executes pixel-processing work across `items` with bounded concurrency
/// (`1..=MAX_CONCURRENT_HIGH_RES_DECODES`) to cap peak decoded RGBA memory for `1024x`/`2048x` packs.
pub fn map_textures_bounded<T, F>(
    items: &[&TexturePayload],
    max_concurrency: usize,
    work: F,
) -> Result<Vec<T>, CoreError>
where
    T: Send,
    F: Fn(&TexturePayload) -> Result<T, CoreError> + Sync,
{
    let workers = max_concurrency.clamp(1, MAX_CONCURRENT_HIGH_RES_DECODES);
    if items.len() <= 1 || workers == 1 {
        return items.iter().map(|payload| work(payload)).collect();
    }

    let worker_count = workers.min(items.len());
    let next_index = AtomicUsize::new(0);
    let mut indexed_results: Vec<(usize, T)> = Vec::with_capacity(items.len());

    std::thread::scope(|scope| -> Result<(), CoreError> {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            handles.push(scope.spawn(|| -> Result<Vec<(usize, T)>, CoreError> {
                let mut local = Vec::new();
                loop {
                    let idx = next_index.fetch_add(1, Ordering::Relaxed);
                    let Some(payload) = items.get(idx) else {
                        break;
                    };
                    let value = work(payload)?;
                    local.push((idx, value));
                }
                Ok(local)
            }));
        }

        for handle in handles {
            let chunk = handle.join().map_err(|_| {
                CoreError::InvalidFormat(String::from("bounded texture worker thread panicked"))
            })??;
            indexed_results.extend(chunk);
        }
        Ok(())
    })?;

    indexed_results.sort_by_key(|(idx, _)| *idx);
    Ok(indexed_results.into_iter().map(|(_, val)| val).collect())
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
            if spec.interpolate {
                // Java blends frame i -> i+1 over the whole frame duration t. After repetition
                // the blend only spans the last `tick_unit` ticks of that duration.
                report.record_lossy(
                    bedrock_texture_path,
                    "Interpolated animation with per-frame timings: blend window shrinks from the frame duration to one tick after repetition",
                );
            }
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

/// Renders a 3D-rotation-aware orthographic 128x64 humanoid helmet armor sheet PNG
/// from a 3D `ModelAsset` and its texture (matching Minecraft's 2× scaled 64×32 head Box UV:
/// East `(0,16)`, North `(16,16)`, West `(32,16)`, South `(48,16)`, Up `(16,0)`, Down `(32,0)`).
pub fn render_model_to_humanoid_helmet_png(
    model: &ModelAsset,
    tex_payload: &TexturePayload,
) -> Result<Vec<u8>, CoreError> {
    let decoded = tex_payload.decode()?;
    let (src_w, src_h) = decoded.dimensions();
    let mut out = RgbaImage::new(128, 64);

    if model.elements.is_empty() || src_w == 0 || src_h == 0 {
        let mut buf = Vec::new();
        out.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)?;
        return Ok(buf);
    }

    // Collect all transformed 3D face quads with their rotated normals and UV corners.
    struct WorldQuad {
        verts: [[f32; 3]; 4],
        normal: [f32; 3],
        uvs: [[f32; 2]; 4],
    }

    let mut quads: Vec<WorldQuad> = Vec::new();
    let mut min_c = [4.0_f32, 4.0, 4.0];
    let mut max_c = [12.0_f32, 12.0, 12.0];

    for el in &model.elements {
        let (origin, rot_mat) = element_rotation_matrix(el);
        for (&dir, face) in &el.faces {
            let local_verts = face_local_quad_verts(dir, el.from, el.to);
            let local_normal = face_local_normal(dir);
            let mut world_verts = [[0.0_f32; 3]; 4];
            for (idx, v) in local_verts.iter().enumerate() {
                let w = apply_rot_around(rot_mat, origin, *v);
                for axis in 0..3 {
                    min_c[axis] = min_c[axis].min(w[axis]);
                    max_c[axis] = max_c[axis].max(w[axis]);
                }
                world_verts[idx] = w;
            }
            let world_normal = apply_rot_vec(rot_mat, local_normal);
            let uv_rect = face
                .uv
                .unwrap_or_else(|| auto_java_uv_for_face(dir, el.from, el.to));
            let uvs = rotated_uv_corners(uv_rect, face.rotation);
            quads.push(WorldQuad {
                verts: world_verts,
                normal: world_normal,
                uvs,
            });
        }
    }

    let span_x = (max_c[0] - min_c[0]).max(0.001);
    let span_y = (max_c[1] - min_c[1]).max(0.001);
    let span_z = (max_c[2] - min_c[2]).max(0.001);

    let views: [(FaceDirection, u32, u32, [f32; 3]); 6] = [
        (FaceDirection::East, 0, 16, [1.0, 0.0, 0.0]),
        (FaceDirection::North, 16, 16, [0.0, 0.0, -1.0]),
        (FaceDirection::West, 32, 16, [-1.0, 0.0, 0.0]),
        (FaceDirection::South, 48, 16, [0.0, 0.0, 1.0]),
        (FaceDirection::Up, 16, 0, [0.0, 1.0, 0.0]),
        (FaceDirection::Down, 32, 0, [0.0, -1.0, 0.0]),
    ];

    for (view_dir, ox, oy, view_norm) in views {
        let mut zbuf = [f32::NEG_INFINITY; 16 * 16];
        for quad in &quads {
            let dot = quad.normal[0] * view_norm[0]
                + quad.normal[1] * view_norm[1]
                + quad.normal[2] * view_norm[2];
            if dot <= 0.05 {
                continue;
            }
            let mut proj = [[0.0_f32; 3]; 4];
            for i in 0..4 {
                let p = quad.verts[i];
                let (u, v, depth) = match view_dir {
                    FaceDirection::North => (
                        (max_c[0] - p[0]) / span_x,
                        (max_c[1] - p[1]) / span_y,
                        -p[2],
                    ),
                    FaceDirection::South => {
                        ((p[0] - min_c[0]) / span_x, (max_c[1] - p[1]) / span_y, p[2])
                    }
                    FaceDirection::East => {
                        ((max_c[2] - p[2]) / span_z, (max_c[1] - p[1]) / span_y, p[0])
                    }
                    FaceDirection::West => (
                        (p[2] - min_c[2]) / span_z,
                        (max_c[1] - p[1]) / span_y,
                        -p[0],
                    ),
                    FaceDirection::Up => {
                        ((max_c[0] - p[0]) / span_x, (max_c[2] - p[2]) / span_z, p[1])
                    }
                    FaceDirection::Down => (
                        (max_c[0] - p[0]) / span_x,
                        (p[2] - min_c[2]) / span_z,
                        -p[1],
                    ),
                };
                proj[i] = [u * 16.0, v * 16.0, depth];
            }
            rasterize_tri(
                &mut out,
                &mut zbuf,
                ox,
                oy,
                [proj[0], proj[1], proj[2]],
                [quad.uvs[0], quad.uvs[1], quad.uvs[2]],
                &decoded,
            );
            rasterize_tri(
                &mut out,
                &mut zbuf,
                ox,
                oy,
                [proj[0], proj[2], proj[3]],
                [quad.uvs[0], quad.uvs[2], quad.uvs[3]],
                &decoded,
            );
        }
    }

    let mut png_bytes = Vec::new();
    out.write_to(&mut Cursor::new(&mut png_bytes), ImageFormat::Png)?;
    Ok(png_bytes)
}

fn element_rotation_matrix(el: &ModelElement) -> ([f32; 3], [[f64; 3]; 3]) {
    let Some(rot) = &el.rotation else {
        return (
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        );
    };
    let deg = if let Some(b) = rot.extra_bedrock_euler {
        [-f64::from(b[0]), -f64::from(b[1]), f64::from(b[2])]
    } else {
        match rot.axis {
            RotationAxis::X => [f64::from(rot.angle), 0.0, 0.0],
            RotationAxis::Y => [0.0, f64::from(rot.angle), 0.0],
            RotationAxis::Z => [0.0, 0.0, f64::from(rot.angle)],
        }
    };
    (rot.origin, euler_zyx_to_mat3(deg))
}

fn apply_rot_around(m: [[f64; 3]; 3], origin: [f32; 3], p: [f32; 3]) -> [f32; 3] {
    let rx = f64::from(p[0] - origin[0]);
    let ry = f64::from(p[1] - origin[1]);
    let rz = f64::from(p[2] - origin[2]);
    [
        (m[0][0] * rx + m[0][1] * ry + m[0][2] * rz + f64::from(origin[0])) as f32,
        (m[1][0] * rx + m[1][1] * ry + m[1][2] * rz + f64::from(origin[1])) as f32,
        (m[2][0] * rx + m[2][1] * ry + m[2][2] * rz + f64::from(origin[2])) as f32,
    ]
}

fn apply_rot_vec(m: [[f64; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    let vx = f64::from(v[0]);
    let vy = f64::from(v[1]);
    let vz = f64::from(v[2]);
    [
        (m[0][0] * vx + m[0][1] * vy + m[0][2] * vz) as f32,
        (m[1][0] * vx + m[1][1] * vy + m[1][2] * vz) as f32,
        (m[2][0] * vx + m[2][1] * vy + m[2][2] * vz) as f32,
    ]
}

fn face_local_normal(dir: FaceDirection) -> [f32; 3] {
    match dir {
        FaceDirection::North => [0.0, 0.0, -1.0],
        FaceDirection::South => [0.0, 0.0, 1.0],
        FaceDirection::East => [1.0, 0.0, 0.0],
        FaceDirection::West => [-1.0, 0.0, 0.0],
        FaceDirection::Up => [0.0, 1.0, 0.0],
        FaceDirection::Down => [0.0, -1.0, 0.0],
    }
}

fn face_local_quad_verts(dir: FaceDirection, f: [f32; 3], t: [f32; 3]) -> [[f32; 3]; 4] {
    // Ordered to match Java UV corners: (u1,v1), (u2,v1), (u2,v2), (u1,v2).
    match dir {
        FaceDirection::North => [
            [t[0], t[1], f[2]],
            [f[0], t[1], f[2]],
            [f[0], f[1], f[2]],
            [t[0], f[1], f[2]],
        ],
        FaceDirection::South => [
            [f[0], t[1], t[2]],
            [t[0], t[1], t[2]],
            [t[0], f[1], t[2]],
            [f[0], f[1], t[2]],
        ],
        FaceDirection::East => [
            [t[0], t[1], t[2]],
            [t[0], t[1], f[2]],
            [t[0], f[1], f[2]],
            [t[0], f[1], t[2]],
        ],
        FaceDirection::West => [
            [f[0], t[1], f[2]],
            [f[0], t[1], t[2]],
            [f[0], f[1], t[2]],
            [f[0], f[1], f[2]],
        ],
        FaceDirection::Up => [
            [f[0], t[1], f[2]],
            [t[0], t[1], f[2]],
            [t[0], t[1], t[2]],
            [f[0], t[1], t[2]],
        ],
        FaceDirection::Down => [
            [f[0], f[1], t[2]],
            [t[0], f[1], t[2]],
            [t[0], f[1], f[2]],
            [f[0], f[1], f[2]],
        ],
    }
}

fn rotated_uv_corners(uv: [f32; 4], rotation_deg: u16) -> [[f32; 2]; 4] {
    let base = [
        [uv[0], uv[1]],
        [uv[2], uv[1]],
        [uv[2], uv[3]],
        [uv[0], uv[3]],
    ];
    let steps = ((rotation_deg / 90) % 4) as usize;
    [
        base[(4 - steps) % 4],
        base[(5 - steps) % 4],
        base[(6 - steps) % 4],
        base[(7 - steps) % 4],
    ]
}

fn rasterize_tri(
    dst: &mut RgbaImage,
    zbuf: &mut [f32; 16 * 16],
    ox: u32,
    oy: u32,
    p: [[f32; 3]; 3],
    uv: [[f32; 2]; 3],
    src: &RgbaImage,
) {
    let (src_w, src_h) = src.dimensions();
    if src_w == 0 || src_h == 0 {
        return;
    }

    let min_x = p[0][0].min(p[1][0]).min(p[2][0]).floor().clamp(0.0, 15.0) as i32;
    let max_x = p[0][0].max(p[1][0]).max(p[2][0]).ceil().clamp(0.0, 15.0) as i32;
    let min_y = p[0][1].min(p[1][1]).min(p[2][1]).floor().clamp(0.0, 15.0) as i32;
    let max_y = p[0][1].max(p[1][1]).max(p[2][1]).ceil().clamp(0.0, 15.0) as i32;

    let edge = |a: [f32; 3], b: [f32; 3], x: f32, y: f32| -> f32 {
        (x - a[0]) * (b[1] - a[1]) - (y - a[1]) * (b[0] - a[0])
    };
    let area = edge(p[0], p[1], p[2][0], p[2][1]);
    if area.abs() < 1e-4 {
        return;
    }

    for py in min_y..=max_y {
        let cy = py as f32 + 0.5;
        for px in min_x..=max_x {
            let cx = px as f32 + 0.5;
            let w0 = edge(p[1], p[2], cx, cy) / area;
            let w1 = edge(p[2], p[0], cx, cy) / area;
            let w2 = edge(p[0], p[1], cx, cy) / area;
            if w0 < -1e-3 || w1 < -1e-3 || w2 < -1e-3 {
                continue;
            }
            let z = w0 * p[0][2] + w1 * p[1][2] + w2 * p[2][2];
            let z_idx = (py as usize) * 16 + (px as usize);
            if z < zbuf[z_idx] {
                continue;
            }
            let u = (w0 * uv[0][0] + w1 * uv[1][0] + w2 * uv[2][0]) * (src_w as f32 / 16.0);
            let v = (w0 * uv[0][1] + w1 * uv[1][1] + w2 * uv[2][1]) * (src_h as f32 / 16.0);
            let sx = (u.floor() as i32).clamp(0, (src_w - 1) as i32) as u32;
            let sy = (v.floor() as i32).clamp(0, (src_h - 1) as i32) as u32;
            let src_px = *src.get_pixel(sx, sy);
            if src_px[3] > 0 {
                zbuf[z_idx] = z;
                let dx = ox + px as u32;
                let dy = oy + py as u32;
                if src_px[3] == 255 {
                    dst.put_pixel(dx, dy, src_px);
                } else {
                    let existing = *dst.get_pixel(dx, dy);
                    dst.put_pixel(dx, dy, blend_over(existing, src_px));
                }
            }
        }
    }
}

fn blend_over(dst: Rgba<u8>, src: Rgba<u8>) -> Rgba<u8> {
    let sa = f32::from(src[3]) / 255.0;
    let da = f32::from(dst[3]) / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 1e-6 {
        return Rgba([0, 0, 0, 0]);
    }
    let ch = |i: usize| -> u8 {
        let v = (f32::from(src[i]) * sa + f32::from(dst[i]) * da * (1.0 - sa)) / out_a;
        v.round().clamp(0.0, 255.0) as u8
    };
    Rgba([
        ch(0),
        ch(1),
        ch(2),
        (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
    ])
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

    #[test]
    fn resolve_animation_geometry_falls_back_non_fatally_for_224_by_40_mismatch() {
        let spec = AnimationSpec {
            frametime: 2,
            interpolate: false,
            frames: Vec::new(),
            frame_width: None,
            frame_height: None,
        };
        let mut report = ConversionReport::new();
        let resolved = resolve_animation_strip_geometry(
            "textures/blocks/custom_256x.png",
            40,
            224,
            &spec,
            &mut report,
        )
        .expect("non-fatal resolution");

        assert!(resolved.fell_back_to_static);
        assert_eq!(resolved.frame_width, 40);
        assert_eq!(resolved.frame_height, 224);
        assert_eq!(resolved.frame_count, 1);
        assert_eq!(report.lossy_count, 1);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.severity == WarningSeverity::Warning
                    && w.message.contains("40x224")
                    && w.message.contains("40")),
            "expected structured warning mentioning 40x224 and frame height 40, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn resolve_animation_geometry_infers_non_square_frames_from_indices() {
        let spec = AnimationSpec {
            frametime: 1,
            interpolate: false,
            frames: (0..7)
                .map(|index| AnimationFrame { index, time: None })
                .collect(),
            frame_width: None,
            frame_height: None,
        };
        let mut report = ConversionReport::new();
        let resolved = resolve_animation_strip_geometry(
            "textures/blocks/strip.png",
            40,
            224,
            &spec,
            &mut report,
        )
        .expect("inferred resolution");

        assert!(!resolved.fell_back_to_static);
        assert_eq!(resolved.frame_width, 40);
        assert_eq!(resolved.frame_height, 32);
        assert_eq!(resolved.frame_count, 7);
        assert_eq!(report.lossy_count, 0);
    }

    #[test]
    fn resolution_tier_and_bounded_decode_support_high_res_textures() {
        assert_eq!(ResolutionTier::from_dimensions(16, 16), ResolutionTier::R16);
        assert_eq!(
            ResolutionTier::from_dimensions(128, 128),
            ResolutionTier::R128
        );
        assert_eq!(
            ResolutionTier::from_dimensions(256, 256),
            ResolutionTier::R256
        );
        assert_eq!(
            ResolutionTier::from_dimensions(256, 4096),
            ResolutionTier::R256
        );
        assert_eq!(
            ResolutionTier::from_dimensions(1024, 1024),
            ResolutionTier::R1024
        );
        assert_eq!(
            ResolutionTier::from_dimensions(2048, 2048),
            ResolutionTier::R2048
        );
        assert_eq!(
            ResolutionTier::from_dimensions(40, 224),
            ResolutionTier::Custom(224)
        );

        let mut payloads = Vec::new();
        for dim in [16u32, 32, 64, 128] {
            let img = RgbaImage::new(dim, dim);
            let mut buf = Vec::new();
            img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
                .expect("encode png");
            payloads.push(TexturePayload::from_raw(buf));
        }
        let refs: Vec<&TexturePayload> = payloads.iter().collect();
        let dims = map_textures_bounded(&refs, MAX_CONCURRENT_HIGH_RES_DECODES, |p| {
            let decoded = p.decode()?;
            Ok(decoded.dimensions())
        })
        .expect("bounded decode");
        assert_eq!(dims, vec![(16, 16), (32, 32), (64, 64), (128, 128)]);
    }
}
