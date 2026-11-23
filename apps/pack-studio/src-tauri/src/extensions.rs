use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rp_io::sanitize_archive_path;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use zip::ZipArchive;

use crate::state::StudioState;

pub const BUILTIN_MINECRAFT_EXT_ID: &str = "packstudio.minecraft-pack-intelligence";

/// Declarative stdio LSP contribution from an extension or user configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LspContributionDto {
    pub server_id: String,
    pub display_name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub language_ids: Vec<String>,
    #[serde(default)]
    pub builtin: bool,
}

/// JSON Schema validation entry loaded from an extension for Monaco `jsonDefaults`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionJsonSchemaDto {
    pub uri: String,
    pub file_match: Vec<String>,
    pub schema: serde_json::Value,
    pub source_extension_id: String,
}

/// Snippet entry loaded from a VS Code `.vsix` or PackStudio extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionSnippetDto {
    pub prefix: String,
    pub body: Vec<String>,
    pub description: String,
    pub language: String,
    pub source_extension_id: String,
}

/// Metadata DTO for any built-in or user-installed extension returned to the Extensions UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledExtensionDto {
    pub id: String,
    pub display_name: String,
    pub publisher: String,
    pub version: String,
    pub description: String,
    pub enabled: bool,
    pub builtin: bool,
    pub install_path: Option<String>,
    pub has_js_entrypoint: bool,
    pub runtime_note: Option<String>,
    pub schema_count: usize,
    pub snippet_count: usize,
    pub grammar_languages: Vec<String>,
    pub lsp_contributions: Vec<LspContributionDto>,
    pub raw_manifest_json: String,
}

/// Aggregate snapshot returned by `ext_list`.
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionsSnapshotDto {
    pub extensions_dir: String,
    pub extensions: Vec<InstalledExtensionDto>,
    pub active_schemas: Vec<ExtensionJsonSchemaDto>,
    pub active_snippets: Vec<ExtensionSnippetDto>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedExtensionState {
    #[serde(default)]
    pub disabled_ids: BTreeSet<String>,
    #[serde(default)]
    pub custom_lsp_servers: Vec<LspContributionDto>,
}

/// Returns `<data_local_dir>/pack-studio/extensions` (or `.cache/pack-studio/extensions`).
#[must_use]
pub fn default_extensions_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("pack-studio")
        .join("extensions")
}

fn state_file_path(root: &Path) -> PathBuf {
    root.join("extensions-state.json")
}

/// Loads persisted extension enablement & custom LSP server configs.
#[must_use]
pub fn load_persisted_state(root: &Path) -> PersistedExtensionState {
    let path = state_file_path(root);
    fs::read_to_string(path)
        .ok()
        .and_then(|txt| serde_json::from_str(&txt).ok())
        .unwrap_or_default()
}

/// Saves persisted extension state atomically to disk.
pub fn save_persisted_state(root: &Path, st: &PersistedExtensionState) -> Result<()> {
    fs::create_dir_all(root)?;
    let bytes = serde_json::to_vec_pretty(st)?;
    fs::write(state_file_path(root), bytes)?;
    Ok(())
}

/// Built-in first-party extension descriptor.
#[must_use]
pub fn builtin_extension_dto(enabled: bool) -> InstalledExtensionDto {
    let manifest_json = serde_json::json!({
        "name": "minecraft-pack-intelligence",
        "publisher": "packstudio",
        "displayName": "Minecraft Resource Pack Intelligence (Built-in LSP)",
        "version": "0.2.3",
        "description": "Built-in diagnostics, completions, and hover documentation for pack.mcmeta, Bedrock manifest.json, terrain_texture.json, item_texture.json, .geo.json, .lang, and Molang.",
        "contributes": {
            "languages": ["json", "mc-lang", "molang"],
            "packstudioLsp": [{
                "server_id": "builtin.minecraft-rp",
                "display_name": "Minecraft Resource Pack LSP (Built-in)",
                "command": "builtin",
                "language_ids": ["json", "mc-lang", "molang"]
            }]
        }
    });

    InstalledExtensionDto {
        id: String::from(BUILTIN_MINECRAFT_EXT_ID),
        display_name: String::from("Minecraft Pack Intelligence (LSP)"),
        publisher: String::from("packstudio"),
        version: String::from("0.2.3"),
        description: String::from(
            "Built-in diagnostics, live asset-path completions, and hover docs for pack.mcmeta, Bedrock manifest.json, texture atlases, .geo.json, .lang, and Molang.",
        ),
        enabled,
        builtin: true,
        install_path: None,
        has_js_entrypoint: false,
        runtime_note: None,
        schema_count: 3,
        snippet_count: 6,
        grammar_languages: vec![
            String::from("json"),
            String::from("mc-lang"),
            String::from("molang"),
        ],
        lsp_contributions: vec![LspContributionDto {
            server_id: String::from("builtin.minecraft-rp"),
            display_name: String::from("Minecraft Resource Pack LSP (Built-in)"),
            command: String::from("builtin"),
            args: Vec::new(),
            language_ids: vec![
                String::from("json"),
                String::from("mc-lang"),
                String::from("molang"),
            ],
            builtin: true,
        }],
        raw_manifest_json: serde_json::to_string_pretty(&manifest_json).unwrap_or_default(),
    }
}

fn builtin_schemas() -> Vec<ExtensionJsonSchemaDto> {
    vec![
        ExtensionJsonSchemaDto {
            uri: String::from("packstudio://schemas/java-pack-mcmeta.json"),
            file_match: vec![String::from("pack.mcmeta")],
            source_extension_id: String::from(BUILTIN_MINECRAFT_EXT_ID),
            schema: serde_json::json!({
                "type": "object",
                "required": ["pack"],
                "properties": {
                    "pack": {
                        "type": "object",
                        "required": ["pack_format", "description"],
                        "properties": {
                            "pack_format": { "type": "integer", "minimum": 1, "description": "Java Edition resource pack format version (e.g. 34 for 1.21, 46 for 1.21.4)" },
                            "description": { "description": "Pack description string or text component" },
                            "supported_formats": { "description": "Range [min, max] or object {min_inclusive, max_inclusive}" }
                        }
                    }
                }
            }),
        },
        ExtensionJsonSchemaDto {
            uri: String::from("packstudio://schemas/bedrock-manifest.json"),
            file_match: vec![String::from("manifest.json")],
            source_extension_id: String::from(BUILTIN_MINECRAFT_EXT_ID),
            schema: serde_json::json!({
                "type": "object",
                "required": ["format_version", "header", "modules"],
                "properties": {
                    "format_version": { "type": "integer", "enum": [1, 2] },
                    "header": {
                        "type": "object",
                        "required": ["name", "description", "uuid", "version", "min_engine_version"],
                        "properties": {
                            "name": { "type": "string" },
                            "description": { "type": "string" },
                            "uuid": { "type": "string" },
                            "version": { "type": "array", "items": { "type": "integer" }, "minItems": 3, "maxItems": 3 },
                            "min_engine_version": { "type": "array", "items": { "type": "integer" }, "minItems": 3, "maxItems": 3 }
                        }
                    },
                    "capabilities": {
                        "type": "array",
                        "items": { "type": "string", "enum": ["raytraced", "pbr", "chemistry", "script_eval"] }
                    }
                }
            }),
        },
    ]
}

fn builtin_snippets() -> Vec<ExtensionSnippetDto> {
    vec![
        ExtensionSnippetDto {
            prefix: String::from("mc-manifest-rp"),
            body: vec![
                String::from("{"),
                String::from("  \"format_version\": 2,"),
                String::from("  \"header\": {"),
                String::from("    \"name\": \"${1:My Resource Pack}\","),
                String::from("    \"description\": \"${2:Custom Bedrock RP}\","),
                String::from("    \"uuid\": \"${3:00000000-0000-4000-8000-000000000001}\","),
                String::from("    \"version\": [1, 0, 0],"),
                String::from("    \"min_engine_version\": [1, 21, 50]"),
                String::from("  },"),
                String::from("  \"modules\": [{"),
                String::from("    \"type\": \"resources\","),
                String::from("    \"uuid\": \"${4:00000000-0000-4000-8000-000000000002}\","),
                String::from("    \"version\": [1, 0, 0]"),
                String::from("  }]"),
                String::from("}"),
            ],
            description: String::from("Bedrock Resource Pack manifest.json template ([1, x, x])"),
            language: String::from("json"),
            source_extension_id: String::from(BUILTIN_MINECRAFT_EXT_ID),
        },
        ExtensionSnippetDto {
            prefix: String::from("mc-pack-mcmeta"),
            body: vec![
                String::from("{"),
                String::from("  \"pack\": {"),
                String::from("    \"pack_format\": ${1:46},"),
                String::from("    \"supported_formats\": [${2:34}, ${1:46}],"),
                String::from("    \"description\": \"${3:§bCustom Java Pack}\""),
                String::from("  }"),
                String::from("}"),
            ],
            description: String::from("Java Edition pack.mcmeta template"),
            language: String::from("json"),
            source_extension_id: String::from(BUILTIN_MINECRAFT_EXT_ID),
        },
        ExtensionSnippetDto {
            prefix: String::from("mc-texture-set"),
            body: vec![
                String::from("{"),
                String::from("  \"format_version\": \"1.16.100\","),
                String::from("  \"minecraft:texture_set\": {"),
                String::from("    \"color\": \"${1:block_color}\","),
                String::from("    \"metalness_emissive_roughness\": \"${2:block_mer}\","),
                String::from("    \"normal\": \"${3:block_normal}\""),
                String::from("  }"),
                String::from("}"),
            ],
            description: String::from("Bedrock Vibrant Visuals / PBR .texture_set.json"),
            language: String::from("json"),
            source_extension_id: String::from(BUILTIN_MINECRAFT_EXT_ID),
        },
    ]
}

fn find_manifest_root(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let direct = dir.join("package.json");
    if direct.is_file() {
        return Some((dir.to_path_buf(), direct));
    }
    let vsix_sub = dir.join("extension").join("package.json");
    if vsix_sub.is_file() {
        return Some((dir.join("extension"), vsix_sub));
    }
    None
}

fn sanitize_ext_id(raw: &str) -> String {
    let clean: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if clean.is_empty() {
        String::from("custom.extension")
    } else {
        clean
    }
}

fn parse_snippets_file(
    path: &Path,
    language: &str,
    ext_id: &str,
) -> Vec<ExtensionSnippetDto> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(val) = rp_core::parse_minecraft_json(&raw) else {
        return Vec::new();
    };
    let Some(obj) = val.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (_name, entry) in obj {
        let Some(prefix) = entry.get("prefix").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let body = match entry.get("body") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect(),
            Some(serde_json::Value::String(s)) => s.lines().map(str::to_string).collect(),
            _ => Vec::new(),
        };
        if body.is_empty() {
            continue;
        }
        let description = entry
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(prefix)
            .to_string();
        out.push(ExtensionSnippetDto {
            prefix: prefix.to_string(),
            body,
            description,
            language: language.to_string(),
            source_extension_id: ext_id.to_string(),
        });
    }
    out
}

fn load_installed_extension_from_dir(
    install_dir: &Path,
    disabled_ids: &BTreeSet<String>,
) -> Option<(
    InstalledExtensionDto,
    Vec<ExtensionJsonSchemaDto>,
    Vec<ExtensionSnippetDto>,
)> {
    let (ext_root, manifest_path) = find_manifest_root(install_dir)?;
    let raw_manifest = fs::read_to_string(&manifest_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&raw_manifest).ok()?;

    let name = val
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("extension");
    let publisher = val
        .get("publisher")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("local");
    let id = sanitize_ext_id(&format!("{publisher}.{name}"));
    let display_name = val
        .get("displayName")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(name)
        .to_string();
    let version = val
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("0.1.0")
        .to_string();
    let description = val
        .get("description")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("PackStudio / VS Code declarative extension")
        .to_string();

    let has_js_entrypoint = val.get("main").is_some() || val.get("browser").is_some();
    let runtime_note = if has_js_entrypoint {
        Some(String::from(
            "Declarative features (JSON schemas, snippets, LSP declarations) are active. Imperative JS vscode.* API entrypoint is skipped (no Electron/Node VS Code Extension Host).",
        ))
    } else {
        None
    };

    let mut schemas = Vec::new();
    let mut snippets = Vec::new();
    let mut grammar_languages = BTreeSet::new();
    let mut lsp_contributions = Vec::new();

    if let Some(contributes) = val.get("contributes") {
        if let Some(arr) = contributes
            .get("jsonValidation")
            .and_then(serde_json::Value::as_array)
        {
            for (idx, item) in arr.iter().enumerate() {
                let file_match: Vec<String> = match item.get("fileMatch") {
                    Some(serde_json::Value::Array(fm)) => fm
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_string)
                        .collect(),
                    Some(serde_json::Value::String(s)) => vec![s.clone()],
                    _ => Vec::new(),
                };
                if file_match.is_empty() {
                    continue;
                }
                let url = item
                    .get("url")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let rel_clean = url.trim_start_matches("./").trim_start_matches('/');
                if !rel_clean.is_empty()
                    && !rel_clean.contains("..")
                    && let Ok(schema_txt) = fs::read_to_string(ext_root.join(rel_clean))
                    && let Ok(schema_val) = rp_core::parse_minecraft_json(&schema_txt)
                {
                    schemas.push(ExtensionJsonSchemaDto {
                        uri: format!("packstudio://ext/{id}/schema-{idx}.json"),
                        file_match,
                        schema: schema_val,
                        source_extension_id: id.clone(),
                    });
                }
            }
        }

        if let Some(arr) = contributes
            .get("snippets")
            .and_then(serde_json::Value::as_array)
        {
            for item in arr {
                let lang = item
                    .get("language")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("json");
                let rel_path = item
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim_start_matches("./")
                    .trim_start_matches('/');
                if !rel_path.is_empty() && !rel_path.contains("..") {
                    snippets.extend(parse_snippets_file(&ext_root.join(rel_path), lang, &id));
                }
            }
        }

        if let Some(arr) = contributes
            .get("grammars")
            .and_then(serde_json::Value::as_array)
        {
            for g in arr {
                if let Some(l) = g.get("language").and_then(serde_json::Value::as_str) {
                    grammar_languages.insert(l.to_string());
                }
            }
        }

        for key in ["packstudioLsp", "languageServers"] {
            if let Some(arr) = contributes.get(key).and_then(serde_json::Value::as_array) {
                for item in arr {
                    let Some(cmd) = item.get("command").and_then(serde_json::Value::as_str) else {
                        continue;
                    };
                    let sid = item
                        .get("server_id")
                        .or_else(|| item.get("id"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("{id}.lsp"));
                    let dname = item
                        .get("display_name")
                        .or_else(|| item.get("name"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(&display_name)
                        .to_string();
                    let args = item
                        .get("args")
                        .and_then(serde_json::Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(serde_json::Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default();
                    let language_ids = item
                        .get("language_ids")
                        .or_else(|| item.get("languages"))
                        .and_then(serde_json::Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(serde_json::Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_else(|| vec![String::from("json")]);
                    lsp_contributions.push(LspContributionDto {
                        server_id: sid,
                        display_name: dname,
                        command: cmd.to_string(),
                        args,
                        language_ids,
                        builtin: false,
                    });
                }
            }
        }
    }

    let enabled = !disabled_ids.contains(&id);
    let dto = InstalledExtensionDto {
        id,
        display_name,
        publisher: publisher.to_string(),
        version,
        description,
        enabled,
        builtin: false,
        install_path: Some(install_dir.display().to_string()),
        has_js_entrypoint,
        runtime_note,
        schema_count: schemas.len(),
        snippet_count: snippets.len(),
        grammar_languages: grammar_languages.into_iter().collect(),
        lsp_contributions,
        raw_manifest_json: serde_json::to_string_pretty(&val).unwrap_or(raw_manifest),
    };

    Some((dto, schemas, snippets))
}

/// Scans `extensions_root` cleanly without Tauri runtime (directly unit-testable).
pub fn scan_extensions_snapshot(extensions_root: &Path) -> ExtensionsSnapshotDto {
    let persisted = load_persisted_state(extensions_root);
    let builtin_enabled = !persisted.disabled_ids.contains(BUILTIN_MINECRAFT_EXT_ID);
    let mut extensions = vec![builtin_extension_dto(builtin_enabled)];
    let mut active_schemas = if builtin_enabled {
        builtin_schemas()
    } else {
        Vec::new()
    };
    let mut active_snippets = if builtin_enabled {
        builtin_snippets()
    } else {
        Vec::new()
    };

    if extensions_root.is_dir()
        && let Ok(entries) = fs::read_dir(extensions_root)
    {
        let mut by_id: BTreeMap<String, InstalledExtensionDto> = BTreeMap::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some((dto, schemas, snippets)) =
                load_installed_extension_from_dir(&path, &persisted.disabled_ids)
            {
                if dto.enabled {
                    active_schemas.extend(schemas);
                    active_snippets.extend(snippets);
                }
                by_id.insert(dto.id.clone(), dto);
            }
        }
        extensions.extend(by_id.into_values());
    }

    ExtensionsSnapshotDto {
        extensions_dir: extensions_root.display().to_string(),
        extensions,
        active_schemas,
        active_snippets,
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let src_p = entry.path();
        let dst_p = dst.join(name);
        if src_p.is_dir() {
            copy_dir_recursive(&src_p, &dst_p)?;
        } else if src_p.is_file() {
            fs::copy(&src_p, &dst_p)?;
        }
    }
    Ok(())
}

/// Safely extracts a `.vsix` / `.zip` archive or copies a directory extension into `extensions_root`.
pub fn install_extension_core(source: &Path, extensions_root: &Path) -> Result<InstalledExtensionDto> {
    if !source.exists() {
        bail!("extension source '{}' does not exist", source.display());
    }
    fs::create_dir_all(extensions_root)?;

    let temp_dir = extensions_root.join(format!(".staging-{}", uuid::Uuid::new_v4()));
    if temp_dir.exists() {
        let _ = fs::remove_dir_all(&temp_dir);
    }
    fs::create_dir_all(&temp_dir)?;

    let extract_res = (|| -> Result<()> {
        if source.is_dir() {
            copy_dir_recursive(source, &temp_dir)?;
        } else {
            let f = File::open(source)
                .with_context(|| format!("failed to open archive '{}'", source.display()))?;
            let mut archive = ZipArchive::new(f)
                .with_context(|| format!("'{}' is not a valid .vsix/.zip archive", source.display()))?;
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i)?;
                let raw_name = entry.name().to_string();
                let clean = sanitize_archive_path(&raw_name)?;
                let out_path = temp_dir.join(&clean);
                if entry.is_dir() {
                    fs::create_dir_all(&out_path)?;
                } else {
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let mut out_file = File::create(&out_path)?;
                    std::io::copy(&mut entry, &mut out_file)?;
                }
            }
        }
        Ok(())
    })();

    if let Err(err) = extract_res {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(err);
    }

    let Some((dto, _, _)) = load_installed_extension_from_dir(&temp_dir, &BTreeSet::new()) else {
        let _ = fs::remove_dir_all(&temp_dir);
        bail!(
            "No valid package.json (or extension/package.json) found in '{}'",
            source.display()
        );
    };

    if dto.id == BUILTIN_MINECRAFT_EXT_ID {
        let _ = fs::remove_dir_all(&temp_dir);
        bail!("Cannot overwrite reserved built-in extension id");
    }

    let final_dir = extensions_root.join(&dto.id);
    if final_dir.exists() {
        fs::remove_dir_all(&final_dir)?;
    }
    fs::rename(&temp_dir, &final_dir)?;

    let mut persisted = load_persisted_state(extensions_root);
    persisted.disabled_ids.remove(&dto.id);
    save_persisted_state(extensions_root, &persisted)?;

    let (final_dto, _, _) = load_installed_extension_from_dir(&final_dir, &persisted.disabled_ids)
        .context("failed reloading installed extension")?;
    Ok(final_dto)
}

/// Tauri Command: List all built-in + installed extensions, schemas, and snippets.
#[tauri::command]
pub fn ext_list() -> ExtensionsSnapshotDto {
    let root = default_extensions_dir();
    scan_extensions_snapshot(&root)
}

/// Tauri Command: Install an extension from `.vsix`, `.zip`, or an unpacked folder.
#[tauri::command]
pub fn ext_install(source_path: String) -> Result<ExtensionsSnapshotDto, String> {
    let root = default_extensions_dir();
    install_extension_core(Path::new(source_path.trim()), &root).map_err(|e| format!("{e:#}"))?;
    Ok(scan_extensions_snapshot(&root))
}

/// Tauri Command: Enable or disable an extension (including the Built-in Minecraft LSP extension).
#[tauri::command]
pub fn ext_set_enabled(
    _app: AppHandle,
    state: tauri::State<'_, StudioState>,
    ext_id: String,
    enabled: bool,
) -> Result<ExtensionsSnapshotDto, String> {
    let root = default_extensions_dir();
    let mut persisted = load_persisted_state(&root);
    if enabled {
        persisted.disabled_ids.remove(&ext_id);
    } else {
        persisted.disabled_ids.insert(ext_id.clone());
    }
    save_persisted_state(&root, &persisted).map_err(|e| format!("{e:#}"))?;

    if ext_id == BUILTIN_MINECRAFT_EXT_ID {
        state
            .builtin_lsp_enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    Ok(scan_extensions_snapshot(&root))
}

/// Tauri Command: Uninstall a user-installed extension directory.
#[tauri::command]
pub fn ext_uninstall(ext_id: String) -> Result<ExtensionsSnapshotDto, String> {
    if ext_id == BUILTIN_MINECRAFT_EXT_ID {
        return Err(String::from(
            "Built-in extension cannot be uninstalled; toggle Disable instead.",
        ));
    }
    let root = default_extensions_dir();
    let clean = sanitize_ext_id(&ext_id);
    let target = root.join(&clean);
    if target.is_dir() {
        fs::remove_dir_all(&target).map_err(|e| format!("failed removing '{}': {e}", target.display()))?;
    }
    let mut persisted = load_persisted_state(&root);
    persisted.disabled_ids.remove(&clean);
    save_persisted_state(&root, &persisted).map_err(|e| format!("{e:#}"))?;
    Ok(scan_extensions_snapshot(&root))
}
