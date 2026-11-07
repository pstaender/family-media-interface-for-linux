mod disc;
mod makemkv;
mod player;
mod resume;
mod store;
mod ui;

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,familynovice=info"),
    )
    .init();

    // `--windowed` for development; the media center normally runs fullscreen.
    let windowed = std::env::args().any(|a| a == "--windowed");

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
        Box::new(|cc| Ok(Box::new(ui::App::new(cc.egui_ctx.clone())))),
    )
}
