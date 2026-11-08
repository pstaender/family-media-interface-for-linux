mod audio;
mod config;
mod control;
mod cover;
mod disc;
mod i18n;
mod library;
mod limits;
mod lock;
mod makemkv;
mod mpris;
mod pictures;
mod pin;
mod player;
mod recent;
mod remote;
mod resume;
// Parts of it are only used by the browser window (web/, includes this file).
#[allow(dead_code)]
mod sites;
mod store;
mod ui;
mod usage;

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
            let tracker = usage::Tracker::load().shared();
            let (remote_tx, remote_rx) = std::sync::mpsc::channel();
            let ctx = cc.egui_ctx.clone();
            let remote_devices = remote::spawn(remote_tx, move || ctx.request_repaint());
            let (ui_tx, ui_rx) = std::sync::mpsc::channel();
            if let Some(path) = config_path {
                let ctx = cc.egui_ctx.clone();
                control::spawn_server(
                    store::control_socket_path(),
                    control::Control {
                        config: shared.clone(),
                        config_path: path,
                        tracker: tracker.clone(),
                        remote_devices,
                        ui_requests: ui_tx,
                    },
                    move || ctx.request_repaint(),
                );
            }
            Ok(Box::new(ui::App::new(
                cc.egui_ctx.clone(),
                shared,
                tracker,
                ui::Inputs {
                    remote: remote_rx,
                    requests: ui_rx,
                },
            )))
        }),
    )
}
