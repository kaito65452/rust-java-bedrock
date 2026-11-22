use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use rp_bedrock::schema::BedrockManifest;
use rp_bedrock::{
    audit_bedrock_container, emit_bedrock_pack, parse_bedrock_pack,
    parse_bedrock_pack_with_options,
};
use rp_core::{
    AuditStatus, BedrockGeoVersion, ConversionReport, ConversionWarning, Edition, MappingTable,
    PipelineStage, RotationSnapMode, SemanticAuditReport, WarningSeverity, parse_minecraft_json,
};
use rp_io::{IoLimits, PackContainer};
use rp_java::{emit_java_pack, parse_java_pack};

use crate::artist::check_uv_atlas_guard;
use crate::workspace::{LoadedPack, PackSource, detect_pack_edition};

/// Target conversion direction selected in the Converter sidebar or Dashboard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConversionDirection {
    #[default]
    JavaToBedrock,
    BedrockToJava,
}

impl ConversionDirection {
    /// Human-readable label for UI selectors.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::JavaToBedrock => "Java → Bedrock",
            Self::BedrockToJava => "Bedrock → Java",
        }
    }

    /// Target Minecraft edition produced by this direction.
    #[must_use]
    pub const fn target_edition(self) -> Edition {
        match self {
            Self::JavaToBedrock => Edition::Bedrock,
            Self::BedrockToJava => Edition::Java,
        }
    }
}

/// Configuration for Preflight Scan and Conversion execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionConfig {
    pub direction: ConversionDirection,
    /// Optional target version string (e.g. `"1.21.0"`, `"26.60"`, `"1.21.4"`, or empty for default).
    pub target_version: String,
    /// Optional output path (`.mcpack`, `.zip`, or directory).
    pub output_path: String,
    /// Convert Java LabPBR (`_n.png`/`_s.png`) into Bedrock `*_mer.png` + `*.texture_set.json`.
    pub convert_pbr: bool,
    /// Enable 3D UV-Atlas Guard checks during preflight and audit.
    pub uv_atlas_guard: bool,
    /// Bedrock -> Java non-22.5° angle handling mode.
    pub rotation_mode: RotationSnapMode,
    /// Bedrock `manifest.json` `format_version` (`1`, `2`, or `3`).
    pub bedrock_manifest_version: u32,
}

impl Default for ConversionConfig {
    fn default() -> Self {
        Self {
            direction: ConversionDirection::JavaToBedrock,
            target_version: String::from("1.21.0"),
            output_path: String::new(),
            convert_pbr: false,
            uv_atlas_guard: true,
            rotation_mode: RotationSnapMode::Preserve,
            bedrock_manifest_version: 2,
        }
    }
}

/// Classification of an individual asset in the Preflight Scan table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreflightAssetStatus {
    Ready,
    Lossy,
    Skipped,
    Warning,
}

impl PreflightAssetStatus {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ready => "READY",
            Self::Lossy => "LOSSY",
            Self::Skipped => "SKIPPED",
            Self::Warning => "WARNING",
        }
    }
}

/// Row rendered in the Conversion & Preflight Dashboard table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightRow {
    pub source_path: String,
    pub category: String,
    pub status: PreflightAssetStatus,
    pub target_hint: String,
    pub detail: String,
}

/// Dry-run Preflight Report produced before committing any conversion output to disk.
#[derive(Debug, Clone)]
pub struct PreflightReport {
    pub source_edition: Edition,
    pub target_edition: Edition,
    pub total_assets: usize,
    pub ready_count: usize,
    pub lossy_count: usize,
    pub skipped_count: usize,
    pub warning_count: usize,
    pub rows: Vec<PreflightRow>,
    pub diagnostics: Vec<ConversionWarning>,
    pub semantic_preview_status: AuditStatus,
    pub semantic_preview_summary: String,
    pub elapsed_ms: u128,
}

/// Result of executing a full background conversion + Semantic Audit Gate.
#[derive(Debug, Clone)]
pub struct ConversionResult {
    pub source_edition: Edition,
    pub target_edition: Edition,
    pub output_container: PackContainer,
    pub output_written_to: Option<PathBuf>,
    pub report: ConversionReport,
    pub gate_status: AuditStatus,
    pub gate_summary: String,
    pub elapsed_ms: u128,
}

/// Result of running standalone Semantic Validation on the currently open pack.
#[derive(Debug, Clone)]
pub struct AuditRunResult {
    pub edition: Edition,
    pub status: AuditStatus,
    pub summary: String,
    pub semantic_audit: Option<SemanticAuditReport>,
    pub diagnostics: Vec<ConversionWarning>,
    pub elapsed_ms: u128,
}

/// Events delivered asynchronously from background worker threads to the UI thread via `mpsc`.
pub enum BackgroundEvent {
    Log {
        level: WarningSeverity,
        message: String,
    },
    PackLoaded(Result<LoadedPack, String>),
    PreflightFinished(Result<PreflightReport, String>),
    ConversionFinished(Result<ConversionResult, String>),
    AuditFinished(Result<AuditRunResult, String>),
}

/// Strictly enforces the Anti-False-Positive Semantic Audit Gate.
///
/// Guarantees `AuditStatus::Passed` is NEVER returned when:
/// - `semantic_audit.manifest_valid == false`,
/// - Bedrock `manifest.json` has `min_engine_version[0] != 1`,
/// - `total_input_textures > 0 && semantically_mapped_textures == 0`,
/// - `forbidden_entries` or `broken_references` are non-empty,
/// - or any `Error`-severity diagnostic exists.
#[must_use]
pub fn evaluate_semantic_gate(
    target_edition: Edition,
    container: &PackContainer,
    audit: Option<&SemanticAuditReport>,
    diagnostics: &[ConversionWarning],
    skipped_count: usize,
    lossy_count: usize,
) -> (AuditStatus, String) {
    // Hard check 1: Bedrock manifest min_engine_version[0] MUST equal 1.
    if target_edition == Edition::Bedrock {
        match container.get("manifest.json") {
            None => {
                return (
                    AuditStatus::Failed,
                    String::from("FAILED: missing required Bedrock manifest.json"),
                );
            }
            Some(bytes) => match parse_minecraft_json::<BedrockManifest>(bytes) {
                Err(err) => {
                    return (
                        AuditStatus::Failed,
                        format!("FAILED: invalid Bedrock manifest.json ({err})"),
                    );
                }
                Ok(manifest) => {
                    if manifest.header.min_engine_version[0] != 1 {
                        return (
                            AuditStatus::Failed,
                            format!(
                                "FAILED: manifest.json min_engine_version {:?} has major != 1",
                                manifest.header.min_engine_version
                            ),
                        );
                    }
                }
            },
        }
    }

    // Hard check 2: Bedrock SemanticAuditReport invariants.
    if let Some(a) = audit
        && (!a.manifest_valid
            || !a.forbidden_entries.is_empty()
            || !a.broken_references.is_empty()
            || (a.total_input_textures > 0 && a.semantically_mapped_textures == 0)
            || a.status == AuditStatus::Failed)
    {
        return (AuditStatus::Failed, a.summary.clone());
    }

    // Hard check 3: Any Error-severity diagnostic fails the gate.
    let error_count = diagnostics
        .iter()
        .filter(|d| d.severity == WarningSeverity::Error)
        .count();
    if error_count > 0 {
        return (
            AuditStatus::Failed,
            format!("FAILED: {error_count} semantic error(s) detected"),
        );
    }

    // Warning / Partial check.
    if let Some(a) = audit
        && (a.status == AuditStatus::Partial || !a.orphan_textures.is_empty())
    {
        return (AuditStatus::Partial, a.summary.clone());
    }

    let warning_count = diagnostics
        .iter()
        .filter(|d| d.severity == WarningSeverity::Warning)
        .count();
    if warning_count > 0 || skipped_count > 0 || lossy_count > 0 {
        return (
            AuditStatus::Partial,
            format!(
                "PARTIAL / WARNING: {warning_count} warning(s), {lossy_count} lossy, {skipped_count} skipped"
            ),
        );
    }

    if let Some(a) = audit {
        (AuditStatus::Passed, a.summary.clone())
    } else {
        (
            AuditStatus::Passed,
            String::from("PASSED: semantic validation succeeded with 0 errors and 0 warnings"),
        )
    }
}

/// Spawns a background thread to load a pack from a directory or `.zip`/`.mcpack` archive.
pub fn spawn_load_pack(path: PathBuf, tx: Sender<BackgroundEvent>) {
    thread::spawn(move || {
        let _ = tx.send(BackgroundEvent::Log {
            level: WarningSeverity::Info,
            message: format!("Loading resource pack '{}'...", path.display()),
        });
        let res = load_pack_sync(&path).map_err(|e| format!("{e:#}"));
        let _ = tx.send(BackgroundEvent::PackLoaded(res));
    });
}

/// Loads a resource pack synchronously from a directory or archive path.
pub fn load_pack_sync(path: &std::path::Path) -> Result<LoadedPack> {
    let container = PackContainer::read_from_path(path, IoLimits::default())
        .with_context(|| format!("failed to read resource pack '{}'", path.display()))?;
    let source = if path.is_dir() {
        PackSource::Directory(path.to_path_buf())
    } else {
        PackSource::Archive(path.to_path_buf())
    };
    LoadedPack::from_container(source, container)
}

/// Spawns a background Preflight Scan without blocking the UI thread.
pub fn spawn_preflight(
    container: PackContainer,
    config: ConversionConfig,
    tx: Sender<BackgroundEvent>,
) {
    thread::spawn(move || {
        let _ = tx.send(BackgroundEvent::Log {
            level: WarningSeverity::Info,
            message: format!("Running Preflight Scan ({})...", config.direction.label()),
        });
        let res = run_preflight_sync(&container, &config).map_err(|e| format!("{e:#}"));
        let _ = tx.send(BackgroundEvent::PreflightFinished(res));
    });
}

/// Executes a full dry-run Preflight Scan (Parse → Map/Emit Preview → Semantic Audit Preview).
pub fn run_preflight_sync(
    container: &PackContainer,
    config: &ConversionConfig,
) -> Result<PreflightReport> {
    let start = Instant::now();
    let mappings = MappingTable::default_embedded()
        .context("failed to load embedded default mapping registry")?;

    let (output, source_edition, target_edition, mut report, ir_textures) =
        execute_core_pipeline(container, config, &mappings)?;

    if config.uv_atlas_guard {
        report.warnings.extend(check_uv_atlas_guard(container));
    }

    let mut rows = Vec::new();

    // Ready textures summary
    for (tex_id, target_path) in &ir_textures {
        let is_orphan = report
            .semantic_audit
            .as_ref()
            .is_some_and(|a| a.orphan_textures.iter().any(|o| o.eq_ignore_ascii_case(target_path)));
        let (status, detail) = if is_orphan {
            (
                PreflightAssetStatus::Warning,
                String::from("Output texture is not referenced by vanilla index or atlas (orphan)"),
            )
        } else {
            (
                PreflightAssetStatus::Ready,
                String::from("Mapped cleanly to target edition"),
            )
        };
        rows.push(PreflightRow {
            source_path: tex_id.clone(),
            category: String::from("Texture"),
            status,
            target_hint: target_path.clone(),
            detail,
        });
    }

    for lossy in &report.lossy_conversions {
        rows.push(PreflightRow {
            source_path: lossy.source_path.clone(),
            category: format!("{}", lossy.stage),
            status: PreflightAssetStatus::Lossy,
            target_hint: String::from("(approximated)"),
            detail: lossy.detail.clone(),
        });
    }

    for skipped in &report.skipped_assets {
        rows.push(PreflightRow {
            source_path: skipped.source_path.clone(),
            category: format!("{}", skipped.stage),
            status: PreflightAssetStatus::Skipped,
            target_hint: String::from("(omitted)"),
            detail: skipped.reason.clone(),
        });
    }

    for warn in &report.warnings {
        if warn.severity == WarningSeverity::Error {
            rows.push(PreflightRow {
                source_path: warn.source_path.clone(),
                category: format!("{}", warn.stage),
                status: PreflightAssetStatus::Warning,
                target_hint: String::from("(audit error)"),
                detail: warn.reason.clone(),
            });
        }
    }

    let ready_count = rows
        .iter()
        .filter(|r| r.status == PreflightAssetStatus::Ready)
        .count();
    let lossy_count = rows
        .iter()
        .filter(|r| r.status == PreflightAssetStatus::Lossy)
        .count();
    let skipped_count = rows
        .iter()
        .filter(|r| r.status == PreflightAssetStatus::Skipped)
        .count();
    let warning_count = rows
        .iter()
        .filter(|r| r.status == PreflightAssetStatus::Warning)
        .count();

    let (gate_status, gate_summary) = evaluate_semantic_gate(
        target_edition,
        &output,
        report.semantic_audit.as_ref(),
        &report.warnings,
        report.skipped_count,
        report.lossy_count,
    );

    Ok(PreflightReport {
        source_edition,
        target_edition,
        total_assets: rows.len(),
        ready_count,
        lossy_count,
        skipped_count,
        warning_count,
        rows,
        diagnostics: report.warnings,
        semantic_preview_status: gate_status,
        semantic_preview_summary: gate_summary,
        elapsed_ms: start.elapsed().as_millis(),
    })
}

/// Spawns a background conversion + Semantic Audit Gate job.
pub fn spawn_convert(
    container: PackContainer,
    config: ConversionConfig,
    tx: Sender<BackgroundEvent>,
) {
    thread::spawn(move || {
        let _ = tx.send(BackgroundEvent::Log {
            level: WarningSeverity::Info,
            message: format!("Executing conversion ({})...", config.direction.label()),
        });
        let res = run_convert_sync(&container, &config).map_err(|e| format!("{e:#}"));
        let _ = tx.send(BackgroundEvent::ConversionFinished(res));
    });
}

/// Runs full conversion + Semantic Audit Gate synchronously (called by background worker & tests).
pub fn run_convert_sync(
    container: &PackContainer,
    config: &ConversionConfig,
) -> Result<ConversionResult> {
    let start = Instant::now();
    let mappings = MappingTable::default_embedded()
        .context("failed to load embedded default mapping registry")?;

    let (output, source_edition, target_edition, mut report, _) =
        execute_core_pipeline(container, config, &mappings)?;

    if config.uv_atlas_guard {
        report.warnings.extend(check_uv_atlas_guard(container));
    }

    let (gate_status, gate_summary) = evaluate_semantic_gate(
        target_edition,
        &output,
        report.semantic_audit.as_ref(),
        &report.warnings,
        report.skipped_count,
        report.lossy_count,
    );

    // Only write output to disk when requested AND the Semantic Audit Gate did not fail.
    let output_written_to = if !config.output_path.trim().is_empty()
        && gate_status != AuditStatus::Failed
    {
        let out_path = PathBuf::from(config.output_path.trim());
        output
            .write_to_path(&out_path)
            .with_context(|| format!("failed to write converted pack to '{}'", out_path.display()))?;
        Some(out_path)
    } else {
        None
    };

    Ok(ConversionResult {
        source_edition,
        target_edition,
        output_container: output,
        output_written_to,
        report,
        gate_status,
        gate_summary,
        elapsed_ms: start.elapsed().as_millis(),
    })
}

/// Spawns a background Semantic Audit job for the currently loaded pack.
pub fn spawn_semantic_audit(
    container: PackContainer,
    uv_atlas_guard: bool,
    tx: Sender<BackgroundEvent>,
) {
    thread::spawn(move || {
        let _ = tx.send(BackgroundEvent::Log {
            level: WarningSeverity::Info,
            message: String::from("Running Semantic Validation Audit..."),
        });
        let res = run_semantic_audit_sync(&container, uv_atlas_guard).map_err(|e| format!("{e:#}"));
        let _ = tx.send(BackgroundEvent::AuditFinished(res));
    });
}

/// Runs standalone Semantic Validation against a `PackContainer` (Java or Bedrock).
pub fn run_semantic_audit_sync(
    container: &PackContainer,
    uv_atlas_guard: bool,
) -> Result<AuditRunResult> {
    let start = Instant::now();
    let mappings = MappingTable::default_embedded()
        .context("failed to load embedded default mapping registry")?;
    let edition = detect_pack_edition(container)?;

    let mut diagnostics: Vec<ConversionWarning> = Vec::new();
    if uv_atlas_guard {
        diagnostics.extend(check_uv_atlas_guard(container));
    }

    match edition {
        Edition::Bedrock => {
            let input_tex_count = container
                .files()
                .keys()
                .filter(|p| {
                    let lower = p.to_ascii_lowercase();
                    lower.ends_with(".png")
                        && lower != "pack_icon.png"
                        && lower != "bug_pack_icon.png"
                })
                .count();
            let audit = audit_bedrock_container(container, input_tex_count, &mappings);
            append_audit_diagnostics(&audit, &mut diagnostics);

            let (status, summary) = evaluate_semantic_gate(
                Edition::Bedrock,
                container,
                Some(&audit),
                &diagnostics,
                0,
                0,
            );

            Ok(AuditRunResult {
                edition,
                status,
                summary,
                semantic_audit: Some(audit),
                diagnostics,
                elapsed_ms: start.elapsed().as_millis(),
            })
        }
        Edition::Java => {
            let (ir, report) =
                parse_java_pack(container, &mappings).context("failed to audit Java pack")?;
            diagnostics.extend(report.warnings);

            if ir.textures.is_empty() && ir.models.is_empty() && ir.sounds.files.is_empty() {
                diagnostics.push(ConversionWarning {
                    source_path: String::from("pack.mcmeta"),
                    stage: PipelineStage::Audit,
                    severity: WarningSeverity::Error,
                    reason: String::from(
                        "Java pack contains 0 textures, 0 models, and 0 sounds",
                    ),
                    actionable_fix: String::from(
                        "Place textures under assets/minecraft/textures/ before exporting or converting.",
                    ),
                    asset: String::from("pack.mcmeta"),
                    message: String::from("Java pack has no assets"),
                });
            }

            let (status, summary) = evaluate_semantic_gate(
                Edition::Java,
                container,
                None,
                &diagnostics,
                report.skipped_count,
                report.lossy_count,
            );

            Ok(AuditRunResult {
                edition,
                status,
                summary,
                semantic_audit: None,
                diagnostics,
                elapsed_ms: start.elapsed().as_millis(),
            })
        }
    }
}

type CorePipelineResult = (
    PackContainer,
    Edition,
    Edition,
    ConversionReport,
    Vec<(String, String)>,
);

fn execute_core_pipeline(
    input: &PackContainer,
    config: &ConversionConfig,
    mappings: &MappingTable,
) -> Result<CorePipelineResult> {
    let source = detect_pack_edition(input)?;
    let target = config.direction.target_edition();

    if !(1..=3).contains(&config.bedrock_manifest_version) {
        bail!(
            "invalid bedrock manifest format_version '{}': expected 1, 2, or 3",
            config.bedrock_manifest_version
        );
    }

    let resolved_target_ver = if config.target_version.trim().is_empty() {
        None
    } else {
        Some(
            mappings
                .resolve_target_version(target, config.target_version.trim())
                .with_context(|| {
                    format!("failed to resolve target version '{}'", config.target_version)
                })?,
        )
    };

    let (mut ir, mut report) = match source {
        Edition::Java => {
            parse_java_pack(input, mappings).context("failed to parse source Java pack")?
        }
        Edition::Bedrock => {
            let target_java_fmt = resolved_target_ver.and_then(|(jf, _)| jf);
            parse_bedrock_pack_with_options(
                input,
                mappings,
                target_java_fmt,
                config.rotation_mode,
            )
            .or_else(|_| parse_bedrock_pack(input, mappings))
            .context("failed to parse source Bedrock pack")?
        }
    };

    if let Some((java_fmt, bedrock_ver)) = resolved_target_ver {
        if let Some(fmt) = java_fmt {
            ir.metadata.java_pack_format = Some(fmt);
        }
        if let Some(bver) = bedrock_ver {
            ir.metadata.bedrock_min_engine_version = Some(bver);
            ir.metadata.bedrock_geo_version = Some(BedrockGeoVersion::for_engine_version(bver));
        }
    }
    ir.metadata.rotation_mode = config.rotation_mode;
    ir.metadata.bedrock_manifest_version = Some(config.bedrock_manifest_version);
    ir.metadata.convert_pbr = config.convert_pbr;

    let total_input_textures = ir.textures.len();
    let texture_map_preview: Vec<(String, String)> = ir
        .textures
        .keys()
        .map(|id| match target {
            Edition::Bedrock => {
                let target_tex = mappings.java_texture_to_bedrock(id);
                (
                    format!("assets/minecraft/textures/{id}.png"),
                    format!("{}.png", target_tex.path),
                )
            }
            Edition::Java => (
                id.clone(),
                format!("assets/minecraft/textures/{id}.png"),
            ),
        })
        .collect();

    let (output, emit_report) = match target {
        Edition::Bedrock => {
            emit_bedrock_pack(&ir, mappings).context("failed to emit Bedrock pack")?
        }
        Edition::Java => emit_java_pack(&ir, mappings).context("failed to emit Java pack")?,
    };
    report.merge(emit_report);

    if target == Edition::Bedrock {
        let audit = audit_bedrock_container(&output, total_input_textures, mappings);
        append_audit_diagnostics(&audit, &mut report.warnings);
        report.semantic_audit = Some(audit);
    }

    Ok((output, source, target, report, texture_map_preview))
}

fn append_audit_diagnostics(audit: &SemanticAuditReport, out: &mut Vec<ConversionWarning>) {
    for forb in &audit.forbidden_entries {
        out.push(ConversionWarning {
            source_path: forb.path.clone(),
            stage: PipelineStage::Audit,
            severity: WarningSeverity::Error,
            reason: format!("[{}] {}", forb.rule_id, forb.reason),
            actionable_fix: forb.actionable_fix.clone(),
            asset: forb.path.clone(),
            message: format!("[{}] {}", forb.rule_id, forb.reason),
        });
    }
    for broken in &audit.broken_references {
        let msg = format!(
            "Broken reference in '{}': missing target '{}'",
            broken.source_file, broken.missing_path
        );
        out.push(ConversionWarning {
            source_path: broken.source_file.clone(),
            stage: PipelineStage::Audit,
            severity: WarningSeverity::Error,
            reason: msg.clone(),
            actionable_fix: String::from(
                "Ensure every texture, locale, or sound referenced in JSON atlases or texture sets exists in the pack.",
            ),
            asset: broken.source_file.clone(),
            message: msg,
        });
    }
    for orphan in &audit.orphan_textures {
        let msg = String::from(
            "Orphan output texture will never be loaded by Bedrock (not a vanilla path and not referenced in any atlas, texture_set, or attachable)",
        );
        out.push(ConversionWarning {
            source_path: orphan.clone(),
            stage: PipelineStage::Audit,
            severity: WarningSeverity::Warning,
            reason: msg.clone(),
            actionable_fix: String::from(
                "Bind the texture through a vanilla blockstate/model or register it in terrain_texture.json / item_texture.json.",
            ),
            asset: orphan.clone(),
            message: msg,
        });
    }
    if audit.total_input_textures > 0 && audit.semantically_mapped_textures == 0 {
        let msg = format!(
            "0 of {} input texture(s) semantically mapped to any readable Bedrock texture or atlas entry",
            audit.total_input_textures
        );
        out.push(ConversionWarning {
            source_path: String::from("manifest.json"),
            stage: PipelineStage::Audit,
            severity: WarningSeverity::Error,
            reason: msg.clone(),
            actionable_fix: String::from(
                "Verify the pack uses valid vanilla texture IDs or bind custom textures through vanilla blockstates/models.",
            ),
            asset: String::from("manifest.json"),
            message: msg,
        });
    }
}
