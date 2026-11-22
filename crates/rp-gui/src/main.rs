#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use egui::Vec2;

use rp_cli::load_mappings;
use rp_gui::app::RpGuiApp;

#[derive(Debug, Parser)]
#[command(
    name = "rp-gui",
    version,
    about = "Desktop GUI Studio for managing, previewing, converting, diffing, and auditing Minecraft Java <-> Bedrock resource packs"
)]
struct GuiArgs {
    /// Initial workspace directory to scan for resource packs (defaults to current directory).
    #[arg(default_value = ".")]
    dir: PathBuf,
    /// Optional external JSON mapping table override file.
    #[arg(long = "mapping")]
    mapping: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = GuiArgs::parse();
    let scan_dir = args.dir.canonicalize().unwrap_or(args.dir);
    let mappings = load_mappings(args.mapping.as_deref())?;

    let mut app = RpGuiApp::new(scan_dir, args.mapping, mappings);
    app.start_scan();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("mc-rp-converter Studio (Java <-> Bedrock)")
            .with_inner_size(Vec2::new(1180.0, 740.0))
            .with_min_inner_size(Vec2::new(820.0, 520.0))
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "mc-rp-converter Studio",
        native_options,
        Box::new(move |_cc| Ok(Box::new(app))),
    )
    .map_err(|err| anyhow::anyhow!("desktop GUI error: {err}"))
}
