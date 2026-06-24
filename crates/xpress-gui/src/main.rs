//! xpress-gui — a small desktop front-end for the xpress engine.
//!
//! Drag files onto the window (or press the global hotkey to optimise the
//! clipboard image); results appear as floating cards showing the savings.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod work;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0, 600.0])
            .with_min_inner_size([360.0, 360.0])
            .with_title("xpress"),
        ..Default::default()
    };

    eframe::run_native(
        "xpress",
        options,
        Box::new(|cc| Ok(Box::new(app::XpressApp::new(cc)))),
    )
}
