#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

//! Desktop GUI Studio (`rp-gui`) built with `eframe`/`egui` for managing, previewing,
//! converting, diffing, and auditing Minecraft Java Edition and Bedrock Edition resource packs.

pub mod app;
pub mod viewport3d;
