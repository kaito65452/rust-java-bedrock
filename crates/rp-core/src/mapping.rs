use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{CoreError, Edition, TextureCategory};

const DEFAULT_MAPPINGS_JSON: &str = include_str!("../data/default_mappings.json");

/// Version correspondence entry between Java `pack_format` and Bedrock `min_engine_version`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackVersionEntry {
    /// Human-readable Minecraft release string (e.g. `"1.20.1"`).
    pub minecraft_version: String,
    /// Java Edition `pack_format` integer.
    pub java_pack_format: u32,
    /// Bedrock Edition `header.min_engine_version` triplet.
    pub bedrock_min_engine_version: [u32; 3],
}

/// Explicit bidirectional texture mapping entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureMappingEntry {
    /// Canonical / Java texture identifier without `.png` (e.g. `"block/oak_planks"`).
    pub java: String,
    /// Bedrock texture path without `.png` (e.g. `"textures/blocks/planks_oak"`).
    pub bedrock: String,
    /// Optional Bedrock `terrain_texture.json` or `item_texture.json` short key.
    pub atlas_key: Option<String>,
}

/// Resolved Bedrock texture destination and atlas metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockTextureTarget {
    /// Relative Bedrock texture path without `.png` (e.g. `"textures/blocks/stone"`).
    pub path: String,
    /// Short atlas registration key for `terrain_texture.json` or `item_texture.json`.
    pub atlas_key: String,
    /// Logical texture category.
    pub category: TextureCategory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MappingFileSchema {
    #[serde(default = "default_schema_version")]
    schema_version: String,
    #[serde(default)]
    pack_format_versions: Vec<PackVersionEntry>,
    #[serde(default)]
    textures: Vec<TextureMappingEntry>,
    #[serde(default)]
    model_parents: HashMap<String, String>,
    #[serde(default)]
    sound_events: HashMap<String, String>,
    #[serde(default)]
    locale_keys: HashMap<String, String>,
}

fn default_schema_version() -> String {
    String::from("1.0")
}

/// Externalized versioned mapping registry for cross-edition resource pack conversion.
#[derive(Debug, Clone)]
pub struct MappingTable {
    schema_version: String,
    pack_format_versions: Vec<PackVersionEntry>,
    textures: Vec<TextureMappingEntry>,
    model_parents: HashMap<String, String>,
    sound_events: HashMap<String, String>,
    locale_keys: HashMap<String, String>,
    // Precomputed lookup indices
    java_to_bedrock_tex: HashMap<String, TextureMappingEntry>,
    bedrock_to_java_tex: HashMap<String, String>,
    reverse_model_parents: HashMap<String, String>,
    reverse_sound_events: HashMap<String, String>,
    reverse_locale_keys: HashMap<String, String>,
}

impl MappingTable {
    /// Loads the embedded default mapping table compiled into `rp-core`.
    pub fn default_embedded() -> Result<Self, CoreError> {
        Self::from_json_str(DEFAULT_MAPPINGS_JSON)
    }

    /// Loads a mapping table from a JSON file on disk.
    pub fn from_file(path: &Path) -> Result<Self, CoreError> {
        let content = fs::read_to_string(path).map_err(|err| {
            CoreError::Mapping(format!(
                "failed to read mapping file '{}': {err}",
                path.display()
            ))
        })?;
        Self::from_json_str(&content)
    }

    /// Parses a mapping table from a JSON string.
    pub fn from_json_str(json: &str) -> Result<Self, CoreError> {
        let parsed: MappingFileSchema = serde_json::from_str(json)
            .map_err(|err| CoreError::Mapping(format!("invalid mapping JSON: {err}")))?;
        let mut table = Self {
            schema_version: parsed.schema_version,
            pack_format_versions: parsed.pack_format_versions,
            textures: parsed.textures,
            model_parents: parsed.model_parents,
            sound_events: parsed.sound_events,
            locale_keys: parsed.locale_keys,
            java_to_bedrock_tex: HashMap::new(),
            bedrock_to_java_tex: HashMap::new(),
            reverse_model_parents: HashMap::new(),
            reverse_sound_events: HashMap::new(),
            reverse_locale_keys: HashMap::new(),
        };
        table.rebuild_indices();
        Ok(table)
    }

    /// Merges an override mapping table on top of `self`, replacing matching keys.
    pub fn merge_override(&mut self, override_table: Self) {
        if !override_table.schema_version.is_empty() {
            self.schema_version = override_table.schema_version;
        }
        for ver in override_table.pack_format_versions {
            if let Some(existing) = self
                .pack_format_versions
                .iter_mut()
                .find(|e| e.minecraft_version == ver.minecraft_version)
            {
                *existing = ver;
            } else {
                self.pack_format_versions.push(ver);
            }
        }
        for tex in override_table.textures {
            if let Some(existing) = self.textures.iter_mut().find(|t| t.java == tex.java) {
                *existing = tex;
            } else {
                self.textures.push(tex);
            }
        }
        self.model_parents.extend(override_table.model_parents);
        self.sound_events.extend(override_table.sound_events);
        self.locale_keys.extend(override_table.locale_keys);
        self.rebuild_indices();
    }

    fn rebuild_indices(&mut self) {
        self.java_to_bedrock_tex.clear();
        self.bedrock_to_java_tex.clear();
        for entry in &self.textures {
            let norm_java = normalize_texture_id(&entry.java);
            let norm_bedrock = normalize_texture_id(&entry.bedrock);
            self.java_to_bedrock_tex.insert(
                norm_java.clone(),
                TextureMappingEntry {
                    java: norm_java.clone(),
                    bedrock: norm_bedrock.clone(),
                    atlas_key: entry.atlas_key.clone(),
                },
            );
            self.bedrock_to_java_tex.insert(norm_bedrock, norm_java);
        }

        self.reverse_model_parents.clear();
        for (java_parent, bedrock_parent) in &self.model_parents {
            let prefer_new = java_parent.starts_with("minecraft:");
            self.reverse_model_parents
                .entry(bedrock_parent.clone())
                .and_modify(|existing| {
                    if prefer_new && !existing.starts_with("minecraft:") {
                        *existing = java_parent.clone();
                    }
                })
                .or_insert_with(|| java_parent.clone());
        }

        self.reverse_sound_events.clear();
        for (java_ev, bedrock_ev) in &self.sound_events {
            self.reverse_sound_events
                .insert(bedrock_ev.clone(), java_ev.clone());
        }

        self.reverse_locale_keys.clear();
        for (java_k, bedrock_k) in &self.locale_keys {
            self.reverse_locale_keys
                .insert(bedrock_k.clone(), java_k.clone());
        }
    }

    /// Returns the schema version string of the loaded mapping file.
    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    /// Returns all known version correspondences.
    #[must_use]
    pub fn pack_format_versions(&self) -> &[PackVersionEntry] {
        &self.pack_format_versions
    }

    /// Resolves a Java/canonical texture ID to its Bedrock path and atlas registration key.
    #[must_use]
    pub fn java_texture_to_bedrock(&self, java_id: &str) -> BedrockTextureTarget {
        let norm = normalize_texture_id(java_id);
        let category = TextureCategory::infer_from_id(&norm);

        if let Some(mapped) = self.java_to_bedrock_tex.get(&norm) {
            let default_key = default_atlas_key_from_bedrock(&mapped.bedrock);
            return BedrockTextureTarget {
                path: mapped.bedrock.clone(),
                atlas_key: mapped.atlas_key.clone().unwrap_or(default_key),
                category,
            };
        }

        let (path, atlas_key) = if let Some(rest) = norm.strip_prefix("block/") {
            (format!("textures/blocks/{rest}"), rest.replace('/', "_"))
        } else if let Some(rest) = norm.strip_prefix("blocks/") {
            (format!("textures/blocks/{rest}"), rest.replace('/', "_"))
        } else if let Some(rest) = norm.strip_prefix("item/") {
            (format!("textures/items/{rest}"), rest.replace('/', "_"))
        } else if let Some(rest) = norm.strip_prefix("items/") {
            (format!("textures/items/{rest}"), rest.replace('/', "_"))
        } else if let Some(rest) = norm.strip_prefix("gui/") {
            (format!("textures/ui/{rest}"), rest.replace('/', "_"))
        } else if norm.starts_with("textures/") {
            (norm.clone(), default_atlas_key_from_bedrock(&norm))
        } else {
            (format!("textures/{norm}"), norm.replace('/', "_"))
        };

        BedrockTextureTarget {
            path,
            atlas_key,
            category,
        }
    }

    /// Resolves a Bedrock texture path (with or without `.png`) to a canonical/Java texture ID.
    #[must_use]
    pub fn bedrock_texture_to_java(&self, bedrock_path: &str) -> String {
        let norm = normalize_texture_id(bedrock_path);
        if let Some(mapped) = self.bedrock_to_java_tex.get(&norm) {
            return mapped.clone();
        }

        if let Some(rest) = norm.strip_prefix("textures/blocks/") {
            format!("block/{rest}")
        } else if let Some(rest) = norm.strip_prefix("textures/items/") {
            format!("item/{rest}")
        } else if let Some(rest) = norm.strip_prefix("textures/ui/") {
            format!("gui/{rest}")
        } else if let Some(rest) = norm.strip_prefix("textures/") {
            rest.to_string()
        } else {
            norm
        }
    }

    /// Maps a Java model parent identifier to Bedrock geometry identifier if mapped.
    #[must_use]
    pub fn java_model_parent_to_bedrock(&self, parent: &str) -> Option<String> {
        self.model_parents.get(parent).cloned()
    }

    /// Maps a Bedrock geometry identifier back to a Java model parent if mapped.
    #[must_use]
    pub fn bedrock_model_parent_to_java(&self, geo_id: &str) -> Option<String> {
        self.reverse_model_parents.get(geo_id).cloned()
    }

    /// Maps a Java sound event ID to Bedrock `sound_definitions.json` event ID.
    #[must_use]
    pub fn java_sound_to_bedrock(&self, event: &str) -> String {
        self.sound_events
            .get(event)
            .cloned()
            .unwrap_or_else(|| event.to_string())
    }

    /// Maps a Bedrock sound definition ID to a Java `sounds.json` event ID.
    #[must_use]
    pub fn bedrock_sound_to_java(&self, event: &str) -> String {
        self.reverse_sound_events
            .get(event)
            .cloned()
            .unwrap_or_else(|| event.to_string())
    }

    /// Maps a Java localization key to its Bedrock `.lang` equivalent using table + heuristics.
    #[must_use]
    pub fn java_locale_key_to_bedrock(&self, key: &str) -> String {
        if let Some(mapped) = self.locale_keys.get(key) {
            return mapped.clone();
        }
        if let Some(rest) = key.strip_prefix("block.minecraft.") {
            format!("tile.{rest}.name")
        } else if let Some(rest) = key.strip_prefix("item.minecraft.") {
            format!("item.{rest}.name")
        } else if let Some(rest) = key.strip_prefix("entity.minecraft.") {
            format!("entity.{rest}.name")
        } else {
            key.to_string()
        }
    }

    /// Maps a Bedrock `.lang` key to its Java localization key equivalent using table + heuristics.
    #[must_use]
    pub fn bedrock_locale_key_to_java(&self, key: &str) -> String {
        if let Some(mapped) = self.reverse_locale_keys.get(key) {
            return mapped.clone();
        }
        if let Some(rest) = key
            .strip_prefix("tile.")
            .and_then(|s| s.strip_suffix(".name"))
        {
            format!("block.minecraft.{rest}")
        } else if let Some(rest) = key
            .strip_prefix("item.")
            .and_then(|s| s.strip_suffix(".name"))
        {
            format!("item.minecraft.{rest}")
        } else if let Some(rest) = key
            .strip_prefix("entity.")
            .and_then(|s| s.strip_suffix(".name"))
        {
            format!("entity.minecraft.{rest}")
        } else {
            key.to_string()
        }
    }

    /// Maps a Java `pack_format` integer to the closest Bedrock `min_engine_version`.
    #[must_use]
    pub fn java_pack_format_to_bedrock(&self, pack_format: u32) -> [u32; 3] {
        if let Some(exact) = self
            .pack_format_versions
            .iter()
            .find(|e| e.java_pack_format == pack_format)
        {
            return exact.bedrock_min_engine_version;
        }
        // Fallback to closest lower-or-equal entry, else 1.20.0.
        self.pack_format_versions
            .iter()
            .filter(|e| e.java_pack_format <= pack_format)
            .max_by_key(|e| e.java_pack_format)
            .map(|e| e.bedrock_min_engine_version)
            .unwrap_or([1, 20, 0])
    }

    /// Maps a Bedrock `min_engine_version` triplet to the closest Java `pack_format`.
    #[must_use]
    pub fn bedrock_engine_to_java_pack_format(&self, ver: [u32; 3]) -> u32 {
        if let Some(exact) = self
            .pack_format_versions
            .iter()
            .find(|e| e.bedrock_min_engine_version == ver)
        {
            return exact.java_pack_format;
        }
        self.pack_format_versions
            .iter()
            .filter(|e| e.bedrock_min_engine_version <= ver)
            .max_by_key(|e| e.bedrock_min_engine_version)
            .map(|e| e.java_pack_format)
            .unwrap_or(15)
    }

    /// Resolves a CLI `--target-version` string (e.g. `"1.20.1"`, `"15"`, `"1.21.0"`)
    /// into Java `pack_format` and/or Bedrock `min_engine_version`.
    pub fn resolve_target_version(
        &self,
        target: Edition,
        version_str: &str,
    ) -> Result<(Option<u32>, Option<[u32; 3]>), CoreError> {
        let trimmed = version_str.trim();
        if let Some(entry) = self
            .pack_format_versions
            .iter()
            .find(|e| e.minecraft_version == trimmed)
        {
            return Ok((
                Some(entry.java_pack_format),
                Some(entry.bedrock_min_engine_version),
            ));
        }

        // Also match `"84.0"` / `"88.0"` style Java minor pack formats if passed directly.
        if let Some((maj_str, min_str)) = trimmed.split_once('.')
            && let (Ok(maj), Ok(min)) = (maj_str.parse::<u32>(), min_str.parse::<u32>())
            && maj >= 69
            && min == 0
            && let Some(entry) = self
                .pack_format_versions
                .iter()
                .find(|e| e.java_pack_format == maj)
        {
            return Ok((
                Some(entry.java_pack_format),
                Some(entry.bedrock_min_engine_version),
            ));
        }

        if let Ok(pack_fmt) = trimmed.parse::<u32>() {
            let bedrock_ver = self.java_pack_format_to_bedrock(pack_fmt);
            return Ok((Some(pack_fmt), Some(bedrock_ver)));
        }

        let parts: Vec<&str> = trimmed.split('.').collect();
        if parts.len() == 2 || parts.len() == 3 {
            let major = parts[0].parse::<u32>().map_err(|_| {
                CoreError::InvalidMetadata(format!("invalid version string '{trimmed}'"))
            })?;
            let minor = parts[1].parse::<u32>().map_err(|_| {
                CoreError::InvalidMetadata(format!("invalid version string '{trimmed}'"))
            })?;
            let patch = if parts.len() == 3 {
                parts[2].parse::<u32>().map_err(|_| {
                    CoreError::InvalidMetadata(format!("invalid version string '{trimmed}'"))
                })?
            } else {
                0
            };
            let ver = [major, minor, patch];
            let pack_fmt = self.bedrock_engine_to_java_pack_format(ver);
            return match target {
                Edition::Java => Ok((Some(pack_fmt), Some(ver))),
                Edition::Bedrock => Ok((Some(pack_fmt), Some(ver))),
            };
        }

        Err(CoreError::InvalidMetadata(format!(
            "unrecognized target version '{trimmed}'"
        )))
    }
}

fn normalize_texture_id(raw: &str) -> String {
    let trimmed = raw.trim().replace('\\', "/");
    let without_ns = trimmed
        .strip_prefix("minecraft:")
        .unwrap_or(trimmed.as_str());
    let without_leading = without_ns.trim_start_matches('/');
    without_leading
        .strip_suffix(".png")
        .unwrap_or(without_leading)
        .to_string()
}

fn default_atlas_key_from_bedrock(bedrock_path: &str) -> String {
    let stripped = bedrock_path
        .strip_prefix("textures/blocks/")
        .or_else(|| bedrock_path.strip_prefix("textures/items/"))
        .or_else(|| bedrock_path.strip_prefix("textures/"))
        .unwrap_or(bedrock_path);
    stripped.replace('/', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_embedded_default_mappings_and_heuristics() {
        let table = MappingTable::default_embedded().expect("default mappings should parse");
        let oak = table.java_texture_to_bedrock("block/oak_planks");
        assert_eq!(oak.path, "textures/blocks/planks_oak");
        assert_eq!(oak.atlas_key, "planks_oak");
        assert_eq!(
            table.bedrock_texture_to_java("textures/blocks/planks_oak.png"),
            "block/oak_planks"
        );

        let custom = table.java_texture_to_bedrock("block/custom/neon_tile");
        assert_eq!(custom.path, "textures/blocks/custom/neon_tile");
        assert_eq!(custom.atlas_key, "custom_neon_tile");
        assert_eq!(
            table.bedrock_texture_to_java(&custom.path),
            "block/custom/neon_tile"
        );
    }

    #[test]
    fn maps_locale_keys_and_versions() {
        let table = MappingTable::default_embedded().expect("default mappings should parse");
        assert_eq!(
            table.java_locale_key_to_bedrock("block.minecraft.stone"),
            "tile.stone.stone.name"
        );
        assert_eq!(
            table.bedrock_locale_key_to_java("tile.stone.stone.name"),
            "block.minecraft.stone"
        );
        assert_eq!(
            table.java_locale_key_to_bedrock("item.minecraft.custom_wand"),
            "item.custom_wand.name"
        );
        assert_eq!(
            table.bedrock_locale_key_to_java("item.custom_wand.name"),
            "item.minecraft.custom_wand"
        );

        assert_eq!(table.java_pack_format_to_bedrock(15), [1, 20, 0]);
        assert_eq!(table.bedrock_engine_to_java_pack_format([1, 20, 0]), 15);
        assert_eq!(
            table
                .resolve_target_version(Edition::Java, "26.1")
                .expect("26.1"),
            (Some(84), Some([26, 1, 0]))
        );
        assert_eq!(
            table
                .resolve_target_version(Edition::Java, "26.2")
                .expect("26.2"),
            (Some(88), Some([26, 2, 0]))
        );
        assert_eq!(
            table
                .resolve_target_version(Edition::Java, "88.0")
                .expect("88.0"),
            (Some(88), Some([26, 2, 0]))
        );
    }
}
