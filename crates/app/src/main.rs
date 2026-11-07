// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod detail;
mod dialogs;
mod panels;
mod settings;
mod start;
mod theme;
mod worker;

#[cfg(test)]
mod ui_tests;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result {
    // git starts Oxbow as its sequence editor during a rebase: do that job and exit before any window opens.
    if let Some(code) = oxbow_engine::editor::run_if_requested() {
        std::process::exit(code);
    }
    // `oxbow <path>` opens that repository (or offers to create one there); without it, the project list.
    let start = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Oxbow").with_inner_size([1280.0, 820.0]).with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native("Oxbow", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc.egui_ctx.clone(), start.as_deref())))))
}
