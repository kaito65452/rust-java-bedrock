use std::collections::{BTreeMap, BTreeSet};

use rp_core::{
    AuditStatus, BrokenReference, ForbiddenEntry, MappingTable, SemanticAuditReport,
    parse_minecraft_json,
};
use rp_io::{PackContainer, is_os_junk};
use rp_transforms::texture::{
    AtlasTexturePath, AtlasVariant, FlipbookEntry, ItemTextureJson, TerrainTextureJson,
};

use crate::schema::{BedrockAttachableFile, BedrockManifest};

/// Audits a Bedrock Edition `PackContainer` against the verified Bedrock Engine Acceptance Rules
/// (`data/spec/bedrock_acceptance_rules.json` / `minecraft.wiki`).
#[must_use]
pub fn audit_bedrock_container(
    container: &PackContainer,
    total_input_textures: usize,
    mappings: &MappingTable,
) -> SemanticAuditReport {
    let mut forbidden_entries: Vec<ForbiddenEntry> = Vec::new();
    let mut broken_references: Vec<BrokenReference> = Vec::new();
    let mut category_breakdown: BTreeMap<String, usize> = BTreeMap::new();

    // 1. Validate manifest.json
    let manifest_valid = validate_manifest(container, &mut forbidden_entries, &mut category_breakdown);

    // 2. Collect all texture references from atlases, flipbooks, texture_sets, attachables, entity/ui/particles.
    let mut referenced_pngs_lower: BTreeSet<String> = BTreeSet::new();
    let mut texture_set_referenced_pngs_lower: BTreeSet<String> = BTreeSet::new();
    let mut terrain_tile_keys: BTreeSet<String> = BTreeSet::new();

    // 2a. terrain_texture.json
    if let Some(bytes) = container.get("textures/terrain_texture.json") {
        match parse_minecraft_json::<TerrainTextureJson>(bytes) {
            Ok(terrain) => {
                if terrain.texture_data.is_empty() {
                    forbidden_entries.push(ForbiddenEntry {
                        path: String::from("textures/terrain_texture.json"),
                        rule_id: String::from("empty_stub_terrain_atlas"),
                        reason: String::from(
                            "textures/terrain_texture.json has empty texture_data stub",
                        ),
                        actionable_fix: String::from(
                            "Omit textures/terrain_texture.json when no block textures are registered.",
                        ),
                    });
                    bump(&mut category_breakdown, "empty_stub");
                }
                for (key, data) in &terrain.texture_data {
                    terrain_tile_keys.insert(key.clone());
                    let key_is_known = mappings.is_known_terrain_key(key);
                    for p in extract_atlas_paths(&data.textures) {
                        let png = ensure_png_ext(&p);
                        if !contains_case_insensitive(container, &png) {
                            broken_references.push(BrokenReference {
                                source_file: String::from("textures/terrain_texture.json"),
                                missing_path: png.clone(),
                            });
                            bump(&mut category_breakdown, "broken_atlas_reference");
                        } else if key_is_known {
                            referenced_pngs_lower.insert(png.to_ascii_lowercase());
                        }
                    }
                }
            }
            Err(err) => {
                forbidden_entries.push(ForbiddenEntry {
                    path: String::from("textures/terrain_texture.json"),
                    rule_id: String::from("invalid_terrain_atlas_json"),
                    reason: format!("Invalid textures/terrain_texture.json: {err}"),
                    actionable_fix: String::from(
                        "Ensure textures/terrain_texture.json conforms to Bedrock atlas.terrain JSON schema.",
                    ),
                });
                bump(&mut category_breakdown, "invalid_json");
            }
        }
    }

    // 2b. item_texture.json
    if let Some(bytes) = container.get("textures/item_texture.json") {
        match parse_minecraft_json::<ItemTextureJson>(bytes) {
            Ok(items) => {
                if items.texture_data.is_empty() {
                    forbidden_entries.push(ForbiddenEntry {
                        path: String::from("textures/item_texture.json"),
                        rule_id: String::from("empty_stub_item_atlas"),
                        reason: String::from(
                            "textures/item_texture.json has empty texture_data stub",
                        ),
                        actionable_fix: String::from(
                            "Omit textures/item_texture.json when no item textures are registered.",
                        ),
                    });
                    bump(&mut category_breakdown, "empty_stub");
                }
                for (key, data) in &items.texture_data {
                    let key_is_known = mappings.is_known_item_key(key);
                    for p in extract_atlas_paths(&data.textures) {
                        let png = ensure_png_ext(&p);
                        if !contains_case_insensitive(container, &png) {
                            broken_references.push(BrokenReference {
                                source_file: String::from("textures/item_texture.json"),
                                missing_path: png.clone(),
                            });
                            bump(&mut category_breakdown, "broken_atlas_reference");
                        } else if key_is_known {
                            referenced_pngs_lower.insert(png.to_ascii_lowercase());
                        }
                    }
                }
            }
            Err(err) => {
                forbidden_entries.push(ForbiddenEntry {
                    path: String::from("textures/item_texture.json"),
                    rule_id: String::from("invalid_item_atlas_json"),
                    reason: format!("Invalid textures/item_texture.json: {err}"),
                    actionable_fix: String::from(
                        "Ensure textures/item_texture.json conforms to Bedrock atlas.items JSON schema.",
                    ),
                });
                bump(&mut category_breakdown, "invalid_json");
            }
        }
    }

    // 2c. flipbook_textures.json
    if let Some(bytes) = container.get("textures/flipbook_textures.json")
        && let Ok(entries) = parse_minecraft_json::<Vec<FlipbookEntry>>(bytes)
    {
        for entry in entries {
            let png = ensure_png_ext(&entry.flipbook_texture);
            if !contains_case_insensitive(container, &png) {
                broken_references.push(BrokenReference {
                    source_file: String::from("textures/flipbook_textures.json"),
                    missing_path: png,
                });
                bump(&mut category_breakdown, "broken_flipbook_reference");
            } else if terrain_tile_keys.contains(&entry.atlas_tile)
                || mappings.is_known_terrain_key(&entry.atlas_tile)
            {
                referenced_pngs_lower.insert(png.to_ascii_lowercase());
            }
        }
    }

    // 2d. texts/languages.json empty stub check
    if let Some(bytes) = container.get("texts/languages.json")
        && let Ok(langs) = parse_minecraft_json::<Vec<String>>(bytes)
    {
        if langs.is_empty() {
            forbidden_entries.push(ForbiddenEntry {
                path: String::from("texts/languages.json"),
                rule_id: String::from("empty_stub_languages_json"),
                reason: String::from(
                    "texts/languages.json is an empty array [] with no corresponding texts/*.lang files",
                ),
                actionable_fix: String::from(
                    "Omit texts/languages.json when no localization files are emitted.",
                ),
            });
            bump(&mut category_breakdown, "empty_stub");
        } else {
            for code in langs {
                let lang_file = format!("texts/{code}.lang");
                if !container.contains(&lang_file) {
                    broken_references.push(BrokenReference {
                        source_file: String::from("texts/languages.json"),
                        missing_path: lang_file,
                    });
                    bump(&mut category_breakdown, "broken_locale_reference");
                }
            }
        }
    }

    // 2e. Scan *.texture_set.json, attachables/*.json, entity/*.json, particles/*.json, ui/*.json
    for (path, bytes) in container.files() {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".texture_set.json") {
            if let Ok(val) = parse_minecraft_json::<serde_json::Value>(bytes)
                && let Some(ts) = val.get("minecraft:texture_set").and_then(|v| v.as_object())
            {
                let has_normal = ts.get("normal").and_then(|v| v.as_str()).is_some();
                let has_height = ts.get("heightmap").and_then(|v| v.as_str()).is_some();
                if has_normal && has_height {
                    forbidden_entries.push(ForbiddenEntry {
                        path: path.clone(),
                        rule_id: String::from("texture_set_normal_and_heightmap_conflict"),
                        reason: String::from(
                            "texture_set.json defines both 'normal' and 'heightmap', which are mutually exclusive in Bedrock",
                        ),
                        actionable_fix: String::from(
                            "Keep either 'normal' or 'heightmap' in minecraft:texture_set, never both.",
                        ),
                    });
                    bump(&mut category_breakdown, "invalid_texture_set");
                }

                let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
                for field in [
                    "color",
                    "normal",
                    "heightmap",
                    "metalness_emissive_roughness",
                    "metalness_emissive_roughness_subsurface",
                ] {
                    if let Some(stem) = ts.get(field).and_then(|v| v.as_str()) {
                        let candidate = if dir.is_empty() {
                            ensure_png_ext(stem)
                        } else {
                            ensure_png_ext(&format!("{dir}/{stem}"))
                        };
                        if contains_case_insensitive(container, &candidate) {
                            let low = candidate.to_ascii_lowercase();
                            referenced_pngs_lower.insert(low.clone());
                            texture_set_referenced_pngs_lower.insert(low);
                        } else {
                            broken_references.push(BrokenReference {
                                source_file: path.clone(),
                                missing_path: candidate,
                            });
                            bump(&mut category_breakdown, "broken_texture_set_reference");
                        }
                    }
                }
            }
        } else if lower.starts_with("attachables/") && lower.ends_with(".json") {
            if let Ok(att) = parse_minecraft_json::<BedrockAttachableFile>(bytes) {
                for tex_ref in att.attachable.description.textures.values() {
                    let png = ensure_png_ext(tex_ref);
                    if contains_case_insensitive(container, &png) {
                        referenced_pngs_lower.insert(png.to_ascii_lowercase());
                    }
                }
            }
        } else if (lower.starts_with("entity/")
            || lower.starts_with("particles/")
            || lower.starts_with("ui/"))
            && lower.ends_with(".json")
            && let Ok(val) = parse_minecraft_json::<serde_json::Value>(bytes)
        {
            collect_embedded_texture_strings(&val, &mut referenced_pngs_lower);
        }
    }

    // 3. Check every file in the output archive for forbidden patterns & texture reachability.
    let mut total_output_textures = 0usize;
    let mut semantically_mapped_textures = 0usize;
    let mut orphan_textures: Vec<String> = Vec::new();

    for path in container.files().keys() {
        let lower = path.to_ascii_lowercase();

        if lower.starts_with("assets/") {
            forbidden_entries.push(ForbiddenEntry {
                path: path.clone(),
                rule_id: String::from("leaked_java_assets"),
                reason: String::from(
                    "Raw Java Edition assets/ file leaked into Bedrock archive and will be ignored by Bedrock",
                ),
                actionable_fix: String::from(
                    "Convert assets/ files into Bedrock paths (textures/, sounds/, texts/) or skip unsupported Java-only files.",
                ),
            });
            bump(&mut category_breakdown, "leaked_java_assets");
            continue;
        }

        if lower == "pack.mcmeta" {
            forbidden_entries.push(ForbiddenEntry {
                path: path.clone(),
                rule_id: String::from("leaked_java_mcmeta"),
                reason: String::from("Java pack.mcmeta must not appear in a Bedrock pack"),
                actionable_fix: String::from("Use Bedrock manifest.json instead of pack.mcmeta."),
            });
            bump(&mut category_breakdown, "leaked_java_assets");
            continue;
        }

        if is_os_junk(path) {
            forbidden_entries.push(ForbiddenEntry {
                path: path.clone(),
                rule_id: String::from("os_junk_files"),
                reason: String::from("OS metadata/junk file present in Bedrock output archive"),
                actionable_fix: String::from(
                    "Filter desktop.ini, .DS_Store, Thumbs.db, and __MACOSX/ during scanning.",
                ),
            });
            bump(&mut category_breakdown, "os_junk");
            continue;
        }

        if lower.starts_with("textures/mob_effect/") {
            forbidden_entries.push(ForbiddenEntry {
                path: path.clone(),
                rule_id: String::from("invalid_bedrock_mob_effect_dir"),
                reason: String::from(
                    "Bedrock does not read textures/mob_effect/; status effect icons must be at textures/ui/<id>_effect.png",
                ),
                actionable_fix: String::from(
                    "Remap assets/minecraft/textures/mob_effect/<id>.png -> textures/ui/<id>_effect.png.",
                ),
            });
            bump(&mut category_breakdown, "invalid_mob_effect_path");
            continue;
        }

        if lower.starts_with("models/entity/block/") && lower.ends_with(".geo.json") {
            forbidden_entries.push(ForbiddenEntry {
                path: path.clone(),
                rule_id: String::from("dead_vanilla_block_geo"),
                reason: String::from(
                    "Pure Bedrock Resource Packs cannot override vanilla block geometry using models/entity/block/*.geo.json",
                ),
                actionable_fix: String::from(
                    "Remove dead block .geo.json files; keep block textures in terrain_texture.json or use a Behavior Pack custom block.",
                ),
            });
            bump(&mut category_breakdown, "dead_vanilla_block_geo");
            continue;
        }

        if lower.starts_with("textures/")
            && (lower.ends_with("_n.png") || lower.ends_with("_s.png"))
            && !texture_set_referenced_pngs_lower.contains(&lower)
        {
            forbidden_entries.push(ForbiddenEntry {
                path: path.clone(),
                rule_id: String::from("raw_unconverted_labpbr"),
                reason: String::from(
                    "Raw Java LabPBR normal (_n.png) or specular (_s.png) map copied without a Bedrock *.texture_set.json",
                ),
                actionable_fix: String::from(
                    "Enable --convert-pbr to convert LabPBR maps into *_mer.png + *.texture_set.json, or omit them when PBR is disabled.",
                ),
            });
            bump(&mut category_breakdown, "raw_unconverted_labpbr");
            continue;
        }

        if lower.ends_with(".png") && lower != "pack_icon.png" && lower != "bug_pack_icon.png" {
            total_output_textures += 1;
            let stem = path.strip_suffix(".png").unwrap_or(path);
            let is_mapped = mappings.is_known_bedrock_texture(stem)
                || referenced_pngs_lower.contains(&lower);
            if is_mapped {
                semantically_mapped_textures += 1;
            } else {
                orphan_textures.push(path.clone());
                bump(&mut category_breakdown, "orphan_texture");
            }
        }
    }

    // 4. Determine honest semantic status verdict (Anti-False-Positive Gate).
    let status = if !manifest_valid
        || !forbidden_entries.is_empty()
        || !broken_references.is_empty()
        || (total_input_textures > 0 && semantically_mapped_textures == 0)
    {
        AuditStatus::Failed
    } else if !orphan_textures.is_empty()
        || (total_input_textures > 0 && semantically_mapped_textures * 2 < total_input_textures)
    {
        AuditStatus::Partial
    } else {
        AuditStatus::Passed
    };

    let summary = match status {
        AuditStatus::Passed => format!(
            "PASSED: {semantically_mapped_textures}/{total_input_textures} input texture(s) semantically mapped, 0 orphan textures, 0 forbidden entries, 0 broken refs"
        ),
        AuditStatus::Partial => format!(
            "PARTIAL / WARNING: {semantically_mapped_textures}/{total_input_textures} input texture(s) mapped, {} orphan texture(s), 0 forbidden entries",
            orphan_textures.len()
        ),
        AuditStatus::Failed => format!(
            "FAILED: manifest_valid={manifest_valid}, mapped={semantically_mapped_textures}/{total_input_textures}, forbidden={}, broken_refs={}, orphan={}",
            forbidden_entries.len(),
            broken_references.len(),
            orphan_textures.len()
        ),
    };

    SemanticAuditReport {
        status,
        total_input_textures,
        total_output_textures,
        semantically_mapped_textures,
        orphan_textures,
        forbidden_entries,
        broken_references,
        manifest_valid,
        summary,
        category_breakdown,
    }
}

fn validate_manifest(
    container: &PackContainer,
    forbidden: &mut Vec<ForbiddenEntry>,
    breakdown: &mut BTreeMap<String, usize>,
) -> bool {
    let Some(bytes) = container.get("manifest.json") else {
        forbidden.push(ForbiddenEntry {
            path: String::from("manifest.json"),
            rule_id: String::from("missing_manifest"),
            reason: String::from("Bedrock Resource Pack is missing required root manifest.json"),
            actionable_fix: String::from("Generate a valid manifest.json with format_version 2 or 3."),
        });
        bump(breakdown, "invalid_manifest");
        return false;
    };

    let manifest = match parse_minecraft_json::<BedrockManifest>(bytes) {
        Ok(m) => m,
        Err(err) => {
            forbidden.push(ForbiddenEntry {
                path: String::from("manifest.json"),
                rule_id: String::from("invalid_manifest_json"),
                reason: format!("manifest.json failed schema parsing: {err}"),
                actionable_fix: String::from(
                    "Fix manifest.json syntax, version formatting, and required header/modules fields.",
                ),
            });
            bump(breakdown, "invalid_manifest");
            return false;
        }
    };

    if let Err(err) = manifest.validate() {
        forbidden.push(ForbiddenEntry {
            path: String::from("manifest.json"),
            rule_id: String::from("invalid_manifest_schema"),
            reason: err.to_string(),
            actionable_fix: String::from("Ensure format_version is 1, 2, or 3 with valid header and modules."),
        });
        bump(breakdown, "invalid_manifest");
        return false;
    }

    let has_resources_module = manifest
        .modules
        .iter()
        .any(|m| m.kind == "resources" || m.kind == "resource");
    if !has_resources_module {
        forbidden.push(ForbiddenEntry {
            path: String::from("manifest.json"),
            rule_id: String::from("missing_resources_module"),
            reason: String::from("manifest.json modules does not declare a 'resources' module"),
            actionable_fix: String::from("Add a module with \"type\": \"resources\" and a unique UUID."),
        });
        bump(breakdown, "invalid_manifest");
        return false;
    }

    if manifest
        .modules
        .iter()
        .any(|m| m.uuid == manifest.header.uuid)
    {
        forbidden.push(ForbiddenEntry {
            path: String::from("manifest.json"),
            rule_id: String::from("duplicate_manifest_uuid"),
            reason: String::from("manifest.json header.uuid and modules[].uuid must be distinct"),
            actionable_fix: String::from("Generate two separate RFC-4122 v4 UUIDs for header and module."),
        });
        bump(breakdown, "invalid_manifest");
        return false;
    }

    if manifest.header.min_engine_version[0] != 1 {
        forbidden.push(ForbiddenEntry {
            path: String::from("manifest.json"),
            rule_id: String::from("invalid_min_engine_major_version"),
            reason: format!(
                "manifest.json header.min_engine_version {:?} has major version {} != 1; Bedrock engine rejects non-1 major versions (e.g. [26, 60, 0])",
                manifest.header.min_engine_version, manifest.header.min_engine_version[0]
            ),
            actionable_fix: String::from(
                "Normalize header.min_engine_version to [1, minor, patch] (e.g. [1, 26, 60] for Bedrock 26.60 or [1, 21, 130] for 1.21.130).",
            ),
        });
        bump(breakdown, "invalid_manifest");
        return false;
    }

    if manifest.format_version == 3
        && manifest
            .metadata
            .as_ref()
            .is_none_or(|m| m.authors.is_empty())
    {
        forbidden.push(ForbiddenEntry {
            path: String::from("manifest.json"),
            rule_id: String::from("v3_missing_metadata_authors"),
            reason: String::from(
                "Bedrock manifest.json format_version 3 requires non-empty metadata.authors",
            ),
            actionable_fix: String::from("Include \"metadata\": { \"authors\": [\"...\"] } in v3 manifest.json."),
        });
        bump(breakdown, "invalid_manifest");
        return false;
    }

    true
}

fn extract_atlas_paths(path: &AtlasTexturePath) -> Vec<String> {
    match path {
        AtlasTexturePath::Single(s) => vec![s.clone()],
        AtlasTexturePath::List(items) => items
            .iter()
            .map(|v| match v {
                AtlasVariant::Path(s) => s.clone(),
                AtlasVariant::Object { path } => path.clone(),
            })
            .collect(),
    }
}

fn ensure_png_ext(raw: &str) -> String {
    let clean = raw.trim().replace('\\', "/").trim_start_matches('/').to_string();
    if clean.to_ascii_lowercase().ends_with(".png") {
        clean
    } else {
        format!("{clean}.png")
    }
}

fn contains_case_insensitive(container: &PackContainer, target: &str) -> bool {
    if container.contains(target) {
        return true;
    }
    container
        .files()
        .keys()
        .any(|k| k.eq_ignore_ascii_case(target))
}

fn collect_embedded_texture_strings(val: &serde_json::Value, out: &mut BTreeSet<String>) {
    match val {
        serde_json::Value::String(s) => {
            if s.starts_with("textures/") {
                out.insert(ensure_png_ext(s).to_ascii_lowercase());
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                collect_embedded_texture_strings(item, out);
            }
        }
        serde_json::Value::Object(obj) => {
            for v in obj.values() {
                collect_embedded_texture_strings(v, out);
            }
        }
        _ => {}
    }
}

fn bump(map: &mut BTreeMap<String, usize>, key: &str) {
    *map.entry(key.to_string()).or_insert(0) += 1;
}
