use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Pipeline stage where a diagnostic was emitted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStage {
    /// Archive / directory scanning and input ingestion.
    #[default]
    Scan,
    /// Model graph, blockstate, item definition, and texture path resolution.
    Resolve,
    /// Asset transformation (textures, animations, PBR, 3D geometry, locales, audio).
    Transform,
    /// Target edition serialization and archive emission.
    Emit,
    /// Post-conversion target engine semantic audit.
    Audit,
}

impl fmt::Display for PipelineStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scan => write!(f, "scan"),
            Self::Resolve => write!(f, "resolve"),
            Self::Transform => write!(f, "transform"),
            Self::Emit => write!(f, "emit"),
            Self::Audit => write!(f, "audit"),
        }
    }
}

/// Severity of a conversion diagnostic warning or error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningSeverity {
    /// Informational note about a fallback or default choice.
    Info,
    /// Non-fatal compatibility concern.
    Warning,
    /// Fatal semantic or schema error that causes conversion/audit failure.
    Error,
}

/// A structured, actionable diagnostic message produced during conversion or auditing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionWarning {
    /// Original file path or asset identifier in the source pack.
    #[serde(default)]
    pub source_path: String,
    /// Pipeline stage where this diagnostic occurred (`scan` | `resolve` | `transform` | `emit` | `audit`).
    #[serde(default)]
    pub stage: PipelineStage,
    /// Diagnostic severity.
    pub severity: WarningSeverity,
    /// Specific technical root cause.
    #[serde(default)]
    pub reason: String,
    /// Concrete remediation step for the user or developer.
    #[serde(default)]
    pub actionable_fix: String,
    /// Compatibility alias mirroring `source_path`.
    pub asset: String,
    /// Compatibility alias mirroring `reason`.
    pub message: String,
}

/// Record of an asset that could not be converted and was skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedAsset {
    /// Original file path or canonical ID of the skipped asset.
    #[serde(default)]
    pub source_path: String,
    /// Pipeline stage where the asset was skipped.
    #[serde(default)]
    pub stage: PipelineStage,
    /// Archive path or canonical ID of the skipped asset (compatibility alias).
    pub asset: String,
    /// Specific technical reason the asset was skipped.
    pub reason: String,
    /// Concrete remediation step for the user or developer.
    #[serde(default)]
    pub actionable_fix: String,
}

/// Record of an asset whose conversion required approximation or data loss.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LossyConversion {
    /// Original file path or canonical ID of the affected asset.
    #[serde(default)]
    pub source_path: String,
    /// Pipeline stage where loss occurred.
    #[serde(default = "default_transform_stage")]
    pub stage: PipelineStage,
    /// Canonical ID or path of the affected asset (compatibility alias).
    pub asset: String,
    /// Description of what information could not be preserved losslessly.
    pub detail: String,
    /// Concrete remediation step for the user or developer.
    #[serde(default)]
    pub actionable_fix: String,
}

const fn default_transform_stage() -> PipelineStage {
    PipelineStage::Transform
}

/// Final verdict of a post-conversion semantic audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditStatus {
    /// Output pack satisfies all target engine semantic rules with zero forbidden or broken entries.
    Passed,
    /// Output pack is partially usable, but contains unmapped/skipped/orphan assets or warnings.
    Partial,
    /// Output pack violates hard target engine semantic rules (e.g. 0 mapped textures, forbidden entries, broken references, invalid manifest).
    Failed,
}

impl fmt::Display for AuditStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Passed => write!(f, "PASSED"),
            Self::Partial => write!(f, "PARTIAL / WARNING"),
            Self::Failed => write!(f, "FAILED"),
        }
    }
}

/// A forbidden file detected in the output archive by the semantic auditor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForbiddenEntry {
    /// Archive path inside the emitted pack.
    pub path: String,
    /// Machine-readable acceptance rule ID (from `data/spec/bedrock_acceptance_rules.json`).
    pub rule_id: String,
    /// Specific technical root cause.
    pub reason: String,
    /// Actionable fix.
    pub actionable_fix: String,
}

/// A broken internal reference (e.g. `terrain_texture.json` or `item_texture.json` pointing to a missing `.png`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokenReference {
    /// Manifest/atlas/definition JSON file containing the broken pointer.
    pub source_file: String,
    /// Missing target archive path.
    pub missing_path: String,
}

/// Post-conversion target engine semantic audit summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticAuditReport {
    /// Overall semantic audit verdict (`Passed`, `Partial`, or `Failed`).
    pub status: AuditStatus,
    /// Total texture files discovered in the input pack.
    pub total_input_textures: usize,
    /// Total `.png` texture files emitted in the output pack (excluding `pack_icon.png`).
    pub total_output_textures: usize,
    /// Output textures verified as readable by the target engine (vanilla path or registered in valid atlas/texture_set/attachable).
    pub semantically_mapped_textures: usize,
    /// Output `.png` paths that will never be read by the Bedrock engine.
    pub orphan_textures: Vec<String>,
    /// Output entries violating hard Bedrock acceptance rules (`assets/*`, `desktop.ini`, `textures/mob_effect/*`, dead `models/entity/block/*.geo.json`, raw LabPBR, empty stubs).
    pub forbidden_entries: Vec<ForbiddenEntry>,
    /// Atlas / flipbook / texture_set / attachable references pointing to missing archive files.
    pub broken_references: Vec<BrokenReference>,
    /// Whether `manifest.json` exists and passes schema + UUID + version validation.
    pub manifest_valid: bool,
    /// Concise human-readable summary of the audit verdict.
    pub summary: String,
    /// Breakdown of diagnostic / skip / mapping issue counts by category.
    #[serde(default)]
    pub category_breakdown: BTreeMap<String, usize>,
}

/// Aggregated report of asset conversions, pass-throughs, skipped items, warnings, and semantic audit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionReport {
    /// Number of assets actively transformed to the target edition schema.
    pub converted_count: usize,
    /// Number of assets copied without pixel or audio transcoding.
    pub passthrough_count: usize,
    /// Number of assets omitted due to incompatibility.
    pub skipped_count: usize,
    /// Number of assets converted with partial fidelity loss.
    pub lossy_count: usize,
    /// Structured warnings and errors generated during inspection, conversion, or audit.
    pub warnings: Vec<ConversionWarning>,
    /// Detailed entries for each skipped asset.
    pub skipped_assets: Vec<SkippedAsset>,
    /// Detailed entries for each lossy conversion.
    pub lossy_conversions: Vec<LossyConversion>,
    /// Post-conversion semantic audit result (populated for Bedrock emissions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_audit: Option<SemanticAuditReport>,
}

impl ConversionReport {
    /// Creates a new empty report.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Increments the count of actively converted assets.
    pub fn record_converted(&mut self) {
        self.converted_count = self.converted_count.saturating_add(1);
    }

    /// Increments the count of pass-through assets.
    pub fn record_passthrough(&mut self) {
        self.passthrough_count = self.passthrough_count.saturating_add(1);
    }

    /// Appends a fully structured diagnostic warning or error with all 4 mandatory fields.
    pub fn record_diagnostic(
        &mut self,
        source_path: impl Into<String>,
        stage: PipelineStage,
        severity: WarningSeverity,
        reason: impl Into<String>,
        actionable_fix: impl Into<String>,
    ) {
        let path_str = source_path.into();
        let reason_str = reason.into();
        self.warnings.push(ConversionWarning {
            asset: path_str.clone(),
            source_path: path_str,
            stage,
            severity,
            message: reason_str.clone(),
            reason: reason_str,
            actionable_fix: actionable_fix.into(),
        });
    }

    /// Appends a diagnostic warning with inferred stage and default remediation guidance.
    pub fn record_warning(
        &mut self,
        asset: impl Into<String>,
        severity: WarningSeverity,
        message: impl Into<String>,
    ) {
        let asset_str = asset.into();
        let msg_str = message.into();
        let fix = default_actionable_fix_for_message(&msg_str);
        self.record_diagnostic(asset_str, PipelineStage::Transform, severity, msg_str, fix);
    }

    /// Records a skipped asset with full structured metadata and increments `skipped_count`.
    pub fn record_skipped_detailed(
        &mut self,
        source_path: impl Into<String>,
        stage: PipelineStage,
        reason: impl Into<String>,
        actionable_fix: impl Into<String>,
    ) {
        let path_str = source_path.into();
        self.skipped_count = self.skipped_count.saturating_add(1);
        self.skipped_assets.push(SkippedAsset {
            asset: path_str.clone(),
            source_path: path_str,
            stage,
            reason: reason.into(),
            actionable_fix: actionable_fix.into(),
        });
    }

    /// Records a skipped asset and increments `skipped_count`.
    pub fn record_skipped(&mut self, asset: impl Into<String>, reason: impl Into<String>) {
        let asset_str = asset.into();
        let reason_str = reason.into();
        let fix = default_actionable_fix_for_message(&reason_str);
        self.record_skipped_detailed(asset_str, PipelineStage::Scan, reason_str, fix);
    }

    /// Records a lossy conversion with full structured metadata and increments `lossy_count`.
    pub fn record_lossy_detailed(
        &mut self,
        source_path: impl Into<String>,
        stage: PipelineStage,
        detail: impl Into<String>,
        actionable_fix: impl Into<String>,
    ) {
        let path_str = source_path.into();
        self.lossy_count = self.lossy_count.saturating_add(1);
        self.lossy_conversions.push(LossyConversion {
            asset: path_str.clone(),
            source_path: path_str,
            stage,
            detail: detail.into(),
            actionable_fix: actionable_fix.into(),
        });
    }

    /// Records a lossy conversion and increments `lossy_count`.
    pub fn record_lossy(&mut self, asset: impl Into<String>, detail: impl Into<String>) {
        let asset_str = asset.into();
        let detail_str = detail.into();
        let fix = default_actionable_fix_for_message(&detail_str);
        self.record_lossy_detailed(asset_str, PipelineStage::Transform, detail_str, fix);
    }

    /// Merges another report into `self`.
    pub fn merge(&mut self, other: Self) {
        self.converted_count = self.converted_count.saturating_add(other.converted_count);
        self.passthrough_count = self
            .passthrough_count
            .saturating_add(other.passthrough_count);
        self.skipped_count = self.skipped_count.saturating_add(other.skipped_count);
        self.lossy_count = self.lossy_count.saturating_add(other.lossy_count);
        self.warnings.extend(other.warnings);
        self.skipped_assets.extend(other.skipped_assets);
        self.lossy_conversions.extend(other.lossy_conversions);
        if other.semantic_audit.is_some() {
            self.semantic_audit = other.semantic_audit;
        }
    }

    /// Returns true if the semantic audit explicitly failed or any `Error`-severity diagnostic exists.
    #[must_use]
    pub fn has_semantic_failure(&self) -> bool {
        self.semantic_audit
            .as_ref()
            .is_some_and(|a| a.status == AuditStatus::Failed)
            || self
                .warnings
                .iter()
                .any(|w| w.severity == WarningSeverity::Error)
    }

    /// Returns the effective conversion status taking the semantic audit into account.
    #[must_use]
    pub fn effective_status(&self) -> AuditStatus {
        if let Some(audit) = &self.semantic_audit {
            return audit.status;
        }
        if self
            .warnings
            .iter()
            .any(|w| w.severity == WarningSeverity::Error)
        {
            return AuditStatus::Failed;
        }
        if self.skipped_count > 0
            || self.lossy_count > 0
            || self
                .warnings
                .iter()
                .any(|w| w.severity == WarningSeverity::Warning)
        {
            return AuditStatus::Partial;
        }
        AuditStatus::Passed
    }

    /// Returns true if any skipped assets, lossy conversions, or warnings with `Warning`/`Error` severity exist.
    #[must_use]
    pub fn has_strict_issues(&self) -> bool {
        self.has_semantic_failure()
            || self.skipped_count > 0
            || self.lossy_count > 0
            || self
                .warnings
                .iter()
                .any(|w| matches!(w.severity, WarningSeverity::Warning | WarningSeverity::Error))
    }
}

fn default_actionable_fix_for_message(msg: &str) -> String {
    let lower = msg.to_ascii_lowercase();
    if lower.contains("shader") || lower.contains("optifine") || lower.contains("mcpatcher") {
        String::from(
            "Remove Java-only shaders/OptiFine files or recreate equivalent visual behavior using Bedrock Vibrant Visuals / RenderDragon PBR resources.",
        )
    } else if lower.contains("override") {
        String::from(
            "Bedrock Resource Packs do not support Java item predicate overrides; use Behavior Pack custom items or attachables instead.",
        )
    } else if lower.contains("animation") || lower.contains("frame") {
        String::from(
            "Ensure the vertical animation strip height is an exact multiple of the frame width/height in its .png.mcmeta or flipbook_textures.json.",
        )
    } else if lower.contains("sound") || lower.contains("ogg") {
        String::from(
            "Bundle the referenced Ogg Vorbis (.ogg) audio file inside sounds/ or remove the dangling sound_definitions entry.",
        )
    } else if lower.contains("rotation") || lower.contains("angle") {
        String::from(
            "Target Java 1.21.11+ / 26.x for arbitrary multi-axis rotations, or pass --rotation-mode snap for legacy Java packs.",
        )
    } else {
        String::from("Inspect the referenced source asset and update its mapping or target format.")
    }
}
