use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use rp_core::{
    ConversionReport, Edition, MappingTable, ModelAsset, ModelCategory, ResourcePackIR,
    TextureAsset, sanitize_java_resource_path,
};
use rp_io::PackContainer;
use rp_transforms::locale::{
    emit_bedrock_lang, emit_java_locale_json, normalize_locale_to_bedrock,
    normalize_locale_to_java, transform_locale_keys,
};
use rp_transforms::texture::render_model_to_humanoid_helmet_png;

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
        let hi_minor = if range.is_none() && hi == 97 { 1 } else { 0 };
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
            Some(json!([hi, hi_minor])),
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
    let mut emitted_tex_ids: BTreeSet<String> = BTreeSet::new();

    for (tex_id, tex) in &ir.textures {
        let base_id = if is_pre_flattening {
            to_pre_flattening_tex_id(tex_id)
        } else {
            tex_id.clone()
        };
        let emitted_id = format_java_id(&base_id, sanitize_paths);
        emitted_tex_ids.insert(emitted_id.clone());
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
    let mut extra_atlas_singles: BTreeSet<String> = BTreeSet::new();

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
                let tex_exists = emitted_tex_ids.contains(&mapped_v)
                    || ir.textures.contains_key(&mapped_v)
                    || ir.textures.contains_key(v);
                if tex_exists {
                    let clean_ref = mapped_v.strip_prefix("minecraft:").unwrap_or(&mapped_v);
                    if !clean_ref.starts_with('#')
                        && !clean_ref.starts_with("block/")
                        && !clean_ref.starts_with("item/")
                        && !clean_ref.starts_with("blocks/")
                        && !clean_ref.starts_with("items/")
                    {
                        if clean_ref.starts_with("models/armor/") {
                            extra_atlas_dirs.insert(String::from("models/armor"));
                        } else {
                            extra_atlas_singles.insert(clean_ref.to_string());
                        }
                    }
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
                        let mut json_el = JavaModelElementJson::from_ir_versioned(el, pack_format);
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

    if pack_format >= 12 && (!extra_atlas_dirs.is_empty() || !extra_atlas_singles.is_empty()) {
        let mut sources: Vec<Value> = extra_atlas_dirs
            .into_iter()
            .map(|dir| {
                json!({
                    "type": "minecraft:directory",
                    "source": dir,
                    "prefix": format!("{dir}/")
                })
            })
            .collect();
        for res in extra_atlas_singles {
            sources.push(json!({
                "type": "minecraft:single",
                "resource": res
            }));
        }
        let atlas_json = json!({ "sources": sources });
        let atlas_bytes = serde_json::to_vec_pretty(&atlas_json)?;
        if !ir
            .passthrough_files
            .contains_key("assets/minecraft/atlases/blocks.json")
        {
            container.insert("assets/minecraft/atlases/blocks.json", atlas_bytes.clone())?;
            report.record_converted();
        }
        if pack_format >= 46
            && !ir
                .passthrough_files
                .contains_key("assets/minecraft/atlases/items.json")
        {
            container.insert("assets/minecraft/atlases/items.json", atlas_bytes)?;
            report.record_converted();
        }
    }

    for att in ir.attachables.values() {
        let item_stem = att
            .item_identifier
            .strip_prefix("minecraft:")
            .unwrap_or(&att.item_identifier);
        let Some(material_name) = helmet_material_for_item(item_stem) else {
            continue;
        };
        let Some((model, tex)) = find_attachable_model_and_texture(ir, att, item_stem) else {
            continue;
        };
        let armor_png = render_model_to_humanoid_helmet_png(model, &tex.payload)?;
        let equip_path =
            format!("assets/minecraft/textures/entity/equipment/humanoid/{material_name}.png");
        if !container.contains(&equip_path) {
            container.insert(&equip_path, armor_png.clone())?;
            report.record_converted();
        }
        let legacy_armor_path =
            format!("assets/minecraft/textures/models/armor/{material_name}_layer_1.png");
        if !container.contains(&legacy_armor_path) {
            container.insert(&legacy_armor_path, armor_png)?;
            report.record_converted();
        }
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

fn helmet_material_for_item(item_stem: &str) -> Option<&'static str> {
    match item_stem {
        "diamond_helmet" => Some("diamond"),
        "netherite_helmet" => Some("netherite"),
        "iron_helmet" => Some("iron"),
        "golden_helmet" => Some("gold"),
        "chainmail_helmet" => Some("chainmail"),
        "leather_helmet" => Some("leather"),
        "turtle_helmet" => Some("turtle_scute"),
        _ => None,
    }
}

fn find_attachable_model_and_texture<'a>(
    ir: &'a ResourcePackIR,
    att: &rp_core::AttachableAsset,
    item_stem: &str,
) -> Option<(&'a ModelAsset, &'a TextureAsset)> {
    let model = ir.models.get(&format!("item/{item_stem}")).or_else(|| {
        att.default_geometry.as_ref().and_then(|geo| {
            let stem = geo.strip_prefix("geometry.").unwrap_or(geo.as_str());
            ir.models
                .get(&format!("entity/{stem}"))
                .or_else(|| ir.models.get(stem))
        })
    })?;
    let tex_id = att
        .default_texture
        .as_deref()
        .or_else(|| model.textures.get("all").map(String::as_str))?;
    let tex = ir.textures.get(tex_id).or_else(|| {
        ir.textures
            .iter()
            .find(|(k, _)| {
                k.eq_ignore_ascii_case(tex_id)
                    || sanitize_java_resource_path(k) == sanitize_java_resource_path(tex_id)
            })
            .map(|(_, v)| v)
    })?;
    Some((model, tex))
}
