use std::collections::{BTreeMap, BTreeSet};

use rp_core::{
    BedrockGeoVersion, ConversionReport, Edition, MappingTable, ModelCategory, PipelineStage,
    ResourcePackIR, TextureCategory, WarningSeverity, normalize_bedrock_engine_triplet,
};
use rp_io::{PackContainer, is_os_junk};
use rp_transforms::locale::{
    emit_bedrock_lang, normalize_locale_to_bedrock, transform_locale_keys,
};
use rp_transforms::model::model_asset_to_bedrock_geo_with_head_bind;
use rp_transforms::pbr::convert_labpbr_to_bedrock_texture_set;
use rp_transforms::resolver::resolve_java_model_texture_bindings;
use rp_transforms::texture::{
    AtlasTextureData, AtlasTexturePath, AtlasVariant, FlipbookEntry, ItemTextureJson,
    TerrainTextureJson, java_animation_to_flipbook, resolve_animation_strip_geometry,
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

    let raw_min_engine = ir.metadata.bedrock_min_engine_version.unwrap_or_else(|| {
        mappings.java_pack_format_to_bedrock(ir.metadata.java_pack_format.unwrap_or(15))
    });
    let min_engine = normalize_bedrock_engine_triplet(raw_min_engine);

    let pack_name = if ir.metadata.name.trim().is_empty() {
        String::from("Converted Resource Pack")
    } else {
        ir.metadata.name.clone()
    };

    if let Some(icon) = &ir.metadata.icon_png {
        container.insert("pack_icon.png", icon.as_ref().clone())?;
        report.record_passthrough();
    }

    // 1. Resolve Java blockstates / items / models (parent chains + #texture variables)
    //    so custom/non-vanilla texture names map onto official Bedrock atlas keys.
    let resolved_bindings = resolve_java_model_texture_bindings(ir, mappings);

    let attachable_textures: BTreeSet<String> = ir
        .attachables
        .values()
        .filter_map(|att| att.default_texture.clone())
        .collect();

    let mut emitted_paths: BTreeMap<String, String> = BTreeMap::new();
    let mut terrain_texture_data: BTreeMap<String, AtlasTextureData> = BTreeMap::new();
    let mut item_texture_data: BTreeMap<String, AtlasTextureData> = BTreeMap::new();
    let mut flipbook_entries: Vec<FlipbookEntry> = Vec::new();

    for (tex_id, tex) in &ir.textures {
        let target = mappings.java_texture_to_bedrock(tex_id);
        let is_explicit = mappings.is_explicitly_mapped_java_texture(tex_id);
        let is_vanilla_path = mappings.is_known_bedrock_texture(&target.path);
        let is_attachable_tex = attachable_textures.contains(tex_id);
        let has_preserved_atlas = tex.atlas_key.is_some();
        let is_model_bound = resolved_bindings.bound_textures.contains(tex_id);

        // Do NOT copy raw unreferenced non-vanilla custom textures into Bedrock as dead orphans.
        if !is_explicit
            && !is_vanilla_path
            && !is_attachable_tex
            && !has_preserved_atlas
            && !is_model_bound
        {
            report.record_skipped_detailed(
                format!("assets/minecraft/textures/{tex_id}.png"),
                PipelineStage::Resolve,
                format!(
                    "Custom texture '{tex_id}' does not match any vanilla Bedrock path and is not referenced by any Java blockstate, item definition, model, attachable, or mapping override"
                ),
                "Reference this texture from a vanilla Java model/blockstate or provide an explicit entry in a --mapping JSON file.",
            );
            continue;
        }

        let png_path = format!("{}.png", target.path);
        let bytes = tex.payload.to_png_bytes()?.into_owned();
        container.insert(&png_path, bytes)?;
        emitted_paths.insert(tex_id.clone(), target.path.clone());
        if tex.payload.is_decoded() {
            report.record_converted();
        } else {
            report.record_passthrough();
        }

        // Register directly-mapped vanilla/explicit block or item textures into their atlases.
        if is_explicit || is_vanilla_path || has_preserved_atlas {
            let atlas_key = tex.atlas_key.clone().unwrap_or_else(|| target.atlas_key.clone());
            if tex.category == TextureCategory::Block
                && (mappings.is_known_terrain_key(&atlas_key) || has_preserved_atlas || is_explicit)
            {
                append_atlas_entry(&mut terrain_texture_data, atlas_key.clone(), target.path.clone());
            } else if tex.category == TextureCategory::Item
                && (mappings.is_known_item_key(&atlas_key) || has_preserved_atlas || is_explicit)
            {
                append_atlas_entry(&mut item_texture_data, atlas_key.clone(), target.path.clone());
            }
        }

        if let Some(anim) = &tex.animation {
            let dims = match (tex.width, tex.height) {
                (Some(w), Some(h)) => Some((w, h)),
                _ => tex.payload.dimensions().ok(),
            };
            let resolved_opt = dims.and_then(|(w, h)| {
                resolve_animation_strip_geometry(&png_path, w, h, anim, &mut report)
            });
            if resolved_opt.is_some_and(|r| r.fell_back_to_static) {
                continue;
            }
            let atlas_tile = tex.atlas_key.clone().unwrap_or(target.atlas_key);
            let mut flip = java_animation_to_flipbook(&atlas_tile, &target.path, anim, &mut report);
            if let Some(resolved) = resolved_opt
                && flip.frames.is_none()
                && resolved.frame_height != resolved.frame_width
                && resolved.frame_count > 1
            {
                flip.frames = Some((0..resolved.frame_count).collect());
            }
            flipbook_entries.push(flip);
            report.record_converted();
        }
    }

    // 2. Apply model-graph-resolved vanilla Bedrock terrain & item atlas bindings
    //    (e.g. models/item/apple.json -> block/custom_tex_a -> item_texture["apple"]).
    for (terrain_key, tex_ids) in &resolved_bindings.terrain_bindings {
        let paths: Vec<String> = tex_ids
            .iter()
            .filter_map(|id| emitted_paths.get(id).cloned())
            .collect();
        if let Some(atlas_path) = paths_to_atlas_value(paths) {
            terrain_texture_data.insert(
                terrain_key.clone(),
                AtlasTextureData {
                    textures: atlas_path,
                },
            );
        }
    }
    for (item_key, tex_ids) in &resolved_bindings.item_bindings {
        let paths: Vec<String> = tex_ids
            .iter()
            .filter_map(|id| emitted_paths.get(id).cloned())
            .collect();
        if let Some(atlas_path) = paths_to_atlas_value(paths) {
            item_texture_data.insert(
                item_key.clone(),
                AtlasTextureData {
                    textures: atlas_path,
                },
            );
        }
    }

    // 3. Handle Java LabPBR maps (`_n.png`, `_s.png`):
    //    - If `convert_pbr` is enabled, emit valid Bedrock `*_mer.png`, `*_normal.png`, `*.texture_set.json`.
    //    - Otherwise omit raw files so they never bloat or corrupt the `.mcpack` as orphans.
    let mut emitted_pbr = false;
    for (base_id, pbr) in &ir.pbr_maps {
        if ir.metadata.convert_pbr {
            let Some(bedrock_base) = emitted_paths.get(base_id) else {
                let src_path = pbr
                    .normal_png
                    .as_ref()
                    .or(pbr.specular_png.as_ref())
                    .map(|(p, _)| p.clone())
                    .unwrap_or_else(|| format!("assets/minecraft/textures/{base_id}_n.png"));
                report.record_diagnostic(
                    src_path,
                    PipelineStage::Transform,
                    WarningSeverity::Warning,
                    format!(
                        "LabPBR sidecar map for '{base_id}' was skipped because its base color texture was not emitted into Bedrock"
                    ),
                    "Ensure the base color texture exists and maps to a valid Bedrock texture path or vanilla model.",
                );
                continue;
            };
            let converted = convert_labpbr_to_bedrock_texture_set(
                bedrock_base,
                pbr.normal_png.as_ref().map(|(_, b)| b.as_slice()),
                pbr.specular_png.as_ref().map(|(_, b)| b.as_slice()),
            )?;
            if let Some(out_pbr) = converted {
                if let Some((mer_path, mer_bytes)) = out_pbr.mer_png {
                    container.insert(&mer_path, mer_bytes)?;
                    report.record_converted();
                }
                if let Some((norm_path, norm_bytes)) = out_pbr.normal_png {
                    container.insert(&norm_path, norm_bytes)?;
                    report.record_converted();
                }
                let (ts_path, ts_bytes) = out_pbr.texture_set_json;
                container.insert(&ts_path, ts_bytes)?;
                report.record_converted();
                emitted_pbr = true;
            }
        } else {
            for (kind, entry) in [
                ("normal (_n.png)", &pbr.normal_png),
                ("specular (_s.png)", &pbr.specular_png),
            ] {
                if let Some((src_path, _)) = entry {
                    report.record_diagnostic(
                        src_path.clone(),
                        PipelineStage::Transform,
                        WarningSeverity::Warning,
                        format!(
                            "Omitted raw Java LabPBR {kind} map because Bedrock PBR conversion is disabled (raw _n/_s PNGs are ignored by Bedrock and bloat archive size)"
                        ),
                        "Enable PBR conversion (--convert-pbr) to emit Bedrock *_mer.png + *_normal.png + *.texture_set.json with capability 'pbr'.",
                    );
                }
            }
        }
    }

    // 4. Emit `manifest.json` (adding `"capabilities": ["pbr"]` if texture sets were emitted).
    let mut manifest = BedrockManifest::new(
        &pack_name,
        &ir.metadata.description,
        ir.metadata.version,
        min_engine,
    );
    if emitted_pbr {
        manifest.capabilities.push(String::from("pbr"));
    }
    let manifest_version = ir.metadata.bedrock_manifest_version.unwrap_or(2);
    container.insert(
        "manifest.json",
        manifest.to_json_vec_versioned(manifest_version)?,
    )?;
    report.record_converted();

    // 5. Emit non-empty texture atlases & flipbooks (never emit empty stub JSONs!).
    if !terrain_texture_data.is_empty() {
        let terrain_atlas = TerrainTextureJson {
            resource_pack_name: pack_name.clone(),
            texture_name: String::from("atlas.terrain"),
            padding: Some(8),
            num_mip_levels: Some(4),
            texture_data: terrain_texture_data,
        };
        container.insert(
            "textures/terrain_texture.json",
            serde_json::to_vec_pretty(&terrain_atlas)?,
        )?;
        report.record_converted();
    }

    if !item_texture_data.is_empty() {
        let item_atlas = ItemTextureJson {
            resource_pack_name: pack_name.clone(),
            texture_name: String::from("atlas.items"),
            texture_data: item_texture_data,
        };
        container.insert(
            "textures/item_texture.json",
            serde_json::to_vec_pretty(&item_atlas)?,
        )?;
        report.record_converted();
    }

    if !flipbook_entries.is_empty() {
        container.insert(
            "textures/flipbook_textures.json",
            serde_json::to_vec_pretty(&flipbook_entries)?,
        )?;
        report.record_converted();
    }

    // 6. Emit valid Bedrock geometries:
    //    - NEVER emit dead `models/entity/block/*.geo.json` for vanilla blocks in a pure RP!
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
        if model.category == ModelCategory::Block {
            if !model.elements.is_empty() {
                let extracted = resolved_bindings
                    .model_textures
                    .get(model_id)
                    .map(|list| list.join(", "))
                    .unwrap_or_else(|| String::from("none"));
                report.record_diagnostic(
                    format!("assets/minecraft/models/{model_id}.json"),
                    PipelineStage::Emit,
                    WarningSeverity::Warning,
                    format!(
                        "Omitted dead models/entity/{model_id}.geo.json: pure Bedrock Resource Packs cannot override vanilla 3D block geometry (extracted texture(s): {extracted})"
                    ),
                    "Keep the extracted texture in terrain_texture.json for vanilla blocks, or create a companion Behavior Pack custom block with minecraft:geometry / item attachable for 3D shapes.",
                );
            }
            continue;
        }

        if model.elements.is_empty() {
            report.record_diagnostic(
                format!("assets/minecraft/models/{model_id}.json"),
                PipelineStage::Emit,
                WarningSeverity::Info,
                "Skipped emitting parent-only Java model with no cuboid elements to Bedrock .geo.json",
                "No action needed; referenced textures are handled through item_texture.json / terrain_texture.json.",
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
                    report.record_diagnostic(
                        format!("sounds/{}.ogg", variant.name),
                        PipelineStage::Emit,
                        WarningSeverity::Info,
                        format!(
                            "Sound definition '{bedrock_event}' references sound '{}' not bundled in pack",
                            variant.name
                        ),
                        "Bundle the .ogg audio file under sounds/ or rely on Bedrock's vanilla sound fallback.",
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

    if !ir.splashes.is_empty() {
        let splashes_json = serde_json::json!({ "splashes": ir.splashes });
        container.insert("splashes.json", serde_json::to_vec_pretty(&splashes_json)?)?;
        report.record_converted();
    }

    let mut emitted_languages: BTreeSet<String> = BTreeSet::new();
    for (code, table) in &ir.locales {
        if table.entries.is_empty() {
            continue;
        }
        let bedrock_code = normalize_locale_to_bedrock(code);
        let transformed = transform_locale_keys(table, mappings, Edition::Bedrock);
        let lang_str = emit_bedrock_lang(&transformed);
        let lang_path = format!("texts/{bedrock_code}.lang");
        container.insert(&lang_path, lang_str.into_bytes())?;
        emitted_languages.insert(bedrock_code);
        report.record_converted();
    }

    // Do NOT emit an empty `texts/languages.json` (`[]`) stub when no `.lang` files exist.
    if !emitted_languages.is_empty() {
        let lang_list: Vec<String> = emitted_languages.into_iter().collect();
        container.insert(
            "texts/languages.json",
            serde_json::to_vec_pretty(&lang_list)?,
        )?;
        report.record_converted();
    }

    for (extra_path, extra_bytes) in &ir.passthrough_files {
        if is_forbidden_bedrock_passthrough(extra_path) {
            report.record_skipped_detailed(
                extra_path.clone(),
                PipelineStage::Emit,
                "Blocked Java-only, overlay, or OS junk file from leaking into Bedrock output archive",
                "Remove source-edition or OS metadata files before packaging for Bedrock Edition.",
            );
            continue;
        }
        if !container.contains(extra_path) {
            container.insert(extra_path, extra_bytes.clone())?;
            report.record_passthrough();
        }
    }

    Ok((container, report))
}

fn append_atlas_entry(
    map: &mut BTreeMap<String, AtlasTextureData>,
    key: String,
    path: String,
) {
    match map.get_mut(&key) {
        None => {
            map.insert(
                key,
                AtlasTextureData {
                    textures: AtlasTexturePath::Single(path),
                },
            );
        }
        Some(existing) => match &mut existing.textures {
            AtlasTexturePath::Single(first) => {
                if *first != path {
                    let prev = first.clone();
                    existing.textures = AtlasTexturePath::List(vec![
                        AtlasVariant::Path(prev),
                        AtlasVariant::Path(path),
                    ]);
                }
            }
            AtlasTexturePath::List(list) => {
                let already = list.iter().any(|v| match v {
                    AtlasVariant::Path(p) | AtlasVariant::Object { path: p } => p == &path,
                });
                if !already {
                    list.push(AtlasVariant::Path(path));
                }
            }
        },
    }
}

fn paths_to_atlas_value(paths: Vec<String>) -> Option<AtlasTexturePath> {
    match paths.len() {
        0 => None,
        1 => paths.into_iter().next().map(AtlasTexturePath::Single),
        _ => Some(AtlasTexturePath::List(
            paths.into_iter().map(AtlasVariant::Path).collect(),
        )),
    }
}

fn is_forbidden_bedrock_passthrough(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    is_os_junk(path)
        || lower.starts_with("assets/")
        || lower == "pack.mcmeta"
        || lower == "pack.png"
        || lower.starts_with("overlay")
}
