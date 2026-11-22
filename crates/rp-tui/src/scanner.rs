use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};

use rp_bedrock::parse_bedrock_pack;
use rp_cli::{
    InspectSummary, VersionSpecificOptions, convert_container_with_options, detect_edition,
    inspect_container,
};
use rp_core::{ConversionReport, Edition, MappingTable, ResourcePackIR};
use rp_io::{IoLimits, PackContainer};
use rp_java::parse_java_pack;

/// Entry discovered in the active scan directory.
#[derive(Debug, Clone)]
pub struct DiscoveredPack {
    /// Absolute or workspace-relative path to `.zip`, `.mcpack`, or pack folder.
    pub path: PathBuf,
    /// Display filename or directory name.
    pub file_name: String,
    /// Size on disk in bytes (archive file size or total directory size).
    pub size_bytes: u64,
    /// Whether this entry is a directory vs archive.
    pub is_dir: bool,
    /// Parsed pack inspection summary, or error text if corrupted/invalid.
    pub summary: Result<InspectSummary, String>,
    /// Selection flag for batch operations.
    pub selected: bool,
}

/// Result of a single-pack bidirectional round-trip & integrity audit.
#[derive(Debug, Clone)]
pub struct AuditOutcome {
    /// Pack filename.
    pub pack_name: String,
    /// Source pack path.
    pub path: PathBuf,
    /// Source edition detected.
    pub source_edition: Edition,
    /// Target edition converted to.
    pub target_edition: Edition,
    /// Whether forward + round-trip + integrity checks succeeded.
    pub passed: bool,
    /// Forward conversion report.
    pub forward_report: Option<ConversionReport>,
    /// Round-trip texture / model / locale retention summary.
    pub detail: String,
    /// Elapsed wall-clock time in milliseconds.
    pub elapsed_ms: u128,
}

/// Scans `dir` (non-recursively, checking `.zip`, `.mcpack`, and subdirectories containing
/// `pack.mcmeta` or `manifest.json`) and inspects each discovered pack.
pub fn scan_directory_for_packs(
    dir: &Path,
    mappings: &MappingTable,
    limits: IoLimits,
) -> Result<Vec<DiscoveredPack>> {
    let entries = fs::read_dir(dir)
        .with_context(|| format!("failed to read directory '{}'", dir.display()))?;

    let mut candidates: Vec<(PathBuf, bool, u64)> = Vec::new();
    for entry_res in entries {
        let Ok(entry) = entry_res else {
            continue;
        };
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };

        if meta.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            if matches!(ext.as_deref(), Some("zip" | "mcpack")) {
                candidates.push((path, false, meta.len()));
            }
        } else if meta.is_dir() && is_resource_pack_dir(&path) {
            let dir_size = compute_dir_size(&path);
            candidates.push((path, true, dir_size));
        }
    }

    candidates.sort_by(|a, b| a.0.cmp(&b.0));

    let mut packs = Vec::with_capacity(candidates.len());
    for (path, is_dir, size_bytes) in candidates {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let summary = match PackContainer::read_from_path(&path, limits) {
            Ok(container) => match inspect_container(&container, mappings) {
                Ok(s) => Ok(s),
                Err(err) => Err(format!("{err:#}")),
            },
            Err(err) => Err(format!("{err}")),
        };

        // Skip plain zip archives that are not Minecraft resource packs unless they failed
        // after finding a manifest/pack.mcmeta.
        if let Err(ref msg) = summary
            && !is_dir
            && msg.contains("Unable to detect resource pack edition")
        {
            continue;
        }

        packs.push(DiscoveredPack {
            path,
            file_name,
            size_bytes,
            is_dir,
            summary,
            selected: false,
        });
    }

    Ok(packs)
}

/// Loads the full canonical `ResourcePackIR` and `ConversionReport` for deep asset exploration.
pub fn load_pack_ir(
    path: &Path,
    mappings: &MappingTable,
    limits: IoLimits,
) -> Result<(Edition, ResourcePackIR, ConversionReport)> {
    let container = PackContainer::read_from_path(path, limits)
        .with_context(|| format!("failed to open pack '{}'", path.display()))?;
    let edition = detect_edition(&container)?;
    let (ir, report) = match edition {
        Edition::Java => parse_java_pack(&container, mappings)?,
        Edition::Bedrock => parse_bedrock_pack(&container, mappings)?,
    };
    Ok((edition, ir, report))
}

/// Runs a full bidirectional conversion (`Source -> Opposite -> Source`) and verifies output
/// container integrity (valid JSON, non-empty assets, valid Java bounds `[-16, 32]`).
pub fn audit_pack_roundtrip(
    path: &Path,
    target_version: Option<&str>,
    version_options: &VersionSpecificOptions,
    mappings: &MappingTable,
    limits: IoLimits,
) -> AuditOutcome {
    let start = Instant::now();
    let pack_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let container = match PackContainer::read_from_path(path, limits) {
        Ok(c) => c,
        Err(err) => {
            return AuditOutcome {
                pack_name,
                path: path.to_path_buf(),
                source_edition: Edition::Java,
                target_edition: Edition::Bedrock,
                passed: false,
                forward_report: None,
                detail: format!("read error: {err}"),
                elapsed_ms: start.elapsed().as_millis(),
            };
        }
    };

    let source = match detect_edition(&container) {
        Ok(ed) => ed,
        Err(err) => {
            return AuditOutcome {
                pack_name,
                path: path.to_path_buf(),
                source_edition: Edition::Java,
                target_edition: Edition::Bedrock,
                passed: false,
                forward_report: None,
                detail: format!("edition error: {err}"),
                elapsed_ms: start.elapsed().as_millis(),
            };
        }
    };
    let target = match source {
        Edition::Java => Edition::Bedrock,
        Edition::Bedrock => Edition::Java,
    };

    let (converted, _, _, forward_report) = match convert_container_with_options(
        &container,
        Some(target),
        target_version,
        mappings,
        false,
        version_options,
    ) {
        Ok(res) => res,
        Err(err) => {
            return AuditOutcome {
                pack_name,
                path: path.to_path_buf(),
                source_edition: source,
                target_edition: target,
                passed: false,
                forward_report: None,
                detail: format!("forward convert error: {err:#}"),
                elapsed_ms: start.elapsed().as_millis(),
            };
        }
    };

    // Audit emitted JSON validity & Java element bounds.
    let mut issues: Vec<String> = Vec::new();
    for (rel, bytes) in converted.files() {
        if rel.ends_with(".json") || rel.ends_with(".mcmeta") {
            match serde_json::from_slice::<serde_json::Value>(bytes) {
                Ok(val) => {
                    if target == Edition::Java
                        && rel.starts_with("assets/minecraft/models/")
                        && let Some(elements) = val.get("elements").and_then(|e| e.as_array())
                    {
                        for (idx, el) in elements.iter().enumerate() {
                            for key in ["from", "to"] {
                                if let Some(arr) = el.get(key).and_then(|v| v.as_array()) {
                                    for c in arr.iter().filter_map(|v| v.as_f64()) {
                                        if !(-16.001..=32.001).contains(&c) {
                                            issues.push(format!(
                                                "{rel}[{idx}].{key} out of [-16,32]: {c:.2}"
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(err) => issues.push(format!("invalid JSON '{rel}': {err}")),
            }
        }
    }

    // Round-trip back to source edition to ensure emitted pack parses cleanly.
    let roundtrip_res = convert_container_with_options(
        &converted,
        Some(source),
        None,
        mappings,
        false,
        &VersionSpecificOptions::default(),
    );

    match roundtrip_res {
        Ok((rt_container, _, _, _)) if issues.is_empty() => AuditOutcome {
            pack_name,
            path: path.to_path_buf(),
            source_edition: source,
            target_edition: target,
            passed: true,
            forward_report: Some(forward_report),
            detail: format!(
                "OK: {} out files -> {} round-trip files, 0 JSON/bounds issues",
                converted.len(),
                rt_container.len()
            ),
            elapsed_ms: start.elapsed().as_millis(),
        },
        Ok(_) => AuditOutcome {
            pack_name,
            path: path.to_path_buf(),
            source_edition: source,
            target_edition: target,
            passed: false,
            forward_report: Some(forward_report),
            detail: format!("{} audit issue(s): {}", issues.len(), issues.join("; ")),
            elapsed_ms: start.elapsed().as_millis(),
        },
        Err(err) => AuditOutcome {
            pack_name,
            path: path.to_path_buf(),
            source_edition: source,
            target_edition: target,
            passed: false,
            forward_report: Some(forward_report),
            detail: format!("round-trip re-parse failed: {err:#}"),
            elapsed_ms: start.elapsed().as_millis(),
        },
    }
}

fn is_resource_pack_dir(dir: &Path) -> bool {
    dir.join("pack.mcmeta").is_file() || dir.join("manifest.json").is_file()
}

fn compute_dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        let Ok(rd) = fs::read_dir(&cur) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if let Ok(m) = entry.metadata() {
                if m.is_file() {
                    total = total.saturating_add(m.len());
                } else if m.is_dir() {
                    stack.push(p);
                }
            }
        }
    }
    total
}

/// Formats a byte count in human-readable units (`B`, `KB`, `MB`).
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
