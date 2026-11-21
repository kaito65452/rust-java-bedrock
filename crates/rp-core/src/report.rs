use serde::{Deserialize, Serialize};

/// Severity of a conversion diagnostic warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningSeverity {
    /// Informational note about a fallback or default choice.
    Info,
    /// Non-fatal compatibility concern.
    Warning,
}

/// A structured diagnostic message produced during parsing or emission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionWarning {
    /// Asset identifier or archive path associated with the warning.
    pub asset: String,
    /// Diagnostic severity.
    pub severity: WarningSeverity,
    /// Human-readable description of the issue.
    pub message: String,
}

/// Record of an asset that could not be converted and was skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedAsset {
    /// Archive path or canonical ID of the skipped asset.
    pub asset: String,
    /// Reason the asset was skipped.
    pub reason: String,
}

/// Record of an asset whose conversion required approximation or data loss.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LossyConversion {
    /// Canonical ID or path of the affected asset.
    pub asset: String,
    /// Description of what information could not be preserved losslessly.
    pub detail: String,
}

/// Aggregated report of asset conversions, pass-throughs, skipped items, and warnings.
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
    /// Structured warnings generated during inspection or conversion.
    pub warnings: Vec<ConversionWarning>,
    /// Detailed entries for each skipped asset.
    pub skipped_assets: Vec<SkippedAsset>,
    /// Detailed entries for each lossy conversion.
    pub lossy_conversions: Vec<LossyConversion>,
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

    /// Appends a diagnostic warning.
    pub fn record_warning(
        &mut self,
        asset: impl Into<String>,
        severity: WarningSeverity,
        message: impl Into<String>,
    ) {
        self.warnings.push(ConversionWarning {
            asset: asset.into(),
            severity,
            message: message.into(),
        });
    }

    /// Records a skipped asset and increments `skipped_count`.
    pub fn record_skipped(&mut self, asset: impl Into<String>, reason: impl Into<String>) {
        self.skipped_count = self.skipped_count.saturating_add(1);
        self.skipped_assets.push(SkippedAsset {
            asset: asset.into(),
            reason: reason.into(),
        });
    }

    /// Records a lossy conversion and increments `lossy_count`.
    pub fn record_lossy(&mut self, asset: impl Into<String>, detail: impl Into<String>) {
        self.lossy_count = self.lossy_count.saturating_add(1);
        self.lossy_conversions.push(LossyConversion {
            asset: asset.into(),
            detail: detail.into(),
        });
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
    }

    /// Returns true if any skipped assets, lossy conversions, or warnings with `Warning` severity exist.
    #[must_use]
    pub fn has_strict_issues(&self) -> bool {
        self.skipped_count > 0
            || self.lossy_count > 0
            || self
                .warnings
                .iter()
                .any(|w| w.severity == WarningSeverity::Warning)
    }
}
