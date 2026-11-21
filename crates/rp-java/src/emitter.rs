use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use rp_core::{
    ConversionReport, Edition, MappingTable, ModelCategory, ResourcePackIR,
    sanitize_java_resource_path,
};
use rp_io::PackContainer;
use rp_transforms::locale::{
    emit_bedrock_lang, emit_java_locale_json, normalize_locale_to_bedrock,
    normalize_locale_to_java, transform_locale_keys,
};

use crate::JavaPackError;
use crate::schema::{
    JavaAnimationMcmeta, JavaDisplayTransformJson, JavaItemDefinitionJson, JavaItemModelDef,
    JavaModelElementJson, JavaModelJson, JavaPackMcmeta, JavaPackSection, JavaSoundEventJson,
    JavaSoundVariantJson, MAX_JAVA_PACK_FORMAT,
};

/// Emits a canonical `ResourcePackIR` into a Java Edition `PackContainer`.
pub fn emit_java_pack(
    ir: &ResourcePackIR,
    mappings: &MappingTable,
) -> Result<(PackContainer, ConversionReport), JavaPackError> {
    let mut container = PackContainer::new();
    let mut report = ConversionReport::new();

    let pack_format = ir
        .metadata
        .java_pack_format
        .or_else(|| {
            ir.metadata
                .bedrock_min_engine_version
                .map(|v| mappings.bedrock_engine_to_java_pack_format(v))
        })
        .unwrap_or(15);

    if pack_format == 0 || pack_format > MAX_JAVA_PACK_FORMAT {
        return Err(JavaPackError::InvalidPackFormat(pack_format));
    }

    let range = ir.metadata.java_supported_formats;
    let (emit_pack_format, supported_formats, min_format, max_format) = if pack_format >= 69 {
        let [lo, hi] = range.unwrap_or([pack_format, pack_format]);
        let legacy_compat = if lo < 69 { Some(pack_format) } else { None };
        let legacy_sup = if lo < 69 {
            Some(json!({ "min_inclusive": lo, "max_inclusive": hi }))
        } else {
            None
        };
        (
            legacy_compat,
            legacy_sup,
            Some(json!([lo, 0])),
            Some(json!([hi, 0])),
        )
    } else if pack_format >= 18 {
        let sup = range.map(|[lo, hi]| json!({ "min_inclusive": lo, "max_inclusive": hi }));
        (Some(pack_format), sup, None, None)
    } else {
        (Some(pack_format), None, None, None)
    };

    let mcmeta = JavaPackMcmeta {
        pack: JavaPackSection {
            pack_format: emit_pack_format,
            description: Value::String(ir.metadata.description.clone()),
            supported_formats,
            min_format,
            max_format,
        },
    };
    container.insert("pack.mcmeta", serde_json::to_vec_pretty(&mcmeta)?)?;
    report.record_converted();

    if let Some(icon) = &ir.metadata.icon_png {
        container.insert("pack.png", icon.as_ref().clone())?;
        report.record_passthrough();
    }

    let is_pre_flattening = pack_format <= 3;
    let sanitize_paths = pack_format >= 3;

    for (tex_id, tex) in &ir.textures {
        let base_id = if is_pre_flattening {
            to_pre_flattening_tex_id(tex_id)
        } else {
            tex_id.clone()
        };
        let emitted_id = format_java_id(&base_id, sanitize_paths);
        let png_path = format!("assets/minecraft/textures/{emitted_id}.png");
        let bytes = tex.payload.to_png_bytes()?.into_owned();
        container.insert(&png_path, bytes)?;
        if tex.payload.is_decoded() {
            report.record_converted();
        } else {
            report.record_passthrough();
        }

        if let Some(anim) = &tex.animation {
            let mcmeta_anim = JavaAnimationMcmeta::from(anim);
            let mcmeta_path = format!("{png_path}.mcmeta");
            container.insert(&mcmeta_path, serde_json::to_vec_pretty(&mcmeta_anim)?)?;
            report.record_converted();
        }
    }

    let emit_item_defs = ir
        .metadata
        .emit_java_items_definitions
        .unwrap_or(pack_format >= 46);
    let mut extra_atlas_dirs: BTreeSet<String> = BTreeSet::new();

    for (model_id, model) in &ir.models {
        let clean_model_id = format_java_id(model_id, sanitize_paths);

        let mut tex_map: BTreeMap<String, String> = model
            .textures
            .iter()
            .map(|(k, v)| {
                let mapped_v = if v.starts_with('#') {
                    v.clone()
                } else {
                    let pre = if is_pre_flattening {
                        to_pre_flattening_tex_id(v)
                    } else {
                        v.clone()
                    };
                    format_java_id(&pre, sanitize_paths)
                };
                if let Some(dir) = atlas_dir_for_texture(&mapped_v) {
                    extra_atlas_dirs.insert(dir);
                }
                (k.clone(), mapped_v)
            })
            .collect();

        if !tex_map.is_empty() && !tex_map.contains_key("particle") {
            let fallback_tex = tex_map
                .get("all")
                .cloned()
                .or_else(|| tex_map.values().next().cloned());
            if let Some(particle_val) = fallback_tex {
                tex_map.insert(String::from("particle"), particle_val);
            }
        }

        let elements = if model.elements.is_empty() {
            None
        } else {
            Some(
                model
                    .elements
                    .iter()
                    .map(|el| {
                        let mut json_el = JavaModelElementJson::from_ir(el);
                        for face in json_el.faces.values_mut() {
                            if !face.texture.starts_with('#') && tex_map.contains_key(&face.texture)
                            {
                                face.texture = format!("#{}", face.texture);
                            }
                        }
                        json_el
                    })
                    .collect(),
            )
        };

        let display = if model.display.is_empty() {
            None
        } else {
            Some(
                model
                    .display
                    .iter()
                    .map(|(k, v)| (k.clone(), JavaDisplayTransformJson::from(v)))
                    .collect(),
            )
        };
        let textures = if tex_map.is_empty() {
            None
        } else {
            Some(tex_map)
        };

        let model_json = JavaModelJson {
            parent: model.parent.clone(),
            ambientocclusion: if model.ambient_occlusion {
                None
            } else {
                Some(false)
            },
            texture_size: if model.texture_size == [16, 16] {
                None
            } else {
                Some(model.texture_size)
            },
            textures,
            elements,
            display,
            overrides: None,
        };

        let model_path = format!("assets/minecraft/models/{clean_model_id}.json");
        container.insert(&model_path, serde_json::to_vec_pretty(&model_json)?)?;
        report.record_converted();

        if emit_item_defs
            && model.category == ModelCategory::Item
            && let Some(item_stem) = clean_model_id.strip_prefix("item/")
        {
            let item_def = JavaItemDefinitionJson {
                model: JavaItemModelDef {
                    kind: String::from("minecraft:model"),
                    model: format!("minecraft:item/{item_stem}"),
                },
            };
            let item_def_path = format!("assets/minecraft/items/{item_stem}.json");
            container.insert(&item_def_path, serde_json::to_vec_pretty(&item_def)?)?;
            report.record_converted();
        }
    }

    if pack_format >= 12
        && !extra_atlas_dirs.is_empty()
        && !ir
            .passthrough_files
            .contains_key("assets/minecraft/atlases/blocks.json")
    {
        let sources: Vec<Value> = extra_atlas_dirs
            .into_iter()
            .map(|dir| {
                json!({
                    "type": "minecraft:directory",
                    "source": dir,
                    "prefix": format!("{dir}/")
                })
            })
            .collect();
        let atlas_json = json!({ "sources": sources });
        container.insert(
            "assets/minecraft/atlases/blocks.json",
            serde_json::to_vec_pretty(&atlas_json)?,
        )?;
        report.record_converted();
    }

    for (sound_id, sound_asset) in &ir.sounds.files {
        let clean_sound_id = format_java_id(sound_id, sanitize_paths);
        let sound_path = format!("assets/minecraft/sounds/{clean_sound_id}.ogg");
        container.insert(&sound_path, sound_asset.data.as_ref().clone())?;
        report.record_passthrough();
    }

    if !ir.sounds.events.is_empty() {
        let mut sounds_map: BTreeMap<String, JavaSoundEventJson> = BTreeMap::new();
        for (event_name, def) in &ir.sounds.events {
            let mut ev_json = JavaSoundEventJson::from(def);
            if sanitize_paths {
                for variant in &mut ev_json.sounds {
                    match variant {
                        JavaSoundVariantJson::Simple(name) => {
                            *name = sanitize_java_resource_path(name);
                        }
                        JavaSoundVariantJson::Detailed { name, .. } => {
                            *name = sanitize_java_resource_path(name);
                        }
                    }
                }
            }
            sounds_map.insert(event_name.clone(), ev_json);
        }
        container.insert(
            "assets/minecraft/sounds.json",
            serde_json::to_vec_pretty(&sounds_map)?,
        )?;
        report.record_converted();
    }

    for (code, table) in &ir.locales {
        if is_pre_flattening {
            let lang_code = if pack_format <= 2 {
                normalize_locale_to_bedrock(code)
            } else {
                normalize_locale_to_java(code)
            };
            let transformed = transform_locale_keys(table, mappings, Edition::Bedrock);
            let lang_str = emit_bedrock_lang(&transformed);
            let lang_path = format!("assets/minecraft/lang/{lang_code}.lang");
            container.insert(&lang_path, lang_str.into_bytes())?;
            report.record_converted();
        } else {
            let java_code = normalize_locale_to_java(code);
            let json_str = emit_java_locale_json(table)?;
            let lang_path = format!("assets/minecraft/lang/{java_code}.json");
            container.insert(&lang_path, json_str.into_bytes())?;
            report.record_converted();
        }
    }

    for (extra_path, extra_bytes) in &ir.passthrough_files {
        if !container.contains(extra_path) {
            container.insert(extra_path, extra_bytes.clone())?;
            report.record_passthrough();
        }
    }

    Ok((container, report))
}

fn format_java_id(raw: &str, sanitize: bool) -> String {
    if sanitize {
        sanitize_java_resource_path(raw)
    } else {
        raw.to_string()
    }
}

fn to_pre_flattening_tex_id(id: &str) -> String {
    if let Some(rest) = id.strip_prefix("block/") {
        format!("blocks/{rest}")
    } else if let Some(rest) = id.strip_prefix("item/") {
        format!("items/{rest}")
    } else {
        id.to_string()
    }
}

fn atlas_dir_for_texture(tex_val: &str) -> Option<String> {
    let clean = tex_val.strip_prefix("minecraft:").unwrap_or(tex_val);
    if clean.starts_with('#')
        || clean.starts_with("block/")
        || clean.starts_with("item/")
        || clean.starts_with("blocks/")
        || clean.starts_with("items/")
    {
        return None;
    }
    if clean.starts_with("models/armor/") {
        return Some(String::from("models/armor"));
    }
    clean
        .split('/')
        .next()
        .filter(|seg| !seg.is_empty())
        .map(|seg| {
            if seg == "models" {
                clean
                    .rsplit_once('/')
                    .map_or_else(|| seg.to_string(), |(dir, _)| dir.to_string())
            } else {
                seg.to_string()
            }
        })
}
