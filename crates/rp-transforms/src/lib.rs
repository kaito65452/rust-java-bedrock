#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

//! Cross-edition asset transformations for textures/animations, 3D geometry coordinates,
//! localization tables, and audio definitions.

pub mod locale;
pub mod model;
pub mod sound;
pub mod texture;
