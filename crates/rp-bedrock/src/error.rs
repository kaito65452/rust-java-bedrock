use thiserror::Error;

/// Error type for Bedrock Edition resource pack parsing and emission.
#[derive(Debug, Error)]
pub enum BedrockPackError {
    /// Core IR, image, or mapping error.
    #[error(transparent)]
    Core(#[from] rp_core::CoreError),

    /// Archive/directory container I/O error.
    #[error(transparent)]
    Io(#[from] rp_io::IoError),

    /// JSON serialization/deserialization failure.
    #[error("JSON parse/serialize error: {0}")]
    Json(#[from] serde_json::Error),

    /// Required `manifest.json` file was missing from the pack root.
    #[error("Missing required 'manifest.json' in Bedrock resource pack")]
    MissingManifest,

    /// Invalid `manifest.json` structure or unsupported `format_version`.
    #[error("Invalid Bedrock manifest.json: {0}")]
    InvalidManifest(String),

    /// Malformed asset JSON or syntax error at a specific path.
    #[error("Failed to parse Bedrock pack file '{path}': {detail}")]
    ParseError {
        /// Archive path that failed to parse.
        path: String,
        /// Underlying failure details.
        detail: String,
    },
}
