use thiserror::Error;

/// Error type for archive/directory ingestion, validation, and emission.
#[derive(Debug, Error)]
pub enum IoError {
    /// Standard filesystem I/O failure.
    #[error("I/O error: {0}")]
    StdIo(#[from] std::io::Error),

    /// Underlying `zip` crate archive error.
    #[error("ZIP archive error: {0}")]
    Zip(#[from] zip::result::ZipError),

    /// Path traversal attempt (`..` segment or symlink escape) detected in archive.
    #[error("Zip Slip path traversal detected in entry: '{raw_path}'")]
    ZipSlipDetected {
        /// Raw untrusted path string from the archive entry.
        raw_path: String,
    },

    /// A single entry exceeded the configured maximum uncompressed size.
    #[error("File '{path}' exceeded max uncompressed size limit ({limit} bytes)")]
    FileSizeLimitExceeded {
        /// Normalized path of the oversized entry.
        path: String,
        /// Configured single-file limit in bytes.
        limit: u64,
    },

    /// Cumulative uncompressed archive size exceeded the configured limit.
    #[error("Total uncompressed size exceeded limit ({limit} bytes)")]
    TotalSizeLimitExceeded {
        /// Configured total limit in bytes.
        limit: u64,
    },

    /// Archive or directory entry count exceeded the configured limit.
    #[error("File count exceeded limit ({limit} entries)")]
    FileCountLimitExceeded {
        /// Configured maximum number of files.
        limit: usize,
    },

    /// Archive structure or entry path is malformed or unsafe.
    #[error("Invalid archive: {0}")]
    InvalidArchive(String),
}
