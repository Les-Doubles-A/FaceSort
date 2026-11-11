#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! FaceSort : application graphique (écrans Analyse, Personnes, Tri).

mod analyse;
mod app;
mod people;
mod tri;

use eframe::egui;

fn main() -> eframe::Result {
    let viewport = egui::ViewportBuilder::default()
        .with_title(format!("FaceSort {}", env!("CARGO_PKG_VERSION")))
        .with_inner_size([1100.0, 760.0])
        .with_min_inner_size([720.0, 480.0]);
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("FaceSort", options, Box::new(|cc| Ok(Box::new(app::FaceSortApp::new(cc)))))
}
