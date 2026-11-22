#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

//! Desktop GUI Studio (`rp-gui`) built with `eframe`/`egui` featuring a streamlined
//! pack browser/converter plus VS Code-style Studio Mode and Blockbench-style 3D inspector.

pub mod app;
pub mod diagnostics;
pub mod file_picker;
pub mod studio;
pub mod theme;
pub mod viewport3d;
