//! MicroCut desktop app (layer L5): window, panels, input.

#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod crash;

use std::process::ExitCode;

fn main() -> ExitCode {
    crash::install_panic_hook();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("MicroCut")
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };

    match eframe::run_native(
        "MicroCut",
        options,
        Box::new(|_cc| Ok(Box::new(app::MicroCutApp::default()))),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("microcut: failed to start the window: {err}");
            ExitCode::FAILURE
        }
    }
}
