use thiserror::Error;

/// Canonical error type for core resource pack IR, metadata, and mapping operations.
#[derive(Debug, Error)]
pub enum CoreError {
    /// JSON serialization or deserialization failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Standard I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Image decoding or encoding failure.
    #[error("Image error: {0}")]
    Image(#[from] image::ImageError),

    /// Pack metadata (`pack.mcmeta` or `manifest.json`) is invalid or missing required fields.
    #[error("Invalid pack metadata: {0}")]
    InvalidMetadata(String),

    /// Image or animation frame dimensions are invalid or inconsistent.
    #[error("Invalid dimensions: {0}")]
    InvalidDimensions(String),

    /// Version or asset mapping table failed to load or parse.
    #[error("Mapping error: {0}")]
    Mapping(String),

    /// An unsupported asset or feature was encountered in strict mode.
    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),

    /// Invalid pack format or syntax error.
    #[error("Invalid format: {0}")]
    InvalidFormat(String),
}
