#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::path::PathBuf;

use pack_studio::PackStudioApp;

fn main() -> eframe::Result {
    let initial_path = std::env::args().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PackStudio — Resource Pack Artist Workbench (Java + Bedrock)")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 600.0]),
        ..eframe::NativeOptions::default()
    };

    eframe::run_native(
        "PackStudio",
        options,
        Box::new(move |_cc| Ok(Box::new(PackStudioApp::new(initial_path)))),
    )
}
