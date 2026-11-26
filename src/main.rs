#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod batch;
mod db;
mod explorer;
mod format;
mod kerberos;
mod settings;

/// Must match the Flatpak app ID and the .desktop file name, so the window
/// gets the right icon and taskbar grouping.
const APP_ID: &str = "io.github.loganhoney.SQLCraft";

fn main() -> eframe::Result {
    kerberos::prepare_config();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("SQLCraft")
            .with_app_id(APP_ID)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("SQLCraft", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
