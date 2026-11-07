mod audio;
mod config;
mod control;
mod disc;
mod library;
mod makemkv;
mod pictures;
mod player;
mod resume;
mod store;
mod ui;

use std::sync::{Arc, Mutex};

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,familynovice=info"),
    )
    .init();

    // `--windowed` for development; the media center normally runs fullscreen.
    let windowed = std::env::args().any(|a| a == "--windowed");

    let config_path = config::config_path();
    if let Some(path) = &config_path {
        log::info!("config: {}", path.display());
    }
    let shared = Arc::new(Mutex::new(config::load_initial(config_path.as_deref())));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("familynovice")
            .with_inner_size([1280.0, 720.0])
            .with_fullscreen(!windowed),
        ..Default::default()
    };
    eframe::run_native(
        "familynovice",
        options,
        Box::new(move |cc| {
            if let Some(path) = config_path {
                let ctx = cc.egui_ctx.clone();
                control::spawn_server(
                    store::control_socket_path(),
                    path,
                    shared.clone(),
                    move || ctx.request_repaint(),
                );
            }
            Ok(Box::new(ui::App::new(cc.egui_ctx.clone(), shared)))
        }),
    )
}
