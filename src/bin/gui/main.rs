#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! FaceSort : application graphique (écrans Analyse, Personnes, Tri).

mod analyse;
mod app;
mod people;
mod tri;

use eframe::egui;

fn load_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../../../assets/icon-256.png")).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData { rgba: img.into_raw(), width, height })
}

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!("FaceSort {}", env!("CARGO_PKG_VERSION")))
        .with_inner_size([1100.0, 760.0])
        .with_min_inner_size([720.0, 480.0]);
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("FaceSort", options, Box::new(|cc| Ok(Box::new(app::FaceSortApp::new(cc)))))
}
