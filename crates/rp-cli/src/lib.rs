use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use rp_bedrock::{emit_bedrock_pack, parse_bedrock_pack, parse_bedrock_pack_with_options};
use rp_core::{
    BedrockGeoVersion, ConversionReport, ConversionWarning, Edition, MappingTable, RotationSnapMode,
};
use rp_io::{IoLimits, PackContainer};
use rp_java::{emit_java_pack, parse_java_pack};

/// Version-specific conversion controls for Java and Bedrock targets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionSpecificOptions {
    /// Java `supported_formats` / `min_format..max_format` range `[min, max]`.
    pub supported_formats: Option<[u32; 2]>,
    /// Handling strategy for Bedrock angles outside Java 22.5° steps (`[-45..45]`).
    pub rotation_mode: RotationSnapMode,
    /// Target Bedrock geometry schema version (`1.10.0`, `1.12.0`, `1.16.0`, `1.21.0`).
    pub bedrock_geo_version: Option<BedrockGeoVersion>,
    /// Target Bedrock `manifest.json` `format_version` (`1`, `2`, or `3`).
    pub bedrock_manifest_version: Option<u32>,
    /// Explicit override for Java `1.21.4+` `assets/minecraft/items/*.json` emission.
    pub emit_java_items_definitions: Option<bool>,
}

/// Options controlling end-to-end pack conversion.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// Input `.zip`, `.mcpack`, or directory path.
    pub input: PathBuf,
    /// Output `.zip`, `.mcpack`, or directory path.
    pub output: PathBuf,
    /// Optional explicit target edition; if `None`, flips detected source edition.
    pub target: Option<Edition>,
    /// Optional target Minecraft version or Java `pack_format` string.
    pub target_version: Option<String>,
    /// Optional custom mapping JSON override file.
    pub mapping_file: Option<PathBuf>,
    /// Abort with an error if any skipped, lossy, or warning issue occurs.
    pub strict: bool,
    /// Optional path to write JSON conversion report.
    pub report_json: Option<PathBuf>,
    /// I/O security and size limits.
    pub limits: IoLimits,
    /// Target-version-specific emitter/conversion overrides.
    pub version_options: VersionSpecificOptions,
}

/// Summary produced by the `inspect` subcommand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectSummary {
    /// Detected pack edition (`Java` or `Bedrock`).
    pub edition: Edition,
    /// Pack display name.
    pub name: String,
    /// Pack description.
    pub description: String,
    /// Java `pack_format` if Java or mapped.
    pub java_pack_format: Option<u32>,
    /// Java `supported_formats` / `min_format..max_format` range if declared.
    pub java_supported_formats: Option<[u32; 2]>,
    /// Bedrock `min_engine_version` if Bedrock or mapped.
    pub bedrock_min_engine_version: Option<[u32; 3]>,
    /// Total files inside the input container.
    pub total_files: usize,
    /// Number of parsed textures.
    pub texture_count: usize,
    /// Number of animated textures.
    pub animated_texture_count: usize,
    /// Number of parsed 3D models.
    pub model_count: usize,
    /// Number of sound events.
    pub sound_event_count: usize,
    /// Number of `.ogg` sound files.
    pub sound_file_count: usize,
    /// Number of language tables.
    pub locale_count: usize,
    /// Potential compatibility warnings and skipped-feature notices.
    pub warnings: Vec<ConversionWarning>,
    /// Count of assets that would be skipped when converting to the opposite edition.
    pub unsupported_asset_count: usize,
}

/// Loads the embedded default mapping table and applies an optional JSON override file.
pub fn load_mappings(mapping_file: Option<&Path>) -> Result<MappingTable> {
    let mut mappings = MappingTable::default_embedded()
        .context("failed to load embedded default mapping registry")?;
    if let Some(path) = mapping_file {
        let override_table = MappingTable::from_file(path)
            .with_context(|| format!("failed to load mapping override '{}'", path.display()))?;
        mappings.merge_override(override_table);
    }
    Ok(mappings)
}

/// Detects whether `container` is a Java or Bedrock resource pack.
pub fn detect_edition(container: &PackContainer) -> Result<Edition> {
    if container.contains("pack.mcmeta") {
        Ok(Edition::Java)
    } else if container.contains("manifest.json") {
        Ok(Edition::Bedrock)
    } else {
        bail!(
            "Unable to detect resource pack edition: missing both 'pack.mcmeta' and 'manifest.json' at archive root"
        )
    }
}

/// Inspects an in-memory `PackContainer` without writing output files.
pub fn inspect_container(
    container: &PackContainer,
    mappings: &MappingTable,
) -> Result<InspectSummary> {
    let edition = detect_edition(container)?;
    let (ir, report) = match edition {
        Edition::Java => {
            parse_java_pack(container, mappings).context("failed to parse Java resource pack")?
        }
        Edition::Bedrock => parse_bedrock_pack(container, mappings)
            .context("failed to parse Bedrock resource pack")?,
    };

    let animated_texture_count = ir
        .textures
        .values()
        .filter(|tex| tex.animation.is_some())
        .count();

    Ok(InspectSummary {
        edition,
        name: ir.metadata.name,
        description: ir.metadata.description,
        java_pack_format: ir.metadata.java_pack_format,
        java_supported_formats: ir.metadata.java_supported_formats,
        bedrock_min_engine_version: ir.metadata.bedrock_min_engine_version,
        total_files: container.len(),
        texture_count: ir.textures.len(),
        animated_texture_count,
        model_count: ir.models.len(),
        sound_event_count: ir.sounds.events.len(),
        sound_file_count: ir.sounds.files.len(),
        locale_count: ir.locales.len(),
        warnings: report.warnings,
        unsupported_asset_count: report.skipped_count,
    })
}

/// Inspects a resource pack at `input` (`.zip`, `.mcpack`, or directory).
pub fn inspect_pack(
    input: &Path,
    mapping_file: Option<&Path>,
    limits: IoLimits,
) -> Result<InspectSummary> {
    let mappings = load_mappings(mapping_file)?;
    let container = PackContainer::read_from_path(input, limits)
        .with_context(|| format!("failed to read resource pack '{}'", input.display()))?;
    inspect_container(&container, &mappings)
}

/// Converts an in-memory `PackContainer` between Java and Bedrock editions using default version options.
pub fn convert_container(
    input: &PackContainer,
    target_override: Option<Edition>,
    target_version: Option<&str>,
    mappings: &MappingTable,
    strict: bool,
) -> Result<(PackContainer, Edition, Edition, ConversionReport)> {
    convert_container_with_options(
        input,
        target_override,
        target_version,
        mappings,
        strict,
        &VersionSpecificOptions::default(),
    )
}

/// Converts an in-memory `PackContainer` with explicit `VersionSpecificOptions`.
pub fn convert_container_with_options(
    input: &PackContainer,
    target_override: Option<Edition>,
    target_version: Option<&str>,
    mappings: &MappingTable,
    strict: bool,
    version_options: &VersionSpecificOptions,
) -> Result<(PackContainer, Edition, Edition, ConversionReport)> {
    let source = detect_edition(input)?;
    let target = target_override.unwrap_or(match source {
        Edition::Java => Edition::Bedrock,
        Edition::Bedrock => Edition::Java,
    });

    if let Some(mver) = version_options.bedrock_manifest_version
        && !(1..=3).contains(&mver)
    {
        bail!("Invalid --bedrock-manifest-version '{mver}': expected 1, 2, or 3");
    }

    let resolved_target_ver = match target_version {
        Some(ver) => Some(
            mappings
                .resolve_target_version(target, ver)
                .with_context(|| format!("failed to resolve target version '{ver}'"))?,
        ),
        None => None,
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
                version_options.rotation_mode,
            )
            .context("failed to parse source Bedrock pack")?
        }
    };

    if let Some((java_fmt, bedrock_ver)) = resolved_target_ver {
        if let Some(fmt) = java_fmt {
            ir.metadata.java_pack_format = Some(fmt);
        }
        if let Some(bver) = bedrock_ver {
            ir.metadata.bedrock_min_engine_version = Some(bver);
            if version_options.bedrock_geo_version.is_none() {
                ir.metadata.bedrock_geo_version = Some(BedrockGeoVersion::for_engine_version(bver));
            }
        }
    }

    if let Some(range) = version_options.supported_formats {
        ir.metadata.java_supported_formats = Some(range);
    }
    ir.metadata.rotation_mode = version_options.rotation_mode;
    if let Some(geo_ver) = version_options.bedrock_geo_version {
        ir.metadata.bedrock_geo_version = Some(geo_ver);
    }
    if let Some(manifest_ver) = version_options.bedrock_manifest_version {
        ir.metadata.bedrock_manifest_version = Some(manifest_ver);
    }
    if let Some(items_defs) = version_options.emit_java_items_definitions {
        ir.metadata.emit_java_items_definitions = Some(items_defs);
    }

    let (output, emit_report) = match target {
        Edition::Bedrock => {
            emit_bedrock_pack(&ir, mappings).context("failed to emit Bedrock pack")?
        }
        Edition::Java => emit_java_pack(&ir, mappings).context("failed to emit Java pack")?,
    };
    report.merge(emit_report);

    if strict && report.has_strict_issues() {
        bail!(
            "Strict mode failed: {} skipped, {} lossy, {} warning(s)",
            report.skipped_count,
            report.lossy_count,
            report.warnings.len()
        );
    }

    Ok((output, source, target, report))
}

/// Executes full disk-to-disk conversion according to `options`.
pub fn convert_pack(options: &ConvertOptions) -> Result<(Edition, Edition, ConversionReport)> {
    let mappings = load_mappings(options.mapping_file.as_deref())?;
    let input_container = PackContainer::read_from_path(&options.input, options.limits)
        .with_context(|| format!("failed to read input pack '{}'", options.input.display()))?;

    let (output_container, source, target, report) = convert_container_with_options(
        &input_container,
        options.target,
        options.target_version.as_deref(),
        &mappings,
        options.strict,
        &options.version_options,
    )?;

    output_container
        .write_to_path(&options.output)
        .with_context(|| format!("failed to write output pack '{}'", options.output.display()))?;

    if let Some(report_path) = &options.report_json {
        if let Some(parent) = report_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&report)?;
        fs::write(report_path, json).with_context(|| {
            format!(
                "failed to write conversion report JSON '{}'",
                report_path.display()
            )
        })?;
    }

    Ok((source, target, report))
}
