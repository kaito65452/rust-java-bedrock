use thiserror::Error;

/// Error type for Java Edition resource pack parsing and emission.
#[derive(Debug, Error)]
pub enum JavaPackError {
    /// Core IR, image, or mapping error.
    #[error(transparent)]
    Core(#[from] rp_core::CoreError),

    /// Archive/directory container I/O error.
    #[error(transparent)]
    Io(#[from] rp_io::IoError),

    /// JSON serialization/deserialization failure.
    #[error("JSON parse/serialize error: {0}")]
    Json(#[from] serde_json::Error),

    /// Required `pack.mcmeta` file was missing from the pack root.
    #[error("Missing required 'pack.mcmeta' in Java resource pack")]
    MissingPackMcmeta,

    /// `pack_format` integer is out of valid bounds (`1..=200`).
    #[error("Invalid Java pack_format '{0}': expected integer in 1..=200")]
    InvalidPackFormat(u32),

    /// Malformed asset JSON or unexpected format at a specific path.
    #[error("Failed to parse Java pack file '{path}': {detail}")]
    ParseError {
        /// Archive path that failed to parse.
        path: String,
        /// Underlying failure details.
        detail: String,
    },
}
