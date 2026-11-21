#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

//! Secure archive and filesystem I/O container with Zip Slip prevention,
//! size limits, deterministic ordering, and parallel asset mapping.

pub mod container;
pub mod error;
pub mod path;

pub use container::{
    DEFAULT_MAX_FILE_COUNT, DEFAULT_MAX_SINGLE_FILE_BYTES, DEFAULT_MAX_TOTAL_UNCOMPRESSED_BYTES,
    IoLimits, PackContainer,
};
pub use error::IoError;
pub use path::sanitize_archive_path;
