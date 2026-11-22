#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

//! Linux Terminal UI (`rp-tui`) for managing, inspecting, converting, and auditing
//! Minecraft Java Edition and Bedrock Edition resource packs.

pub mod app;
pub mod preview;
pub mod scanner;
pub mod ui;
