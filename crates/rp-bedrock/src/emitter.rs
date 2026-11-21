use std::collections::{BTreeMap, BTreeSet};

use rp_core::{
    BedrockGeoVersion, ConversionReport, Edition, MappingTable, ModelCategory, ResourcePackIR,
    WarningSeverity,
};
use rp_io::PackContainer;
use rp_transforms::locale::{
    emit_bedrock_lang, normalize_locale_to_bedrock, transform_locale_keys,
};
use rp_transforms::model::model_asset_to_bedrock_geo_with_head_bind;
use rp_transforms::texture::{
    FlipbookEntry, build_item_texture_json, build_terrain_texture_json, java_animation_to_flipbook,
};

use crate::BedrockPackError;
use crate::schema::{
    BedrockAttachableBody, BedrockAttachableDescription, BedrockAttachableFile, BedrockManifest,
    BedrockSoundDefJson, BedrockSoundDefinitionsFile,
};

/// Emits a canonical `ResourcePackIR` into a Bedrock Edition `PackContainer`.
pub fn emit_bedrock_pack(
    ir: &ResourcePackIR,
    mappings: &MappingTable,
) -> Result<(PackContainer, ConversionReport), BedrockPackError> {
    let mut container = PackContainer::new();
    let mut report = ConversionReport::new();

    let min_engine = ir.metadata.bedrock_min_engine_version.unwrap_or_else(|| {
        mappings.java_pack_format_to_bedrock(ir.metadata.java_pack_format.unwrap_or(15))
    });

    let pack_name = if ir.metadata.name.trim().is_empty() {
        String::from("Converted Resource Pack")
    } else {
        ir.metadata.name.clone()
    };

    let manifest = BedrockManifest::new(
        &pack_name,
        &ir.metadata.description,
        ir.metadata.version,
        min_engine,
    );
    let manifest_version = ir.metadata.bedrock_manifest_version.unwrap_or(2);
    container.insert(
        "manifest.json",
        manifest.to_json_vec_versioned(manifest_version)?,
    )?;
    report.record_converted();

    if let Some(icon) = &ir.metadata.icon_png {
        container.insert("pack_icon.png", icon.as_ref().clone())?;
        report.record_passthrough();
    }

    let mut flipbook_entries: Vec<FlipbookEntry> = Vec::new();
    for (tex_id, tex) in &ir.textures {
        let target = mappings.java_texture_to_bedrock(tex_id);
        let png_path = format!("{}.png", target.path);
        let bytes = tex.payload.to_png_bytes()?.into_owned();
        container.insert(&png_path, bytes)?;
        if tex.payload.is_decoded() {
            report.record_converted();
        } else {
            report.record_passthrough();
        }

        if let Some(anim) = &tex.animation {
            let atlas_tile = tex.atlas_key.clone().unwrap_or(target.atlas_key);
            let flip = java_animation_to_flipbook(&atlas_tile, &target.path, anim, &mut report);
            flipbook_entries.push(flip);
            report.record_converted();
        }
    }

    let terrain_atlas = build_terrain_texture_json(&pack_name, &ir.textures, mappings);
    container.insert(
        "textures/terrain_texture.json",
        serde_json::to_vec_pretty(&terrain_atlas)?,
    )?;
    report.record_converted();

    let item_atlas = build_item_texture_json(&pack_name, &ir.textures, mappings);
    container.insert(
        "textures/item_texture.json",
        serde_json::to_vec_pretty(&item_atlas)?,
    )?;
    report.record_converted();

    if !flipbook_entries.is_empty() {
        container.insert(
            "textures/flipbook_textures.json",
            serde_json::to_vec_pretty(&flipbook_entries)?,
        )?;
        report.record_converted();
    }

    let geo_version = ir
        .metadata
        .bedrock_geo_version
        .unwrap_or_else(|| BedrockGeoVersion::for_engine_version(min_engine));

    let attachable_targets: BTreeSet<String> = ir
        .attachables
        .values()
        .flat_map(|att| {
            let mut refs = Vec::new();
            let item_name = att
                .item_identifier
                .strip_prefix("minecraft:")
                .unwrap_or(&att.item_identifier);
            refs.push(format!("item/{item_name}"));
            if let Some(geo_id) = &att.default_geometry {
                let stem = geo_id.strip_prefix("geometry.").unwrap_or(geo_id.as_str());
                refs.push(stem.to_string());
                refs.push(format!("entity/{stem}"));
            }
            refs
        })
        .collect();

    for (model_id, model) in &ir.models {
        if model.elements.is_empty() {
            report.record_warning(
                model_id,
                WarningSeverity::Info,
                "Skipped emitting empty parent-only Java model to Bedrock .geo.json",
            );
            continue;
        }
        let bind_to_head = attachable_targets.contains(model_id.as_str())
            || (!ir.attachables.is_empty() && model.category == ModelCategory::Item);
        let geo = model_asset_to_bedrock_geo_with_head_bind(
            model,
            mappings,
            geo_version,
            bind_to_head,
            &mut report,
        );
        let geo_path = if let Some(rest) = model_id.strip_prefix("entity/") {
            format!("models/entity/{rest}.geo.json")
        } else {
            format!("models/entity/{model_id}.geo.json")
        };
        container.insert(&geo_path, geo.to_json_vec_versioned(geo_version)?)?;
        report.record_converted();
    }

    for (att_id, att) in &ir.attachables {
        let mut textures = BTreeMap::new();
        if let Some(tex) = &att.default_texture {
            let target = mappings.java_texture_to_bedrock(tex);
            textures.insert(String::from("default"), target.path);
        }
        let mut geometry = BTreeMap::new();
        if let Some(geo_id) = &att.default_geometry {
            geometry.insert(String::from("default"), geo_id.clone());
        }
        let file = BedrockAttachableFile {
            format_version: String::from("1.10.0"),
            attachable: BedrockAttachableBody {
                description: BedrockAttachableDescription {
                    identifier: att.item_identifier.clone(),
                    item: None,
                    materials: BTreeMap::from([(String::from("default"), String::from("armor"))]),
                    textures,
                    geometry,
                    render_controllers: vec![serde_json::Value::String(String::from(
                        "controller.render.armor",
                    ))],
                },
            },
        };
        let att_path = format!("attachables/{att_id}.json");
        container.insert(&att_path, serde_json::to_vec_pretty(&file)?)?;
        report.record_converted();
    }

    for (sound_id, sound_asset) in &ir.sounds.files {
        let sound_path = format!("sounds/{sound_id}.ogg");
        container.insert(&sound_path, sound_asset.data.as_ref().clone())?;
        report.record_passthrough();
    }

    if !ir.sounds.events.is_empty() {
        let mut sound_defs = BTreeMap::new();
        for (event_name, def) in &ir.sounds.events {
            let bedrock_event = mappings.java_sound_to_bedrock(event_name);
            for variant in &def.sounds {
                if !ir.sounds.files.contains_key(&variant.name) {
                    report.record_warning(
                        format!("sounds/{}.ogg", variant.name),
                        WarningSeverity::Info,
                        format!(
                            "Sound definition '{bedrock_event}' references sound '{}' not bundled in pack",
                            variant.name
                        ),
                    );
                }
            }
            sound_defs.insert(bedrock_event, BedrockSoundDefJson::from(def));
        }

        let file = BedrockSoundDefinitionsFile {
            format_version: String::from("1.14.0"),
            sound_definitions: sound_defs,
        };
        container.insert(
            "sounds/sound_definitions.json",
            serde_json::to_vec_pretty(&file)?,
        )?;
        report.record_converted();
    }

    let mut emitted_languages: BTreeSet<String> = BTreeSet::new();
    for (code, table) in &ir.locales {
        let bedrock_code = normalize_locale_to_bedrock(code);
        let transformed = transform_locale_keys(table, mappings, Edition::Bedrock);
        let lang_str = emit_bedrock_lang(&transformed);
        let lang_path = format!("texts/{bedrock_code}.lang");
        container.insert(&lang_path, lang_str.into_bytes())?;
        emitted_languages.insert(bedrock_code);
        report.record_converted();
    }

    let lang_list: Vec<String> = emitted_languages.into_iter().collect();
    container.insert(
        "texts/languages.json",
        serde_json::to_vec_pretty(&lang_list)?,
    )?;
    report.record_converted();

    for (extra_path, extra_bytes) in &ir.passthrough_files {
        if !container.contains(extra_path) {
            container.insert(extra_path, extra_bytes.clone())?;
            report.record_passthrough();
        }
    }

    Ok((container, report))
}
