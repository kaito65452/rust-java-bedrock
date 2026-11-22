use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use rp_bedrock::schema::BedrockManifest;
use rp_bedrock::{
    audit_bedrock_container, emit_bedrock_pack, parse_bedrock_pack,
    parse_bedrock_pack_with_options,
};
use rp_core::{
    AuditStatus, BedrockGeoVersion, ConversionReport, ConversionWarning, Edition, MappingTable,
    PipelineStage, RotationSnapMode, SemanticAuditReport, WarningSeverity,
    normalize_bedrock_engine_triplet, parse_minecraft_json,
};
use rp_io::{IoLimits, PackContainer, is_os_junk, sanitize_archive_path};
use rp_java::{emit_java_pack, parse_java_pack};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Window};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::state::StudioState;
use crate::texture::{
    TextureFormatKind, check_uv_atlas_guard, composite_grass_side_tga, decode_texture_bytes,
    encode_texture_bytes, make_texture_opaque, resize_nearest_neighbor,
};
use crate::workspace::{detect_pack_edition, sanitize_minecraft_slug};

/// Handling mode for the 3D UV-Atlas Guard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UvAtlasGuardMode {
    #[default]
    SafeFallback2d,
    Skip,
    Warn,
}

/// Converter options exposed from the Converter Studio UI to `run_preflight` & `execute_convert`.
#[derive(Debug, Clone, Deserialize)]
pub struct ConverterOptions {
    #[serde(default)]
    pub target_version: String,
    #[serde(default = "default_true")]
    pub composite_grass_tga: bool,
    #[serde(default)]
    pub uv_atlas_guard_mode: UvAtlasGuardMode,
    #[serde(default)]
    pub convert_pbr: bool,
    #[serde(default)]
    pub auto_downscale: Option<u32>,
    #[serde(default = "default_true")]
    pub strip_junk_and_raw_java: bool,
    #[serde(default)]
    pub rotation_mode: RotationSnapMode,
    #[serde(default = "default_manifest_ver")]
    pub bedrock_manifest_version: u32,
}

const fn default_true() -> bool {
    true
}
const fn default_manifest_ver() -> u32 {
    2
}

impl Default for ConverterOptions {
    fn default() -> Self {
        Self {
            target_version: String::from("1.21.0"),
            composite_grass_tga: true,
            uv_atlas_guard_mode: UvAtlasGuardMode::SafeFallback2d,
            convert_pbr: false,
            auto_downscale: None,
            strip_junk_and_raw_java: true,
            rotation_mode: RotationSnapMode::Preserve,
            bedrock_manifest_version: 2,
        }
    }
}

/// Progress event emitted on `"converter://progress"`.
#[derive(Debug, Clone, Serialize)]
pub struct ConvertProgressEvent {
    pub fraction: f32,
    pub message: String,
}

/// Individual row returned inside `PreflightReportDto`.
#[derive(Debug, Clone, Serialize)]
pub struct PreflightRowDto {
    pub source_path: String,
    pub category: String,
    pub status: String,
    pub target_hint: String,
    pub detail: String,
}

/// Preflight report returned by `run_preflight`.
#[derive(Debug, Clone, Serialize)]
pub struct PreflightReportDto {
    pub source_edition: Edition,
    pub target_edition: Edition,
    pub total_assets: usize,
    pub ready_count: usize,
    pub composite_count: usize,
    pub lossy_count: usize,
    pub skipped_count: usize,
    pub warning_count: usize,
    pub rows: Vec<PreflightRowDto>,
    pub diagnostics: Vec<ConversionWarning>,
    pub semantic_preview_status: AuditStatus,
    pub semantic_preview_summary: String,
    pub elapsed_ms: u128,
}

/// Conversion result returned by `execute_convert`.
#[derive(Debug, Clone, Serialize)]
pub struct ConversionResultDto {
    pub source_edition: Edition,
    pub target_edition: Edition,
    pub output_written_to: Option<String>,
    pub converted_count: usize,
    pub passthrough_count: usize,
    pub skipped_count: usize,
    pub lossy_count: usize,
    pub warnings: Vec<ConversionWarning>,
    pub gate_status: AuditStatus,
    pub gate_summary: String,
    pub elapsed_ms: u128,
}

/// Result returned by `run_audit`.
#[derive(Debug, Clone, Serialize)]
pub struct AuditResultDto {
    pub edition: Edition,
    pub status: AuditStatus,
    pub summary: String,
    pub problems: Vec<ConversionWarning>,
    pub elapsed_ms: u128,
}

/// Export configuration passed to `package_pack`.
#[derive(Debug, Clone, Deserialize)]
pub struct PackageExportConfig {
    pub format: String, // "zip" | "mcpack" | "mcaddon"
    pub filename_template: String,
    pub output_dir: String,
    #[serde(default = "default_true")]
    pub strip_junk_files: bool,
    #[serde(default = "default_balanced")]
    pub compression: String, // "fast" | "balanced" | "maximum"
    #[serde(default = "default_true")]
    pub run_semantic_audit: bool,
    #[serde(default)]
    pub allow_export_with_errors: bool,
}

fn default_balanced() -> String {
    String::from("balanced")
}

/// Summary returned by `package_pack`.
#[derive(Debug, Clone, Serialize)]
pub struct PackageExportSummary {
    pub output_path: String,
    pub size_bytes: u64,
    pub asset_count: usize,
    pub stripped_junk_count: usize,
    pub audit_status: Option<AuditStatus>,
    pub audit_summary: String,
}

/// Resolves the conversion direction string (`"java_to_bedrock"` / `"bedrock_to_java"`).
fn parse_direction(direction: &str) -> Result<Edition> {
    match direction.trim().to_ascii_lowercase().as_str() {
        "java_to_bedrock" | "bedrock" => Ok(Edition::Bedrock),
        "bedrock_to_java" | "java" => Ok(Edition::Java),
        other => bail!("invalid conversion direction '{other}'"),
    }
}

/// Strictly enforces the Anti-False-Positive Semantic Audit Gate.
#[must_use]
pub fn evaluate_semantic_gate(
    target_edition: Edition,
    container: &PackContainer,
    audit: Option<&SemanticAuditReport>,
    diagnostics: &[ConversionWarning],
    skipped_count: usize,
    lossy_count: usize,
) -> (AuditStatus, String) {
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

    if let Some(a) = audit
        && (!a.manifest_valid
            || !a.forbidden_entries.is_empty()
            || !a.broken_references.is_empty()
            || (a.total_input_textures > 0 && a.semantically_mapped_textures == 0)
            || a.status == AuditStatus::Failed)
    {
        return (AuditStatus::Failed, a.summary.clone());
    }

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

type PipelineRun = (
    PackContainer,
    Edition,
    Edition,
    ConversionReport,
    Vec<(String, String)>,
    usize,
);

fn execute_pipeline_core(
    input: &PackContainer,
    target: Edition,
    options: &ConverterOptions,
    mappings: &MappingTable,
) -> Result<PipelineRun> {
    let source = detect_pack_edition(input)?;
    if !(1..=3).contains(&options.bedrock_manifest_version) {
        bail!(
            "invalid bedrock manifest format_version '{}': expected 1, 2, or 3",
            options.bedrock_manifest_version
        );
    }

    let resolved_target_ver = if options.target_version.trim().is_empty() {
        None
    } else {
        Some(
            mappings
                .resolve_target_version(target, options.target_version.trim())
                .with_context(|| {
                    format!("failed to resolve target version '{}'", options.target_version)
                })?,
        )
    };

    let (mut ir, mut report) = match source {
        Edition::Java => parse_java_pack(input, mappings).context("failed parsing Java pack")?,
        Edition::Bedrock => {
            let target_java_fmt = resolved_target_ver.and_then(|(jf, _)| jf);
            parse_bedrock_pack_with_options(input, mappings, target_java_fmt, options.rotation_mode)
                .or_else(|_| parse_bedrock_pack(input, mappings))
                .context("failed parsing Bedrock pack")?
        }
    };

    if let Some((java_fmt, bedrock_ver)) = resolved_target_ver {
        if let Some(fmt) = java_fmt {
            ir.metadata.java_pack_format = Some(fmt);
        }
        if let Some(bver) = bedrock_ver {
            let norm = normalize_bedrock_engine_triplet(bver);
            ir.metadata.bedrock_min_engine_version = Some(norm);
            ir.metadata.bedrock_geo_version = Some(BedrockGeoVersion::for_engine_version(norm));
        }
    }
    if let Some(bver) = ir.metadata.bedrock_min_engine_version {
        ir.metadata.bedrock_min_engine_version = Some(normalize_bedrock_engine_triplet(bver));
    }
    ir.metadata.rotation_mode = options.rotation_mode;
    ir.metadata.bedrock_manifest_version = Some(options.bedrock_manifest_version);
    ir.metadata.convert_pbr = options.convert_pbr;

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
            Edition::Java => (id.clone(), format!("assets/minecraft/textures/{id}.png")),
        })
        .collect();

    let (mut output, emit_report) = match target {
        Edition::Bedrock => emit_bedrock_pack(&ir, mappings).context("failed emitting Bedrock pack")?,
        Edition::Java => emit_java_pack(&ir, mappings).context("failed emitting Java pack")?,
    };
    report.merge(emit_report);

    let mut composite_count = 0usize;

    // Optional: Composite grass_side.tga & ensure opaque grass_top for Bedrock
    if target == Edition::Bedrock && options.composite_grass_tga {
        if let Some(side_bytes) = input
            .get("assets/minecraft/textures/block/grass_block_side.png")
            .or_else(|| input.get("textures/blocks/grass_side.png"))
            && let Ok((side_img, _)) = decode_texture_bytes(side_bytes, "grass_side.png")
        {
            let overlay_img = input
                .get("assets/minecraft/textures/block/grass_block_side_overlay.png")
                .or_else(|| input.get("textures/blocks/grass_side_overlay.png"))
                .and_then(|b| decode_texture_bytes(b, "overlay.png").ok())
                .map(|(img, _)| img);

            let plains_tint =
                glam::Vec3::new(0x91 as f32 / 255.0, 0xBD as f32 / 255.0, 0x59 as f32 / 255.0);
            if let Ok((_img, tga_bytes)) =
                composite_grass_side_tga(&side_img, overlay_img.as_ref(), plains_tint, true)
            {
                let _ = output.insert("textures/blocks/grass_side.tga", tga_bytes);
                composite_count += 1;
            }
        }
        if let Some(top_bytes) = output.get("textures/blocks/grass_top.png").map(ToOwned::to_owned)
            && let Ok((top_img, _)) = decode_texture_bytes(&top_bytes, "grass_top.png")
        {
            let opaque = make_texture_opaque(&top_img);
            if let Ok(encoded) = encode_texture_bytes(&opaque, TextureFormatKind::Png) {
                let _ = output.insert("textures/blocks/grass_top.png", encoded);
                composite_count += 1;
            }
        }
    }

    // Optional: Auto-Downscale oversized textures via fast_image_resize nearest-neighbor
    if let Some(target_sz) = options.auto_downscale.filter(|&s| s >= 8 && s <= 1024) {
        let tex_keys: Vec<String> = output
            .files()
            .keys()
            .filter(|p| {
                let l = p.to_ascii_lowercase();
                l.ends_with(".png") && l != "pack.png" && l != "pack_icon.png"
            })
            .cloned()
            .collect();
        for key in tex_keys {
            if let Some(bytes) = output.get(&key).map(ToOwned::to_owned)
                && let Ok((img, fmt)) = decode_texture_bytes(&bytes, &key)
            {
                let (w, h) = img.dimensions();
                if w > target_sz {
                    let dst_h = if h > w && w > 0 {
                        ((u64::from(h) * u64::from(target_sz)) / u64::from(w)).max(1) as u32
                    } else {
                        target_sz
                    };
                    if let Ok(resized) = resize_nearest_neighbor(&img, target_sz, dst_h)
                        && let Ok(enc) = encode_texture_bytes(&resized, fmt)
                    {
                        let _ = output.insert(&key, enc);
                    }
                }
            }
        }
    }

    // Optional: Strip OS junk & leaked raw Java entries
    if options.strip_junk_and_raw_java {
        let junk_keys: Vec<String> = output
            .files()
            .keys()
            .filter(|k| {
                is_os_junk(k)
                    || (target == Edition::Bedrock
                        && (k.starts_with("assets/") || k.eq_ignore_ascii_case("pack.mcmeta")))
            })
            .cloned()
            .collect();
        for k in junk_keys {
            output.remove(&k);
        }
    }

    // 3D UV-Atlas Guard checks
    let uv_warnings = check_uv_atlas_guard(input);
    if !uv_warnings.is_empty() {
        match options.uv_atlas_guard_mode {
            UvAtlasGuardMode::Warn | UvAtlasGuardMode::SafeFallback2d => {
                report.warnings.extend(uv_warnings);
            }
            UvAtlasGuardMode::Skip => {
                for w in uv_warnings {
                    report.record_skipped_detailed(
                        w.source_path,
                        PipelineStage::Resolve,
                        w.reason,
                        w.actionable_fix,
                    );
                }
            }
        }
    }

    if target == Edition::Bedrock {
        let audit = audit_bedrock_container(&output, total_input_textures, mappings);
        append_audit_diagnostics(&audit, &mut report.warnings);
        report.semantic_audit = Some(audit);
    }

    Ok((
        output,
        source,
        target,
        report,
        texture_map_preview,
        composite_count,
    ))
}

fn load_source_container(
    state: &StudioState,
    input_path: Option<String>,
) -> Result<PackContainer> {
    if let Some(p) = input_path.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        PackContainer::read_from_path(Path::new(p), IoLimits::default())
            .with_context(|| format!("failed reading pack '{p}'"))
    } else {
        let guard = state
            .active_pack
            .lock()
            .map_err(|_| anyhow::anyhow!("active pack mutex poisoned"))?;
        let pack = guard
            .as_ref()
            .context("No resource pack currently open")?;
        Ok(pack.container.clone())
    }
}

/// Core pure preflight function (testable directly without Tauri runtime).
pub fn run_preflight_core(
    container: &PackContainer,
    direction: &str,
    options: &ConverterOptions,
) -> Result<PreflightReportDto> {
    let start = Instant::now();
    let target = parse_direction(direction)?;
    let mappings = MappingTable::default_embedded()?;

    let (output, source_edition, target_edition, report, ir_textures, composite_count) =
        execute_pipeline_core(container, target, options, &mappings)?;

    let mut rows = Vec::new();
    for (src_path, target_path) in &ir_textures {
        let is_orphan = report
            .semantic_audit
            .as_ref()
            .is_some_and(|a| a.orphan_textures.iter().any(|o| o.eq_ignore_ascii_case(target_path)));
        let (status, detail) = if is_orphan {
            (
                "WARNING",
                "Output texture is not referenced by vanilla index or atlas (orphan)",
            )
        } else {
            ("READY", "Mapped cleanly to target edition")
        };
        rows.push(PreflightRowDto {
            source_path: src_path.clone(),
            category: String::from("Texture"),
            status: String::from(status),
            target_hint: target_path.clone(),
            detail: String::from(detail),
        });
    }

    for lossy in &report.lossy_conversions {
        rows.push(PreflightRowDto {
            source_path: lossy.source_path.clone(),
            category: lossy.stage.to_string(),
            status: String::from("LOSSY"),
            target_hint: String::from("(approximated)"),
            detail: lossy.detail.clone(),
        });
    }
    for skipped in &report.skipped_assets {
        rows.push(PreflightRowDto {
            source_path: skipped.source_path.clone(),
            category: skipped.stage.to_string(),
            status: String::from("SKIPPED"),
            target_hint: String::from("(omitted)"),
            detail: skipped.reason.clone(),
        });
    }
    for warn in &report.warnings {
        if warn.severity == WarningSeverity::Error {
            rows.push(PreflightRowDto {
                source_path: warn.source_path.clone(),
                category: warn.stage.to_string(),
                status: String::from("ERROR"),
                target_hint: String::from("(audit error)"),
                detail: warn.reason.clone(),
            });
        }
    }

    let ready_count = rows.iter().filter(|r| r.status == "READY").count();
    let lossy_count = rows.iter().filter(|r| r.status == "LOSSY").count();
    let skipped_count = rows.iter().filter(|r| r.status == "SKIPPED").count();
    let warning_count = rows
        .iter()
        .filter(|r| r.status == "WARNING" || r.status == "ERROR")
        .count();

    let (gate_status, gate_summary) = evaluate_semantic_gate(
        target_edition,
        &output,
        report.semantic_audit.as_ref(),
        &report.warnings,
        report.skipped_count,
        report.lossy_count,
    );

    Ok(PreflightReportDto {
        source_edition,
        target_edition,
        total_assets: rows.len(),
        ready_count,
        composite_count,
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

/// Core pure convert function (testable directly without Tauri runtime).
pub fn execute_convert_core(
    container: &PackContainer,
    output_path: &str,
    direction: &str,
    options: &ConverterOptions,
    mut on_progress: impl FnMut(f32, &str),
) -> Result<(PackContainer, ConversionResultDto)> {
    let start = Instant::now();
    on_progress(0.15, "Loading mapping table & parsing source pack...");
    let target = parse_direction(direction)?;
    let mappings = MappingTable::default_embedded()?;

    on_progress(0.45, "Transforming textures, models, locales & audio...");
    let (output, source_edition, target_edition, report, _, _) =
        execute_pipeline_core(container, target, options, &mappings)?;

    on_progress(0.75, "Running Semantic Audit Gate...");
    let (gate_status, gate_summary) = evaluate_semantic_gate(
        target_edition,
        &output,
        report.semantic_audit.as_ref(),
        &report.warnings,
        report.skipped_count,
        report.lossy_count,
    );

    let clean_out = output_path.trim();
    let written_to = if !clean_out.is_empty() && gate_status != AuditStatus::Failed {
        on_progress(0.90, "Writing converted resource pack...");
        let out_pb = PathBuf::from(clean_out);
        output
            .write_to_path(&out_pb)
            .with_context(|| format!("failed writing converted pack '{}'", out_pb.display()))?;
        Some(out_pb.display().to_string())
    } else {
        None
    };

    on_progress(1.0, "Conversion finished.");

    let dto = ConversionResultDto {
        source_edition,
        target_edition,
        output_written_to: written_to,
        converted_count: report.converted_count,
        passthrough_count: report.passthrough_count,
        skipped_count: report.skipped_count,
        lossy_count: report.lossy_count,
        warnings: report.warnings,
        gate_status,
        gate_summary,
        elapsed_ms: start.elapsed().as_millis(),
    };

    Ok((output, dto))
}

/// Core pure audit function (testable directly without Tauri runtime).
pub fn run_audit_core(container: &PackContainer) -> Result<AuditResultDto> {
    let start = Instant::now();
    let mappings = MappingTable::default_embedded()?;
    let edition = detect_pack_edition(container)?;
    let mut problems = check_uv_atlas_guard(container);

    match edition {
        Edition::Bedrock => {
            let input_tex_count = container
                .files()
                .keys()
                .filter(|p| {
                    let l = p.to_ascii_lowercase();
                    l.ends_with(".png") && l != "pack_icon.png" && l != "bug_pack_icon.png"
                })
                .count();
            let audit = audit_bedrock_container(container, input_tex_count, &mappings);
            append_audit_diagnostics(&audit, &mut problems);

            let (status, summary) = evaluate_semantic_gate(
                Edition::Bedrock,
                container,
                Some(&audit),
                &problems,
                0,
                0,
            );
            Ok(AuditResultDto {
                edition,
                status,
                summary,
                problems,
                elapsed_ms: start.elapsed().as_millis(),
            })
        }
        Edition::Java => {
            let (ir, report) = parse_java_pack(container, &mappings)?;
            problems.extend(report.warnings);

            if ir.textures.is_empty() && ir.models.is_empty() && ir.sounds.files.is_empty() {
                problems.push(ConversionWarning {
                    source_path: String::from("pack.mcmeta"),
                    stage: PipelineStage::Audit,
                    severity: WarningSeverity::Error,
                    reason: String::from("Java pack contains 0 textures, 0 models, and 0 sounds"),
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
                &problems,
                report.skipped_count,
                report.lossy_count,
            );
            Ok(AuditResultDto {
                edition,
                status,
                summary,
                problems,
                elapsed_ms: start.elapsed().as_millis(),
            })
        }
    }
}

/// Core packaging function (testable directly without Tauri runtime).
pub fn package_pack_core(
    container: &PackContainer,
    companion_bp_dir: Option<&Path>,
    cfg: &PackageExportConfig,
    edition: Edition,
    pack_name: &str,
    version: [u32; 3],
) -> Result<PackageExportSummary> {
    let mut cleaned = PackContainer::new();
    let mut stripped_junk_count = 0usize;
    for (path, bytes) in container.files() {
        if cfg.strip_junk_files && is_packaging_junk(path) {
            stripped_junk_count += 1;
            continue;
        }
        cleaned.insert(path, bytes.clone())?;
    }

    let mut audit_status = None;
    let mut audit_summary = String::from("Skipped");
    if cfg.run_semantic_audit {
        let audit = run_audit_core(&cleaned)?;
        audit_status = Some(audit.status);
        audit_summary = audit.summary.clone();
        if audit.status == AuditStatus::Failed && !cfg.allow_export_with_errors {
            bail!(
                "Packaging blocked by Semantic Audit Gate (FAILED): {}",
                audit.summary
            );
        }
    }

    let out_dir = PathBuf::from(cfg.output_dir.trim());
    if cfg.output_dir.trim().is_empty() {
        bail!("Output directory must not be empty");
    }
    fs::create_dir_all(&out_dir)?;

    let ext = match cfg.format.trim().to_ascii_lowercase().as_str() {
        "mcpack" => "mcpack",
        "mcaddon" => "mcaddon",
        _ => "zip",
    };

    let safe_name = sanitize_minecraft_slug(pack_name);
    let safe_name = if safe_name.is_empty() {
        String::from("resource_pack")
    } else {
        safe_name
    };
    let ver_str = format!("{}.{}.{}", version[0], version[1], version[2]);
    let ed_str = match edition {
        Edition::Java => "java",
        Edition::Bedrock => "bedrock",
    };
    let stem = cfg
        .filename_template
        .trim()
        .replace("{pack_name}", &safe_name)
        .replace("{version}", &ver_str)
        .replace("{edition}", ed_str);
    let stem = stem
        .trim_end_matches(".zip")
        .trim_end_matches(".mcpack")
        .trim_end_matches(".mcaddon");
    let filename = format!("{}.{ext}", if stem.is_empty() { "pack_export" } else { stem });
    let output_path = out_dir.join(filename);

    let level = match cfg.compression.trim().to_ascii_lowercase().as_str() {
        "fast" => 1,
        "maximum" => 9,
        _ => 6,
    };
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(level));

    let file = File::create(&output_path)?;
    let mut zip = ZipWriter::new(file);
    let mut asset_count = 0usize;

    if ext == "mcaddon" {
        for (path, bytes) in cleaned.files() {
            let clean = sanitize_archive_path(path)?;
            zip.start_file(format!("RP/{clean}"), options)?;
            zip.write_all(bytes)?;
            asset_count += 1;
        }
        if let Some(bp_dir) = companion_bp_dir
            && bp_dir.is_dir()
        {
            let bp_container = PackContainer::read_from_directory(bp_dir, IoLimits::default())?;
            for (path, bytes) in bp_container.files() {
                if cfg.strip_junk_files && is_packaging_junk(path) {
                    stripped_junk_count += 1;
                    continue;
                }
                let clean = sanitize_archive_path(path)?;
                zip.start_file(format!("BP/{clean}"), options)?;
                zip.write_all(bytes)?;
                asset_count += 1;
            }
        }
    } else {
        for (path, bytes) in cleaned.files() {
            let clean = sanitize_archive_path(path)?;
            zip.start_file(clean, options)?;
            zip.write_all(bytes)?;
            asset_count += 1;
        }
    }

    zip.finish()?;
    let size_bytes = fs::metadata(&output_path).map_or(0, |m| m.len());

    Ok(PackageExportSummary {
        output_path: output_path.display().to_string(),
        size_bytes,
        asset_count,
        stripped_junk_count,
        audit_status,
        audit_summary,
    })
}

fn is_packaging_junk(path: &str) -> bool {
    if is_os_junk(path) {
        return true;
    }
    let norm = path.replace('\\', "/");
    let lower = norm.to_ascii_lowercase();
    if lower == ".git"
        || lower.starts_with(".git/")
        || lower.contains("/.git/")
        || lower.starts_with(".vscode/")
        || lower.contains("/.vscode/")
        || lower.starts_with(".cache/")
    {
        return true;
    }
    let fname = lower.rsplit('/').next().unwrap_or(&lower);
    fname.ends_with(".bak")
        || fname.ends_with(".tmp")
        || fname.ends_with(".swp")
        || fname.ends_with('~')
}

/// Tauri Command: Run background Preflight Scan without writing output to disk.
#[tauri::command]
pub async fn run_preflight(
    state: tauri::State<'_, StudioState>,
    input: Option<String>,
    direction: String,
    options: ConverterOptions,
) -> Result<PreflightReportDto, String> {
    let container = load_source_container(&state, input).map_err(|e| format!("{e:#}"))?;
    tauri::async_runtime::spawn_blocking(move || run_preflight_core(&container, &direction, &options))
        .await
        .map_err(|e| format!("preflight worker panicked: {e}"))?
        .map_err(|e| format!("{e:#}"))
}

/// Tauri Command: Execute full background conversion + Semantic Audit Gate, emitting `converter://progress`.
#[tauri::command]
pub async fn execute_convert(
    window: Window,
    state: tauri::State<'_, StudioState>,
    input: Option<String>,
    output: String,
    direction: String,
    options: ConverterOptions,
) -> Result<ConversionResultDto, String> {
    let container = load_source_container(&state, input).map_err(|e| format!("{e:#}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        let (_out_container, dto) =
            execute_convert_core(&container, &output, &direction, &options, |fraction, msg| {
                let _ = window.emit(
                    "converter://progress",
                    ConvertProgressEvent {
                        fraction,
                        message: msg.to_string(),
                    },
                );
            })?;
        Ok::<_, anyhow::Error>(dto)
    })
    .await
    .map_err(|e| format!("convert worker panicked: {e}"))?
    .map_err(|e| format!("{e:#}"))
}

/// Tauri Command: Run standalone Semantic Validation Audit against active pack or `project_path`.
#[tauri::command]
pub async fn run_audit(
    state: tauri::State<'_, StudioState>,
    project_path: Option<String>,
) -> Result<AuditResultDto, String> {
    let container = load_source_container(&state, project_path).map_err(|e| format!("{e:#}"))?;
    tauri::async_runtime::spawn_blocking(move || run_audit_core(&container))
        .await
        .map_err(|e| format!("audit worker panicked: {e}"))?
        .map_err(|e| format!("{e:#}"))
}

/// Tauri Command: Package active or specified pack into `.zip`, `.mcpack`, or `.mcaddon`.
#[tauri::command]
pub async fn package_pack(
    state: tauri::State<'_, StudioState>,
    project_path: Option<String>,
    export_config: PackageExportConfig,
) -> Result<PackageExportSummary, String> {
    let (container, companion_bp_dir, edition, pack_name) = if let Some(p) =
        project_path.as_deref().map(str::trim).filter(|s| !s.is_empty())
    {
        let c = PackContainer::read_from_path(Path::new(p), IoLimits::default())
            .map_err(|e| format!("{e:#}"))?;
        let ed = detect_pack_edition(&c).map_err(|e| format!("{e:#}"))?;
        (c, None, ed, String::from("resource_pack"))
    } else {
        let guard = state
            .active_pack
            .lock()
            .map_err(|_| String::from("active pack mutex poisoned"))?;
        let pack = guard
            .as_ref()
            .ok_or_else(|| String::from("No resource pack currently open"))?;
        (
            pack.container.clone(),
            pack.companion_bp_dir.clone(),
            pack.edition,
            pack.pack_name.clone(),
        )
    };

    let version = extract_pack_version(&container, edition);
    tauri::async_runtime::spawn_blocking(move || {
        package_pack_core(
            &container,
            companion_bp_dir.as_deref(),
            &export_config,
            edition,
            &pack_name,
            version,
        )
    })
    .await
    .map_err(|e| format!("package worker panicked: {e}"))?
    .map_err(|e| format!("{e:#}"))
}

fn extract_pack_version(container: &PackContainer, edition: Edition) -> [u32; 3] {
    if edition == Edition::Bedrock
        && let Some(bytes) = container.get("manifest.json")
        && let Ok(m) = parse_minecraft_json::<BedrockManifest>(bytes)
    {
        return m.header.version;
    }
    [1, 0, 0]
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
