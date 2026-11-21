use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use rp_core::{
    AnimationSpec, AttachableAsset, BedrockGeoVersion, ConversionReport, DisplayTransform, Edition,
    MappingTable, ModelAsset, ModelCategory, PackMetadata, ResourcePackIR, RotationSnapMode,
    SoundAsset, TextureAsset, TextureCategory, TexturePayload, WarningSeverity,
    sanitize_java_resource_path, sanitize_jsonc_bytes,
};
use rp_io::PackContainer;
use rp_transforms::locale::{normalize_locale_to_java, parse_bedrock_lang, transform_locale_keys};
use rp_transforms::model::{BedrockGeoFile, bedrock_geo_to_model_assets_with_options};
use rp_transforms::sound::validate_sound_asset_ogg;
use rp_transforms::texture::{
    AtlasTexturePath, AtlasVariant, FlipbookEntry, ItemTextureJson, TerrainTextureJson,
    compute_animation_strip_geometry, flipbook_to_java_animation, inspect_png_dimensions,
};

use crate::BedrockPackError;
use crate::schema::{BedrockAttachableFile, BedrockManifest, BedrockSoundDefinitionsInput};

/// Parses a Bedrock Edition `PackContainer` into canonical `ResourcePackIR`
/// without eagerly decoding PNG pixel data.
pub fn parse_bedrock_pack(
    container: &PackContainer,
    mappings: &MappingTable,
) -> Result<(ResourcePackIR, ConversionReport), BedrockPackError> {
    parse_bedrock_pack_with_options(container, mappings, None, RotationSnapMode::Preserve)
}

/// Parses a Bedrock Edition `PackContainer` with explicit target Java `pack_format`
/// and `RotationSnapMode` for angle validation/snapping.
pub fn parse_bedrock_pack_with_options(
    container: &PackContainer,
    mappings: &MappingTable,
    target_java_pack_format: Option<u32>,
    rotation_mode: RotationSnapMode,
) -> Result<(ResourcePackIR, ConversionReport), BedrockPackError> {
    let mut report = ConversionReport::new();

    let manifest_bytes = container
        .get("manifest.json")
        .ok_or(BedrockPackError::MissingManifest)?;
    let manifest_clean = sanitize_jsonc_bytes(manifest_bytes);
    let manifest: BedrockManifest =
        serde_json::from_slice(&manifest_clean).map_err(|err| BedrockPackError::ParseError {
            path: String::from("manifest.json"),
            detail: err.to_string(),
        })?;
    manifest.validate()?;

    let module_uuid = manifest.modules.first().map(|m| m.uuid);
    let min_engine = manifest.header.min_engine_version;
    let mapped_java_pack_format = mappings.bedrock_engine_to_java_pack_format(min_engine);
    let effective_java_pack_format = target_java_pack_format.unwrap_or(mapped_java_pack_format);

    let icon_png = if let Some(icon_bytes) = container.get("pack_icon.png") {
        inspect_png_dimensions(icon_bytes)?;
        Some(Arc::new(icon_bytes.to_vec()))
    } else {
        None
    };

    // Parallel header-only PNG dimension verification via rayon.
    let png_dims: BTreeMap<String, (u32, u32)> = container
        .par_map_files(|path, bytes| {
            if is_bedrock_texture_png(path) {
                let dims = inspect_png_dimensions(bytes)?;
                Ok::<_, rp_core::CoreError>(Some((path.to_string(), dims)))
            } else {
                Ok(None)
            }
        })?
        .into_iter()
        .flatten()
        .collect();

    // Parse optional terrain_texture.json & item_texture.json for atlas key recovery.
    let mut atlas_lookup: HashMap<String, (String, TextureCategory)> = HashMap::new();
    if let Some(terrain_bytes) = container.get("textures/terrain_texture.json") {
        let terrain_clean = sanitize_jsonc_bytes(terrain_bytes);
        let terrain: TerrainTextureJson =
            serde_json::from_slice(&terrain_clean).map_err(|err| BedrockPackError::ParseError {
                path: String::from("textures/terrain_texture.json"),
                detail: err.to_string(),
            })?;
        for (atlas_key, data) in terrain.texture_data {
            for p in extract_atlas_paths(&data.textures) {
                atlas_lookup.insert(
                    normalize_bedrock_tex_key(&p),
                    (atlas_key.clone(), TextureCategory::Block),
                );
            }
        }
    }
    if let Some(item_bytes) = container.get("textures/item_texture.json") {
        let item_clean = sanitize_jsonc_bytes(item_bytes);
        let items: ItemTextureJson =
            serde_json::from_slice(&item_clean).map_err(|err| BedrockPackError::ParseError {
                path: String::from("textures/item_texture.json"),
                detail: err.to_string(),
            })?;
        for (atlas_key, data) in items.texture_data {
            for p in extract_atlas_paths(&data.textures) {
                atlas_lookup.insert(
                    normalize_bedrock_tex_key(&p),
                    (atlas_key.clone(), TextureCategory::Item),
                );
            }
        }
    }

    let mut pending_animations: BTreeMap<String, (AnimationSpec, Option<String>)> = BTreeMap::new();
    if let Some(flipbook_bytes) = container.get("textures/flipbook_textures.json") {
        let flipbook_clean = sanitize_jsonc_bytes(flipbook_bytes);
        let entries: Vec<FlipbookEntry> =
            serde_json::from_slice(&flipbook_clean).map_err(|err| {
                BedrockPackError::ParseError {
                    path: String::from("textures/flipbook_textures.json"),
                    detail: err.to_string(),
                }
            })?;
        for entry in entries {
            let norm_tex = normalize_bedrock_tex_key(&entry.flipbook_texture);
            let spec = flipbook_to_java_animation(&entry);
            pending_animations.insert(norm_tex, (spec, Some(entry.atlas_tile)));
        }
    }

    let mut ir = ResourcePackIR {
        metadata: PackMetadata {
            name: manifest.header.name,
            description: manifest.header.description,
            source_edition: Edition::Bedrock,
            java_pack_format: Some(effective_java_pack_format),
            java_supported_formats: None,
            bedrock_min_engine_version: Some(min_engine),
            version: manifest.header.version,
            header_uuid: Some(manifest.header.uuid),
            module_uuid,
            icon_png,
            rotation_mode,
            bedrock_geo_version: Some(BedrockGeoVersion::for_engine_version(min_engine)),
            bedrock_manifest_version: Some(manifest.format_version),
            emit_java_items_definitions: None,
        },
        ..ResourcePackIR::default()
    };

    let mut geo_id_to_model_id: HashMap<String, String> = HashMap::new();

    for (path, bytes) in container.files() {
        if path == "manifest.json"
            || path == "pack_icon.png"
            || path == "textures/terrain_texture.json"
            || path == "textures/item_texture.json"
            || path == "textures/flipbook_textures.json"
            || path == "textures/textures_list.json"
            || path == "texts/languages.json"
        {
            continue;
        }

        if let Some(bedrock_rel) = path.strip_suffix(".png")
            && path.starts_with("textures/")
        {
            let canonical_id = mappings.bedrock_texture_to_java(bedrock_rel);
            let lookup_key = bedrock_rel.to_ascii_lowercase();
            let (atlas_key, category) = if let Some((key, cat)) = atlas_lookup.get(&lookup_key) {
                (Some(key.clone()), *cat)
            } else {
                (
                    None,
                    infer_bedrock_texture_category(bedrock_rel, &canonical_id),
                )
            };

            ir.textures.insert(
                canonical_id.clone(),
                TextureAsset {
                    id: canonical_id,
                    category,
                    atlas_key,
                    payload: TexturePayload::from_raw(bytes.clone()),
                    animation: None,
                },
            );
            continue;
        }

        if path.starts_with("models/") && path.ends_with(".json") {
            let model_clean = sanitize_jsonc_bytes(bytes);
            let geo_file = BedrockGeoFile::from_json_slice(&model_clean).map_err(|err| {
                BedrockPackError::ParseError {
                    path: path.clone(),
                    detail: err.to_string(),
                }
            })?;
            let (model_id_hint, category_hint) = infer_bedrock_model_id_and_category(path);
            let models = bedrock_geo_to_model_assets_with_options(
                &geo_file,
                &model_id_hint,
                category_hint,
                Some(effective_java_pack_format),
                rotation_mode,
                &mut report,
            );
            for (geo_def, model) in geo_file.geometries.iter().zip(models) {
                geo_id_to_model_id.insert(
                    geo_def.description.identifier.to_ascii_lowercase(),
                    model.id.clone(),
                );
                ir.models.insert(model.id.clone(), model);
            }
            continue;
        }

        if path == "sounds/sound_definitions.json" {
            let sounds_clean = sanitize_jsonc_bytes(bytes);
            let parsed: BedrockSoundDefinitionsInput = serde_json::from_slice(&sounds_clean)
                .map_err(|err| BedrockPackError::ParseError {
                    path: path.clone(),
                    detail: err.to_string(),
                })?;
            let defs = match parsed {
                BedrockSoundDefinitionsInput::Modern(m) => m.sound_definitions,
                BedrockSoundDefinitionsInput::Legacy(l) => l,
            };
            for (bedrock_ev, def) in &defs {
                let canonical_ev = mappings.bedrock_sound_to_java(bedrock_ev);
                ir.sounds.events.insert(canonical_ev, def.into());
            }
            continue;
        }

        if let Some(rel_sound) = path
            .strip_prefix("sounds/")
            .and_then(|s| s.strip_suffix(".ogg"))
        {
            validate_sound_asset_ogg(path, bytes, &mut report);
            ir.sounds.files.insert(
                rel_sound.to_string(),
                SoundAsset {
                    id: rel_sound.to_string(),
                    data: Arc::new(bytes.clone()),
                },
            );
            continue;
        }

        if let Some(raw_locale) = path
            .strip_prefix("texts/")
            .and_then(|s| s.strip_suffix(".lang"))
        {
            let text = std::str::from_utf8(bytes).map_err(|err| BedrockPackError::ParseError {
                path: path.clone(),
                detail: format!("invalid UTF-8 in .lang file: {err}"),
            })?;
            let parsed_table = parse_bedrock_lang(text)?;
            let canonical_table = transform_locale_keys(&parsed_table, mappings, Edition::Java);
            let java_code = normalize_locale_to_java(raw_locale);
            ir.locales.insert(java_code, canonical_table);
            continue;
        }

        if let Some(stem) = path
            .strip_prefix("attachables/")
            .and_then(|s| s.strip_suffix(".json"))
        {
            let att_clean = sanitize_jsonc_bytes(bytes);
            let parsed: BedrockAttachableFile =
                serde_json::from_slice(&att_clean).map_err(|err| BedrockPackError::ParseError {
                    path: path.clone(),
                    detail: err.to_string(),
                })?;
            if stem.ends_with(".player") {
                report.record_warning(
                    path,
                    WarningSeverity::Info,
                    "Player attachable variant handled alongside primary item attachable",
                );
                continue;
            }
            let desc = parsed.attachable.description;
            let default_texture = desc
                .textures
                .get("default")
                .map(|t| mappings.bedrock_texture_to_java(t));
            let default_geometry = desc.geometry.get("default").cloned();
            let item_identifier = extract_attachable_item_id(&desc);
            let clean_stem =
                sanitize_java_resource_path(stem.strip_suffix(".armor_stand").unwrap_or(stem));
            ir.attachables.insert(
                clean_stem.clone(),
                AttachableAsset {
                    id: clean_stem,
                    item_identifier,
                    default_texture,
                    default_geometry,
                },
            );
            continue;
        }

        if path.starts_with("render_controllers/")
            || path.starts_with("materials/")
            || path.starts_with("entity/")
            || path.starts_with("animation_controllers/")
            || path.starts_with("animations/")
        {
            report.record_skipped(
                path,
                "Bedrock-specific entity controller/material/animation file has no direct Java resource pack equivalent",
            );
            continue;
        }

        ir.passthrough_files.insert(path.clone(), bytes.clone());
        report.record_passthrough();
    }

    for (bedrock_rel, (spec, atlas_tile)) in pending_animations {
        let canonical_id = mappings.bedrock_texture_to_java(&bedrock_rel);
        let tex_key = ir
            .textures
            .keys()
            .find(|k| k.eq_ignore_ascii_case(&canonical_id))
            .cloned();
        let png_path = format!("{bedrock_rel}.png");
        if let Some(matched_id) = tex_key
            && let Some(tex) = ir.textures.get_mut(&matched_id)
        {
            let dims_opt = png_dims.get(&png_path).or_else(|| {
                png_dims
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&png_path))
                    .map(|(_, v)| v)
            });
            if let Some(&(w, h)) = dims_opt {
                compute_animation_strip_geometry(w, h, &spec)?;
            }
            if tex.atlas_key.is_none() {
                tex.atlas_key = atlas_tile;
            }
            tex.animation = Some(spec);
        } else {
            report.record_warning(
                "textures/flipbook_textures.json",
                WarningSeverity::Warning,
                format!("Flipbook entry references missing texture '{png_path}'"),
            );
        }
    }

    // Link attachable textures & synthesize Java item models when an attachable references a parsed geometry.
    let mut synthesized_models: Vec<ModelAsset> = Vec::new();
    let mut first_helmet_model: Option<ModelAsset> = None;
    for attachable in ir.attachables.values() {
        let Some(geo_id) = &attachable.default_geometry else {
            continue;
        };
        let geo_stem = geo_id.strip_prefix("geometry.").unwrap_or(geo_id.as_str());
        let candidate_model_keys = [
            format!("entity/{geo_stem}"),
            format!("block/{geo_stem}"),
            format!("item/{geo_stem}"),
            geo_stem.to_string(),
        ];

        let matched_key = geo_id_to_model_id
            .get(&geo_id.to_ascii_lowercase())
            .cloned()
            .or_else(|| {
                candidate_model_keys
                    .iter()
                    .find(|k| ir.models.contains_key(k.as_str()))
                    .cloned()
            });

        if let Some(model_key) = matched_key {
            if let Some(tex_id) = &attachable.default_texture
                && let Some(model) = ir.models.get_mut(&model_key)
                && model
                    .textures
                    .get("all")
                    .is_some_and(|v| v == &model_key || v.starts_with("entity/"))
            {
                model.textures.insert(String::from("all"), tex_id.clone());
            }

            if let Some(base_model) = ir.models.get(&model_key) {
                let item_name = attachable
                    .item_identifier
                    .strip_prefix("minecraft:")
                    .unwrap_or(&attachable.item_identifier);
                let item_model_id = format!("item/{item_name}");
                if !ir.models.contains_key(&item_model_id) {
                    let mut item_model = base_model.clone();
                    item_model.id = item_model_id;
                    item_model.category = ModelCategory::Item;
                    if let Some(tex_id) = &attachable.default_texture {
                        item_model
                            .textures
                            .insert(String::from("all"), tex_id.clone());
                    }
                    if item_model.display.is_empty() {
                        item_model.display = default_wearable_display_transforms();
                    }
                    if item_name.ends_with("_helmet") && first_helmet_model.is_none() {
                        first_helmet_model = Some(item_model.clone());
                    }
                    synthesized_models.push(item_model);
                }
            }
        }
    }
    if let Some(helmet_model) = first_helmet_model
        && !ir.models.contains_key("item/carved_pumpkin")
        && !synthesized_models
            .iter()
            .any(|m| m.id == "item/carved_pumpkin")
    {
        let mut pumpkin_model = helmet_model;
        pumpkin_model.id = String::from("item/carved_pumpkin");
        synthesized_models.push(pumpkin_model);
    }
    for model in synthesized_models {
        ir.models.insert(model.id.clone(), model);
    }

    Ok((ir, report))
}

fn extract_attachable_item_id(desc: &crate::schema::BedrockAttachableDescription) -> String {
    if let Some(item_val) = &desc.item {
        if let Some(s) = item_val.as_str() {
            return s.to_string();
        }
        if let Some(obj) = item_val.as_object()
            && let Some(first_key) = obj.keys().next()
        {
            return first_key.clone();
        }
    }
    desc.identifier
        .strip_suffix(".armor_stand")
        .or_else(|| desc.identifier.strip_suffix(".player"))
        .unwrap_or(&desc.identifier)
        .to_string()
}

fn is_bedrock_texture_png(path: &str) -> bool {
    path.starts_with("textures/") && path.ends_with(".png")
}

fn normalize_bedrock_tex_key(raw: &str) -> String {
    let clean = raw.trim().replace('\\', "/");
    let no_lead = clean.trim_start_matches('/');
    no_lead
        .strip_suffix(".png")
        .unwrap_or(no_lead)
        .to_ascii_lowercase()
}

fn default_wearable_display_transforms() -> BTreeMap<String, DisplayTransform> {
    BTreeMap::from([
        (
            String::from("gui"),
            DisplayTransform {
                rotation: [25.0, 145.0, 0.0],
                translation: [0.0, 0.0, 0.0],
                scale: [0.85, 0.85, 0.85],
            },
        ),
        (
            String::from("head"),
            DisplayTransform {
                rotation: [0.0, 0.0, 0.0],
                translation: [0.0, 0.0, -4.25],
                scale: [1.1, 1.1, 1.1],
            },
        ),
        (
            String::from("thirdperson_righthand"),
            DisplayTransform {
                rotation: [75.0, 45.0, 0.0],
                translation: [0.0, 2.5, 0.0],
                scale: [0.375, 0.375, 0.375],
            },
        ),
        (
            String::from("firstperson_righthand"),
            DisplayTransform {
                rotation: [0.0, 45.0, 0.0],
                translation: [0.0, 0.0, 0.0],
                scale: [0.4, 0.4, 0.4],
            },
        ),
        (
            String::from("ground"),
            DisplayTransform {
                rotation: [0.0, 0.0, 0.0],
                translation: [0.0, 3.0, 0.0],
                scale: [0.25, 0.25, 0.25],
            },
        ),
        (
            String::from("fixed"),
            DisplayTransform {
                rotation: [0.0, 0.0, 0.0],
                translation: [0.0, 0.0, 0.0],
                scale: [0.5, 0.5, 0.5],
            },
        ),
    ])
}

fn extract_atlas_paths(paths: &AtlasTexturePath) -> Vec<String> {
    match paths {
        AtlasTexturePath::Single(p) => vec![p.clone()],
        AtlasTexturePath::List(items) => items
            .iter()
            .map(|v| match v {
                AtlasVariant::Path(p) => p.clone(),
                AtlasVariant::Object { path } => path.clone(),
            })
            .collect(),
    }
}

fn infer_bedrock_texture_category(bedrock_rel: &str, canonical_id: &str) -> TextureCategory {
    if bedrock_rel.starts_with("textures/blocks/") {
        TextureCategory::Block
    } else if bedrock_rel.starts_with("textures/items/") {
        TextureCategory::Item
    } else if bedrock_rel.starts_with("textures/ui/") {
        TextureCategory::Gui
    } else {
        TextureCategory::infer_from_id(canonical_id)
    }
}

fn strip_geo_suffix(s: &str) -> Option<&str> {
    s.strip_suffix(".geo.json")
        .or_else(|| s.strip_suffix(".json"))
}

fn infer_bedrock_model_id_and_category(path: &str) -> (String, ModelCategory) {
    if let Some(rest) = path
        .strip_prefix("models/entity/")
        .and_then(strip_geo_suffix)
    {
        if rest.starts_with("block/") {
            (rest.to_string(), ModelCategory::Block)
        } else if rest.starts_with("item/") {
            (rest.to_string(), ModelCategory::Item)
        } else {
            (format!("entity/{rest}"), ModelCategory::Entity)
        }
    } else if let Some(rest) = path
        .strip_prefix("models/blocks/")
        .and_then(strip_geo_suffix)
    {
        (format!("block/{rest}"), ModelCategory::Block)
    } else {
        let rest = path
            .strip_prefix("models/")
            .and_then(strip_geo_suffix)
            .unwrap_or(path);
        (rest.to_string(), ModelCategory::Entity)
    }
}
