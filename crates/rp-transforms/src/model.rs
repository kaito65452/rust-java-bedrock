use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use rp_core::{
    BedrockGeoVersion, ConversionReport, ElementFace, ElementRotation, FaceDirection, MappingTable,
    ModelAsset, ModelCategory, ModelElement, RotationAxis, RotationSnapMode, WarningSeverity,
};

/// Top-level Bedrock `*.geo.json` file (`format_version: "1.12.0"`+).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BedrockGeoFile {
    /// Geometry schema version (e.g. `"1.12.0"`).
    pub format_version: String,
    /// Array of geometry definitions inside this file.
    #[serde(rename = "minecraft:geometry")]
    pub geometries: Vec<BedrockGeometry>,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyBedrockGeometry {
    #[serde(alias = "texture_width", default = "default_tex_dim")]
    texturewidth: u32,
    #[serde(alias = "texture_height", default = "default_tex_dim")]
    textureheight: u32,
    #[serde(default)]
    visible_bounds_width: Option<f32>,
    #[serde(default)]
    visible_bounds_height: Option<f32>,
    #[serde(default)]
    visible_bounds_offset: Option<[f32; 3]>,
    #[serde(default)]
    bones: Vec<BedrockBone>,
}

impl BedrockGeoFile {
    /// Parses either modern `1.12.0`+ (`minecraft:geometry`) or legacy `1.8.0`/`1.10.0`
    /// (`geometry.<id>` top-level keys) Bedrock geometry JSON.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        let Some(obj) = value.as_object() else {
            return serde_json::from_value(value);
        };

        if obj.contains_key("minecraft:geometry") {
            return serde_json::from_value(value);
        }

        let format_version = obj
            .get("format_version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("1.10.0")
            .to_string();

        let mut geometries = Vec::new();
        for (key, val) in obj {
            if key.starts_with("geometry.") {
                let legacy: LegacyBedrockGeometry = serde_json::from_value(val.clone())?;
                geometries.push(BedrockGeometry {
                    description: BedrockGeoDescription {
                        identifier: key.clone(),
                        texture_width: legacy.texturewidth,
                        texture_height: legacy.textureheight,
                        visible_bounds_width: legacy.visible_bounds_width,
                        visible_bounds_height: legacy.visible_bounds_height,
                        visible_bounds_offset: legacy.visible_bounds_offset,
                    },
                    bones: legacy.bones,
                });
            }
        }

        if geometries.is_empty() {
            return serde_json::from_value(value);
        }

        Ok(Self {
            format_version,
            geometries,
        })
    }

    /// Serializes geometry in either legacy `1.10.0` (`geometry.<id>`) or modern `1.12.0+` format.
    pub fn to_json_vec_versioned(
        &self,
        version: BedrockGeoVersion,
    ) -> Result<Vec<u8>, serde_json::Error> {
        if version == BedrockGeoVersion::V1_10_0 {
            let mut root = serde_json::Map::new();
            root.insert(
                String::from("format_version"),
                serde_json::Value::String(String::from("1.10.0")),
            );
            for geo in &self.geometries {
                let mut body = serde_json::Map::new();
                body.insert(
                    String::from("texturewidth"),
                    serde_json::Value::from(geo.description.texture_width),
                );
                body.insert(
                    String::from("textureheight"),
                    serde_json::Value::from(geo.description.texture_height),
                );
                if let Some(w) = geo.description.visible_bounds_width {
                    body.insert(
                        String::from("visible_bounds_width"),
                        serde_json::Value::from(w),
                    );
                }
                if let Some(h) = geo.description.visible_bounds_height {
                    body.insert(
                        String::from("visible_bounds_height"),
                        serde_json::Value::from(h),
                    );
                }
                if let Some(off) = geo.description.visible_bounds_offset {
                    body.insert(
                        String::from("visible_bounds_offset"),
                        serde_json::to_value(off)?,
                    );
                }
                body.insert(String::from("bones"), serde_json::to_value(&geo.bones)?);
                root.insert(
                    geo.description.identifier.clone(),
                    serde_json::Value::Object(body),
                );
            }
            return serde_json::to_vec_pretty(&serde_json::Value::Object(root));
        }

        let mut versioned = self.clone();
        versioned.format_version = version.as_str().to_string();
        serde_json::to_vec_pretty(&versioned)
    }
}

/// Single geometry block in `minecraft:geometry`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BedrockGeometry {
    /// Geometry metadata & texture dimensions.
    pub description: BedrockGeoDescription,
    /// Skeletal bones containing cubes.
    #[serde(default)]
    pub bones: Vec<BedrockBone>,
}

/// Bedrock geometry description header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BedrockGeoDescription {
    /// Geometry identifier (e.g. `"geometry.block.custom_cube"`).
    pub identifier: String,
    /// Texture atlas width in pixels/UV units.
    #[serde(default = "default_tex_dim")]
    pub texture_width: u32,
    /// Texture atlas height in pixels/UV units.
    #[serde(default = "default_tex_dim")]
    pub texture_height: u32,
    /// Visible bounding box width in blocks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible_bounds_width: Option<f32>,
    /// Visible bounding box height in blocks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible_bounds_height: Option<f32>,
    /// Visible bounding box offset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible_bounds_offset: Option<[f32; 3]>,
}

fn default_tex_dim() -> u32 {
    16
}

/// Bedrock skeleton bone containing cubes and optional bone-level pivot/rotation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BedrockBone {
    /// Bone name.
    pub name: String,
    /// Optional parent bone name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Bone pivot in Bedrock model space.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pivot: Option<[f32; 3]>,
    /// Bone Euler rotation `[rx, ry, rz]` in degrees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<[f32; 3]>,
    /// Optional bone-level inflation inherited by cubes without an explicit `inflate`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inflate: Option<f32>,
    /// Optional bone-level Box UV mirror flag inherited by cubes without an explicit `mirror`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror: Option<bool>,
    /// Cuboids attached to this bone.
    #[serde(default)]
    pub cubes: Vec<BedrockCube>,
}

/// Single cuboid inside a Bedrock bone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BedrockCube {
    /// Minimum corner `[x, y, z]` in Bedrock coordinates.
    pub origin: [f32; 3],
    /// Dimensions `[dx, dy, dz]`.
    pub size: [f32; 3],
    /// Per-cube rotation pivot in Bedrock coordinates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pivot: Option<[f32; 3]>,
    /// Per-cube Euler rotation `[rx, ry, rz]` in degrees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<[f32; 3]>,
    /// Optional cube inflation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inflate: Option<f32>,
    /// Optional per-cube Box UV horizontal mirror flag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror: Option<bool>,
    /// Box UV `[u, v]` or per-face UV map.
    pub uv: BedrockCubeUv,
}

/// Bedrock cube UV representation: either `[u, v]` box UV or per-face UV object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BedrockCubeUv {
    /// Legacy/entity box UV origin `[u, v]`.
    Box([f32; 2]),
    /// Per-face UV object (`1.12.0`+).
    PerFace(BTreeMap<String, BedrockFaceUv>),
}

/// Per-face UV slice in Bedrock `1.12.0`+.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BedrockFaceUv {
    /// Top-left UV origin `[u, v]`.
    pub uv: [f32; 2],
    /// Optional UV extent `[du, dv]` (defaults to face dimensions when omitted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uv_size: Option<[f32; 2]>,
    /// Optional clockwise UV rotation in degrees (`0`, `90`, `180`, `270`; Bedrock `1.21.0`+).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uv_rotation: Option<u16>,
    /// Optional Bedrock material instance name (preserves Java texture variable binding).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material_instance: Option<String>,
}

#[inline]
fn clean_zero(v: f32) -> f32 {
    if v.abs() < 1e-6 { 0.0 } else { v }
}

/// Builds a 3×3 rotation matrix from `[rx, ry, rz]` degrees using intrinsic `ZYX` / extrinsic `XYZ`
/// order (`R_z(rz) * R_y(ry) * R_x(rx)`), matching Three.js `'ZYX'`, Blockbench, Bedrock bones,
/// and Minecraft Java `1.21.11+` (`25w46a+` / `26.x`) element rotations.
#[must_use]
pub fn euler_zyx_to_mat3(deg: [f64; 3]) -> [[f64; 3]; 3] {
    let (sx, cx) = deg[0].to_radians().sin_cos();
    let (sy, cy) = deg[1].to_radians().sin_cos();
    let (sz, cz) = deg[2].to_radians().sin_cos();
    [
        [cy * cz, cz * sx * sy - cx * sz, cx * cz * sy + sx * sz],
        [cy * sz, cx * cz + sx * sy * sz, cx * sy * sz - cz * sx],
        [-sy, cy * sx, cx * cy],
    ]
}

/// Extracts `[rx, ry, rz]` Euler angles in degrees (intrinsic `ZYX` / extrinsic `XYZ`)
/// from a 3×3 rotation matrix `R = R_z(rz) * R_y(ry) * R_x(rx)`.
#[must_use]
pub fn mat3_to_euler_zyx(m: [[f64; 3]; 3]) -> [f64; 3] {
    let sy = (-m[2][0]).clamp(-1.0, 1.0);
    let y = sy.asin();
    let (x, z) = if m[2][0].abs() < 0.999_999 {
        (m[2][1].atan2(m[2][2]), m[1][0].atan2(m[0][0]))
    } else {
        ((-m[1][2]).atan2(m[1][1]), 0.0)
    };
    [x.to_degrees(), y.to_degrees(), z.to_degrees()]
}

#[inline]
fn mat3_mul(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0_f64; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

#[inline]
fn mat3_mul_vec(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Converts a Java `[0..16]` bounding box `(from, to)` into Bedrock `(origin, size)`.
#[must_use]
pub fn java_to_bedrock_box(from: [f32; 3], to: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let size = [
        clean_zero(to[0] - from[0]),
        clean_zero(to[1] - from[1]),
        clean_zero(to[2] - from[2]),
    ];
    let origin = [
        clean_zero(8.0 - to[0]),
        clean_zero(from[1]),
        clean_zero(from[2] - 8.0),
    ];
    (origin, size)
}

/// Converts a Bedrock `(origin, size)` cuboid back into Java `[0..16]` `(from, to)`.
#[must_use]
pub fn bedrock_to_java_box(origin: [f32; 3], size: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let tx = 8.0 - origin[0];
    let fx = tx - size[0];
    let fy = origin[1];
    let ty = fy + size[1];
    let fz = origin[2] + 8.0;
    let tz = fz + size[2];
    (
        [clean_zero(fx), clean_zero(fy), clean_zero(fz)],
        [clean_zero(tx), clean_zero(ty), clean_zero(tz)],
    )
}

/// Converts a Java rotation pivot `[0..16]` into Bedrock pivot coordinates.
#[must_use]
pub fn java_to_bedrock_pivot(origin: [f32; 3]) -> [f32; 3] {
    [
        clean_zero(8.0 - origin[0]),
        clean_zero(origin[1]),
        clean_zero(origin[2] - 8.0),
    ]
}

/// Converts a Bedrock pivot into Java `[0..16]` rotation origin coordinates.
#[must_use]
pub fn bedrock_to_java_pivot(pivot: [f32; 3]) -> [f32; 3] {
    [
        clean_zero(8.0 - pivot[0]),
        clean_zero(pivot[1]),
        clean_zero(pivot[2] + 8.0),
    ]
}

/// Converts a Java single-axis rotation into Bedrock Euler rotation `[rx, ry, rz]`.
#[must_use]
pub fn java_to_bedrock_rotation(axis: RotationAxis, angle: f32) -> [f32; 3] {
    match axis {
        RotationAxis::X => [clean_zero(-angle), 0.0, 0.0],
        RotationAxis::Y => [0.0, clean_zero(-angle), 0.0],
        RotationAxis::Z => [0.0, 0.0, clean_zero(angle)],
    }
}

/// Converts a Bedrock Euler `[rx, ry, rz]` rotation into Java single-axis rotation.
///
/// Multi-axis rotations select the largest-magnitude axis and record a `LossyConversion`.
#[must_use]
pub fn bedrock_to_java_rotation(
    rot: [f32; 3],
    asset_id: &str,
    report: &mut ConversionReport,
) -> Option<(RotationAxis, f32, Option<[f32; 3]>)> {
    bedrock_to_java_rotation_with_mode(rot, asset_id, None, RotationSnapMode::Preserve, report)
}

/// Converts a Bedrock Euler `[rx, ry, rz]` rotation into Java rotation metadata using
/// target `java_pack_format` rules:
/// - `< 63` (`< 1.21.6`): single-axis, 22.5° increments in `[-45, 45]`
/// - `63..75` (`1.21.6`–`1.21.10`): single-axis, arbitrary float in `[-45, 45]`
/// - `>= 75` (`1.21.11+` / `26.x`): unrestricted multi-axis `{x, y, z}` and arbitrary angles
#[must_use]
pub fn bedrock_to_java_rotation_with_mode(
    rot: [f32; 3],
    asset_id: &str,
    target_java_pack_format: Option<u32>,
    rotation_mode: RotationSnapMode,
    report: &mut ConversionReport,
) -> Option<(RotationAxis, f32, Option<[f32; 3]>)> {
    let eps = 1e-5;
    let non_zero_count = rot.iter().filter(|v| v.abs() > eps).count();
    if non_zero_count == 0 {
        return None;
    }

    let ax = rot[0].abs();
    let ay = rot[1].abs();
    let az = rot[2].abs();

    let (axis, raw_angle) = if ax >= ay && ax >= az {
        (RotationAxis::X, clean_zero(-rot[0]))
    } else if ay >= ax && ay >= az {
        (RotationAxis::Y, clean_zero(-rot[1]))
    } else {
        (RotationAxis::Z, clean_zero(rot[2]))
    };

    let supports_multi_axis = target_java_pack_format.is_some_and(|fmt| fmt >= 75);
    let supports_arbitrary_single_axis = target_java_pack_format.is_some_and(|fmt| fmt >= 63);

    if supports_multi_axis && rotation_mode == RotationSnapMode::Preserve {
        let extra = if non_zero_count > 1 || raw_angle.abs() > 45.0 {
            Some([clean_zero(rot[0]), clean_zero(rot[1]), clean_zero(rot[2])])
        } else {
            None
        };
        return Some((axis, raw_angle, extra));
    }

    let extra = if non_zero_count > 1 {
        report.record_lossy(
            asset_id,
            format!(
                "Multi-axis Bedrock rotation {:?} reduced to single Java axis {:?} ({raw_angle} deg)",
                rot, axis
            ),
        );
        Some([clean_zero(rot[0]), clean_zero(rot[1]), clean_zero(rot[2])])
    } else {
        None
    };

    let angle = match rotation_mode {
        RotationSnapMode::Preserve => raw_angle,
        RotationSnapMode::Snap22_5 => {
            let snapped = clean_zero((raw_angle.clamp(-45.0, 45.0) / 22.5).round() * 22.5);
            if (snapped - raw_angle).abs() > 1e-3 {
                report.record_lossy(
                    asset_id,
                    format!(
                        "Snapped rotation angle {raw_angle} deg to {snapped} deg for legacy Java compatibility"
                    ),
                );
            }
            snapped
        }
        RotationSnapMode::Clamp45 => {
            let clamped = clean_zero(raw_angle.clamp(-45.0, 45.0));
            if (clamped - raw_angle).abs() > 1e-3 {
                report.record_lossy(
                    asset_id,
                    format!("Clamped rotation angle {raw_angle} deg to {clamped} deg"),
                );
            }
            clamped
        }
    };

    if supports_arbitrary_single_axis {
        if !supports_multi_axis && angle.abs() > 45.0 + 1e-3 {
            report.record_warning(
                asset_id,
                WarningSeverity::Warning,
                format!(
                    "Rotation angle {angle} deg exceeds Java 1.21.6-1.21.10 [-45..45] degree limit"
                ),
            );
        }
    } else if !is_standard_java_angle(angle) {
        report.record_warning(
            asset_id,
            WarningSeverity::Warning,
            format!(
                "Rotation angle {angle} deg is outside standard Java 22.5-degree increments ([-45..45])"
            ),
        );
    }

    Some((axis, angle, extra))
}

fn is_standard_java_angle(angle: f32) -> bool {
    const VALID: [f32; 5] = [-45.0, -22.5, 0.0, 22.5, 45.0];
    VALID.iter().any(|v| (angle - v).abs() < 1e-3)
}

/// Converts raw Java `[u1, v1, u2, v2]` UV coordinates into Bedrock `{ uv, uv_size }`
/// without directional face flipping.
#[must_use]
pub fn java_to_bedrock_uv(uv: [f32; 4]) -> ([f32; 2], [f32; 2]) {
    (
        [clean_zero(uv[0]), clean_zero(uv[1])],
        [clean_zero(uv[2] - uv[0]), clean_zero(uv[3] - uv[1])],
    )
}

/// Converts raw Bedrock `{ uv, uv_size }` into Java `[u1, v1, u2, v2]` UV coordinates
/// without directional face flipping.
#[must_use]
pub fn bedrock_to_java_uv(uv: [f32; 2], uv_size: [f32; 2]) -> [f32; 4] {
    [
        clean_zero(uv[0]),
        clean_zero(uv[1]),
        clean_zero(uv[0] + uv_size[0]),
        clean_zero(uv[1] + uv_size[1]),
    ]
}

/// Converts Java per-face `[u1, v1, u2, v2]` UV coordinates into Bedrock `{ uv, uv_size }`,
/// applying the 180° flip required by Bedrock on `up` and `down` faces (`bedrock.js` L966–971).
#[must_use]
pub fn java_to_bedrock_face_uv(dir: FaceDirection, uv: [f32; 4]) -> ([f32; 2], [f32; 2]) {
    let (mut buv, mut bsize) = java_to_bedrock_uv(uv);
    if matches!(dir, FaceDirection::Up | FaceDirection::Down) {
        buv[0] = clean_zero(buv[0] + bsize[0]);
        buv[1] = clean_zero(buv[1] + bsize[1]);
        bsize[0] = clean_zero(-bsize[0]);
        bsize[1] = clean_zero(-bsize[1]);
    }
    (buv, bsize)
}

/// Converts Bedrock per-face `{ uv, uv_size }` into Java `[u1, v1, u2, v2]` UV coordinates,
/// applying the 180° flip required by Bedrock on `up` and `down` faces (`bedrock.js` L732–734).
#[must_use]
pub fn bedrock_to_java_face_uv(dir: FaceDirection, uv: [f32; 2], uv_size: [f32; 2]) -> [f32; 4] {
    let raw = bedrock_to_java_uv(uv, uv_size);
    if matches!(dir, FaceDirection::Up | FaceDirection::Down) {
        [raw[2], raw[3], raw[0], raw[1]]
    } else {
        raw
    }
}

/// Converts a canonical `ModelAsset` into Bedrock `BedrockGeoFile` (`1.12.0` default).
#[must_use]
pub fn model_asset_to_bedrock_geo(
    model: &ModelAsset,
    mappings: &MappingTable,
    report: &mut ConversionReport,
) -> BedrockGeoFile {
    model_asset_to_bedrock_geo_versioned(model, mappings, BedrockGeoVersion::V1_12_0, report)
}

/// Converts a canonical `ModelAsset` into Bedrock `BedrockGeoFile` with explicit `BedrockGeoVersion`.
#[must_use]
pub fn model_asset_to_bedrock_geo_versioned(
    model: &ModelAsset,
    mappings: &MappingTable,
    geo_version: BedrockGeoVersion,
    report: &mut ConversionReport,
) -> BedrockGeoFile {
    model_asset_to_bedrock_geo_with_head_bind(model, mappings, geo_version, false, report)
}

/// Converts a canonical `ModelAsset` into Bedrock `BedrockGeoFile` with optional player-head bone binding.
#[must_use]
pub fn model_asset_to_bedrock_geo_with_head_bind(
    model: &ModelAsset,
    mappings: &MappingTable,
    geo_version: BedrockGeoVersion,
    bind_to_head: bool,
    report: &mut ConversionReport,
) -> BedrockGeoFile {
    if model.has_overrides {
        report.record_lossy(
            &model.id,
            format!(
                "Java item model overrides ({} predicates) are not directly supported in Bedrock .geo.json",
                model.overrides_count
            ),
        );
    }

    let tex_w = model.texture_size[0].max(1);
    let tex_h = model.texture_size[1].max(1);
    let uv_scale_x = tex_w as f32 / 16.0;
    let uv_scale_y = tex_h as f32 / 16.0;

    let y_offset = if bind_to_head
        && !model.elements.is_empty()
        && model.elements.iter().all(|e| e.to[1] <= 20.0)
    {
        20.0
    } else {
        0.0
    };

    let identifier = model_id_to_bedrock_identifier(&model.id, mappings);
    let mut bone_map: BTreeMap<String, Vec<BedrockCube>> = BTreeMap::new();

    for element in &model.elements {
        let bone_name = element
            .group
            .clone()
            .unwrap_or_else(|| String::from("root"));
        let (mut origin, size) = java_to_bedrock_box(element.from, element.to);
        if y_offset != 0.0 {
            origin[1] = clean_zero(origin[1] + y_offset);
        }

        let (pivot, rotation) = match &element.rotation {
            Some(rot) => {
                let mut b_pivot = java_to_bedrock_pivot(rot.origin);
                if y_offset != 0.0 {
                    b_pivot[1] = clean_zero(b_pivot[1] + y_offset);
                }
                (
                    Some(b_pivot),
                    Some(
                        rot.extra_bedrock_euler
                            .unwrap_or_else(|| java_to_bedrock_rotation(rot.axis, rot.angle)),
                    ),
                )
            }
            None => (None, None),
        };

        let mut per_face = BTreeMap::new();
        for (dir, face) in &element.faces {
            let java_uv = face
                .uv
                .unwrap_or_else(|| auto_java_uv_for_face(*dir, element.from, element.to));
            let scaled_uv = [
                java_uv[0] * uv_scale_x,
                java_uv[1] * uv_scale_y,
                java_uv[2] * uv_scale_x,
                java_uv[3] * uv_scale_y,
            ];
            let (uv, uv_size) = java_to_bedrock_face_uv(*dir, scaled_uv);
            let mat = if face.texture.is_empty() {
                None
            } else {
                Some(face.texture.clone())
            };
            let uv_rotation = match face.rotation % 360 {
                0 => None,
                rot => Some(rot),
            };
            per_face.insert(
                dir.as_str().to_string(),
                BedrockFaceUv {
                    uv,
                    uv_size: Some(uv_size),
                    uv_rotation,
                    material_instance: mat,
                },
            );
        }

        let cube = BedrockCube {
            origin,
            size,
            pivot,
            rotation,
            inflate: None,
            mirror: None,
            uv: BedrockCubeUv::PerFace(per_face),
        };

        bone_map.entry(bone_name).or_default().push(cube);
    }

    if bone_map.is_empty()
        && let Some(parent) = &model.parent
    {
        report.record_warning(
            &model.id,
            WarningSeverity::Info,
            format!(
                "Model inherits from '{parent}' with no explicit geometry elements; emitting empty bone wrapper"
            ),
        );
    }

    let mut bones = Vec::new();
    if bind_to_head && !bone_map.contains_key("head") {
        bones.push(BedrockBone {
            name: String::from("head"),
            parent: None,
            pivot: Some([0.0, 24.0, 0.0]),
            rotation: None,
            inflate: None,
            mirror: None,
            cubes: Vec::new(),
        });
    }
    for (name, cubes) in bone_map {
        let is_head_bone = name == "head";
        let (parent, pivot) = if bind_to_head && !is_head_bone {
            (Some(String::from("head")), Some([0.0, 24.0, 0.0]))
        } else if bind_to_head {
            (None, Some([0.0, 24.0, 0.0]))
        } else {
            (None, Some([0.0, 0.0, 0.0]))
        };
        bones.push(BedrockBone {
            name,
            parent,
            pivot,
            rotation: None,
            inflate: None,
            mirror: None,
            cubes,
        });
    }

    BedrockGeoFile {
        format_version: geo_version.as_str().to_string(),
        geometries: vec![BedrockGeometry {
            description: BedrockGeoDescription {
                identifier,
                texture_width: tex_w,
                texture_height: tex_h,
                visible_bounds_width: Some(2.0),
                visible_bounds_height: Some(2.0),
                visible_bounds_offset: Some([0.0, 0.5, 0.0]),
            },
            bones,
        }],
    }
}

/// Converts a parsed Bedrock `BedrockGeoFile` into one or more canonical `ModelAsset`s.
#[must_use]
pub fn bedrock_geo_to_model_assets(
    geo_file: &BedrockGeoFile,
    model_id_hint: &str,
    category_hint: ModelCategory,
    report: &mut ConversionReport,
) -> Vec<ModelAsset> {
    bedrock_geo_to_model_assets_with_options(
        geo_file,
        model_id_hint,
        category_hint,
        None,
        RotationSnapMode::Preserve,
        report,
    )
}

/// Converts a parsed Bedrock `BedrockGeoFile` into canonical `ModelAsset`s with target-version
/// rotation handling and normalized `[0..16]` Java UV scaling.
#[must_use]
pub fn bedrock_geo_to_model_assets_with_options(
    geo_file: &BedrockGeoFile,
    model_id_hint: &str,
    category_hint: ModelCategory,
    target_java_pack_format: Option<u32>,
    rotation_mode: RotationSnapMode,
    report: &mut ConversionReport,
) -> Vec<ModelAsset> {
    let multi = geo_file.geometries.len() > 1;
    let mut assets = Vec::with_capacity(geo_file.geometries.len());

    for (idx, geo) in geo_file.geometries.iter().enumerate() {
        let model_id = if !multi {
            model_id_hint.to_string()
        } else {
            let derived = bedrock_identifier_to_model_id(&geo.description.identifier);
            if derived.is_empty() {
                format!("{model_id_hint}_{idx}")
            } else {
                derived
            }
        };

        let tex_w = geo.description.texture_width.max(1);
        let tex_h = geo.description.texture_height.max(1);
        let uv_scale_x = 16.0 / tex_w as f32;
        let uv_scale_y = 16.0 / tex_h as f32;

        let mut elements = Vec::new();
        let mut textures = BTreeMap::new();
        let bone_lookup: HashMap<String, &BedrockBone> = geo
            .bones
            .iter()
            .map(|b| (b.name.to_ascii_lowercase(), b))
            .collect();

        let has_nonzero_rot = |rot: Option<[f32; 3]>| -> bool {
            rot.is_some_and(|r| r.iter().any(|v| v.abs() > 1e-5))
        };

        for bone in &geo.bones {
            // Build ancestor chain [bone, parent, grandparent, ...] with cycle guard.
            let mut chain: Vec<&BedrockBone> = Vec::new();
            let mut visited: HashSet<String> = HashSet::new();
            let mut cur_opt: Option<&BedrockBone> = Some(bone);
            while let Some(cur) = cur_opt {
                let key = cur.name.to_ascii_lowercase();
                if !visited.insert(key) {
                    break;
                }
                chain.push(cur);
                cur_opt = cur
                    .parent
                    .as_deref()
                    .and_then(|p| bone_lookup.get(&p.to_ascii_lowercase()).copied());
            }

            let parent_has_rot = chain[1..].iter().any(|b| has_nonzero_rot(b.rotation));
            let self_bone_has_rot = has_nonzero_rot(bone.rotation);

            for cube in &bone.cubes {
                let (mut from, mut to) = bedrock_to_java_box(cube.origin, cube.size);
                let uninflated_from = from;
                let uninflated_to = to;

                // Apply cube.inflate ?? bone.inflate (`bedrock.js` L743-746, `java_block.ts` L230-235).
                let inflate = cube.inflate.or(bone.inflate).unwrap_or(0.0);
                if inflate.abs() > 1e-6 {
                    for axis in 0..3 {
                        from[axis] = clean_zero(from[axis] - inflate);
                        to[axis] = clean_zero(to[axis] + inflate);
                    }
                }
                for axis in 0..3 {
                    if from[axis] > to[axis] {
                        std::mem::swap(&mut from[axis], &mut to[axis]);
                    }
                }

                let cube_has_rot = has_nonzero_rot(cube.rotation);
                let needs_se3_baking = parent_has_rot || (self_bone_has_rot && cube_has_rot);

                let (effective_pivot_java, effective_bedrock_rot) = if !needs_se3_baking {
                    if cube_has_rot {
                        let p = cube
                            .pivot
                            .or(bone.pivot)
                            .map(bedrock_to_java_pivot)
                            .unwrap_or([8.0, 8.0, 8.0]);
                        (p, cube.rotation)
                    } else if self_bone_has_rot {
                        let p = bone
                            .pivot
                            .or(cube.pivot)
                            .map(bedrock_to_java_pivot)
                            .unwrap_or([8.0, 8.0, 8.0]);
                        (p, bone.rotation)
                    } else {
                        ([8.0, 8.0, 8.0], None)
                    }
                } else {
                    // Exact SE(3) rigid-body composition in Java/Blockbench space (`ZYX` order).
                    let half = [
                        0.5 * f64::from(to[0] - from[0]),
                        0.5 * f64::from(to[1] - from[1]),
                        0.5 * f64::from(to[2] - from[2]),
                    ];
                    let mut center = [
                        0.5 * f64::from(from[0] + to[0]),
                        0.5 * f64::from(from[1] + to[1]),
                        0.5 * f64::from(from[2] + to[2]),
                    ];
                    let mut r_acc = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

                    if let Some(cr) = cube.rotation.filter(|r| r.iter().any(|v| v.abs() > 1e-5)) {
                        let p =
                            bedrock_to_java_pivot(cube.pivot.or(bone.pivot).unwrap_or([0.0; 3]));
                        let p64 = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
                        let rc = euler_zyx_to_mat3([
                            -f64::from(cr[0]),
                            -f64::from(cr[1]),
                            f64::from(cr[2]),
                        ]);
                        let rel = [center[0] - p64[0], center[1] - p64[1], center[2] - p64[2]];
                        let rot_rel = mat3_mul_vec(rc, rel);
                        center = [
                            rot_rel[0] + p64[0],
                            rot_rel[1] + p64[1],
                            rot_rel[2] + p64[2],
                        ];
                        r_acc = mat3_mul(rc, r_acc);
                    }

                    for anc in &chain {
                        if let Some(br) = anc.rotation.filter(|r| r.iter().any(|v| v.abs() > 1e-5))
                        {
                            let p = bedrock_to_java_pivot(anc.pivot.unwrap_or([0.0; 3]));
                            let p64 = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
                            let rb = euler_zyx_to_mat3([
                                -f64::from(br[0]),
                                -f64::from(br[1]),
                                f64::from(br[2]),
                            ]);
                            let rel = [center[0] - p64[0], center[1] - p64[1], center[2] - p64[2]];
                            let rot_rel = mat3_mul_vec(rb, rel);
                            center = [
                                rot_rel[0] + p64[0],
                                rot_rel[1] + p64[1],
                                rot_rel[2] + p64[2],
                            ];
                            r_acc = mat3_mul(rb, r_acc);
                        }
                    }

                    from = [
                        clean_zero((center[0] - half[0]) as f32),
                        clean_zero((center[1] - half[1]) as f32),
                        clean_zero((center[2] - half[2]) as f32),
                    ];
                    to = [
                        clean_zero((center[0] + half[0]) as f32),
                        clean_zero((center[1] + half[1]) as f32),
                        clean_zero((center[2] + half[2]) as f32),
                    ];
                    let pivot_java = [
                        clean_zero(center[0] as f32),
                        clean_zero(center[1] as f32),
                        clean_zero(center[2] as f32),
                    ];
                    let java_euler = mat3_to_euler_zyx(r_acc);
                    let bedrock_euler = [
                        clean_zero(-java_euler[0] as f32),
                        clean_zero(-java_euler[1] as f32),
                        clean_zero(java_euler[2] as f32),
                    ];
                    (pivot_java, Some(bedrock_euler))
                };

                let rotation = effective_bedrock_rot.and_then(|euler| {
                    bedrock_to_java_rotation_with_mode(
                        euler,
                        &model_id,
                        target_java_pack_format,
                        rotation_mode,
                        report,
                    )
                    .map(|(axis, angle, extra)| ElementRotation {
                        origin: effective_pivot_java,
                        axis,
                        angle,
                        rescale: false,
                        extra_bedrock_euler: extra,
                    })
                });

                let mut faces = BTreeMap::new();
                match &cube.uv {
                    BedrockCubeUv::PerFace(map) => {
                        for (face_name, face_uv) in map {
                            if let Some(dir) = parse_face_direction(face_name) {
                                let tex_ref = face_uv
                                    .material_instance
                                    .clone()
                                    .unwrap_or_else(|| String::from("#all"));
                                let scaled_uv = if let Some(uv_size) = face_uv.uv_size {
                                    let raw_uv = bedrock_to_java_face_uv(dir, face_uv.uv, uv_size);
                                    [
                                        clean_zero(raw_uv[0] * uv_scale_x),
                                        clean_zero(raw_uv[1] * uv_scale_y),
                                        clean_zero(raw_uv[2] * uv_scale_x),
                                        clean_zero(raw_uv[3] * uv_scale_y),
                                    ]
                                } else {
                                    auto_java_uv_for_face(dir, uninflated_from, uninflated_to)
                                };
                                let uv_rot = face_uv.uv_rotation.unwrap_or(0) % 360;
                                faces.insert(
                                    dir,
                                    ElementFace {
                                        uv: Some(scaled_uv),
                                        texture: tex_ref,
                                        cullface: None,
                                        rotation: uv_rot,
                                        tintindex: None,
                                    },
                                );
                            }
                        }
                    }
                    BedrockCubeUv::Box([u, v]) => {
                        report.record_lossy(
                            &model_id,
                            "Bedrock box UV [u, v] approximated as per-face Java UVs",
                        );
                        let mirror = cube.mirror.unwrap_or(bone.mirror.unwrap_or(false));
                        faces = bedrock_box_uv_to_java_faces(
                            [*u, *v],
                            cube.size,
                            mirror,
                            uv_scale_x,
                            uv_scale_y,
                        );
                    }
                }

                elements.push(ModelElement {
                    group: Some(bone.name.clone()),
                    from,
                    to,
                    rotation,
                    faces,
                    shade: true,
                    shade_direction_override: None,
                    light_emission: None,
                });
            }
        }

        normalize_elements_to_java_bounds(&model_id, geo, &mut elements, report);

        if !elements.is_empty() {
            let default_hint = default_texture_hint_for_model(&model_id);
            textures
                .entry(String::from("all"))
                .or_insert_with(|| default_hint.clone());
            for el in &elements {
                for face in el.faces.values() {
                    if let Some(var) = face.texture.strip_prefix('#')
                        && !var.is_empty()
                    {
                        textures
                            .entry(var.to_string())
                            .or_insert_with(|| default_hint.clone());
                    }
                }
            }
        }

        assets.push(ModelAsset {
            id: model_id,
            category: category_hint,
            parent: None,
            textures,
            texture_size: [tex_w, tex_h],
            ambient_occlusion: true,
            elements,
            display: BTreeMap::new(),
            has_overrides: false,
            overrides_count: 0,
        });
    }

    assets
}

/// Converts Bedrock Box UV `[u, v]` + uninflated cube size + `mirror` flag into
/// exact per-face Java UVs matching Blockbench `cube.js` L1304–1352.
#[must_use]
pub fn bedrock_box_uv_to_java_faces(
    uv_offset: [f32; 2],
    uninflated_size: [f32; 3],
    mirror: bool,
    uv_scale_x: f32,
    uv_scale_y: f32,
) -> BTreeMap<FaceDirection, ElementFace> {
    let dx = (uninflated_size[0].abs() + 1e-7).floor();
    let dy = (uninflated_size[1].abs() + 1e-7).floor();
    let dz = (uninflated_size[2].abs() + 1e-7).floor();

    let mut rects: [(FaceDirection, [f32; 2], [f32; 2]); 6] = [
        (FaceDirection::East, [0.0, dz], [dz, dy]),
        (FaceDirection::West, [dz + dx, dz], [dz, dy]),
        (FaceDirection::Up, [dz + dx, dz], [-dx, -dz]),
        (FaceDirection::Down, [dz + dx * 2.0, 0.0], [-dx, dz]),
        (FaceDirection::South, [dz * 2.0 + dx, dz], [dx, dy]),
        (FaceDirection::North, [dz, dz], [dx, dy]),
    ];

    if mirror {
        for (_, from, size) in &mut rects {
            from[0] += size[0];
            size[0] = -size[0];
        }
        // Swap East (index 0) and West (index 1) UV rectangles (`cube.js` L1320-1332).
        let east_from = rects[0].1;
        let east_size = rects[0].2;
        rects[0].1 = rects[1].1;
        rects[0].2 = rects[1].2;
        rects[1].1 = east_from;
        rects[1].2 = east_size;
    }

    let mut faces = BTreeMap::new();
    for (dir, from, size) in rects {
        let raw = [
            from[0] + uv_offset[0],
            from[1] + uv_offset[1],
            from[0] + size[0] + uv_offset[0],
            from[1] + size[1] + uv_offset[1],
        ];
        let scaled = [
            clean_zero((raw[0] * uv_scale_x).clamp(0.0, 16.0)),
            clean_zero((raw[1] * uv_scale_y).clamp(0.0, 16.0)),
            clean_zero((raw[2] * uv_scale_x).clamp(0.0, 16.0)),
            clean_zero((raw[3] * uv_scale_y).clamp(0.0, 16.0)),
        ];
        faces.insert(
            dir,
            ElementFace {
                uv: Some(scaled),
                texture: String::from("#all"),
                cullface: None,
                rotation: 0,
                tintindex: None,
            },
        );
    }
    faces
}

fn normalize_elements_to_java_bounds(
    model_id: &str,
    geo: &BedrockGeometry,
    elements: &mut [ModelElement],
    report: &mut ConversionReport,
) {
    if elements.is_empty() {
        return;
    }

    // Recenter entity wearables modeled relative to Bedrock's player head bone (pivot ~ [0, 24, 0],
    // head volume [-4..4, 24..32, -4..4]). Shifting by [+px, 4 - py, -pz] maps the player's
    // head center to Java [8, 8, 8] ([4..12]^3) while preserving exact side/front/top placement!
    if let Some(head_pivot) = geo
        .bones
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case("head") && b.pivot.is_some_and(|p| p[1] >= 16.0))
        .and_then(|b| b.pivot)
    {
        let max_y = elements
            .iter()
            .map(|e| e.to[1])
            .fold(f32::NEG_INFINITY, f32::max);
        if max_y > 20.0 {
            let shift = [head_pivot[0], 4.0 - head_pivot[1], -head_pivot[2]];
            for el in elements.iter_mut() {
                for i in 0..3 {
                    el.from[i] = clean_zero(el.from[i] + shift[i]);
                    el.to[i] = clean_zero(el.to[i] + shift[i]);
                    if let Some(rot) = &mut el.rotation {
                        rot.origin[i] = clean_zero(rot.origin[i] + shift[i]);
                    }
                }
            }
        }
    }

    let mut min_c = [f32::INFINITY; 3];
    let mut max_c = [f32::NEG_INFINITY; 3];
    for el in elements.iter() {
        for i in 0..3 {
            min_c[i] = min_c[i].min(el.from[i]).min(el.to[i]);
            max_c[i] = max_c[i].max(el.from[i]).max(el.to[i]);
        }
    }

    let out_of_bounds = (0..3).any(|i| min_c[i] < -16.0 || max_c[i] > 32.0);
    if !out_of_bounds {
        return;
    }

    let spans = [
        max_c[0] - min_c[0],
        max_c[1] - min_c[1],
        max_c[2] - min_c[2],
    ];
    let max_span = spans[0].max(spans[1]).max(spans[2]);

    if max_span <= 48.0 {
        for i in 0..3 {
            if min_c[i] < -16.0 || max_c[i] > 32.0 {
                let shift = 8.0 - 0.5 * (min_c[i] + max_c[i]);
                for el in elements.iter_mut() {
                    el.from[i] = clean_zero((el.from[i] + shift).clamp(-16.0, 32.0));
                    el.to[i] = clean_zero((el.to[i] + shift).clamp(-16.0, 32.0));
                    if let Some(rot) = &mut el.rotation {
                        rot.origin[i] = clean_zero(rot.origin[i] + shift);
                    }
                }
            }
        }
        report.record_warning(
            model_id,
            WarningSeverity::Info,
            "Shifted model coordinates to fit within Java's [-16, 32] element bounds",
        );
    } else {
        let scale = 48.0 / max_span;
        let center = [
            0.5 * (min_c[0] + max_c[0]),
            0.5 * (min_c[1] + max_c[1]),
            0.5 * (min_c[2] + max_c[2]),
        ];
        for el in elements.iter_mut() {
            for i in 0..3 {
                el.from[i] =
                    clean_zero((8.0 + (el.from[i] - center[i]) * scale).clamp(-16.0, 32.0));
                el.to[i] = clean_zero((8.0 + (el.to[i] - center[i]) * scale).clamp(-16.0, 32.0));
                if let Some(rot) = &mut el.rotation {
                    rot.origin[i] = clean_zero(8.0 + (rot.origin[i] - center[i]) * scale);
                }
            }
        }
        report.record_lossy(
            model_id,
            format!(
                "Model span ({max_span:.2}) exceeded Java's 48-unit [-16, 32] limit; uniformly scaled by {scale:.4}"
            ),
        );
    }
}

fn model_id_to_bedrock_identifier(model_id: &str, mappings: &MappingTable) -> String {
    if let Some(mapped) = mappings.java_model_parent_to_bedrock(model_id) {
        return mapped;
    }
    let clean = model_id
        .strip_prefix("minecraft:")
        .unwrap_or(model_id)
        .replace('/', ".");
    if clean.starts_with("geometry.") {
        clean
    } else {
        format!("geometry.{clean}")
    }
}

fn bedrock_identifier_to_model_id(identifier: &str) -> String {
    let rest = identifier.strip_prefix("geometry.").unwrap_or(identifier);
    if let Some(name) = rest.strip_prefix("block.") {
        format!("block/{name}")
    } else if let Some(name) = rest.strip_prefix("item.") {
        format!("item/{name}")
    } else {
        format!("entity/{rest}")
    }
}

fn default_texture_hint_for_model(model_id: &str) -> String {
    if let Some(name) = model_id.strip_prefix("block/") {
        format!("block/{name}")
    } else if let Some(name) = model_id.strip_prefix("item/") {
        format!("item/{name}")
    } else if let Some(name) = model_id.strip_prefix("entity/") {
        format!("entity/{name}")
    } else {
        model_id.to_string()
    }
}

fn parse_face_direction(name: &str) -> Option<FaceDirection> {
    match name.to_ascii_lowercase().as_str() {
        "north" => Some(FaceDirection::North),
        "south" => Some(FaceDirection::South),
        "east" => Some(FaceDirection::East),
        "west" => Some(FaceDirection::West),
        "up" => Some(FaceDirection::Up),
        "down" => Some(FaceDirection::Down),
        _ => None,
    }
}

/// Computes Java's default relative auto-UV for a face when `uv` is omitted
/// (matching Minecraft Java `FaceBakery` and Blockbench `cube.js` L710–759).
#[must_use]
pub fn auto_java_uv_for_face(dir: FaceDirection, from: [f32; 3], to: [f32; 3]) -> [f32; 4] {
    match dir {
        FaceDirection::North => [16.0 - to[0], 16.0 - to[1], 16.0 - from[0], 16.0 - from[1]],
        FaceDirection::South => [from[0], 16.0 - to[1], to[0], 16.0 - from[1]],
        FaceDirection::West => [from[2], 16.0 - to[1], to[2], 16.0 - from[1]],
        FaceDirection::East => [16.0 - to[2], 16.0 - to[1], 16.0 - from[2], 16.0 - from[1]],
        FaceDirection::Up => [from[0], from[2], to[0], to[2]],
        FaceDirection::Down => [from[0], 16.0 - to[2], to[0], 16.0 - from[2]],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_cubes_transform_accurately() {
        // Full 1x1x1 block [0,0,0]..[16,16,16] -> Bedrock origin [-8,0,-8], size [16,16,16]
        let (origin, size) = java_to_bedrock_box([0.0, 0.0, 0.0], [16.0, 16.0, 16.0]);
        assert_eq!(origin, [-8.0, 0.0, -8.0]);
        assert_eq!(size, [16.0, 16.0, 16.0]);

        let (from, to) = bedrock_to_java_box(origin, size);
        assert_eq!(from, [0.0, 0.0, 0.0]);
        assert_eq!(to, [16.0, 16.0, 16.0]);

        // Asymmetric offset cube
        let j_from = [2.0, 4.0, 6.0];
        let j_to = [5.0, 10.0, 14.0];
        let (b_origin, b_size) = java_to_bedrock_box(j_from, j_to);
        assert_eq!(b_origin, [3.0, 4.0, -2.0]);
        assert_eq!(b_size, [3.0, 6.0, 8.0]);

        let (rt_from, rt_to) = bedrock_to_java_box(b_origin, b_size);
        assert_eq!(rt_from, j_from);
        assert_eq!(rt_to, j_to);
    }

    #[test]
    fn pivot_rotation_and_uv_roundtrip() {
        let j_pivot = [8.0, 8.0, 8.0];
        let b_pivot = java_to_bedrock_pivot(j_pivot);
        assert_eq!(b_pivot, [0.0, 8.0, 0.0]);
        assert_eq!(bedrock_to_java_pivot(b_pivot), j_pivot);

        let mut report = ConversionReport::new();
        for (axis, angle, expected_b) in [
            (RotationAxis::X, 22.5, [-22.5, 0.0, 0.0]),
            (RotationAxis::Y, -45.0, [0.0, 45.0, 0.0]),
            (RotationAxis::Z, 45.0, [0.0, 0.0, 45.0]),
        ] {
            let b_rot = java_to_bedrock_rotation(axis, angle);
            assert_eq!(b_rot, expected_b);
            let parsed = bedrock_to_java_rotation(b_rot, "block/test", &mut report)
                .expect("non-zero rotation");
            assert_eq!(parsed.0, axis);
            assert!((parsed.1 - angle).abs() < 1e-6);
            assert!(parsed.2.is_none());
        }

        // Multi-axis rotation flags lossy conversion
        let multi = bedrock_to_java_rotation([10.0, -45.0, 0.0], "entity/test", &mut report)
            .expect("multi-axis");
        assert_eq!(multi.0, RotationAxis::Y);
        assert!((multi.1 - 45.0).abs() < 1e-6);
        assert_eq!(report.lossy_count, 1);

        let uv = [2.0, 4.0, 10.0, 12.0];
        let (buv, bsize) = java_to_bedrock_uv(uv);
        assert_eq!(buv, [2.0, 4.0]);
        assert_eq!(bsize, [8.0, 8.0]);
        assert_eq!(bedrock_to_java_uv(buv, bsize), uv);
    }

    #[test]
    fn full_model_asset_geo_roundtrip() {
        let mappings = MappingTable::default_embedded().expect("mappings");
        let mut faces = BTreeMap::new();
        faces.insert(
            FaceDirection::North,
            ElementFace {
                uv: Some([0.0, 0.0, 16.0, 16.0]),
                texture: String::from("#all"),
                cullface: None,
                rotation: 0,
                tintindex: None,
            },
        );
        faces.insert(
            FaceDirection::Up,
            ElementFace {
                uv: Some([2.0, 4.0, 10.0, 12.0]),
                texture: String::from("#all"),
                cullface: None,
                rotation: 90,
                tintindex: None,
            },
        );
        faces.insert(
            FaceDirection::Down,
            ElementFace {
                uv: Some([1.0, 3.0, 9.0, 11.0]),
                texture: String::from("#all"),
                cullface: None,
                rotation: 270,
                tintindex: None,
            },
        );
        let model = ModelAsset {
            id: String::from("block/pedestal"),
            category: ModelCategory::Block,
            parent: None,
            textures: BTreeMap::from([(String::from("all"), String::from("block/stone"))]),
            texture_size: [16, 16],
            ambient_occlusion: true,
            elements: vec![ModelElement {
                group: Some(String::from("base")),
                from: [2.0, 0.0, 2.0],
                to: [14.0, 8.0, 14.0],
                rotation: Some(ElementRotation {
                    origin: [8.0, 4.0, 8.0],
                    axis: RotationAxis::Y,
                    angle: 45.0,
                    rescale: false,
                    extra_bedrock_euler: None,
                }),
                faces,
                shade: true,
                shade_direction_override: None,
                light_emission: None,
            }],
            display: BTreeMap::new(),
            has_overrides: false,
            overrides_count: 0,
        };

        let mut report = ConversionReport::new();
        let geo = model_asset_to_bedrock_geo(&model, &mappings, &mut report);
        assert_eq!(geo.format_version, "1.12.0");
        assert_eq!(
            geo.geometries[0].description.identifier,
            "geometry.block.pedestal"
        );
        // Verify Bedrock `up` face UV is flipped 180° (`bedrock.js` L966-971) and preserves uv_rotation.
        if let BedrockCubeUv::PerFace(ref pf) = geo.geometries[0].bones[0].cubes[0].uv {
            let up = &pf["up"];
            assert_eq!(up.uv, [10.0, 12.0]);
            assert_eq!(up.uv_size, Some([-8.0, -8.0]));
            assert_eq!(up.uv_rotation, Some(90));
            let down = &pf["down"];
            assert_eq!(down.uv, [9.0, 11.0]);
            assert_eq!(down.uv_size, Some([-8.0, -8.0]));
            assert_eq!(down.uv_rotation, Some(270));
        } else {
            panic!("expected per-face UV");
        }

        let back =
            bedrock_geo_to_model_assets(&geo, "block/pedestal", ModelCategory::Block, &mut report);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].elements, model.elements);
        assert_eq!(report.lossy_count, 0);
    }

    #[test]
    fn parses_legacy_1_10_bedrock_geometry_format() {
        let legacy_json = br#"{
            "format_version": "1.10.0",
            "geometry.patnt": {
                "texturewidth": 64,
                "textureheight": 32,
                "bones": [
                    {
                        "name": "patnt",
                        "pivot": [0, 0, 0],
                        "cubes": [
                            {"origin": [-8, 0, -8], "size": [16, 16, 16], "uv": [0, 0]}
                        ]
                    }
                ]
            }
        }"#;
        let geo = BedrockGeoFile::from_json_slice(legacy_json).expect("legacy 1.10.0 parse");
        assert_eq!(geo.format_version, "1.10.0");
        assert_eq!(geo.geometries.len(), 1);
        assert_eq!(geo.geometries[0].description.identifier, "geometry.patnt");
        assert_eq!(geo.geometries[0].description.texture_width, 64);
        assert_eq!(geo.geometries[0].description.texture_height, 32);
    }

    #[test]
    fn normalizes_head_pivoted_and_oversized_models_to_java_bounds() {
        let head_geo = BedrockGeoFile::from_json_slice(
            br#"{
                "format_version": "1.12.0",
                "minecraft:geometry": [{
                    "description": { "identifier": "geometry.mask", "texture_width": 32, "texture_height": 32 },
                    "bones": [
                        { "name": "head", "pivot": [0, 24, 0] },
                        {
                            "name": "mask",
                            "pivot": [0, 24, 0],
                            "cubes": [{ "origin": [-4, 20, -4], "size": [8, 14.8, 8], "uv": { "north": { "uv": [0, 0], "uv_size": [8, 8] } } }]
                        }
                    ]
                }]
            }"#,
        )
        .expect("head geo");

        let mut report = ConversionReport::new();
        let models = bedrock_geo_to_model_assets(
            &head_geo,
            "entity/mask",
            ModelCategory::Entity,
            &mut report,
        );
        let el = &models[0].elements[0];
        assert!((el.from[1] - 0.0).abs() < 1e-4);
        assert!((el.to[1] - 14.8).abs() < 1e-4);
        assert!(
            el.from
                .iter()
                .chain(el.to.iter())
                .all(|&c| (-16.0..=32.0).contains(&c))
        );
        assert_eq!(report.lossy_count, 0);

        let huge_geo = BedrockGeoFile::from_json_slice(
            br#"{
                "format_version": "1.12.0",
                "minecraft:geometry": [{
                    "description": { "identifier": "geometry.titan", "texture_width": 16, "texture_height": 16 },
                    "bones": [{
                        "name": "body",
                        "cubes": [{ "origin": [-40, 0, -40], "size": [80, 96, 80], "uv": { "north": { "uv": [0, 0], "uv_size": [8, 8] } } }]
                    }]
                }]
            }"#,
        )
        .expect("huge geo");
        let huge_models = bedrock_geo_to_model_assets(
            &huge_geo,
            "entity/titan",
            ModelCategory::Entity,
            &mut report,
        );
        let huge_el = &huge_models[0].elements[0];
        assert!(
            huge_el
                .from
                .iter()
                .chain(huge_el.to.iter())
                .all(|&c| (-16.0..=32.0).contains(&c))
        );
        assert_eq!(report.lossy_count, 1);
    }

    #[test]
    fn bedrock_inflate_box_uv_mirror_and_auto_uv_match_blockbench() {
        // Test inflate (cube.inflate ?? bone.inflate) + Box UV with mirror + floored dimensions
        let geo = BedrockGeoFile::from_json_slice(
            br#"{
                "format_version": "1.12.0",
                "minecraft:geometry": [{
                    "description": { "identifier": "geometry.mirrored", "texture_width": 64, "texture_height": 64 },
                    "bones": [{
                        "name": "limb",
                        "inflate": 0.5,
                        "mirror": true,
                        "cubes": [{
                            "origin": [0, 0, 0],
                            "size": [4.9, 12.2, 4.1],
                            "uv": [16, 16]
                        }]
                    }]
                }]
            }"#,
        )
        .expect("parse mirrored geo");

        let mut report = ConversionReport::new();
        let models = bedrock_geo_to_model_assets(
            &geo,
            "entity/mirrored",
            ModelCategory::Entity,
            &mut report,
        );
        let el = &models[0].elements[0];
        // Uninflated Java box: from = [8 - 4.9, 0, 8] = [3.1, 0, 8], to = [8, 12.2, 12.1]
        // Inflated by 0.5: from = [2.6, -0.5, 7.5], to = [8.5, 12.7, 12.6]
        assert!((el.from[0] - 2.6).abs() < 1e-4);
        assert!((el.from[1] - (-0.5)).abs() < 1e-4);
        assert!((el.to[0] - 8.5).abs() < 1e-4);
        assert!((el.to[2] - 12.6).abs() < 1e-4);

        // Floored uninflated size: dx=4, dy=12, dz=4, uv_offset=[16, 16], scale=16/64=0.25
        // Unmirrored Up: [u+dz+dx, v+dz, u+dz, v] = [24, 20, 20, 16]
        // Mirrored Up: horizontal flip => [20, 20, 24, 16], scaled by 0.25 => [5.0, 5.0, 6.0, 4.0]
        assert_eq!(el.faces[&FaceDirection::Up].uv, Some([5.0, 5.0, 6.0, 4.0]));
        // Mirrored East takes West [u+dz+dx, v+dz, +dz, +dy] = [24, 20, +4, +12], flipped X => [28, 20, 24, 32] * 0.25 = [7, 5, 6, 8]
        assert_eq!(
            el.faces[&FaceDirection::East].uv,
            Some([7.0, 5.0, 6.0, 8.0])
        );

        // Verify auto_java_uv_for_face on asymmetric cube [2, 3, 4]..[6, 10, 12]
        assert_eq!(
            auto_java_uv_for_face(FaceDirection::North, [2.0, 3.0, 4.0], [6.0, 10.0, 12.0]),
            [10.0, 6.0, 14.0, 13.0]
        );
        assert_eq!(
            auto_java_uv_for_face(FaceDirection::East, [2.0, 3.0, 4.0], [6.0, 10.0, 12.0]),
            [4.0, 6.0, 12.0, 13.0]
        );
        assert_eq!(
            auto_java_uv_for_face(FaceDirection::Down, [2.0, 3.0, 4.0], [6.0, 10.0, 12.0]),
            [2.0, 4.0, 6.0, 12.0]
        );
    }

    #[test]
    fn se3_parent_bone_rotation_baking_is_exact_and_lossless_for_26_x() {
        // Parent bone rotated 90 deg around Y (Bedrock [0, -90, 0] => Java +90 deg Y) at pivot [0, 0, 0]
        // Child bone has a cube centered at Java (10, 4, 8) (Bedrock origin [-3, 3, -1], size [2, 2, 2]).
        // Rotating (+2, 0, 0) by +90 deg around Y (Java pivot [8, 0, 8]) moves it to (0, 0, -2) => Java center (8, 4, 6)!
        let geo = BedrockGeoFile::from_json_slice(
            br#"{
                "format_version": "1.12.0",
                "minecraft:geometry": [{
                    "description": { "identifier": "geometry.rig", "texture_width": 16, "texture_height": 16 },
                    "bones": [
                        { "name": "root", "pivot": [0, 0, 0], "rotation": [0, -90, 0] },
                        {
                            "name": "child",
                            "parent": "root",
                            "pivot": [0, 0, 0],
                            "cubes": [{
                                "origin": [-3, 3, -1],
                                "size": [2, 2, 2],
                                "pivot": [-2, 4, 0],
                                "rotation": [-30, 0, 0],
                                "uv": { "north": { "uv": [0, 0], "uv_size": [2, 2] } }
                            }]
                        }
                    ]
                }]
            }"#,
        )
        .expect("parse rig");

        let mut report = ConversionReport::new();
        let models = bedrock_geo_to_model_assets_with_options(
            &geo,
            "entity/rig",
            ModelCategory::Entity,
            Some(88),
            RotationSnapMode::Preserve,
            &mut report,
        );
        assert_eq!(report.lossy_count, 0);
        let el = &models[0].elements[0];
        let rot = el.rotation.as_ref().expect("baked rotation");
        // Expected center: (8, 4, 6)
        assert!(
            (rot.origin[0] - 8.0).abs() < 1e-4,
            "origin={:?}",
            rot.origin
        );
        assert!(
            (rot.origin[1] - 4.0).abs() < 1e-4,
            "origin={:?}",
            rot.origin
        );
        assert!(
            (rot.origin[2] - 6.0).abs() < 1e-4,
            "origin={:?}",
            rot.origin
        );
        assert!((el.from[0] - 7.0).abs() < 1e-4);
        assert!((el.to[2] - 7.0).abs() < 1e-4);

        // Verify the composed rotation matrix matches R_y(+90) * R_x(+30)
        let expected_m = mat3_mul(
            euler_zyx_to_mat3([0.0, 90.0, 0.0]),
            euler_zyx_to_mat3([30.0, 0.0, 0.0]),
        );
        let b_euler = rot.extra_bedrock_euler.expect("multi-axis euler");
        let actual_m = euler_zyx_to_mat3([
            -f64::from(b_euler[0]),
            -f64::from(b_euler[1]),
            f64::from(b_euler[2]),
        ]);
        for i in 0..3 {
            for j in 0..3 {
                assert!(
                    (expected_m[i][j] - actual_m[i][j]).abs() < 1e-4,
                    "M[{i}][{j}]: expected {}, got {}",
                    expected_m[i][j],
                    actual_m[i][j]
                );
            }
        }
    }
}
