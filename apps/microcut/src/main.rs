//! MicroCut desktop app (layer L5): window, panels, input.

#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod crash;
mod export;
mod loader;
mod playback;
mod preview;
mod report;
mod sync;
mod thumbs;
mod timeline;
mod tracks;
mod volume;

use std::path::PathBuf;
use std::process::ExitCode;

/// The app icon (`assets/make_icon.py` draws it).
const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

fn main() -> ExitCode {
    crash::install_panic_hook();
    // `microcut <video>` opens the video right away.
    let initial = std::env::args_os().nth(1).map(PathBuf::from);

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("MicroCut")
        .with_app_id("microcut")
        .with_inner_size([960.0, 640.0])
        .with_min_inner_size([480.0, 320.0]);
    // Without a readable icon the window keeps the default one.
    if let Ok(icon) = eframe::icon_data::from_png_bytes(ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    match eframe::run_native(
        "MicroCut",
        options,
        Box::new(move |cc| Ok(Box::new(app::MicroCutApp::new(&cc.egui_ctx, initial)))),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("microcut: failed to start the window: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_decodes() {
        let icon = eframe::icon_data::from_png_bytes(ICON_PNG).unwrap();
        assert_eq!((icon.width, icon.height), (1024, 1024));
    }
}
