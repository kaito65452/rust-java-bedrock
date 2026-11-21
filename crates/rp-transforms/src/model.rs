use std::collections::{BTreeMap, BTreeSet};

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
    /// UV extent `[du, dv]`.
    pub uv_size: [f32; 2],
    /// Optional Bedrock material instance name (preserves Java texture variable binding).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material_instance: Option<String>,
}

#[inline]
fn clean_zero(v: f32) -> f32 {
    if v.abs() < 1e-6 { 0.0 } else { v }
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

/// Converts a Bedrock Euler `[rx, ry, rz]` rotation into Java single-axis rotation using
/// target `java_pack_format` rules (`>= 63` allows arbitrary angles) and `RotationSnapMode`.
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

    let extra = if non_zero_count > 1 {
        report.record_lossy(
            asset_id,
            format!(
                "Multi-axis Bedrock rotation {:?} reduced to single Java axis {:?} ({raw_angle} deg)",
                rot, axis
            ),
        );
        Some(rot)
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

    let supports_arbitrary_angles = target_java_pack_format.is_some_and(|fmt| fmt >= 63);
    if !supports_arbitrary_angles && !is_standard_java_angle(angle) {
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

/// Converts Java `[u1, v1, u2, v2]` UV coordinates into Bedrock `{ uv, uv_size }`.
#[must_use]
pub fn java_to_bedrock_uv(uv: [f32; 4]) -> ([f32; 2], [f32; 2]) {
    (
        [clean_zero(uv[0]), clean_zero(uv[1])],
        [clean_zero(uv[2] - uv[0]), clean_zero(uv[3] - uv[1])],
    )
}

/// Converts Bedrock `{ uv, uv_size }` into Java `[u1, v1, u2, v2]` UV coordinates.
#[must_use]
pub fn bedrock_to_java_uv(uv: [f32; 2], uv_size: [f32; 2]) -> [f32; 4] {
    [
        clean_zero(uv[0]),
        clean_zero(uv[1]),
        clean_zero(uv[0] + uv_size[0]),
        clean_zero(uv[1] + uv_size[1]),
    ]
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
        16.0
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
            let (uv, uv_size) = java_to_bedrock_uv(scaled_uv);
            let mat = if face.texture.is_empty() {
                None
            } else {
                Some(face.texture.clone())
            };
            per_face.insert(
                dir.as_str().to_string(),
                BedrockFaceUv {
                    uv,
                    uv_size,
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
        let parent_bone_names: BTreeSet<&str> = geo
            .bones
            .iter()
            .filter_map(|b| b.parent.as_deref())
            .collect();

        for bone in &geo.bones {
            let is_parent_or_mixed = parent_bone_names.contains(bone.name.as_str())
                || bone.cubes.iter().any(|c| c.rotation.is_some());
            for cube in &bone.cubes {
                let (mut from, mut to) = bedrock_to_java_box(cube.origin, cube.size);
                for axis in 0..3 {
                    if from[axis] > to[axis] {
                        std::mem::swap(&mut from[axis], &mut to[axis]);
                    }
                }

                let effective_pivot = cube.pivot.or(bone.pivot);
                let effective_rot = if is_parent_or_mixed {
                    cube.rotation
                } else {
                    cube.rotation.or(bone.rotation)
                };

                let rotation = effective_rot.and_then(|euler| {
                    bedrock_to_java_rotation_with_mode(
                        euler,
                        &model_id,
                        target_java_pack_format,
                        rotation_mode,
                        report,
                    )
                    .map(|(axis, angle, extra)| ElementRotation {
                        origin: effective_pivot
                            .map(bedrock_to_java_pivot)
                            .unwrap_or([8.0, 8.0, 8.0]),
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
                                let raw_uv = bedrock_to_java_uv(face_uv.uv, face_uv.uv_size);
                                let scaled_uv = [
                                    clean_zero(raw_uv[0] * uv_scale_x),
                                    clean_zero(raw_uv[1] * uv_scale_y),
                                    clean_zero(raw_uv[2] * uv_scale_x),
                                    clean_zero(raw_uv[3] * uv_scale_y),
                                ];
                                faces.insert(
                                    dir,
                                    ElementFace {
                                        uv: Some(scaled_uv),
                                        texture: tex_ref,
                                        cullface: None,
                                        rotation: 0,
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
                        let dx = cube.size[0].abs();
                        let dy = cube.size[1].abs();
                        let dz = cube.size[2].abs();
                        for dir in FaceDirection::ALL {
                            let raw = match dir {
                                FaceDirection::Up => [u + dz, *v, u + dz + dx, v + dz],
                                FaceDirection::Down => [u + dz + dx, *v, u + dz + dx + dx, v + dz],
                                FaceDirection::East => [*u, v + dz, u + dz, v + dz + dy],
                                FaceDirection::North => [u + dz, v + dz, u + dz + dx, v + dz + dy],
                                FaceDirection::West => {
                                    [u + dz + dx, v + dz, u + dz + dx + dz, v + dz + dy]
                                }
                                FaceDirection::South => {
                                    [u + dz + dx + dz, v + dz, u + dz + dx + dz + dx, v + dz + dy]
                                }
                            };
                            let clamped = [
                                (raw[0] * uv_scale_x).clamp(0.0, 16.0),
                                (raw[1] * uv_scale_y).clamp(0.0, 16.0),
                                (raw[2] * uv_scale_x).clamp(0.0, 16.0),
                                (raw[3] * uv_scale_y).clamp(0.0, 16.0),
                            ];
                            faces.insert(
                                dir,
                                ElementFace {
                                    uv: Some(clamped),
                                    texture: String::from("#all"),
                                    cullface: None,
                                    rotation: 0,
                                    tintindex: None,
                                },
                            );
                        }
                    }
                }

                elements.push(ModelElement {
                    group: Some(bone.name.clone()),
                    from,
                    to,
                    rotation,
                    faces,
                    shade: true,
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

fn normalize_elements_to_java_bounds(
    model_id: &str,
    geo: &BedrockGeometry,
    elements: &mut [ModelElement],
    report: &mut ConversionReport,
) {
    if elements.is_empty() {
        return;
    }

    // Recenter entity wearables modeled relative to Bedrock's player head bone (pivot Y ~ 24).
    if geo
        .bones
        .iter()
        .any(|b| b.name.eq_ignore_ascii_case("head") && b.pivot.is_some_and(|p| p[1] >= 16.0))
    {
        let max_y = elements
            .iter()
            .map(|e| e.to[1])
            .fold(f32::NEG_INFINITY, f32::max);
        if max_y > 20.0 {
            let mut min_c = [f32::INFINITY; 3];
            let mut max_c = [f32::NEG_INFINITY; 3];
            for el in elements.iter() {
                for i in 0..3 {
                    min_c[i] = min_c[i].min(el.from[i]).min(el.to[i]);
                    max_c[i] = max_c[i].max(el.from[i]).max(el.to[i]);
                }
            }
            for i in 0..3 {
                let shift = 8.0 - 0.5 * (min_c[i] + max_c[i]);
                for el in elements.iter_mut() {
                    el.from[i] = clean_zero(el.from[i] + shift);
                    el.to[i] = clean_zero(el.to[i] + shift);
                    if let Some(rot) = &mut el.rotation {
                        rot.origin[i] = clean_zero(rot.origin[i] + shift);
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

fn auto_java_uv_for_face(dir: FaceDirection, from: [f32; 3], to: [f32; 3]) -> [f32; 4] {
    match dir {
        FaceDirection::North | FaceDirection::South => {
            [from[0], 16.0 - to[1], to[0], 16.0 - from[1]]
        }
        FaceDirection::East | FaceDirection::West => [from[2], 16.0 - to[1], to[2], 16.0 - from[1]],
        FaceDirection::Up | FaceDirection::Down => [from[0], from[2], to[0], to[2]],
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
        assert!((el.from[1] - 0.6).abs() < 1e-4);
        assert!((el.to[1] - 15.4).abs() < 1e-4);
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
}
