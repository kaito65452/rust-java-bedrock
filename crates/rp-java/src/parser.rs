use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rp_core::{
    AnimationSpec, BedrockGeoVersion, ConversionReport, DisplayTransform, Edition, MappingTable,
    ModelAsset, ModelCategory, PackMetadata, ResolutionTier, ResourcePackIR, RotationSnapMode,
    SoundAsset, TextureAsset, TextureCategory, TexturePayload, WarningSeverity,
    decode_minecraft_text, parse_minecraft_json,
};
use rp_io::PackContainer;
use rp_transforms::locale::{
    normalize_locale_to_java, parse_bedrock_lang, parse_java_locale_json, transform_locale_keys,
};
use rp_transforms::sound::validate_sound_asset_ogg;
use rp_transforms::texture::{inspect_png_dimensions, resolve_animation_strip_geometry};

use crate::JavaPackError;
use crate::schema::{JavaAnimationMcmeta, JavaModelJson, JavaPackMcmeta, JavaSoundEventJson};

/// Parses a Java Edition `PackContainer` into canonical `ResourcePackIR` without eagerly decoding PNG pixels.
pub fn parse_java_pack(
    container: &PackContainer,
    mappings: &MappingTable,
) -> Result<(ResourcePackIR, ConversionReport), JavaPackError> {
    let mut report = ConversionReport::new();

    let mcmeta_bytes = container
        .get("pack.mcmeta")
        .ok_or(JavaPackError::MissingPackMcmeta)?;
    let mcmeta_val: serde_json::Value =
        parse_minecraft_json(mcmeta_bytes).map_err(|err| JavaPackError::ParseError {
            path: String::from("pack.mcmeta"),
            detail: err.to_string(),
        })?;
    let overlay_dirs = extract_overlay_dirs(&mcmeta_val);
    let mcmeta: JavaPackMcmeta =
        serde_json::from_value(mcmeta_val).map_err(|err| JavaPackError::ParseError {
            path: String::from("pack.mcmeta"),
            detail: err.to_string(),
        })?;
    let (pack_format, supported_range, description) = mcmeta.validate_and_extract()?;
    let bedrock_ver = mappings.java_pack_format_to_bedrock(pack_format);

    let icon_png = if let Some(icon_bytes) = container.get("pack.png") {
        inspect_png_dimensions(icon_bytes)?;
        Some(Arc::new(icon_bytes.to_vec()))
    } else {
        None
    };

    // Parallel header-only PNG dimension validation via rayon (no pixel buffer allocation).
    let png_dims: BTreeMap<String, (u32, u32)> = container
        .par_map_files(|path, bytes| {
            if is_java_texture_png(path) {
                let dims = inspect_png_dimensions(bytes)?;
                Ok::<_, rp_core::CoreError>(Some((path.to_string(), dims)))
            } else {
                Ok(None)
            }
        })?
        .into_iter()
        .flatten()
        .collect();

    let mut ir = ResourcePackIR {
        metadata: PackMetadata {
            name: String::from("Java Resource Pack"),
            description,
            source_edition: Edition::Java,
            java_pack_format: Some(pack_format),
            java_supported_formats: supported_range,
            bedrock_min_engine_version: Some(bedrock_ver),
            version: [1, 0, 0],
            header_uuid: None,
            module_uuid: None,
            icon_png,
            rotation_mode: RotationSnapMode::Preserve,
            bedrock_geo_version: Some(BedrockGeoVersion::for_engine_version(bedrock_ver)),
            bedrock_manifest_version: None,
            emit_java_items_definitions: None,
        },
        ..ResourcePackIR::default()
    };

    let mut pending_animations: BTreeMap<String, AnimationSpec> = BTreeMap::new();

    for (path, bytes) in container.files() {
        if path == "pack.mcmeta" || path == "pack.png" {
            continue;
        }

        if let Some(rel_tex) = path
            .strip_prefix("assets/minecraft/textures/")
            .and_then(|s| s.strip_suffix(".png"))
        {
            let id = normalize_pre_flattening_tex_id(rel_tex);
            let category = TextureCategory::infer_from_id(&id);
            let (width, height, resolution_tier) = match png_dims.get(path) {
                Some(&(w, h)) => (
                    Some(w),
                    Some(h),
                    Some(ResolutionTier::from_dimensions(w, h)),
                ),
                None => (None, None, None),
            };
            ir.textures.insert(
                id.clone(),
                TextureAsset {
                    id,
                    category,
                    atlas_key: None,
                    width,
                    height,
                    resolution_tier,
                    payload: TexturePayload::from_raw(bytes.clone()),
                    animation: None,
                },
            );
            continue;
        }

        if let Some(rel_tex) = path
            .strip_prefix("assets/minecraft/textures/")
            .and_then(|s| s.strip_suffix(".png.mcmeta"))
        {
            let mcmeta: JavaAnimationMcmeta =
                parse_minecraft_json(bytes).map_err(|err| JavaPackError::ParseError {
                    path: path.clone(),
                    detail: err.to_string(),
                })?;
            if let Some(body) = mcmeta.animation {
                let id = normalize_pre_flattening_tex_id(rel_tex);
                pending_animations.insert(id, AnimationSpec::from(&body));
            }
            continue;
        }

        if let Some(rel_model) = path
            .strip_prefix("assets/minecraft/models/")
            .and_then(|s| s.strip_suffix(".json"))
        {
            let model_json: JavaModelJson =
                parse_minecraft_json(bytes).map_err(|err| JavaPackError::ParseError {
                    path: path.clone(),
                    detail: err.to_string(),
                })?;

            let category = infer_model_category(rel_model);
            let mut elements = Vec::new();
            if let Some(raw_elements) = &model_json.elements {
                for raw_el in raw_elements {
                    elements.push(raw_el.to_ir()?);
                }
            }

            let overrides_count = model_json.overrides.as_ref().map_or(0, Vec::len);
            let has_overrides = overrides_count > 0;
            if has_overrides {
                report.record_warning(
                    path,
                    WarningSeverity::Warning,
                    format!(
                        "Model defines {overrides_count} item predicate override(s) which are unsupported in Bedrock Edition"
                    ),
                );
            }

            let textures = model_json
                .textures
                .unwrap_or_default()
                .into_iter()
                .map(|(k, v)| (k, v.strip_prefix("minecraft:").unwrap_or(&v).to_string()))
                .collect();

            let display: BTreeMap<String, DisplayTransform> = model_json
                .display
                .unwrap_or_default()
                .into_iter()
                .map(|(k, v)| (k, DisplayTransform::from(&v)))
                .collect();

            ir.models.insert(
                rel_model.to_string(),
                ModelAsset {
                    id: rel_model.to_string(),
                    category,
                    parent: model_json.parent,
                    textures,
                    texture_size: model_json.texture_size.unwrap_or([16, 16]),
                    ambient_occlusion: model_json.ambientocclusion.unwrap_or(true),
                    elements,
                    display,
                    has_overrides,
                    overrides_count,
                },
            );
            continue;
        }

        if path == "assets/minecraft/sounds.json" {
            let parsed: BTreeMap<String, JavaSoundEventJson> = parse_minecraft_json(bytes)
                .map_err(|err| JavaPackError::ParseError {
                    path: path.clone(),
                    detail: err.to_string(),
                })?;
            for (event_name, def) in &parsed {
                ir.sounds.events.insert(event_name.clone(), def.into());
            }
            continue;
        }

        if let Some(rel_sound) = path
            .strip_prefix("assets/minecraft/sounds/")
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

        if let Some((_ns, sub)) = path
            .strip_prefix("assets/")
            .and_then(|rest| rest.split_once('/'))
        {
            if let Some(raw_locale) = sub
                .strip_prefix("lang/")
                .and_then(|s| s.strip_suffix(".json"))
                && !raw_locale.contains('/')
            {
                let text = decode_minecraft_text(bytes);
                let table = parse_java_locale_json(&text)?;
                let code = normalize_locale_to_java(raw_locale);
                let dest = ir.locales.entry(code).or_default();
                dest.entries.extend(table.entries);
                dest.comments.extend(table.comments);
                continue;
            }

            if let Some(raw_locale) = sub
                .strip_prefix("lang/")
                .and_then(|s| s.strip_suffix(".lang"))
                && !raw_locale.contains('/')
            {
                let text = decode_minecraft_text(bytes);
                let parsed = parse_bedrock_lang(&text)?;
                let table = transform_locale_keys(&parsed, mappings, Edition::Java);
                let code = normalize_locale_to_java(raw_locale);
                let dest = ir.locales.entry(code).or_default();
                dest.entries.extend(table.entries);
                dest.comments.extend(table.comments);
                continue;
            }

            if sub.starts_with("shaders/")
                || sub.starts_with("optifine/")
                || sub.starts_with("mcpatcher/")
            {
                report.record_skipped(
                    path,
                    "Java-only shader or OptiFine/MCPatcher extension has no Bedrock equivalent",
                );
                continue;
            }
        }

        if path.starts_with("assets/minecraft/items/") && path.ends_with(".json") {
            let _: serde_json::Value =
                parse_minecraft_json(bytes).map_err(|err| JavaPackError::ParseError {
                    path: path.clone(),
                    detail: err.to_string(),
                })?;
            ir.passthrough_files.insert(path.clone(), bytes.clone());
            report.record_passthrough();
            continue;
        }

        if let Some((top_dir, _)) = path.split_once('/')
            && (overlay_dirs.contains(top_dir) || top_dir.starts_with("overlay"))
        {
            ir.passthrough_files.insert(path.clone(), bytes.clone());
            report.record_passthrough();
            continue;
        }

        if path.starts_with("assets/minecraft/blockstates/") {
            report.record_warning(
                path,
                WarningSeverity::Info,
                "Java blockstate definitions are not directly emitted to Bedrock",
            );
            continue;
        }

        if !path.starts_with("assets/") {
            ir.passthrough_files.insert(path.clone(), bytes.clone());
            report.record_passthrough();
        } else {
            report.record_warning(
                path,
                WarningSeverity::Info,
                "Unrecognized Java asset path preserved as pass-through",
            );
            ir.passthrough_files.insert(path.clone(), bytes.clone());
            report.record_passthrough();
        }
    }

    for (tex_id, mut spec) in pending_animations {
        let png_path = format!("assets/minecraft/textures/{tex_id}.png");
        let legacy_png_path = tex_id
            .strip_prefix("block/")
            .map(|rest| format!("assets/minecraft/textures/blocks/{rest}.png"))
            .or_else(|| {
                tex_id
                    .strip_prefix("item/")
                    .map(|rest| format!("assets/minecraft/textures/items/{rest}.png"))
            });
        if let Some(tex) = ir.textures.get_mut(&tex_id) {
            let found_dims = png_dims
                .get(&png_path)
                .or_else(|| legacy_png_path.as_ref().and_then(|p| png_dims.get(p)));
            if let Some(&(w, h)) = found_dims {
                if let Some(resolved) =
                    resolve_animation_strip_geometry(&png_path, w, h, &spec, &mut report)
                {
                    if resolved.fell_back_to_static {
                        tex.animation = None;
                    } else {
                        if spec.frame_height.is_none()
                            && resolved.frame_height != resolved.frame_width
                        {
                            spec.frame_height = Some(resolved.frame_height);
                        }
                        tex.animation = Some(spec);
                    }
                }
            } else {
                tex.animation = Some(spec);
            }
        } else {
            report.record_warning(
                format!("{png_path}.mcmeta"),
                WarningSeverity::Warning,
                "Animation .png.mcmeta has no matching PNG texture file",
            );
        }
    }

    Ok((ir, report))
}

fn extract_overlay_dirs(val: &serde_json::Value) -> BTreeSet<String> {
    let mut dirs = BTreeSet::new();
    if let Some(entries) = val
        .get("overlays")
        .and_then(|o| o.get("entries"))
        .and_then(serde_json::Value::as_array)
    {
        for entry in entries {
            if let Some(dir) = entry.get("directory").and_then(serde_json::Value::as_str) {
                dirs.insert(dir.trim_matches('/').to_string());
            }
        }
    }
    dirs
}

fn normalize_pre_flattening_tex_id(rel_tex: &str) -> String {
    if let Some(rest) = rel_tex.strip_prefix("blocks/") {
        format!("block/{rest}")
    } else if let Some(rest) = rel_tex.strip_prefix("items/") {
        format!("item/{rest}")
    } else {
        rel_tex.to_string()
    }
}

fn is_java_texture_png(path: &str) -> bool {
    path.starts_with("assets/minecraft/textures/") && path.ends_with(".png")
}

fn infer_model_category(rel_model: &str) -> ModelCategory {
    if rel_model.starts_with("item/") || rel_model.starts_with("items/") {
        ModelCategory::Item
    } else if rel_model.starts_with("entity/") {
        ModelCategory::Entity
    } else {
        ModelCategory::Block
    }
}
