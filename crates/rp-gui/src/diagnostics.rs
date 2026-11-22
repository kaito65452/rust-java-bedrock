use std::collections::{BTreeMap, BTreeSet};

use rp_core::{ConversionReport, WarningSeverity};

/// Severity / review filter for diagnostics panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticFilter {
    All,
    NeedsReview,
    Lossy,
    Warning,
    Error,
    Info,
}

impl DiagnosticFilter {
    /// All filter choices in toolbar order.
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::NeedsReview,
        Self::Lossy,
        Self::Warning,
        Self::Error,
        Self::Info,
    ];

    /// Display label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::NeedsReview => "Needs Review",
            Self::Lossy => "Lossy",
            Self::Warning => "Warning",
            Self::Error => "Error",
            Self::Info => "Info",
        }
    }
}

/// Logical diagnostic category for grouping and badge styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticKind {
    Error,
    Skipped,
    Lossy,
    Warning,
    Info,
}

impl DiagnosticKind {
    /// Short badge text.
    #[must_use]
    pub const fn badge(self) -> &'static str {
        match self {
            Self::Error => "ERR",
            Self::Skipped => "SKIP",
            Self::Lossy => "LOSSY",
            Self::Warning => "WARN",
            Self::Info => "INFO",
        }
    }

    /// Whether this diagnostic requires human review in Studio Mode.
    #[must_use]
    pub const fn needs_review(self) -> bool {
        !matches!(self, Self::Info)
    }

    /// Returns whether this kind passes `filter`.
    #[must_use]
    pub const fn matches_filter(self, filter: DiagnosticFilter) -> bool {
        match filter {
            DiagnosticFilter::All => true,
            DiagnosticFilter::NeedsReview => self.needs_review(),
            DiagnosticFilter::Lossy => matches!(self, Self::Lossy),
            DiagnosticFilter::Warning => matches!(self, Self::Warning | Self::Skipped),
            DiagnosticFilter::Error => matches!(self, Self::Error),
            DiagnosticFilter::Info => matches!(self, Self::Info),
        }
    }
}

/// Collapsed cluster of identical diagnostic messages across one or more assets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedDiagnostic {
    /// Severity / kind of the cluster.
    pub kind: DiagnosticKind,
    /// Shared message / reason string.
    pub message: String,
    /// Deduplicated, sorted asset paths/IDs affected by this diagnostic.
    pub assets: Vec<String>,
}

/// Fast lookup index of assets flagged as needing review (lossy, skipped, warning, or error).
#[derive(Debug, Clone, Default)]
pub struct NeedsReviewIndex {
    /// Canonical or raw asset IDs with at least one review-worthy diagnostic.
    pub flagged_assets: BTreeSet<String>,
    /// Grouped diagnostic clusters sorted by severity (errors first) then count descending.
    pub groups: Vec<GroupedDiagnostic>,
}

impl NeedsReviewIndex {
    /// Builds a grouped diagnostic index from one or more `ConversionReport`s.
    #[must_use]
    pub fn from_reports<'a>(reports: impl IntoIterator<Item = &'a ConversionReport>) -> Self {
        let mut buckets: BTreeMap<(DiagnosticKind, String), BTreeSet<String>> = BTreeMap::new();
        let mut flagged_assets = BTreeSet::new();

        for report in reports {
            for item in &report.lossy_conversions {
                flagged_assets.insert(normalize_asset_key(&item.asset));
                buckets
                    .entry((DiagnosticKind::Lossy, item.detail.clone()))
                    .or_default()
                    .insert(item.asset.clone());
            }
            for item in &report.skipped_assets {
                flagged_assets.insert(normalize_asset_key(&item.asset));
                buckets
                    .entry((DiagnosticKind::Skipped, item.reason.clone()))
                    .or_default()
                    .insert(item.asset.clone());
            }
            for w in &report.warnings {
                let kind = match w.severity {
                    WarningSeverity::Info => DiagnosticKind::Info,
                    WarningSeverity::Warning => DiagnosticKind::Warning,
                    WarningSeverity::Error => DiagnosticKind::Error,
                };
                if kind.needs_review() {
                    flagged_assets.insert(normalize_asset_key(&w.source_path));
                }
                let display_msg = if w.actionable_fix.is_empty() {
                    format!("[{}] {}", w.stage, w.reason)
                } else {
                    format!("[{}] {} — Fix: {}", w.stage, w.reason, w.actionable_fix)
                };
                buckets
                    .entry((kind, display_msg))
                    .or_default()
                    .insert(w.source_path.clone());
            }
        }

        let mut groups: Vec<GroupedDiagnostic> = buckets
            .into_iter()
            .map(|((kind, message), set)| GroupedDiagnostic {
                kind,
                message,
                assets: set.into_iter().collect(),
            })
            .collect();

        groups.sort_by(|a, b| {
            a.kind
                .cmp(&b.kind)
                .then_with(|| b.assets.len().cmp(&a.assets.len()))
                .then_with(|| a.message.cmp(&b.message))
        });

        Self {
            flagged_assets,
            groups,
        }
    }

    /// Returns `true` if `asset_id` (e.g. `"block/stone"`, `"entity/reyzz_mask"`) has any
    /// lossy, skipped, warning, or error diagnostic.
    #[must_use]
    pub fn asset_needs_review(&self, asset_id: &str) -> bool {
        let norm = normalize_asset_key(asset_id);
        if self.flagged_assets.contains(&norm) {
            return true;
        }
        self.flagged_assets
            .iter()
            .any(|f| f.ends_with(&norm) || norm.ends_with(f.as_str()))
    }

    /// Counts total individual asset diagnostics matching `filter`.
    #[must_use]
    pub fn count_matching(&self, filter: DiagnosticFilter) -> usize {
        self.groups
            .iter()
            .filter(|g| g.kind.matches_filter(filter))
            .map(|g| g.assets.len())
            .sum()
    }
}

/// Normalizes an asset path or canonical ID for cross-matching report entries with IR keys.
#[must_use]
pub fn normalize_asset_key(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .replace('\\', "/")
        .trim_start_matches("assets/minecraft/")
        .trim_start_matches("models/")
        .trim_start_matches("textures/")
        .trim_start_matches("sounds/")
        .to_string();
    for suffix in [".geo.json", ".json", ".png.mcmeta", ".png", ".ogg", ".lang"] {
        if let Some(stripped) = trimmed.strip_suffix(suffix) {
            return stripped.to_ascii_lowercase();
        }
    }
    trimmed.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_repetitive_warnings_and_indexes_needs_review() {
        let mut rep = ConversionReport::new();
        rep.record_warning(
            "item/wooden_sword",
            WarningSeverity::Info,
            "Skipped emitting empty parent-only Java model to Bedrock .geo.json",
        );
        rep.record_warning(
            "item/stone_sword",
            WarningSeverity::Info,
            "Skipped emitting empty parent-only Java model to Bedrock .geo.json",
        );
        rep.record_lossy(
            "entity/pabullet",
            "Bedrock box UV [u, v] approximated as per-face Java UVs",
        );
        rep.record_skipped(
            "assets/minecraft/shaders/core/test.fsh",
            "Java shader unsupported",
        );

        let idx = NeedsReviewIndex::from_reports([&rep]);
        assert_eq!(idx.groups.len(), 3);
        // Skipped + Lossy should appear before Info
        assert_eq!(idx.groups[0].kind, DiagnosticKind::Skipped);
        assert_eq!(idx.groups[1].kind, DiagnosticKind::Lossy);
        assert_eq!(idx.groups[2].kind, DiagnosticKind::Info);
        assert_eq!(idx.groups[2].assets.len(), 2);

        assert!(idx.asset_needs_review("entity/pabullet"));
        assert!(!idx.asset_needs_review("item/wooden_sword"));
        assert_eq!(idx.count_matching(DiagnosticFilter::NeedsReview), 2);
        assert_eq!(idx.count_matching(DiagnosticFilter::All), 4);
    }
}
