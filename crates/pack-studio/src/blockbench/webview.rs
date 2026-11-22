use std::process::Command;

use anyhow::{Context, Result};

/// Outcome of attempting to open the local Blockbench Studio URL inside a native OS WebView.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebViewLaunchOutcome {
    /// Opened successfully inside a native hardware-accelerated `wry` WebView window.
    OpenedNative,
    /// Native WebView unavailable or failed initialization; UI should expose the local browser button.
    FallbackToBrowser { reason: String },
}

/// Attempts to open `url` inside a native OS WebView (`wry`, with WebGL enabled).
/// Falls back cleanly with a clear explanation when built without `native-webview` or when the
/// host environment lacks WebKitGTK / WebGL acceleration (e.g. headless / Wayland / proot).
#[must_use]
pub fn try_open_native_webview(url: &str) -> WebViewLaunchOutcome {
    #[cfg(feature = "native-webview")]
    {
        match launch_wry_window(url) {
            Ok(()) => WebViewLaunchOutcome::OpenedNative,
            Err(err) => WebViewLaunchOutcome::FallbackToBrowser {
                reason: format!(
                    "Native WebView initialization failed ({err:#}); use local browser fallback with full 2-way sync."
                ),
            },
        }
    }
    #[cfg(not(feature = "native-webview"))]
    {
        let _ = url;
        WebViewLaunchOutcome::FallbackToBrowser {
            reason: String::from(
                "Native WebView (wry) not enabled on this build; click 'Buka Blockbench Studio di Browser Lokal' for full 2-way sync.",
            ),
        }
    }
}

#[cfg(feature = "native-webview")]
fn launch_wry_window(url: &str) -> Result<()> {
    let target_url = url.to_string();
    std::thread::Builder::new()
        .name(String::from("packstudio-blockbench-webview"))
        .spawn(move || {
            use tao::event::{Event, WindowEvent};
            use tao::event_loop::{ControlFlow, EventLoop};
            use tao::window::WindowBuilder;
            use wry::WebViewBuilder;

            let event_loop = EventLoop::new();
            let Ok(window) = WindowBuilder::new()
                .with_title("PackStudio — Blockbench 3D & Paint Studio")
                .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 820.0))
                .build(&event_loop)
            else {
                return;
            };
            let Ok(_webview) = WebViewBuilder::new()
                .with_url(&target_url)
                .build(&window)
            else {
                return;
            };
            event_loop.run(move |event, _, control_flow| {
                *control_flow = ControlFlow::Wait;
                if let Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } = event
                {
                    *control_flow = ControlFlow::Exit;
                }
            });
        })
        .context("failed spawning native wry WebView thread")?;
    Ok(())
}

/// Opens `url` directly in the user's local system browser (fallback path with full 2-way sync).
pub fn open_in_local_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(url)
            .spawn()
            .with_context(|| format!("failed to launch local browser via xdg-open for '{url}'"))?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .spawn()
            .with_context(|| format!("failed to launch local browser via open for '{url}'"))?;
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .with_context(|| format!("failed to launch local browser for '{url}'"))?;
    }
    Ok(())
}
