//! Tor V3 Vanity Generator GUI
//!
//! A graphical interface for generating Tor v3 vanity addresses.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 700.0])
            .with_min_inner_size([500.0, 500.0])
            .with_icon(load_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Tor V3 Vanity Generator",
        options,
        Box::new(|cc| Ok(Box::new(app::VanityApp::new(cc)))),
    )
}

fn load_icon() -> egui::IconData {
    // Default icon (can be replaced with actual icon data)
    egui::IconData::default()
}
