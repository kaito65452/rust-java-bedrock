use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use rp_core::{
    AnimationFrame, AnimationSpec, DisplayTransform, ElementFace, ElementRotation, FaceDirection,
    ModelElement, RotationAxis, SoundEntry, SoundEventDef,
};

use crate::JavaPackError;

/// Maximum accepted Java `pack_format` integer.
pub const MAX_JAVA_PACK_FORMAT: u32 = 200;

/// Root `pack.mcmeta` schema for Java Edition resource packs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JavaPackMcmeta {
    /// Core pack header section.
    pub pack: JavaPackSection,
}

/// `pack` section inside `pack.mcmeta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JavaPackSection {
    /// Java resource pack format integer (e.g. `15` for 1.20.1, `34` for 1.21; optional in 1.21.9+ / 26.x).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pack_format: Option<u32>,
    /// Pack description (string or Minecraft JSON text component).
    pub description: Value,
    /// Optional multi-version compatibility range (`1.20.2+`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_formats: Option<Value>,
    /// Minimum supported pack format (`1.21.9+` / `26.x`, integer or `[major, minor]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_format: Option<Value>,
    /// Maximum supported pack format (`1.21.9+` / `26.x`, integer or `[major, minor]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_format: Option<Value>,
}

impl JavaPackMcmeta {
    /// Validates format bounds (`1..=200`) and extracts `(pack_format, supported_range, description)`.
    pub fn validate_and_extract(&self) -> Result<(u32, Option<[u32; 2]>, String), JavaPackError> {
        let max_fmt = parse_format_number(self.pack.max_format.as_ref());
        let min_fmt = parse_format_number(self.pack.min_format.as_ref());
        let supported = parse_supported_formats(self.pack.supported_formats.as_ref());
        let pack_format = self
            .pack
            .pack_format
            .or_else(|| {
                max_fmt
                    .filter(|&f| (1..=MAX_JAVA_PACK_FORMAT).contains(&f))
                    .or_else(|| min_fmt.filter(|&f| (1..=MAX_JAVA_PACK_FORMAT).contains(&f)))
                    .or(min_fmt)
                    .or(max_fmt)
                    .or_else(|| {
                        supported
                            .map(|[lo, _]| lo)
                            .filter(|&f| (1..=MAX_JAVA_PACK_FORMAT).contains(&f))
                    })
            })
            .ok_or(JavaPackError::InvalidPackFormat(0))?;

        if pack_format == 0 || pack_format > MAX_JAVA_PACK_FORMAT {
            return Err(JavaPackError::InvalidPackFormat(pack_format));
        }

        let range = match (min_fmt, max_fmt) {
            (Some(lo), Some(hi)) => Some([lo.min(hi), lo.max(hi)]),
            _ => supported,
        };

        Ok((
            pack_format,
            range,
            extract_text_component(&self.pack.description),
        ))
    }

    /// Validates `pack_format` bounds (`1..=200`) and extracts description text.
    pub fn validate_and_extract_description(&self) -> Result<String, JavaPackError> {
        let (_, _, desc) = self.validate_and_extract()?;
        Ok(desc)
    }
}

fn parse_format_number(val: Option<&Value>) -> Option<u32> {
    match val? {
        Value::Number(n) => n.as_u64().and_then(|v| u32::try_from(v).ok()),
        Value::Array(arr) => arr
            .first()
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok()),
        _ => None,
    }
}

fn parse_supported_formats(val: Option<&Value>) -> Option<[u32; 2]> {
    match val? {
        Value::Number(n) => {
            let v = n.as_u64().and_then(|x| u32::try_from(x).ok())?;
            Some([v, v])
        }
        Value::Array(arr) if arr.len() == 1 => {
            let a = arr[0].as_u64().and_then(|x| u32::try_from(x).ok())?;
            Some([a, a])
        }
        Value::Array(arr) if arr.len() >= 2 => {
            let a = arr[0].as_u64().and_then(|x| u32::try_from(x).ok())?;
            let b = arr[1].as_u64().and_then(|x| u32::try_from(x).ok())?;
            Some([a.min(b), a.max(b)])
        }
        Value::Object(obj) => {
            let min = obj
                .get("min_inclusive")
                .and_then(Value::as_u64)
                .and_then(|x| u32::try_from(x).ok())?;
            let max = obj
                .get("max_inclusive")
                .and_then(Value::as_u64)
                .and_then(|x| u32::try_from(x).ok())
                .unwrap_or(min);
            Some([min.min(max), min.max(max)])
        }
        _ => None,
    }
}

/// Java `1.21.4+` (`pack_format >= 46`) client item definition (`assets/minecraft/items/*.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaItemDefinitionJson {
    /// Root model dispatch specification.
    pub model: JavaItemModelDef,
}

/// Inner `model` block for `assets/minecraft/items/*.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaItemModelDef {
    /// Model type (e.g. `"minecraft:model"`).
    #[serde(rename = "type")]
    pub kind: String,
    /// Target model identifier (e.g. `"minecraft:item/diamond_sword"`).
    pub model: String,
}

fn extract_text_component(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Object(map) => {
            let mut out = map
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(Value::Array(extra)) = map.get("extra") {
                for item in extra {
                    out.push_str(&extract_text_component(item));
                }
            }
            out
        }
        Value::Array(items) => items
            .iter()
            .map(extract_text_component)
            .collect::<Vec<_>>()
            .join(""),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Java `*.png.mcmeta` animation sidecar file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JavaAnimationMcmeta {
    /// Animation configuration block.
    pub animation: Option<JavaAnimationBody>,
}

/// Inner `animation` object of `*.png.mcmeta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct JavaAnimationBody {
    /// Default frame duration in ticks (`1 tick = 0.05s`).
    #[serde(default = "default_frametime")]
    pub frametime: u32,
    /// Whether frames interpolate smoothly.
    #[serde(default)]
    pub interpolate: bool,
    /// Explicit frame width in pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Explicit frame height in pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Optional frame index/time list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<Vec<JavaAnimationFrameEntry>>,
}

fn default_frametime() -> u32 {
    1
}

/// Java `.png.mcmeta` frame entry (integer index or `{index, time}` object).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JavaAnimationFrameEntry {
    /// Simple frame index.
    Index(u32),
    /// Frame index with custom duration in ticks.
    Detailed {
        /// Zero-based strip index.
        index: u32,
        /// Optional duration in ticks.
        time: Option<u32>,
    },
}

impl From<&JavaAnimationBody> for AnimationSpec {
    fn from(body: &JavaAnimationBody) -> Self {
        let frames = body
            .frames
            .as_ref()
            .map(|list| {
                list.iter()
                    .map(|entry| match entry {
                        JavaAnimationFrameEntry::Index(idx) => AnimationFrame {
                            index: *idx,
                            time: None,
                        },
                        JavaAnimationFrameEntry::Detailed { index, time } => AnimationFrame {
                            index: *index,
                            time: *time,
                        },
                    })
                    .collect()
            })
            .unwrap_or_default();

        Self {
            frametime: body.frametime.max(1),
            interpolate: body.interpolate,
            frames,
            frame_width: body.width,
            frame_height: body.height,
        }
    }
}

impl From<&AnimationSpec> for JavaAnimationMcmeta {
    fn from(spec: &AnimationSpec) -> Self {
        let frames = if spec.frames.is_empty() {
            None
        } else {
            Some(
                spec.frames
                    .iter()
                    .map(|f| match f.time {
                        Some(t) => JavaAnimationFrameEntry::Detailed {
                            index: f.index,
                            time: Some(t),
                        },
                        None => JavaAnimationFrameEntry::Index(f.index),
                    })
                    .collect(),
            )
        };

        Self {
            animation: Some(JavaAnimationBody {
                frametime: spec.frametime.max(1),
                interpolate: spec.interpolate,
                width: spec.frame_width,
                height: spec.frame_height,
                frames,
            }),
        }
    }
}

/// Java block/item JSON model (`assets/minecraft/models/**/*.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct JavaModelJson {
    /// Optional parent model reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Ambient occlusion flag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ambientocclusion: Option<bool>,
    /// Texture resolution `[width, height]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texture_size: Option<[u32; 2]>,
    /// Texture variable bindings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub textures: Option<BTreeMap<String, String>>,
    /// Cuboid elements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elements: Option<Vec<JavaModelElementJson>>,
    /// Display transforms by perspective slot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<BTreeMap<String, JavaDisplayTransformJson>>,
    /// Item model predicate overrides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overrides: Option<Vec<Value>>,
}

/// Java model element JSON representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JavaModelElementJson {
    /// Optional element name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Minimum corner `[x, y, z]`.
    pub from: [f32; 3],
    /// Maximum corner `[x, y, z]`.
    pub to: [f32; 3],
    /// Optional element rotation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<JavaElementRotationJson>,
    /// Shading flag (`< 26.3`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shade: Option<bool>,
    /// Java `26.3+` (`pack_format >= 97`) shade direction override (replaces `shade: false`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shade_direction_override: Option<String>,
    /// Java `1.21.4+` / `26.x` per-element light emission (`0..=15`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_emission: Option<u8>,
    /// Per-face UV/texture definitions.
    #[serde(default)]
    pub faces: BTreeMap<String, JavaElementFaceJson>,
}

/// Java element rotation JSON block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JavaElementRotationJson {
    /// Rotation origin `[x, y, z]`.
    pub origin: [f32; 3],
    /// Single-axis rotation axis (`"x"`, `"y"`, `"z"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<RotationAxis>,
    /// Single-axis rotation angle in degrees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle: Option<f32>,
    /// Java `1.21.11+` / `26.x` Euler X rotation in degrees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f32>,
    /// Java `1.21.11+` / `26.x` Euler Y rotation in degrees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f32>,
    /// Java `1.21.11+` / `26.x` Euler Z rotation in degrees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z: Option<f32>,
    /// Whether to rescale cube along non-rotated axes.
    #[serde(default, skip_serializing_if = "is_false")]
    pub rescale: bool,
}

#[inline]
fn clean_zero_f32(v: f32) -> f32 {
    if v.abs() < 1e-6 { 0.0 } else { v }
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// Java face UV/texture JSON block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JavaElementFaceJson {
    /// Optional UV `[u1, v1, u2, v2]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uv: Option<[f32; 4]>,
    /// Texture variable or path.
    pub texture: String,
    /// Optional cullface direction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cullface: Option<FaceDirection>,
    /// Optional texture UV rotation (`0`, `90`, `180`, `270`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<u16>,
    /// Optional tint index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tintindex: Option<i32>,
}

/// Java display perspective transform JSON block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JavaDisplayTransformJson {
    /// Euler rotation.
    #[serde(default)]
    pub rotation: [f32; 3],
    /// Translation vector.
    #[serde(default)]
    pub translation: [f32; 3],
    /// Scale vector.
    #[serde(default = "default_scale")]
    pub scale: [f32; 3],
}

fn default_scale() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

impl From<&JavaDisplayTransformJson> for DisplayTransform {
    fn from(d: &JavaDisplayTransformJson) -> Self {
        Self {
            rotation: d.rotation,
            translation: d.translation,
            scale: d.scale,
        }
    }
}

impl From<&DisplayTransform> for JavaDisplayTransformJson {
    fn from(d: &DisplayTransform) -> Self {
        Self {
            rotation: d.rotation,
            translation: d.translation,
            scale: d.scale,
        }
    }
}

impl JavaModelElementJson {
    /// Converts a JSON element into canonical `ModelElement`.
    pub fn to_ir(&self) -> Result<ModelElement, JavaPackError> {
        let mut faces = BTreeMap::new();
        for (face_name, face_json) in &self.faces {
            let dir = parse_face(face_name).ok_or_else(|| JavaPackError::ParseError {
                path: String::from("model.elements.faces"),
                detail: format!("unknown face direction '{face_name}'"),
            })?;
            faces.insert(
                dir,
                ElementFace {
                    uv: face_json.uv,
                    texture: face_json.texture.clone(),
                    cullface: face_json.cullface,
                    rotation: face_json.rotation.unwrap_or(0),
                    tintindex: face_json.tintindex,
                },
            );
        }

        let rotation = match &self.rotation {
            Some(r) => match (r.axis, r.angle) {
                (Some(axis), Some(angle)) => Some(ElementRotation {
                    origin: r.origin,
                    axis,
                    angle,
                    rescale: r.rescale,
                    extra_bedrock_euler: None,
                }),
                _ if r.x.is_some() || r.y.is_some() || r.z.is_some() => {
                    let rx = r.x.unwrap_or(0.0);
                    let ry = r.y.unwrap_or(0.0);
                    let rz = r.z.unwrap_or(0.0);
                    let eps = 1e-5;
                    let non_zero_count = [rx, ry, rz].iter().filter(|v| v.abs() > eps).count();
                    if non_zero_count == 0 {
                        None
                    } else {
                        let (axis, angle) = if rx.abs() >= ry.abs() && rx.abs() >= rz.abs() {
                            (RotationAxis::X, clean_zero_f32(rx))
                        } else if ry.abs() >= rx.abs() && ry.abs() >= rz.abs() {
                            (RotationAxis::Y, clean_zero_f32(ry))
                        } else {
                            (RotationAxis::Z, clean_zero_f32(rz))
                        };
                        let extra_bedrock_euler = if non_zero_count > 1 || angle.abs() > 45.0 {
                            Some([clean_zero_f32(-rx), clean_zero_f32(-ry), clean_zero_f32(rz)])
                        } else {
                            None
                        };
                        Some(ElementRotation {
                            origin: r.origin,
                            axis,
                            angle,
                            rescale: r.rescale,
                            extra_bedrock_euler,
                        })
                    }
                }
                _ => {
                    return Err(JavaPackError::ParseError {
                        path: String::from("model.elements.rotation"),
                        detail: String::from(
                            "expected either 'axis'/'angle' or Euler 'x'/'y'/'z' rotation fields",
                        ),
                    });
                }
            },
            None => None,
        };

        let shade = match (self.shade, &self.shade_direction_override) {
            (Some(s), _) => s,
            (None, Some(_)) => false,
            (None, None) => true,
        };

        Ok(ModelElement {
            group: self.name.clone(),
            from: self.from,
            to: self.to,
            rotation,
            faces,
            shade,
            shade_direction_override: self.shade_direction_override.clone(),
            light_emission: self.light_emission,
        })
    }

    /// Converts a canonical `ModelElement` into legacy Java JSON representation (`pack_format < 75`).
    #[must_use]
    pub fn from_ir(el: &ModelElement) -> Self {
        Self::from_ir_versioned(el, 15)
    }

    /// Converts a canonical `ModelElement` into version-aware Java JSON representation:
    /// - `pack_format >= 75` (`1.21.11+` / `26.x`): emits `{x, y, z, origin}` when multi-axis or `|angle| > 45°`
    /// - `pack_format >= 97` (`26.3+`): emits `shade_direction_override` instead of `shade: false`
    #[must_use]
    pub fn from_ir_versioned(el: &ModelElement, pack_format: u32) -> Self {
        let mut faces = BTreeMap::new();
        for (dir, face) in &el.faces {
            faces.insert(
                dir.as_str().to_string(),
                JavaElementFaceJson {
                    uv: face.uv,
                    texture: face.texture.clone(),
                    cullface: face.cullface,
                    rotation: if face.rotation == 0 {
                        None
                    } else {
                        Some(face.rotation)
                    },
                    tintindex: face.tintindex,
                },
            );
        }

        let rotation = el.rotation.as_ref().map(|r| {
            let [rx, ry, rz] = if let Some(b) = r.extra_bedrock_euler {
                [
                    clean_zero_f32(-b[0]),
                    clean_zero_f32(-b[1]),
                    clean_zero_f32(b[2]),
                ]
            } else {
                match r.axis {
                    RotationAxis::X => [clean_zero_f32(r.angle), 0.0, 0.0],
                    RotationAxis::Y => [0.0, clean_zero_f32(r.angle), 0.0],
                    RotationAxis::Z => [0.0, 0.0, clean_zero_f32(r.angle)],
                }
            };
            let non_zero_axes = [rx, ry, rz].iter().filter(|v| v.abs() > 1e-5).count();
            let exceeds_45 = [rx, ry, rz].iter().any(|v| v.abs() > 45.0 + 1e-4);

            if pack_format >= 75 && (non_zero_axes > 1 || exceeds_45) {
                JavaElementRotationJson {
                    origin: r.origin,
                    axis: None,
                    angle: None,
                    x: Some(rx),
                    y: Some(ry),
                    z: Some(rz),
                    rescale: r.rescale,
                }
            } else {
                JavaElementRotationJson {
                    origin: r.origin,
                    axis: Some(r.axis),
                    angle: Some(r.angle),
                    x: None,
                    y: None,
                    z: None,
                    rescale: r.rescale,
                }
            }
        });

        let (shade, shade_direction_override) = if pack_format >= 97 {
            let override_dir = el
                .shade_direction_override
                .clone()
                .or_else(|| (!el.shade).then(|| String::from("up")));
            (None, override_dir)
        } else {
            let legacy_shade = if !el.shade || el.shade_direction_override.is_some() {
                Some(false)
            } else {
                None
            };
            (legacy_shade, None)
        };

        Self {
            name: el.group.clone(),
            from: el.from,
            to: el.to,
            rotation,
            shade,
            shade_direction_override,
            light_emission: el.light_emission.filter(|&v| v > 0),
            faces,
        }
    }
}

fn parse_face(name: &str) -> Option<FaceDirection> {
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

/// Java `assets/minecraft/sounds.json` sound event entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct JavaSoundEventJson {
    /// Sound category.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Subtitle localization key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Replace vanilla sounds flag.
    #[serde(default, skip_serializing_if = "is_false")]
    pub replace: bool,
    /// Sound variants.
    #[serde(default)]
    pub sounds: Vec<JavaSoundVariantJson>,
}

/// Sound variant entry in `sounds.json` (plain path or detailed object).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JavaSoundVariantJson {
    /// Simple sound path (e.g. `"mob/zombie/say1"`).
    Simple(String),
    /// Detailed sound entry object.
    Detailed {
        /// Sound path or nested event name.
        name: String,
        /// Volume multiplier.
        #[serde(skip_serializing_if = "Option::is_none")]
        volume: Option<f32>,
        /// Pitch multiplier.
        #[serde(skip_serializing_if = "Option::is_none")]
        pitch: Option<f32>,
        /// Random selection weight.
        #[serde(skip_serializing_if = "Option::is_none")]
        weight: Option<u32>,
        /// Stream from disk flag.
        #[serde(skip_serializing_if = "Option::is_none")]
        stream: Option<bool>,
        /// Optional `"sound"` or `"event"` type.
        #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
    },
}

impl From<&JavaSoundEventJson> for SoundEventDef {
    fn from(ev: &JavaSoundEventJson) -> Self {
        let sounds = ev
            .sounds
            .iter()
            .map(|s| match s {
                JavaSoundVariantJson::Simple(name) => SoundEntry {
                    name: normalize_sound_id(name),
                    ..SoundEntry::default()
                },
                JavaSoundVariantJson::Detailed {
                    name,
                    volume,
                    pitch,
                    weight,
                    stream,
                    ..
                } => SoundEntry {
                    name: normalize_sound_id(name),
                    volume: volume.unwrap_or(1.0),
                    pitch: pitch.unwrap_or(1.0),
                    weight: weight.unwrap_or(1),
                    stream: stream.unwrap_or(false),
                },
            })
            .collect();

        Self {
            category: ev.category.clone(),
            subtitle: ev.subtitle.clone(),
            replace: ev.replace,
            sounds,
        }
    }
}

impl From<&SoundEventDef> for JavaSoundEventJson {
    fn from(ev: &SoundEventDef) -> Self {
        let sounds = ev
            .sounds
            .iter()
            .map(|s| {
                let is_default = (s.volume - 1.0).abs() < 1e-6
                    && (s.pitch - 1.0).abs() < 1e-6
                    && s.weight == 1
                    && !s.stream;
                if is_default {
                    JavaSoundVariantJson::Simple(s.name.clone())
                } else {
                    JavaSoundVariantJson::Detailed {
                        name: s.name.clone(),
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
                        kind: None,
                    }
                }
            })
            .collect();

        Self {
            category: ev.category.clone(),
            subtitle: ev.subtitle.clone(),
            replace: ev.replace,
            sounds,
        }
    }
}

fn normalize_sound_id(raw: &str) -> String {
    raw.trim()
        .strip_prefix("minecraft:")
        .unwrap_or(raw.trim())
        .trim_start_matches('/')
        .strip_suffix(".ogg")
        .unwrap_or(
            raw.trim()
                .strip_prefix("minecraft:")
                .unwrap_or(raw.trim())
                .trim_start_matches('/'),
        )
        .to_string()
}
