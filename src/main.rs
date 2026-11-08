mod audio;
mod blocks;
mod bricks;
mod config;
mod control;
mod cover;
mod disc;
mod dodger;
mod gamepad;
mod highscores;
mod i18n;
mod inhibit;
mod library;
mod limits;
mod lock;
mod makemkv;
mod mpris;
mod pictures;
mod pin;
mod player;
mod pong;
mod power;
mod recent;
mod remote;
mod resume;
mod screensaver;
mod sound;
mod stats;
// Parts of it are only used by the browser window (web/, includes this file).
#[allow(dead_code)]
mod sites;
mod store;
mod ui;
mod usage;
mod volume;

use std::sync::{Arc, Mutex};

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,curator=info"),
    )
    .init();

    // Started by a kiosk loop that outlived its session: don't show up in the next one.
    if store::started_by_orphaned_kiosk(std::path::Path::new("/proc"), std::process::id()) {
        log::error!("started by a curator-kiosk loop whose session has ended; exiting");
        return Ok(());
    }
    // One media center per user: a second one (e.g. started by a kiosk loop whose session
    // already ended) would cover the screen without getting the admin panel's changes.
    let _instance = match store::lock_instance(&store::instance_lock_path()) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => {
            log::error!("another curator is already running for this user; exiting");
            return Ok(());
        }
        Err(e) => {
            log::warn!("single-instance lock not available ({e}); continuing");
            None
        }
    };

    // `--windowed` for development; the media center normally runs fullscreen.
    let windowed = std::env::args().any(|a| a == "--windowed");

    store::migrate_old_folders();
    let config_path = config::config_path();
    if let Some(path) = &config_path {
        log::info!("config: {}", path.display());
    }
    let shared = Arc::new(Mutex::new(config::load_initial(config_path.as_deref())));
    // Old usage statistics go when the media center starts (and when it's unlocked).
    {
        let state = shared.lock().unwrap_or_else(|p| p.into_inner());
        if state.config.statistics.enabled {
            stats::prune(state.config.statistics.keep_days);
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("curator")
            .with_inner_size([1280.0, 720.0])
            .with_fullscreen(!windowed),
        ..Default::default()
    };
    eframe::run_native(
        "curator",
        options,
        Box::new(move |cc| {
            let tracker = usage::Tracker::load().shared();
            let (remote_tx, remote_rx) = std::sync::mpsc::channel();
            let (pads_tx, pads_rx) = std::sync::mpsc::channel();
            let ctx = cc.egui_ctx.clone();
            let remote_devices = remote::spawn(remote_tx, pads_tx, move || ctx.request_repaint());
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
                    pads: pads_rx,
                    requests: ui_rx,
                },
            )))
        }),
    )
}
