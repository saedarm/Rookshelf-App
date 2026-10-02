// No console window behind the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod categorize;
mod db;
mod isbn;
mod lookup;
mod pie;

fn main() -> eframe::Result {
    // library.db lives next to the .exe, so the whole app is one portable folder.
    let db_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("library.db")))
        .unwrap_or_else(|| "library.db".into());

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Rookshelf — home library")
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([800.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Rookshelf",
        options,
        Box::new(move |cc| Ok(Box::new(app::RookshelfApp::new(cc, db_path)))),
    )
}
