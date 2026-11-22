use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};

use rp_cli::{ConvertOptions, VersionSpecificOptions, convert_pack, inspect_pack, load_mappings};
use rp_core::{BedrockGeoVersion, ConversionReport, Edition, RotationSnapMode};
use rp_io::IoLimits;

#[derive(Debug, Parser)]
#[command(
    name = "rp-cli",
    version,
    about = "Bidirectional Minecraft Resource Pack Converter (Java Edition <-> Bedrock Edition)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Convert a Java or Bedrock resource pack (.zip/.mcpack/directory) to the target edition.
    Convert {
        /// Input resource pack path (.zip, .mcpack, or directory).
        input: PathBuf,
        /// Output resource pack path (.zip, .mcpack, or directory).
        #[arg(short = 'o', long = "output")]
        output: PathBuf,
        /// Target edition (defaults to opposite of detected source edition).
        #[arg(long = "to", value_enum)]
        to: Option<TargetEdition>,
        /// Target Minecraft version or Java pack_format (e.g. "1.8.9", "1.20.1", "1.21.11", "26.1", "26.2", "26.3", "97").
        #[arg(long = "target-version")]
        target_version: Option<String>,
        /// Multi-version Java compatibility range `MIN,MAX` (e.g. "15,88").
        #[arg(long = "supported-formats", value_parser = parse_format_range)]
        supported_formats: Option<[u32; 2]>,
        /// Strategy for handling Bedrock rotation angles outside Java 22.5-degree increments.
        #[arg(long = "rotation-mode", value_enum, default_value = "preserve")]
        rotation_mode: CliRotationMode,
        /// Override target Bedrock geometry schema version.
        #[arg(long = "bedrock-geo-version", value_enum)]
        bedrock_geo_version: Option<CliBedrockGeoVersion>,
        /// Override Bedrock manifest.json format_version (1, 2, or 3 SemVer).
        #[arg(long = "bedrock-manifest-version", value_parser = clap::value_parser!(u32).range(1..=3))]
        bedrock_manifest_version: Option<u32>,
        /// Force emitting Java 1.21.4+ client item definitions (`assets/minecraft/items/*.json`).
        #[arg(long = "items-model-defs", conflicts_with = "no_items_model_defs")]
        items_model_defs: bool,
        /// Disable emitting Java 1.21.4+ client item definitions (`assets/minecraft/items/*.json`).
        #[arg(long = "no-items-model-defs", conflicts_with = "items_model_defs")]
        no_items_model_defs: bool,
        /// Optional external JSON mapping table override file.
        #[arg(long = "mapping")]
        mapping: Option<PathBuf>,
        /// Fail with non-zero exit status if any skipped, lossy, or warning issue occurs.
        #[arg(long = "strict")]
        strict: bool,
        /// Optional path to write structured conversion report JSON.
        #[arg(long = "report-json")]
        report_json: Option<PathBuf>,
    },
    /// Inspect a resource pack's edition, format/engine version, asset counts, and warnings.
    Inspect {
        /// Input resource pack path (.zip, .mcpack, or directory).
        input: PathBuf,
        /// Optional external JSON mapping table override file.
        #[arg(long = "mapping")]
        mapping: Option<PathBuf>,
    },
    /// List supported Minecraft Java & Bedrock target versions and format features.
    Versions {
        /// Optional external JSON mapping table override file.
        #[arg(long = "mapping")]
        mapping: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TargetEdition {
    Java,
    Bedrock,
}

impl From<TargetEdition> for Edition {
    fn from(value: TargetEdition) -> Self {
        match value {
            TargetEdition::Java => Self::Java,
            TargetEdition::Bedrock => Self::Bedrock,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliRotationMode {
    Preserve,
    Snap,
    Clamp,
}

impl From<CliRotationMode> for RotationSnapMode {
    fn from(value: CliRotationMode) -> Self {
        match value {
            CliRotationMode::Preserve => Self::Preserve,
            CliRotationMode::Snap => Self::Snap22_5,
            CliRotationMode::Clamp => Self::Clamp45,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliBedrockGeoVersion {
    #[value(name = "1.10.0")]
    V1_10_0,
    #[value(name = "1.12.0")]
    V1_12_0,
    #[value(name = "1.16.0")]
    V1_16_0,
    #[value(name = "1.21.0")]
    V1_21_0,
}

impl From<CliBedrockGeoVersion> for BedrockGeoVersion {
    fn from(value: CliBedrockGeoVersion) -> Self {
        match value {
            CliBedrockGeoVersion::V1_10_0 => Self::V1_10_0,
            CliBedrockGeoVersion::V1_12_0 => Self::V1_12_0,
            CliBedrockGeoVersion::V1_16_0 => Self::V1_16_0,
            CliBedrockGeoVersion::V1_21_0 => Self::V1_21_0,
        }
    }
}

fn parse_format_range(raw: &str) -> Result<[u32; 2], String> {
    let normalized = raw.trim().replace("..", ",").replace('-', ",");
    let parts: Vec<&str> = normalized.split(',').collect();
    if parts.len() != 2 {
        return Err(String::from(
            "expected format range as MIN,MAX (e.g. 15,88)",
        ));
    }
    let min_v = parts[0]
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("invalid integer '{}'", parts[0]))?;
    let max_v = parts[1]
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("invalid integer '{}'", parts[1]))?;
    if min_v == 0 || max_v == 0 {
        return Err(String::from("pack format numbers must be >= 1"));
    }
    Ok([min_v.min(max_v), min_v.max(max_v)])
}

fn make_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    let style = ProgressStyle::with_template("{spinner:.cyan} {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_spinner());
    pb.set_style(style);
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

fn print_conversion_summary(source: Edition, target: Edition, report: &ConversionReport) {
    println!();
    println!("Conversion Summary ({source} -> {target})");
    println!("+-------------------------+---------+");
    println!("| Metric                  | Count   |");
    println!("+-------------------------+---------+");
    println!(
        "| Converted assets        | {:>7} |",
        report.converted_count
    );
    println!(
        "| Pass-through assets     | {:>7} |",
        report.passthrough_count
    );
    println!("| Lossy conversions       | {:>7} |", report.lossy_count);
    println!("| Skipped / unsupported   | {:>7} |", report.skipped_count);
    println!("| Diagnostic warnings     | {:>7} |", report.warnings.len());
    println!("+-------------------------+---------+");

    if !report.lossy_conversions.is_empty() {
        println!("\nLossy Conversions:");
        for item in &report.lossy_conversions {
            println!("  - [{}] {}", item.asset, item.detail);
        }
    }
    if !report.skipped_assets.is_empty() {
        println!("\nSkipped Assets:");
        for item in &report.skipped_assets {
            println!("  - [{}] {}", item.asset, item.reason);
        }
    }
    if !report.warnings.is_empty() {
        println!("\nWarnings:");
        for w in &report.warnings {
            println!("  - [{:?}] {}: {}", w.severity, w.asset, w.message);
        }
    }
}

fn describe_java_features(pack_format: u32) -> &'static str {
    if pack_format <= 3 {
        "Pre-Flattening (blocks/, items/, .lang)"
    } else if pack_format < 18 {
        "Flattened (block/, item/, .json)"
    } else if pack_format < 46 {
        "supported_formats"
    } else if pack_format < 63 {
        "items/*.json (1.21.4+)"
    } else if pack_format < 69 {
        "items/*.json + arb angle in [-45,45]"
    } else if pack_format < 75 {
        "min/max_format + items/*.json + [-45,45]"
    } else if pack_format < 97 {
        "min/max_format + multi-axis Euler {x,y,z}"
    } else {
        "97.1 + multi-axis Euler + shade_direction_override"
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Convert {
            input,
            output,
            to,
            target_version,
            supported_formats,
            rotation_mode,
            bedrock_geo_version,
            bedrock_manifest_version,
            items_model_defs,
            no_items_model_defs,
            mapping,
            strict,
            report_json,
        } => {
            if let Some(mver) = bedrock_manifest_version
                && !(1..=3).contains(&mver)
            {
                bail!("--bedrock-manifest-version must be 1, 2, or 3");
            }
            let emit_items_override = if items_model_defs {
                Some(true)
            } else if no_items_model_defs {
                Some(false)
            } else {
                None
            };

            let spinner = make_spinner("Converting resource pack...");
            let options = ConvertOptions {
                input,
                output,
                target: to.map(Into::into),
                target_version,
                mapping_file: mapping,
                strict,
                report_json,
                limits: IoLimits::default(),
                version_options: VersionSpecificOptions {
                    supported_formats,
                    rotation_mode: rotation_mode.into(),
                    bedrock_geo_version: bedrock_geo_version.map(Into::into),
                    bedrock_manifest_version,
                    emit_java_items_definitions: emit_items_override,
                },
            };
            let res = convert_pack(&options);
            spinner.finish_and_clear();
            let (source, target, report) = res?;
            print_conversion_summary(source, target, &report);
        }
        Commands::Inspect { input, mapping } => {
            let spinner = make_spinner("Inspecting resource pack...");
            let res = inspect_pack(&input, mapping.as_deref(), IoLimits::default());
            spinner.finish_and_clear();
            let summary = res?;

            println!();
            println!("Resource Pack Inspection");
            println!("+-------------------------+----------------------------------+");
            println!(
                "| Edition                 | {:<32} |",
                summary.edition.to_string()
            );
            println!("| Name                    | {:<32} |", summary.name);
            if let Some(fmt) = summary.java_pack_format {
                println!("| Java pack_format        | {:<32} |", fmt);
            }
            if let Some([lo, hi]) = summary.java_supported_formats {
                let range_str = format!("{lo}..={hi}");
                println!("| Java supported_formats  | {:<32} |", range_str);
            }
            if let Some(ver) = summary.bedrock_min_engine_version {
                let ver_str = format!("{}.{}.{}", ver[0], ver[1], ver[2]);
                println!("| Bedrock min_engine      | {:<32} |", ver_str);
            }
            println!("| Total files             | {:<32} |", summary.total_files);
            println!(
                "| Textures                | {:<32} |",
                summary.texture_count
            );
            println!(
                "| Animated textures       | {:<32} |",
                summary.animated_texture_count
            );
            println!("| 3D Models               | {:<32} |", summary.model_count);
            println!(
                "| Sound events            | {:<32} |",
                summary.sound_event_count
            );
            println!(
                "| Audio (.ogg) files      | {:<32} |",
                summary.sound_file_count
            );
            println!("| Locale tables           | {:<32} |", summary.locale_count);
            println!(
                "| Unsupported assets      | {:<32} |",
                summary.unsupported_asset_count
            );
            println!("+-------------------------+----------------------------------+");

            if !summary.warnings.is_empty() {
                println!("\nCompatibility Warnings:");
                for w in &summary.warnings {
                    println!("  - [{:?}] {}: {}", w.severity, w.asset, w.message);
                }
            }
        }
        Commands::Versions { mapping } => {
            let table = load_mappings(mapping.as_deref())?;
            println!();
            println!("Supported Minecraft Versions & Target Format Features");
            println!(
                "+------------+----------+--------------+----------+------------------------------------------------+"
            );
            println!(
                "| MC Version | Java Fmt | Bedrock Eng  | Geo Ver  | Java Pack Features                             |"
            );
            println!(
                "+------------+----------+--------------+----------+------------------------------------------------+"
            );
            for entry in table.pack_format_versions() {
                let bver = format!(
                    "{}.{}.{}",
                    entry.bedrock_min_engine_version[0],
                    entry.bedrock_min_engine_version[1],
                    entry.bedrock_min_engine_version[2]
                );
                let geo_ver =
                    BedrockGeoVersion::for_engine_version(entry.bedrock_min_engine_version)
                        .as_str();
                let feats = describe_java_features(entry.java_pack_format);
                println!(
                    "| {:<10} | {:>8} | {:<12} | {:<8} | {:<46} |",
                    entry.minecraft_version, entry.java_pack_format, bver, geo_ver, feats
                );
            }
            println!(
                "+------------+----------+--------------+----------+------------------------------------------------+"
            );
        }
    }

    Ok(())
}
