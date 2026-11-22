use std::collections::{BTreeMap, BTreeSet};

use rp_core::{FaceDirection, MappingTable, ModelAsset, ModelCategory, ResourcePackIR};

/// Output of tracing Java `blockstates/`, `items/`, `models/block/`, and `models/item/`
/// through `parent` chains and `#texture` variables into valid Bedrock atlas bindings.
#[derive(Debug, Clone, Default)]
pub struct ResolvedModelBindings {
    /// Vanilla Bedrock `terrain_texture.json` key -> ordered deduplicated canonical Java texture IDs.
    pub terrain_bindings: BTreeMap<String, Vec<String>>,
    /// Vanilla Bedrock `item_texture.json` key -> ordered deduplicated canonical Java texture IDs.
    pub item_bindings: BTreeMap<String, Vec<String>>,
    /// All canonical Java texture IDs referenced by at least one vanilla target model/blockstate/item def.
    pub bound_textures: BTreeSet<String>,
    /// Per-model resolved canonical texture IDs (for diagnostics when skipping 3D vanilla block geo).
    pub model_textures: BTreeMap<String, Vec<String>>,
}

/// Traces Java `blockstates`, `items` definitions, and `models` (following `parent` chains and
/// `#texture` variables) to resolve custom/non-vanilla textures onto official Bedrock atlas keys.
#[must_use]
pub fn resolve_java_model_texture_bindings(
    ir: &ResourcePackIR,
    mappings: &MappingTable,
) -> ResolvedModelBindings {
    let mut out = ResolvedModelBindings::default();

    // Pre-resolve every model's effective texture map and per-face texture map.
    let mut resolved_models: BTreeMap<String, ResolvedSingleModel> = BTreeMap::new();
    for model_id in ir.models.keys() {
        let resolved = resolve_single_model(model_id, &ir.models);
        let all_tex = resolved.existing_textures(&ir.textures);
        if !all_tex.is_empty() {
            out.model_textures.insert(model_id.clone(), all_tex);
        }
        resolved_models.insert(model_id.clone(), resolved);
    }

    // 1. Collect Block targets first from `assets/minecraft/blockstates/*.json` and `assets/minecraft/models/block/*.json`.
    let mut block_targets: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (block_id, model_refs) in &ir.java_blockstates {
        let entry = block_targets.entry(block_id.clone()).or_default();
        for m in model_refs {
            push_unique(entry, normalize_model_ref(m));
        }
    }
    for (model_id, model) in &ir.models {
        if model.category == ModelCategory::Block
            && let Some(stem) = model_id
                .strip_prefix("block/")
                .or_else(|| model_id.strip_prefix("blocks/"))
            && !stem.contains('/')
        {
            let entry = block_targets.entry(stem.to_string()).or_default();
            push_unique(entry, model_id.clone());
            // Vanilla staged crop models (`<crop>_stage<N>`) belong to base block `<crop>` even
            // when a pack overrides only the model files without shipping `blockstates/<crop>.json`.
            if let Some((base_block, _)) = split_stage_suffix(stem) {
                let base_entry = block_targets.entry(base_block.to_string()).or_default();
                push_unique(base_entry, model_id.clone());
            }
        }
    }

    // 2. Collect Item targets from `assets/minecraft/items/*.json` and `assets/minecraft/models/item/*.json`.
    let mut item_targets: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (item_id, model_refs) in &ir.java_item_defs {
        let entry = item_targets.entry(item_id.clone()).or_default();
        for m in model_refs {
            push_unique(entry, normalize_model_ref(m));
        }
    }
    for (model_id, model) in &ir.models {
        if model.category == ModelCategory::Item
            && let Some(stem) = model_id
                .strip_prefix("item/")
                .or_else(|| model_id.strip_prefix("items/"))
            && !stem.contains('/')
        {
            let entry = item_targets.entry(stem.to_string()).or_default();
            push_unique(entry, model_id.clone());
        }
    }

    for (item_id, model_ids) in &item_targets {
        let bedrock_item_key = resolve_vanilla_item_key(item_id, mappings);
        let carried_terrain_key = mappings.vanilla_index().carried_terrain_key(item_id);
        for model_id in model_ids {
            let Some(resolved) = resolved_models.get(model_id) else {
                continue;
            };
            let Some(primary_tex) = resolved.primary_texture(&ir.textures) else {
                continue;
            };

            if let Some(ref item_key) = bedrock_item_key {
                let list = out.item_bindings.entry(item_key.clone()).or_default();
                push_unique(list, primary_tex.clone());
                out.bound_textures.insert(primary_tex.clone());
            } else if let Some(ref carried_key) = carried_terrain_key {
                // Bedrock renders block-items like ferns, grass, waterlily, and double plants via
                // dedicated `*_carried` terrain_texture.json keys without polluting block faces.
                let list = out.terrain_bindings.entry(carried_key.clone()).or_default();
                push_unique(list, primary_tex.clone());
                out.bound_textures.insert(primary_tex.clone());
            } else if !block_targets.contains_key(item_id)
                && let Some(block_keys) = mappings.vanilla_index().block_texture_keys(item_id)
            {
                // Block-item without separate item_texture key or block_targets entry.
                for atlas_key in block_keys.values() {
                    if atlas_key == "sunflower_additional" && item_id != "sunflower" {
                        continue;
                    }
                    if mappings.is_known_terrain_key(atlas_key) {
                        let list = out.terrain_bindings.entry(atlas_key.clone()).or_default();
                        push_unique(list, primary_tex.clone());
                        out.bound_textures.insert(primary_tex.clone());
                    }
                }
            }
        }
    }

    // 3. Resolve Block targets onto Bedrock terrain_texture.json atlas keys.
    for (block_id, raw_model_ids) in &block_targets {
        let block_face_map = mappings.vanilla_index().block_texture_keys(block_id);
        let direct_terrain_key = resolve_vanilla_terrain_key(block_id, mappings);

        let mut model_ids = raw_model_ids.clone();
        model_ids.sort_by(|a, b| {
            let sa = model_stage_index(a).unwrap_or(usize::MAX);
            let sb = model_stage_index(b).unwrap_or(usize::MAX);
            sa.cmp(&sb).then_with(|| a.cmp(b))
        });

        // For Bedrock multi-stage array crops (wheat: 8, carrots/potatoes/beetroot: 4, cocoa: 3),
        // preserve exact stage-indexed array positions so growth stage N maps to index N.
        if let Some(expected_stages) = bedrock_crop_stage_count(block_id)
            && let Some(face_keys) = block_face_map
            && let Some(atlas_key) = face_keys.values().next()
            && face_keys.values().all(|k| k == atlas_key)
            && mappings.is_known_terrain_key(atlas_key)
        {
            let mut stage_tex: BTreeMap<usize, String> = BTreeMap::new();
            for model_id in &model_ids {
                if let Some(stage_idx) = model_stage_index(model_id)
                    && let Some(resolved) = resolved_models.get(model_id)
                    && let Some(tex_id) = resolved.primary_texture(&ir.textures)
                {
                    stage_tex.insert(stage_idx, tex_id);
                }
            }
            if !stage_tex.is_empty() {
                let mut slots: Vec<String> = Vec::with_capacity(expected_stages);
                let mut last_seen: Option<String> = stage_tex.values().next().cloned();
                for idx in 0..expected_stages {
                    if let Some(tex_id) = stage_tex.get(&idx) {
                        last_seen = Some(tex_id.clone());
                    }
                    if let Some(ref tex_id) = last_seen {
                        slots.push(tex_id.clone());
                    }
                }
                // If Java has an extra ripe stage (e.g. stage4 for 8-age carrots/potatoes),
                // bind it to the final mature Bedrock slot.
                if let Some((&max_stage, max_tex)) = stage_tex.iter().next_back()
                    && max_stage >= expected_stages
                    && let Some(last_slot) = slots.last_mut()
                {
                    *last_slot = max_tex.clone();
                }
                if !slots.is_empty() {
                    for tex_id in &slots {
                        out.bound_textures.insert(tex_id.clone());
                    }
                    out.terrain_bindings.insert(atlas_key.clone(), slots);
                    continue;
                }
            }
        }

        for model_id in &model_ids {
            let Some(resolved) = resolved_models.get(model_id) else {
                continue;
            };
            let primary_tex = resolved.primary_texture(&ir.textures);
            let half = model_half(model_id);
            let stage_idx = model_stage_index(model_id);

            if let Some(face_keys) = block_face_map {
                // If Bedrock uses distinct per-stage terrain keys (e.g. sweet_berry_bush_0..3),
                // bind stage N exclusively to its matching `*_N` key.
                if let Some(stage_num) = stage_idx {
                    let stage_suffix = format!("_{stage_num}");
                    let matching_stage_key = face_keys
                        .values()
                        .find(|k| k.ends_with(&stage_suffix) && mappings.is_known_terrain_key(k));
                    if let Some(atlas_key) = matching_stage_key {
                        if let Some(tex_id) = primary_tex {
                            let list = out.terrain_bindings.entry(atlas_key.clone()).or_default();
                            push_unique(list, tex_id.clone());
                            out.bound_textures.insert(tex_id);
                        }
                        continue;
                    }
                }

                let mut bound_any = false;
                for (bedrock_face, atlas_key) in face_keys {
                    if !mappings.is_known_terrain_key(atlas_key) {
                        continue;
                    }
                    // Bedrock's blocks.json assigns `side: "sunflower_additional"` to legacy double
                    // plants; only `sunflower` itself should ever populate `sunflower_additional`.
                    if atlas_key == "sunflower_additional" && block_id != "sunflower" {
                        continue;
                    }
                    match half {
                        Some(PlantHalf::Bottom) if is_top_face_or_key(bedrock_face, atlas_key) => {
                            continue;
                        }
                        Some(PlantHalf::Top) if is_bottom_face_or_key(bedrock_face, atlas_key) => {
                            continue;
                        }
                        _ => {}
                    }
                    let chosen = resolved
                        .texture_for_bedrock_face(bedrock_face, &ir.textures)
                        .or_else(|| primary_tex.clone());
                    if let Some(tex_id) = chosen {
                        let list = out.terrain_bindings.entry(atlas_key.clone()).or_default();
                        push_unique(list, tex_id.clone());
                        out.bound_textures.insert(tex_id);
                        bound_any = true;
                    }
                }
                if bound_any
                    && let Some(ref extra_key) = direct_terrain_key
                    && let Some(ref tex_id) = primary_tex
                    && !out.terrain_bindings.contains_key(extra_key)
                {
                    let list = out.terrain_bindings.entry(extra_key.clone()).or_default();
                    push_unique(list, tex_id.clone());
                    out.bound_textures.insert(tex_id.clone());
                }
            } else if let Some(ref terrain_key) = direct_terrain_key
                && let Some(ref tex_id) = primary_tex
            {
                let list = out.terrain_bindings.entry(terrain_key.clone()).or_default();
                push_unique(list, tex_id.clone());
                out.bound_textures.insert(tex_id.clone());
            }
        }
    }

    out
}

#[derive(Debug, Clone, Default)]
struct ResolvedSingleModel {
    textures: BTreeMap<String, String>,
    face_textures: BTreeMap<FaceDirection, String>,
}

impl ResolvedSingleModel {
    fn existing_textures(&self, available: &BTreeMap<String, rp_core::TextureAsset>) -> Vec<String> {
        let mut out = Vec::new();
        for tex in self.textures.values() {
            if available.contains_key(tex) {
                push_unique(&mut out, tex.clone());
            }
        }
        for tex in self.face_textures.values() {
            if available.contains_key(tex) {
                push_unique(&mut out, tex.clone());
            }
        }
        out
    }

    fn primary_texture(&self, available: &BTreeMap<String, rp_core::TextureAsset>) -> Option<String> {
        const PRIORITY_KEYS: &[&str] = &[
            "layer0", "all", "texture", "cross", "crop", "plant", "stem", "stalk", "side", "front",
            "particle", "0", "1", "top", "up", "north", "south", "east", "west", "bottom", "down",
        ];
        for key in PRIORITY_KEYS {
            if let Some(tex) = self.textures.get(*key)
                && available.contains_key(tex)
            {
                return Some(tex.clone());
            }
        }
        for dir in FaceDirection::ALL {
            if let Some(tex) = self.face_textures.get(&dir)
                && available.contains_key(tex)
            {
                return Some(tex.clone());
            }
        }
        for tex in self.textures.values() {
            if available.contains_key(tex) {
                return Some(tex.clone());
            }
        }
        None
    }

    fn texture_for_bedrock_face(
        &self,
        bedrock_face: &str,
        available: &BTreeMap<String, rp_core::TextureAsset>,
    ) -> Option<String> {
        let candidates: &[&str] = match bedrock_face {
            "up" => &["up", "top", "end", "all"],
            "down" => &["down", "bottom", "end", "all"],
            "side" => &["side", "wall", "north", "south", "east", "west", "all"],
            "north" => &["north", "front", "side", "all"],
            "south" => &["south", "back", "side", "all"],
            "east" => &["east", "side", "all"],
            "west" => &["west", "side", "all"],
            _ => &["all", "texture", "cross", "plant", "0"],
        };
        for key in candidates {
            if let Some(tex) = self.textures.get(*key)
                && available.contains_key(tex)
            {
                return Some(tex.clone());
            }
        }
        let dir = match bedrock_face {
            "up" => Some(FaceDirection::Up),
            "down" => Some(FaceDirection::Down),
            "north" => Some(FaceDirection::North),
            "south" => Some(FaceDirection::South),
            "east" => Some(FaceDirection::East),
            "west" => Some(FaceDirection::West),
            "side" => Some(FaceDirection::North),
            _ => None,
        };
        if let Some(d) = dir
            && let Some(tex) = self.face_textures.get(&d)
            && available.contains_key(tex)
        {
            return Some(tex.clone());
        }
        None
    }
}

fn resolve_single_model(
    start_id: &str,
    models: &BTreeMap<String, ModelAsset>,
) -> ResolvedSingleModel {
    // Walk parent chain: chain[0] = child, chain[last] = deepest ancestor in pack.
    let mut chain: Vec<&ModelAsset> = Vec::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut cur_id = start_id.to_string();

    for _ in 0..16 {
        if !visited.insert(cur_id.clone()) {
            break;
        }
        let Some(model) = models.get(&cur_id) else {
            break;
        };
        chain.push(model);
        let Some(parent) = &model.parent else {
            break;
        };
        cur_id = normalize_model_ref(parent);
    }

    // Merge textures from ancestor -> child (child overrides parent).
    let mut raw_textures: BTreeMap<String, String> = BTreeMap::new();
    for m in chain.iter().rev() {
        for (k, v) in &m.textures {
            raw_textures.insert(k.clone(), v.clone());
        }
    }

    // Resolve `#var` indirections recursively.
    let mut resolved_textures: BTreeMap<String, String> = BTreeMap::new();
    for key in raw_textures.keys() {
        if let Some(final_tex) = resolve_texture_var(key, &raw_textures) {
            resolved_textures.insert(key.clone(), normalize_tex_ref(&final_tex));
        }
    }

    // Pick elements from the nearest model in the chain that defines them (Java override semantics).
    let mut face_textures: BTreeMap<FaceDirection, String> = BTreeMap::new();
    if let Some(owner) = chain.iter().find(|m| !m.elements.is_empty()) {
        for el in &owner.elements {
            for (&dir, face) in &el.faces {
                if face_textures.contains_key(&dir) {
                    continue;
                }
                let raw_ref = face.texture.as_str();
                let resolved = if let Some(var_name) = raw_ref.strip_prefix('#') {
                    resolved_textures.get(var_name).cloned()
                } else {
                    Some(normalize_tex_ref(raw_ref))
                };
                if let Some(tex) = resolved {
                    face_textures.insert(dir, tex);
                }
            }
        }
    }

    ResolvedSingleModel {
        textures: resolved_textures,
        face_textures,
    }
}

fn resolve_texture_var(start_var: &str, map: &BTreeMap<String, String>) -> Option<String> {
    let mut cur = map.get(start_var)?.as_str();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    seen.insert(start_var);

    for _ in 0..16 {
        if let Some(next_var) = cur.strip_prefix('#') {
            if !seen.insert(next_var) {
                return None;
            }
            cur = map.get(next_var)?.as_str();
        } else {
            return Some(cur.to_string());
        }
    }
    None
}

fn resolve_vanilla_item_key(item_id: &str, mappings: &MappingTable) -> Option<String> {
    let java_tex_id = format!("item/{item_id}");
    let target = mappings.java_texture_to_bedrock(&java_tex_id);
    if mappings.is_known_item_key(&target.atlas_key) {
        return Some(target.atlas_key);
    }
    if mappings.vanilla_index().is_vanilla_item_key(item_id) {
        return Some(item_id.to_string());
    }
    None
}

fn resolve_vanilla_terrain_key(block_id: &str, mappings: &MappingTable) -> Option<String> {
    let java_tex_id = format!("block/{block_id}");
    let target = mappings.java_texture_to_bedrock(&java_tex_id);
    if mappings.is_known_terrain_key(&target.atlas_key) {
        return Some(target.atlas_key);
    }
    if mappings.vanilla_index().is_vanilla_terrain_key(block_id) {
        return Some(block_id.to_string());
    }
    None
}

fn normalize_model_ref(raw: &str) -> String {
    let trimmed = raw.trim().replace('\\', "/");
    let no_ns = trimmed
        .strip_prefix("minecraft:")
        .unwrap_or(trimmed.as_str());
    let no_lead = no_ns.trim_start_matches('/');
    no_lead.strip_suffix(".json").unwrap_or(no_lead).to_string()
}

fn normalize_tex_ref(raw: &str) -> String {
    let trimmed = raw.trim().replace('\\', "/");
    let no_ns = trimmed
        .strip_prefix("minecraft:")
        .unwrap_or(trimmed.as_str());
    let no_lead = no_ns.trim_start_matches('/');
    let no_ext = no_lead.strip_suffix(".png").unwrap_or(no_lead);
    if let Some(rest) = no_ext.strip_prefix("blocks/") {
        format!("block/{rest}")
    } else if let Some(rest) = no_ext.strip_prefix("items/") {
        format!("item/{rest}")
    } else {
        no_ext.to_string()
    }
}

fn push_unique(vec: &mut Vec<String>, val: String) {
    if !vec.contains(&val) {
        vec.push(val);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlantHalf {
    Bottom,
    Top,
}

fn split_stage_suffix(stem: &str) -> Option<(&str, usize)> {
    let (base, digits) = stem.rsplit_once("_stage")?;
    if base.is_empty() || digits.is_empty() {
        return None;
    }
    let idx = digits.parse::<usize>().ok()?;
    Some((base, idx))
}

fn model_stage_index(model_id: &str) -> Option<usize> {
    let stem = model_id.rsplit('/').next().unwrap_or(model_id);
    split_stage_suffix(stem).map(|(_, idx)| idx)
}

fn bedrock_crop_stage_count(block_id: &str) -> Option<usize> {
    match block_id {
        "wheat" => Some(8),
        "carrots" | "potatoes" | "beetroots" | "beetroot" => Some(4),
        "cocoa" => Some(3),
        _ => None,
    }
}

fn model_half(model_id: &str) -> Option<PlantHalf> {
    let stem = model_id.rsplit('/').next().unwrap_or(model_id);
    if stem.ends_with("_bottom")
        || stem.ends_with("_lower")
        || stem.contains("_bottom_")
        || stem.contains("_lower_")
    {
        Some(PlantHalf::Bottom)
    } else if stem.ends_with("_top")
        || stem.ends_with("_upper")
        || stem.contains("_top_")
        || stem.contains("_upper_")
    {
        Some(PlantHalf::Top)
    } else {
        None
    }
}

fn is_top_face_or_key(bedrock_face: &str, atlas_key: &str) -> bool {
    bedrock_face == "up"
        || atlas_key.ends_with("_top")
        || atlas_key.contains("_top_")
        || atlas_key == "sunflower_additional"
}

fn is_bottom_face_or_key(bedrock_face: &str, atlas_key: &str) -> bool {
    bedrock_face == "down"
        || atlas_key.ends_with("_bottom")
        || atlas_key.contains("_bot_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_core::{ElementFace, ModelElement, TextureAsset, TextureCategory, TexturePayload};

    fn dummy_tex(id: &str) -> TextureAsset {
        TextureAsset {
            id: id.to_string(),
            category: TextureCategory::infer_from_id(id),
            atlas_key: None,
            width: Some(16),
            height: Some(16),
            resolution_tier: None,
            payload: TexturePayload::from_raw(vec![0]),
            animation: None,
        }
    }

    #[test]
    fn resolves_generic_custom_textures_through_parent_and_hash_vars() {
        let mappings = MappingTable::default_embedded().expect("mappings");
        let mut ir = ResourcePackIR::default();
        ir.textures.insert(
            String::from("block/custom_tex_a"),
            dummy_tex("block/custom_tex_a"),
        );
        ir.textures.insert(
            String::from("block/custom_tex_b"),
            dummy_tex("block/custom_tex_b"),
        );
        ir.textures.insert(
            String::from("block/custom_sub/tex_c"),
            dummy_tex("block/custom_sub/tex_c"),
        );

        // models/item/apple.json -> layer0: "block/custom_tex_a"
        ir.models.insert(
            String::from("item/apple"),
            ModelAsset {
                id: String::from("item/apple"),
                category: ModelCategory::Item,
                parent: Some(String::from("minecraft:item/generated")),
                textures: BTreeMap::from([(
                    String::from("layer0"),
                    String::from("minecraft:block/custom_tex_a"),
                )]),
                texture_size: [16, 16],
                ambient_occlusion: true,
                elements: Vec::new(),
                display: BTreeMap::new(),
                has_overrides: false,
                overrides_count: 0,
            },
        );

        // models/block/bamboo_base.json + models/block/bamboo.json with #var indirection
        ir.models.insert(
            String::from("block/bamboo_base"),
            ModelAsset {
                id: String::from("block/bamboo_base"),
                category: ModelCategory::Block,
                parent: None,
                textures: BTreeMap::from([(String::from("all"), String::from("#stem"))]),
                texture_size: [16, 16],
                ambient_occlusion: true,
                elements: Vec::new(),
                display: BTreeMap::new(),
                has_overrides: false,
                overrides_count: 0,
            },
        );
        ir.models.insert(
            String::from("block/bamboo"),
            ModelAsset {
                id: String::from("block/bamboo"),
                category: ModelCategory::Block,
                parent: Some(String::from("block/bamboo_base")),
                textures: BTreeMap::from([(
                    String::from("stem"),
                    String::from("block/custom_tex_b"),
                )]),
                texture_size: [16, 16],
                ambient_occlusion: true,
                elements: Vec::new(),
                display: BTreeMap::new(),
                has_overrides: false,
                overrides_count: 0,
            },
        );

        // blockstates/short_grass.json -> block/grass_variant_1 -> block/custom_sub/tex_c
        ir.java_blockstates.insert(
            String::from("short_grass"),
            vec![String::from("block/grass_variant_1")],
        );
        ir.models.insert(
            String::from("block/grass_variant_1"),
            ModelAsset {
                id: String::from("block/grass_variant_1"),
                category: ModelCategory::Block,
                parent: Some(String::from("block/cross")),
                textures: BTreeMap::from([(
                    String::from("0"),
                    String::from("block/custom_sub/tex_c"),
                )]),
                texture_size: [16, 16],
                ambient_occlusion: true,
                elements: vec![ModelElement {
                    group: None,
                    from: [0.0, 0.0, 8.0],
                    to: [16.0, 16.0, 8.0],
                    rotation: None,
                    faces: BTreeMap::from([(
                        FaceDirection::North,
                        ElementFace {
                            uv: None,
                            texture: String::from("#0"),
                            cullface: None,
                            rotation: 0,
                            tintindex: None,
                        },
                    )]),
                    shade: true,
                    shade_direction_override: None,
                    light_emission: None,
                }],
                display: BTreeMap::new(),
                has_overrides: false,
                overrides_count: 0,
            },
        );

        let bindings = resolve_java_model_texture_bindings(&ir, &mappings);
        assert_eq!(
            bindings.item_bindings.get("apple"),
            Some(&vec![String::from("block/custom_tex_a")])
        );
        assert_eq!(
            bindings.terrain_bindings.get("bamboo_stem"),
            Some(&vec![String::from("block/custom_tex_b")])
        );
        assert_eq!(
            bindings.terrain_bindings.get("short_grass"),
            Some(&vec![String::from("block/custom_sub/tex_c")])
        );
        assert!(bindings.bound_textures.contains("block/custom_tex_a"));
        assert!(bindings.bound_textures.contains("block/custom_tex_b"));
        assert!(bindings.bound_textures.contains("block/custom_sub/tex_c"));
    }

    #[test]
    fn isolates_double_plant_halves_carried_items_and_staged_crops_without_orphans() {
        let mappings = MappingTable::default_embedded().expect("mappings");
        let mut ir = ResourcePackIR::default();
        for tex in [
            "block/custom_grass_bot",
            "block/custom_grass_top",
            "block/custom_lily_carried",
            "block/custom_berry_0",
            "block/custom_berry_3",
            "block/custom_potato_0",
            "block/custom_potato_3",
            "block/custom_potato_4",
        ] {
            ir.textures.insert(tex.to_string(), dummy_tex(tex));
        }

        // Tall grass bottom + top must NOT pollute sunflower_additional or cross-contaminate halves.
        ir.java_blockstates.insert(
            String::from("tall_grass"),
            vec![
                String::from("block/tall_grass_bottom"),
                String::from("block/tall_grass_top"),
            ],
        );
        let mk_block = |id: &str, tex: &str| ModelAsset {
            id: id.to_string(),
            category: ModelCategory::Block,
            parent: None,
            textures: BTreeMap::from([(String::from("particle"), tex.to_string())]),
            texture_size: [16, 16],
            ambient_occlusion: true,
            elements: Vec::new(),
            display: BTreeMap::new(),
            has_overrides: false,
            overrides_count: 0,
        };
        ir.models.insert(
            String::from("block/tall_grass_bottom"),
            mk_block("block/tall_grass_bottom", "block/custom_grass_bot"),
        );
        ir.models.insert(
            String::from("block/tall_grass_top"),
            mk_block("block/tall_grass_top", "block/custom_grass_top"),
        );
        // Block-item lily_pad should resolve to waterlily_carried.
        ir.models.insert(
            String::from("item/lily_pad"),
            ModelAsset {
                id: String::from("item/lily_pad"),
                category: ModelCategory::Item,
                parent: None,
                textures: BTreeMap::from([(
                    String::from("layer0"),
                    String::from("block/custom_lily_carried"),
                )]),
                texture_size: [16, 16],
                ambient_occlusion: true,
                elements: Vec::new(),
                display: BTreeMap::new(),
                has_overrides: false,
                overrides_count: 0,
            },
        );
        // Sweet berry bush stages map to separate sweet_berry_bush_0 / sweet_berry_bush_3 keys.
        ir.models.insert(
            String::from("block/sweet_berry_bush_stage0"),
            mk_block("block/sweet_berry_bush_stage0", "block/custom_berry_0"),
        );
        ir.models.insert(
            String::from("block/sweet_berry_bush_stage3"),
            mk_block("block/sweet_berry_bush_stage3", "block/custom_berry_3"),
        );
        // Potatoes with stage4 overriding slot 3 must NOT leave stage3's unused texture in bound_textures.
        ir.models.insert(
            String::from("block/potatoes_stage0"),
            mk_block("block/potatoes_stage0", "block/custom_potato_0"),
        );
        ir.models.insert(
            String::from("block/potatoes_stage3"),
            mk_block("block/potatoes_stage3", "block/custom_potato_3"),
        );
        ir.models.insert(
            String::from("block/potatoes_stage4"),
            mk_block("block/potatoes_stage4", "block/custom_potato_4"),
        );

        let bindings = resolve_java_model_texture_bindings(&ir, &mappings);
        assert!(!bindings.terrain_bindings.contains_key("sunflower_additional"));
        assert_eq!(
            bindings.terrain_bindings.get("tall_grass_bottom"),
            Some(&vec![String::from("block/custom_grass_bot")])
        );
        assert_eq!(
            bindings.terrain_bindings.get("tall_grass_top"),
            Some(&vec![String::from("block/custom_grass_top")])
        );
        assert_eq!(
            bindings.terrain_bindings.get("waterlily_carried"),
            Some(&vec![String::from("block/custom_lily_carried")])
        );
        assert_eq!(
            bindings.terrain_bindings.get("sweet_berry_bush_0"),
            Some(&vec![String::from("block/custom_berry_0")])
        );
        assert_eq!(
            bindings.terrain_bindings.get("sweet_berry_bush_3"),
            Some(&vec![String::from("block/custom_berry_3")])
        );
        assert_eq!(
            bindings.terrain_bindings.get("potatoes"),
            Some(&vec![
                String::from("block/custom_potato_0"),
                String::from("block/custom_potato_0"),
                String::from("block/custom_potato_0"),
                String::from("block/custom_potato_4"),
            ])
        );
        assert!(!bindings.bound_textures.contains("block/custom_potato_3"));
        assert!(bindings.bound_textures.contains("block/custom_potato_4"));
    }
}
