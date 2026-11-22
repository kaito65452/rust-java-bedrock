use std::io::Cursor;

use image::{ImageFormat, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

use rp_core::CoreError;

/// Bedrock `*.texture_set.json` schema verified against `minecraft.wiki/w/Vibrant_Visuals#Texture_sets`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BedrockTextureSetFile {
    /// Schema version (`"1.16.100"` for MER, `"1.21.30"` for MERS).
    pub format_version: String,
    /// Inner `minecraft:texture_set` block.
    #[serde(rename = "minecraft:texture_set")]
    pub texture_set: BedrockTextureSetBody,
}

/// Inner `minecraft:texture_set` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BedrockTextureSetBody {
    /// Required color texture stem in the same directory.
    pub color: String,
    /// Optional MER (`_mer`) texture stem (R=metalness, G=emissive, B=roughness).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metalness_emissive_roughness: Option<String>,
    /// Optional normal map (`_normal`) texture stem (mutually exclusive with `heightmap`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normal: Option<String>,
    /// Optional heightmap texture stem (mutually exclusive with `normal`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heightmap: Option<String>,
}

/// Converted Bedrock PBR outputs ready to be inserted alongside the base color PNG.
#[derive(Debug, Clone)]
pub struct ConvertedBedrockPbr {
    /// Optional `*_mer.png` encoded PNG bytes.
    pub mer_png: Option<(String, Vec<u8>)>,
    /// Optional `*_normal.png` encoded PNG bytes.
    pub normal_png: Option<(String, Vec<u8>)>,
    /// Required `*.texture_set.json` path and pretty JSON bytes.
    pub texture_set_json: (String, Vec<u8>),
}

/// Converts Java LabPBR `_s.png` (specular) and/or `_n.png` (normal) maps into Bedrock
/// `*_mer.png`, `*_normal.png`, and `*.texture_set.json` in the same directory as `bedrock_base_path`.
///
/// Channel mapping follows LabPBR 1.3 -> Bedrock MER (`minecraft.wiki/w/Vibrant_Visuals#Texture_sets`):
/// - `MER.R` (metalness): `255` when LabPBR `_s.G >= 230` (hardcoded/metal F0), else `0`
/// - `MER.G` (emissive): LabPBR `_s.A` (`0..=254` -> `a`, `255` -> `0`)
/// - `MER.B` (roughness): `255 - LabPBR _s.R` (smoothness inverted to roughness)
pub fn convert_labpbr_to_bedrock_texture_set(
    bedrock_base_path: &str,
    normal_bytes: Option<&[u8]>,
    specular_bytes: Option<&[u8]>,
) -> Result<Option<ConvertedBedrockPbr>, CoreError> {
    if normal_bytes.is_none() && specular_bytes.is_none() {
        return Ok(None);
    }

    let (dir_prefix, color_stem) = match bedrock_base_path.rsplit_once('/') {
        Some((dir, stem)) => (format!("{dir}/"), stem),
        None => (String::new(), bedrock_base_path),
    };

    let mer_png = if let Some(spec_raw) = specular_bytes {
        let src = image::load_from_memory_with_format(spec_raw, ImageFormat::Png)?.into_rgba8();
        let (w, h) = src.dimensions();
        let mut mer = RgbaImage::new(w, h);
        for (x, y, px) in src.enumerate_pixels() {
            let smoothness = px[0];
            let f0_metal = px[1];
            let raw_emissive = px[3];
            let metalness = if f0_metal >= 230 { 255u8 } else { 0u8 };
            let emissive = if raw_emissive == 255 { 0u8 } else { raw_emissive };
            let roughness = 255u8.saturating_sub(smoothness);
            mer.put_pixel(x, y, Rgba([metalness, emissive, roughness, 255]));
        }
        let mut encoded = Vec::new();
        mer.write_to(&mut Cursor::new(&mut encoded), ImageFormat::Png)?;
        let stem = format!("{color_stem}_mer");
        Some((format!("{dir_prefix}{stem}.png"), stem, encoded))
    } else {
        None
    };

    let normal_png = if let Some(norm_raw) = normal_bytes {
        let src = image::load_from_memory_with_format(norm_raw, ImageFormat::Png)?.into_rgba8();
        let (w, h) = src.dimensions();
        let mut out_norm = RgbaImage::new(w, h);
        for (x, y, px) in src.enumerate_pixels() {
            // Reconstruct tangent-space Z if LabPBR stored AO in B or left B zeroed.
            let nx = (f32::from(px[0]) / 255.0) * 2.0 - 1.0;
            let ny = (f32::from(px[1]) / 255.0) * 2.0 - 1.0;
            let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
            let b = ((nz * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
            out_norm.put_pixel(x, y, Rgba([px[0], px[1], b, 255]));
        }
        let mut encoded = Vec::new();
        out_norm.write_to(&mut Cursor::new(&mut encoded), ImageFormat::Png)?;
        let stem = format!("{color_stem}_normal");
        Some((format!("{dir_prefix}{stem}.png"), stem, encoded))
    } else {
        None
    };

    let ts = BedrockTextureSetFile {
        format_version: String::from("1.16.100"),
        texture_set: BedrockTextureSetBody {
            color: color_stem.to_string(),
            metalness_emissive_roughness: mer_png.as_ref().map(|(_, s, _)| s.clone()),
            normal: normal_png.as_ref().map(|(_, s, _)| s.clone()),
            heightmap: None,
        },
    };
    let json_bytes = serde_json::to_vec_pretty(&ts)
        .map_err(|e| CoreError::InvalidFormat(format!("failed to serialize texture_set: {e}")))?;

    Ok(Some(ConvertedBedrockPbr {
        mer_png: mer_png.map(|(p, _, b)| (p, b)),
        normal_png: normal_png.map(|(p, _, b)| (p, b)),
        texture_set_json: (format!("{dir_prefix}{color_stem}.texture_set.json"), json_bytes),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_png(rgba: [u8; 4]) -> Vec<u8> {
        let mut img = RgbaImage::new(4, 4);
        for p in img.pixels_mut() {
            *p = Rgba(rgba);
        }
        let mut buf = Vec::new();
        img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .expect("png");
        buf
    }

    #[test]
    fn converts_labpbr_normal_and_specular_into_mer_and_texture_set() {
        let n_png = make_png([128, 128, 255, 255]);
        // Smoothness=200 (roughness=55), F0=240 (metal=255), Emissive=80
        let s_png = make_png([200, 240, 0, 80]);

        let out = convert_labpbr_to_bedrock_texture_set(
            "textures/blocks/custom_tex_a",
            Some(&n_png),
            Some(&s_png),
        )
        .expect("convert")
        .expect("some");

        let (mer_path, mer_bytes) = out.mer_png.expect("mer");
        assert_eq!(mer_path, "textures/blocks/custom_tex_a_mer.png");
        let mer_img = image::load_from_memory_with_format(&mer_bytes, ImageFormat::Png)
            .expect("decode mer")
            .into_rgba8();
        assert_eq!(mer_img.get_pixel(0, 0).0, [255, 80, 55, 255]);

        let (norm_path, _) = out.normal_png.expect("normal");
        assert_eq!(norm_path, "textures/blocks/custom_tex_a_normal.png");

        let (ts_path, ts_bytes) = out.texture_set_json;
        assert_eq!(ts_path, "textures/blocks/custom_tex_a.texture_set.json");
        let parsed: BedrockTextureSetFile = serde_json::from_slice(&ts_bytes).expect("parse ts");
        assert_eq!(parsed.format_version, "1.16.100");
        assert_eq!(parsed.texture_set.color, "custom_tex_a");
        assert_eq!(
            parsed.texture_set.metalness_emissive_roughness.as_deref(),
            Some("custom_tex_a_mer")
        );
        assert_eq!(
            parsed.texture_set.normal.as_deref(),
            Some("custom_tex_a_normal")
        );
        assert!(parsed.texture_set.heightmap.is_none());
    }
}
