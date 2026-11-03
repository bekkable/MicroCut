//! Top-level UI state and frame loop.

use eframe::egui;

/// Root application state.
#[derive(Debug, Default)]
pub struct MicroCutApp {}

impl eframe::App for MicroCutApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.heading("Hello World!");
            });
        });
    }
}
