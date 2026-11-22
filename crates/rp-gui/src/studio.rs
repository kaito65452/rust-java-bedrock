use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use rp_core::{ModelAsset, ResourcePackIR, decode_minecraft_text, sanitize_minecraft_json};

/// Left VS Code-style Activity Bar sections in Studio Mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivitySection {
    /// Asset tree & search/filter (`All` / `Needs Review` / `Manual Overrides`).
    Explorer,
    /// Grouped warnings, lossy items, and unmapped assets needing review.
    Warnings,
    /// Batch conversion presets, round-trip audit, and pack compare.
    BatchPresets,
}

impl ActivitySection {
    pub const ALL: [Self; 3] = [Self::Explorer, Self::Warnings, Self::BatchPresets];

    #[must_use]
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Explorer => "📁",
            Self::Warnings => "⚠",
            Self::BatchPresets => "⚡",
        }
    }

    #[must_use]
    pub const fn tooltip(self) -> &'static str {
        match self {
            Self::Explorer => "Asset Explorer",
            Self::Warnings => "Warnings & Needs Review",
            Self::BatchPresets => "Batch, Audit & Presets",
        }
    }
}

/// Side Bar filter pill inside Studio Mode's Explorer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideBarFilter {
    All,
    NeedsReview,
    ManualOverrides,
}

impl SideBarFilter {
    pub const ALL: [Self; 3] = [Self::All, Self::NeedsReview, Self::ManualOverrides];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::NeedsReview => "Needs Review",
            Self::ManualOverrides => "Overrides",
        }
    }
}

/// Kind of text/JSON asset opened in the Studio Text Editor tab.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TextAssetTarget {
    /// Virtual metadata editor for pack metadata (`pack.mcmeta` / `manifest.json`).
    MetadataJson,
    /// Locale table keyed by language code (e.g. `"en_us"`).
    LocaleJson(String),
    /// Attachable definition keyed by attachable ID.
    AttachableJson(String),
    /// Raw passthrough file keyed by pack-relative path.
    Passthrough(String),
}

impl TextAssetTarget {
    /// Stable canonical key used in `ManualOverrideStore`.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::MetadataJson => String::from("meta://pack_metadata.json"),
            Self::LocaleJson(code) => format!("locale://{code}.json"),
            Self::AttachableJson(id) => format!("attachable://{id}.json"),
            Self::Passthrough(path) => format!("raw://{path}"),
        }
    }

    /// Short tab title.
    #[must_use]
    pub fn title(&self) -> String {
        match self {
            Self::MetadataJson => String::from("metadata.json"),
            Self::LocaleJson(code) => format!("{code}.json"),
            Self::AttachableJson(id) => format!("{id}.attachable.json"),
            Self::Passthrough(path) => path.rsplit('/').next().unwrap_or(path.as_str()).to_string(),
        }
    }

    /// Whether JSON validation should run on this buffer.
    #[must_use]
    pub fn expects_json(&self) -> bool {
        match self {
            Self::MetadataJson | Self::LocaleJson(_) | Self::AttachableJson(_) => true,
            Self::Passthrough(p) => p.ends_with(".json") || p.ends_with(".mcmeta"),
        }
    }
}

/// An open editor tab inside the central VS Code-style Studio workbench.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StudioEditorTab {
    /// Blockbench-style 3D model / geometry viewport + property inspector.
    Model(String),
    /// Texture & animation strip inspector with pixel grid + atlas status.
    Texture(String),
    /// Direct text / JSON / lang / manifest editor with live validation.
    Text(TextAssetTarget),
}

impl StudioEditorTab {
    /// Tab title shown in the editor tab strip.
    #[must_use]
    pub fn title(&self) -> String {
        match self {
            Self::Model(id) => format!("🧊 {}", short_id(id)),
            Self::Texture(id) => format!("🖼 {}", short_id(id)),
            Self::Text(t) => format!("📝 {}", t.title()),
        }
    }

    /// Canonical asset key used to check manual override status.
    #[must_use]
    pub fn override_key(&self) -> String {
        match self {
            Self::Model(id) => format!("model://{id}"),
            Self::Texture(id) => format!("texture://{id}"),
            Self::Text(t) => t.key(),
        }
    }
}

fn short_id(id: &str) -> &str {
    id.rsplit('/').next().unwrap_or(id)
}

/// Serializable record of a manual override made inside Studio Mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverrideSummaryEntry {
    pub key: String,
    pub kind: String,
    pub note: String,
}

/// Exportable Studio overrides + custom `MappingTable`-compatible texture rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudioOverridesExport {
    pub schema_version: String,
    pub textures: Vec<CustomTextureRule>,
    pub manual_overrides: Vec<OverrideSummaryEntry>,
}

/// Single Java <-> Bedrock texture mapping rule compatible with `rp-core` `MappingTable`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTextureRule {
    pub java: String,
    pub bedrock: String,
    pub atlas_key: Option<String>,
}

/// Tracks all manual Studio edits separately from auto-converted IR so users can
/// inspect what was overridden, reset any asset to auto-converted values, or export rules.
#[derive(Debug, Clone, Default)]
pub struct ManualOverrideStore {
    /// Original auto-converted models snapshotted before the first manual edit.
    pub model_originals: HashMap<String, ModelAsset>,
    /// Active manually edited models.
    pub model_overrides: BTreeMap<String, ModelAsset>,
    /// Original auto-generated text buffers for Text/JSON tabs.
    pub text_originals: HashMap<String, String>,
    /// Live working text buffers (including unsaved keystrokes).
    pub text_buffers: HashMap<String, String>,
    /// Committed manual text overrides.
    pub text_overrides: BTreeMap<String, String>,
    /// Manual Java texture ID -> Bedrock path overrides (exportable as `--mapping` JSON).
    pub custom_texture_mappings: BTreeMap<String, String>,
}

impl ManualOverrideStore {
    /// Clears all overrides when switching to a different pack.
    pub fn clear(&mut self) {
        self.model_originals.clear();
        self.model_overrides.clear();
        self.text_originals.clear();
        self.text_buffers.clear();
        self.text_overrides.clear();
        self.custom_texture_mappings.clear();
    }

    /// Total count of active manual overrides across models, text/JSON files, and texture rules.
    #[must_use]
    pub fn total_override_count(&self) -> usize {
        self.model_overrides.len() + self.text_overrides.len() + self.custom_texture_mappings.len()
    }

    /// Returns `true` if `model_id` has been manually modified.
    #[must_use]
    pub fn is_model_overridden(&self, model_id: &str) -> bool {
        self.model_overrides.contains_key(model_id)
    }

    /// Returns `true` if `tex_id` has a custom texture mapping rule.
    #[must_use]
    pub fn is_texture_overridden(&self, tex_id: &str) -> bool {
        self.custom_texture_mappings.contains_key(tex_id)
    }

    /// Returns `true` if a text target key has been manually committed.
    #[must_use]
    pub fn is_text_overridden(&self, key: &str) -> bool {
        self.text_overrides.contains_key(key)
    }

    /// Snapshots `original` (if not already stored) and records `edited` as a manual override.
    pub fn record_model_edit(&mut self, model_id: &str, original: &ModelAsset, edited: ModelAsset) {
        self.model_originals
            .entry(model_id.to_string())
            .or_insert_with(|| original.clone());
        self.model_overrides.insert(model_id.to_string(), edited);
    }

    /// Reverts `model_id` back to its auto-converted snapshot, returning the original if present.
    pub fn reset_model_to_auto(&mut self, model_id: &str) -> Option<ModelAsset> {
        self.model_overrides.remove(model_id);
        self.model_originals.remove(model_id)
    }

    /// Ensures a text buffer exists for `target`, seeding it from `ir` on first open.
    pub fn ensure_text_buffer(
        &mut self,
        target: &TextAssetTarget,
        ir: &ResourcePackIR,
    ) -> &mut String {
        let key = target.key();
        if !self.text_buffers.contains_key(&key) {
            let initial = serialize_text_target(target, ir);
            self.text_originals
                .entry(key.clone())
                .or_insert_with(|| initial.clone());
            self.text_buffers.insert(key.clone(), initial);
        }
        self.text_buffers.entry(key).or_default()
    }

    /// Commits the current working text buffer into `ir` (validating JSON when required)
    /// and marks it as a manual override.
    pub fn commit_text_edit(
        &mut self,
        target: &TextAssetTarget,
        ir: &mut ResourcePackIR,
    ) -> Result<(), String> {
        let key = target.key();
        let content = self.text_buffers.get(&key).cloned().unwrap_or_default();

        if target.expects_json() {
            validate_json_str(&content)?;
        }

        apply_text_target_to_ir(target, &content, ir)?;
        self.text_overrides.insert(key, content);
        Ok(())
    }

    /// Resets a text/JSON asset back to its original auto-loaded content.
    pub fn reset_text_to_auto(
        &mut self,
        target: &TextAssetTarget,
        ir: &mut ResourcePackIR,
    ) -> Result<(), String> {
        let key = target.key();
        self.text_overrides.remove(&key);
        if let Some(orig) = self.text_originals.get(&key).cloned() {
            self.text_buffers.insert(key, orig.clone());
            apply_text_target_to_ir(target, &orig, ir)?;
        }
        Ok(())
    }

    /// Applies all active manual model/text overrides onto `ir`, returning count applied.
    pub fn apply_to_ir(&self, ir: &mut ResourcePackIR) -> usize {
        let mut applied = 0usize;
        for (id, model) in &self.model_overrides {
            if ir.models.contains_key(id) {
                ir.models.insert(id.clone(), model.clone());
                applied += 1;
            }
        }
        for (key, text) in &self.text_overrides {
            if let Some(target) = parse_text_target_key(key)
                && apply_text_target_to_ir(&target, text, ir).is_ok()
            {
                applied += 1;
            }
        }
        applied
    }

    /// Exports custom texture mapping rules + override summary as a `MappingTable`-compatible JSON file.
    pub fn export_json(&self, path: &Path) -> Result<()> {
        let textures = self
            .custom_texture_mappings
            .iter()
            .map(|(java, bedrock)| {
                let atlas_key = bedrock.rsplit('/').next().map(str::to_string);
                CustomTextureRule {
                    java: java.clone(),
                    bedrock: bedrock.clone(),
                    atlas_key,
                }
            })
            .collect();

        let mut manual_overrides = Vec::new();
        for (id, model) in &self.model_overrides {
            manual_overrides.push(OverrideSummaryEntry {
                key: format!("model://{id}"),
                kind: String::from("model"),
                note: format!("{} elements edited manually", model.elements.len()),
            });
        }
        for key in self.text_overrides.keys() {
            manual_overrides.push(OverrideSummaryEntry {
                key: key.clone(),
                kind: String::from("text_json"),
                note: String::from("Manual text/JSON override"),
            });
        }
        for (java, bedrock) in &self.custom_texture_mappings {
            manual_overrides.push(OverrideSummaryEntry {
                key: format!("texture://{java}"),
                kind: String::from("texture_mapping"),
                note: format!("{java} -> {bedrock}"),
            });
        }

        let payload = StudioOverridesExport {
            schema_version: String::from("1.0"),
            textures,
            manual_overrides,
        };
        let bytes = serde_json::to_vec_pretty(&payload)?;
        fs::write(path, bytes)
            .with_context(|| format!("failed to write overrides JSON '{}'", path.display()))?;
        Ok(())
    }
}

/// Validates JSON text (after Minecraft-lenient BOM/control-char/trailing-comma sanitization)
/// and returns a concise line/column error message on failure.
pub fn validate_json_str(raw: &str) -> Result<(), String> {
    let clean = sanitize_minecraft_json(raw);
    serde_json::from_str::<serde_json::Value>(&clean)
        .map(|_| ())
        .map_err(|err| {
            format!(
                "JSON error at line {}, col {}: {err}",
                err.line(),
                err.column()
            )
        })
}

fn serialize_text_target(target: &TextAssetTarget, ir: &ResourcePackIR) -> String {
    match target {
        TextAssetTarget::MetadataJson => {
            let val = serde_json::json!({
                "name": ir.metadata.name,
                "description": ir.metadata.description,
                "source_edition": ir.metadata.source_edition,
                "java_pack_format": ir.metadata.java_pack_format,
                "java_supported_formats": ir.metadata.java_supported_formats,
                "bedrock_min_engine_version": ir.metadata.bedrock_min_engine_version,
                "version": ir.metadata.version,
            });
            serde_json::to_string_pretty(&val).unwrap_or_default()
        }
        TextAssetTarget::LocaleJson(code) => {
            if let Some(table) = ir.locales.get(code) {
                let sorted: BTreeMap<&String, &String> = table.entries.iter().collect();
                serde_json::to_string_pretty(&sorted).unwrap_or_else(|_| String::from("{}"))
            } else {
                String::from("{}")
            }
        }
        TextAssetTarget::AttachableJson(id) => {
            if let Some(att) = ir.attachables.get(id) {
                serde_json::to_string_pretty(att).unwrap_or_else(|_| String::from("{}"))
            } else {
                String::from("{}")
            }
        }
        TextAssetTarget::Passthrough(path) => ir
            .passthrough_files
            .get(path)
            .map(|b| decode_minecraft_text(b).into_owned())
            .unwrap_or_default(),
    }
}

fn apply_text_target_to_ir(
    target: &TextAssetTarget,
    content: &str,
    ir: &mut ResourcePackIR,
) -> Result<(), String> {
    match target {
        TextAssetTarget::MetadataJson => {
            let clean = sanitize_minecraft_json(content);
            let val: serde_json::Value =
                serde_json::from_str(&clean).map_err(|e| format!("invalid metadata JSON: {e}"))?;
            if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
                ir.metadata.name = name.to_string();
            }
            if let Some(desc) = val.get("description").and_then(|v| v.as_str()) {
                ir.metadata.description = desc.to_string();
            }
            if let Some(fmt) = val.get("java_pack_format").and_then(|v| v.as_u64())
                && let Ok(f) = u32::try_from(fmt)
            {
                ir.metadata.java_pack_format = Some(f);
            }
            if let Some(arr) = val
                .get("java_supported_formats")
                .and_then(serde_json::Value::as_array)
                && arr.len() == 2
                && let (Some(lo), Some(hi)) = (arr[0].as_u64(), arr[1].as_u64())
                && let (Ok(lo), Ok(hi)) = (u32::try_from(lo), u32::try_from(hi))
            {
                ir.metadata.java_supported_formats = Some([lo, hi]);
            }
            Ok(())
        }
        TextAssetTarget::LocaleJson(code) => {
            let clean = sanitize_minecraft_json(content);
            let parsed: HashMap<String, String> =
                serde_json::from_str(&clean).map_err(|e| format!("invalid locale JSON: {e}"))?;
            let entry = ir.locales.entry(code.clone()).or_default();
            entry.entries = parsed;
            Ok(())
        }
        TextAssetTarget::AttachableJson(id) => {
            let clean = sanitize_minecraft_json(content);
            let parsed: rp_core::AttachableAsset = serde_json::from_str(&clean)
                .map_err(|e| format!("invalid attachable JSON: {e}"))?;
            ir.attachables.insert(id.clone(), parsed);
            Ok(())
        }
        TextAssetTarget::Passthrough(path) => {
            ir.passthrough_files
                .insert(path.clone(), content.as_bytes().to_vec());
            Ok(())
        }
    }
}

fn parse_text_target_key(key: &str) -> Option<TextAssetTarget> {
    if key == "meta://pack_metadata.json" {
        Some(TextAssetTarget::MetadataJson)
    } else if let Some(rest) = key.strip_prefix("locale://") {
        let code = rest.strip_suffix(".json").unwrap_or(rest);
        Some(TextAssetTarget::LocaleJson(code.to_string()))
    } else if let Some(rest) = key.strip_prefix("attachable://") {
        let id = rest.strip_suffix(".json").unwrap_or(rest);
        Some(TextAssetTarget::AttachableJson(id.to_string()))
    } else {
        key.strip_prefix("raw://")
            .map(|p| TextAssetTarget::Passthrough(p.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_core::ModelCategory;

    #[test]
    fn manual_override_store_tracks_resets_and_validates_json() {
        let mut store = ManualOverrideStore::default();
        let orig = ModelAsset {
            id: String::from("block/test"),
            category: ModelCategory::Block,
            parent: None,
            textures: BTreeMap::new(),
            texture_size: [16, 16],
            ambient_occlusion: true,
            elements: Vec::new(),
            display: BTreeMap::new(),
            has_overrides: false,
            overrides_count: 0,
        };
        let mut edited = orig.clone();
        edited.texture_size = [32, 32];

        store.record_model_edit("block/test", &orig, edited.clone());
        assert!(store.is_model_overridden("block/test"));
        assert_eq!(store.total_override_count(), 1);

        let reverted = store.reset_model_to_auto("block/test").expect("reverted");
        assert_eq!(reverted.texture_size, [16, 16]);
        assert!(!store.is_model_overridden("block/test"));

        assert!(validate_json_str(r#"{"ok": true}"#).is_ok());
        assert!(validate_json_str(r#"{"bad": }"#).is_err());
    }
}
