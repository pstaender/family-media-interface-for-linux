//! The home screen ("front row"): rows of large tiles, controlled with keyboard/remote or mouse.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Child;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

use crate::config::Config;
use crate::control::SharedConfig;
use crate::disc::{self, DriveReport, DriveState, MediaKind};
use crate::makemkv::{self, DiscInfo, ScanProgress};
use crate::player::{self, MovieOptions, PlayMode};
use crate::{resume, store};

const BG: Color32 = Color32::from_rgb(0x12, 0x14, 0x1c);
const TILE_BG: Color32 = Color32::from_rgb(0x22, 0x26, 0x34);
const TEXT: Color32 = Color32::from_rgb(0xf2, 0xf4, 0xf8);
const TEXT_DIM: Color32 = Color32::from_rgb(0x9a, 0xa1, 0xb3);
const ACCENT: Color32 = Color32::from_rgb(0x3d, 0x8b, 0xff);
/// Opacity of tiles that can't be used right now.
const UNAVAILABLE_ALPHA: f32 = 0.35;

enum ScanState {
    Running {
        progress: Option<ScanProgress>,
        phase_started: Instant,
    },
    Done(DiscInfo),
    Failed,
}

impl ScanState {
    fn running() -> Self {
        ScanState::Running {
            progress: None,
            phase_started: Instant::now(),
        }
    }

    /// Text and estimated overall progress (0–1) while a scan is running.
    fn progress(&self) -> Option<(&'static str, f32)> {
        match self {
            ScanState::Running {
                progress: Some(p),
                phase_started,
            } => Some((p.phase.label(), p.estimate(phase_started.elapsed()))),
            ScanState::Running { progress: None, .. } => Some(("Reading disc", 0.0)),
            _ => None,
        }
    }
}

enum ScanEvent {
    Progress(String, ScanProgress),
    Done(String, Result<DiscInfo, String>),
}

enum Screen {
    Home,
    /// Asking how to start the Blu-ray.
    Choose {
        selected: usize,
        /// Where the movie was stopped last time, if it can be resumed.
        resume: Option<f64>,
    },
    Ejecting,
    Playing {
        child: Child,
        started: Instant,
    },
    Message(String),
}

/// What the Blu-ray tile shows.
struct BlurayTile {
    available: bool,
    title: String,
    subtitle: String,
    device: Option<PathBuf>,
    disc_key: Option<String>,
    /// Stable id of the disc (for cache and resume); `None` if the disc has no UUID/label.
    disc_id: Option<String>,
    /// Estimated progress (0–1) while the disc is being read.
    progress: Option<f32>,
}

#[derive(Clone, Copy, PartialEq)]
enum Choice {
    /// (b) Resume the main movie at this position.
    Resume(f64),
    MainMovie,
    DiscMenu,
    Eject,
    Back,
}

/// The options shown for a Blu-ray. "Resume" only appears if the movie was played before,
/// "Disc menu" only if the parents allow it.
fn choices(resume: Option<f64>, allow_disc_menu: bool) -> Vec<Choice> {
    let mut list = Vec::new();
    if let Some(pos) = resume {
        list.push(Choice::Resume(pos));
    }
    list.push(Choice::MainMovie);
    if allow_disc_menu {
        list.push(Choice::DiscMenu);
    }
    list.extend([Choice::Eject, Choice::Back]);
    list
}

pub struct App {
    ctx: egui::Context,
    drives_rx: Receiver<Vec<DriveReport>>,
    drives: Vec<DriveReport>,
    scan_tx: Sender<ScanEvent>,
    scan_rx: Receiver<ScanEvent>,
    scans: HashMap<String, ScanState>,
    eject_tx: Sender<Result<(), String>>,
    eject_rx: Receiver<Result<(), String>>,
    config: SharedConfig,
    screen: Screen,
}

impl App {
    pub fn new(ctx: egui::Context, config: SharedConfig) -> Self {
        let (drives_tx, drives_rx) = mpsc::channel();
        let repaint = ctx.clone();
        disc::spawn_watcher(Duration::from_secs(2), drives_tx, move || {
            repaint.request_repaint()
        });
        let (scan_tx, scan_rx) = mpsc::channel();
        let (eject_tx, eject_rx) = mpsc::channel();
        Self {
            ctx,
            drives_rx,
            drives: Vec::new(),
            scan_tx,
            scan_rx,
            scans: HashMap::new(),
            eject_tx,
            eject_rx,
            config,
            screen: Screen::Home,
        }
    }

    /// The current config (may change at any time through a reload).
    fn config(&self) -> (Config, Option<String>) {
        let state = self.config.lock().unwrap_or_else(|p| p.into_inner());
        (state.config.clone(), state.error.clone())
    }

    /// Pull updates from background threads.
    fn poll_background(&mut self) {
        while let Ok(drives) = self.drives_rx.try_recv() {
            self.drives = drives;
        }
        while let Ok(event) = self.scan_rx.try_recv() {
            match event {
                ScanEvent::Progress(key, new) => {
                    if let Some(ScanState::Running {
                        progress,
                        phase_started,
                    }) = self.scans.get_mut(&key)
                    {
                        if progress.map(|p| p.phase) != Some(new.phase) {
                            *phase_started = Instant::now();
                        }
                        *progress = Some(new);
                    }
                }
                ScanEvent::Done(key, Ok(info)) => {
                    self.scans.insert(key, ScanState::Done(info));
                }
                ScanEvent::Done(key, Err(e)) => {
                    log::warn!("disc scan failed: {e}");
                    self.scans.insert(key, ScanState::Failed);
                }
            }
        }
        while let Ok(result) = self.eject_rx.try_recv() {
            self.screen = match result {
                Ok(()) => Screen::Home,
                Err(e) => Screen::Message(e),
            };
        }
        // Every new Blu-ray: use the cached disc info, or start a MakeMKV scan.
        for report in &self.drives {
            if let DriveState::Loaded(d) = &report.state {
                let key = d.key(&report.device);
                if d.kind == MediaKind::BluRay && !self.scans.contains_key(&key) {
                    let cache_path = d.id().and_then(|id| store::disc_cache_path(&id));
                    if let Some(info) = cache_path.as_deref().and_then(store::load::<DiscInfo>) {
                        log::info!("disc info for {key} loaded from cache");
                        self.scans.insert(key, ScanState::Done(info));
                        continue;
                    }
                    self.scans.insert(key.clone(), ScanState::running());
                    let (tx, device, ctx) = (
                        self.scan_tx.clone(),
                        report.device.clone(),
                        self.ctx.clone(),
                    );
                    std::thread::spawn(move || {
                        let result = makemkv::scan(&device, |p| {
                            let _ = tx.send(ScanEvent::Progress(key.clone(), p));
                            ctx.request_repaint();
                        });
                        if let (Ok(info), Some(path)) = (&result, &cache_path)
                            && let Err(e) = store::save(path, info)
                        {
                            log::warn!("could not cache disc info: {e}");
                        }
                        let _ = tx.send(ScanEvent::Done(key, result));
                        ctx.request_repaint();
                    });
                }
            }
        }
        // Keep the estimated progress moving while a scan runs.
        if self.scans.values().any(|s| s.progress().is_some()) {
            self.ctx.request_repaint_after(Duration::from_millis(250));
        }
        if let Screen::Playing { child, started } = &mut self.screen {
            match child.try_wait() {
                Ok(None) => self.ctx.request_repaint_after(Duration::from_millis(500)),
                Ok(Some(status))
                    if !status.success() && started.elapsed() < Duration::from_secs(10) =>
                {
                    self.screen = Screen::Message(format!(
                        "The player stopped right away ({status}).\n\
                         Is MakeMKV installed and registered? See scripts/setup.sh."
                    ));
                }
                Ok(Some(_)) => self.screen = Screen::Home,
                Err(e) => self.screen = Screen::Message(format!("Lost track of the player: {e}")),
            }
        }
    }

    fn bluray_tile(&self) -> BlurayTile {
        let tile = |available, title: &str, subtitle: &str| BlurayTile {
            available,
            title: title.into(),
            subtitle: subtitle.into(),
            device: None,
            disc_key: None,
            disc_id: None,
            progress: None,
        };
        let (config, error) = self.config();
        if !config.discs.enabled {
            return match error {
                Some(_) => tile(false, "Blu-ray", "Settings could not be loaded"),
                None => tile(false, "Blu-ray", "Discs are turned off"),
            };
        }
        if self.drives.is_empty() {
            return tile(false, "Blu-ray", "No disc drive found");
        }
        // Prefer a drive with a Blu-ray in it.
        let bd = self.drives.iter().find_map(|r| match &r.state {
            DriveState::Loaded(d) if d.kind == MediaKind::BluRay => Some((r, d)),
            _ => None,
        });
        if let Some((report, d)) = bd {
            let key = d.key(&report.device);
            let fallback = d
                .label
                .as_deref()
                .map(disc::pretty_label)
                .unwrap_or_else(|| "Blu-ray".into());
            let scan = self.scans.get(&key);
            let progress = scan.and_then(ScanState::progress);
            let (title, subtitle) = match (scan, progress) {
                (Some(ScanState::Done(info)), _) => {
                    (info.name.clone().unwrap_or(fallback), "Blu-ray".to_string())
                }
                (_, Some((label, f))) => (fallback, progress_text(label, f)),
                _ => (fallback, "Blu-ray".to_string()),
            };
            return BlurayTile {
                available: true,
                title,
                subtitle,
                device: Some(report.device.clone()),
                disc_key: Some(key),
                disc_id: d.id(),
                progress: progress.map(|(_, f)| f),
            };
        }
        let state = &self.drives[0].state;
        match state {
            DriveState::Loaded(d) if d.kind == MediaKind::Dvd => {
                tile(false, "Blu-ray", "DVDs are not supported yet")
            }
            DriveState::Loaded(_) => tile(false, "Blu-ray", "This disc is not a Blu-ray"),
            DriveState::NotReady => tile(false, "Blu-ray", "Reading disc…"),
            DriveState::TrayOpen => tile(false, "Blu-ray", "Tray is open"),
            DriveState::Empty => tile(false, "Blu-ray", "Insert a Blu-ray"),
            DriveState::Error(e) => {
                log::debug!("drive error: {e}");
                tile(false, "Blu-ray", "Drive not accessible")
            }
        }
    }

    fn scan_done(&self, key: Option<&String>) -> bool {
        matches!(
            key.and_then(|k| self.scans.get(k)),
            Some(ScanState::Done(_) | ScanState::Failed)
        )
    }

    fn start(&mut self, mode: PlayMode, tile: &BlurayTile, start_secs: Option<f64>) {
        let Some(device) = &tile.device else { return };
        let (config, _) = self.config();
        // Re-check the settings: they may have changed while the menu was open.
        if !config.discs.enabled || (mode == PlayMode::DiscMenu && !config.discs.allow_disc_menu) {
            self.screen = Screen::Home;
            return;
        }
        let info = match tile.disc_key.as_ref().and_then(|k| self.scans.get(k)) {
            Some(ScanState::Done(info)) => Some(info),
            _ => None,
        };
        // Track the position (for "Resume") only for discs we can recognize again.
        let track = (mode == PlayMode::MainMovie)
            .then_some(tile.disc_id.clone())
            .flatten();
        let socket = store::mpv_socket_path();
        if track.is_some() {
            store::remove(&socket); // stale socket from an earlier run
        }
        let options = MovieOptions {
            start_secs,
            ipc_socket: track.is_some().then_some(socket.as_path()),
            audio_languages: &config.discs.audio_languages,
            subtitle_languages: &config.discs.subtitle_languages,
        };
        let mut cmd = player::command(mode, device, info, &options);
        log::info!("starting player: {cmd:?}");
        self.screen = match cmd.spawn() {
            Ok(child) => {
                if let Some(id) = track {
                    resume::spawn_monitor(socket, id);
                }
                Screen::Playing {
                    child,
                    started: Instant::now(),
                }
            }
            Err(e) => Screen::Message(format!(
                "Could not start the player ({:?}): {e}.\nRun scripts/setup.sh to install it.",
                cmd.get_program()
            )),
        };
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_background();
        let ctx = ui.ctx().clone();
        let screen_rect = ui.max_rect();
        let s = (screen_rect.height() / 1080.0).max(0.5); // scale factor
        let painter = ui.painter().clone();
        painter.rect_filled(screen_rect, 0.0, BG);

        let (enter, escape, up, down, quit) = ctx.input(|i| {
            (
                i.key_pressed(Key::Enter) || i.key_pressed(Key::Space),
                i.key_pressed(Key::Escape) || i.key_pressed(Key::Backspace),
                i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::ArrowRight),
                i.modifiers.ctrl && i.key_pressed(Key::Q),
            )
        });
        // TODO: protect leaving the media center with the PIN.
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Header
        let margin = 80.0 * s;
        painter.text(
            screen_rect.min + Vec2::new(margin, 50.0 * s),
            Align2::LEFT_TOP,
            "familynovice",
            FontId::proportional(34.0 * s),
            TEXT_DIM,
        );

        // Row: Discs
        painter.text(
            screen_rect.min + Vec2::new(margin, 170.0 * s),
            Align2::LEFT_TOP,
            "Discs",
            FontId::proportional(40.0 * s),
            TEXT,
        );
        let tile = self.bluray_tile();
        let tile_rect = Rect::from_min_size(
            screen_rect.min + Vec2::new(margin, 240.0 * s),
            Vec2::new(340.0 * s, 400.0 * s),
        );
        let home = matches!(self.screen, Screen::Home);
        let response = ui.interact(
            tile_rect,
            ui.make_persistent_id("bluray_tile"),
            Sense::click(),
        );
        let focus = ctx.animate_bool_with_time(
            ui.make_persistent_id("bluray_focus"),
            home && tile.available,
            0.15,
        );
        paint_bluray_tile(&painter, tile_rect, s, &tile, focus);

        match &self.screen {
            Screen::Home => {
                if tile.available && (enter || response.clicked()) {
                    self.screen = Screen::Choose {
                        selected: 0,
                        resume: tile.disc_id.as_deref().and_then(resume::load),
                    };
                }
            }
            Screen::Choose { selected, resume } => {
                let (mut selected, resume) = (*selected, *resume);
                let list = choices(resume, self.config().0.discs.allow_disc_menu);
                if up {
                    selected = selected.saturating_sub(1);
                }
                if down {
                    selected = (selected + 1).min(list.len() - 1);
                }
                let scan_ready = self.scan_done(tile.disc_key.as_ref());
                let clicked = paint_chooser(
                    ui,
                    &painter,
                    screen_rect,
                    s,
                    &tile,
                    &list,
                    selected,
                    scan_ready,
                );
                let activated = clicked.or(enter.then_some(selected));
                self.screen = Screen::Choose { selected, resume };
                if escape || !tile.available {
                    self.screen = Screen::Home;
                } else if let Some(i) = activated {
                    match list[i] {
                        // The main movie needs the scan result (which playlist to play).
                        Choice::Resume(_) | Choice::MainMovie if !scan_ready => {}
                        Choice::Resume(pos) => self.start(PlayMode::MainMovie, &tile, Some(pos)),
                        Choice::MainMovie => self.start(PlayMode::MainMovie, &tile, None),
                        Choice::DiscMenu => self.start(PlayMode::DiscMenu, &tile, None),
                        Choice::Eject => {
                            if let Some(device) = tile.device.clone() {
                                let (tx, ctx) = (self.eject_tx.clone(), ctx.clone());
                                std::thread::spawn(move || {
                                    let _ = tx.send(disc::eject(&device));
                                    ctx.request_repaint();
                                });
                                self.screen = Screen::Ejecting;
                            }
                        }
                        Choice::Back => self.screen = Screen::Home,
                    }
                }
            }
            Screen::Ejecting => {
                paint_overlay(
                    &painter,
                    screen_rect,
                    s,
                    "Ejecting…",
                    "The tray opens in a moment.",
                );
            }
            Screen::Playing { .. } => {
                paint_overlay(
                    &painter,
                    screen_rect,
                    s,
                    "Playing…",
                    "The movie is playing.",
                );
            }
            Screen::Message(msg) => {
                paint_overlay(&painter, screen_rect, s, "Something went wrong", msg);
                if enter || escape || ctx.input(|i| i.pointer.any_click()) {
                    self.screen = Screen::Home;
                }
            }
        }
    }
}

fn paint_bluray_tile(painter: &egui::Painter, rect: Rect, s: f32, tile: &BlurayTile, focus: f32) {
    let alpha = if tile.available {
        1.0
    } else {
        UNAVAILABLE_ALPHA
    };
    let c = |col: Color32| col.gamma_multiply(alpha);
    let rect = rect.expand(focus * 12.0 * s);
    painter.rect_filled(rect, CornerRadius::same((18.0 * s) as u8), c(TILE_BG));
    if focus > 0.0 {
        painter.rect_stroke(
            rect,
            CornerRadius::same((18.0 * s) as u8),
            Stroke::new(4.0 * s, ACCENT.gamma_multiply(focus)),
            StrokeKind::Outside,
        );
    }
    let center = Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.38);
    paint_disc_icon(painter, center, rect.width() * 0.32, alpha);
    painter.text(
        Pos2::new(rect.center().x, rect.max.y - 110.0 * s),
        Align2::CENTER_TOP,
        elide(&tile.title, 22),
        FontId::proportional(30.0 * s),
        c(TEXT),
    );
    // Shrink long subtitles (e.g. "Unlocking copy protection… 56%") to fit the tile.
    let max_width = rect.width() - 40.0 * s;
    let mut size = 22.0 * s;
    let width = painter
        .layout_no_wrap(tile.subtitle.clone(), FontId::proportional(size), TEXT_DIM)
        .size()
        .x;
    if width > max_width {
        size *= max_width / width;
    }
    painter.text(
        Pos2::new(rect.center().x, rect.max.y - 65.0 * s),
        Align2::CENTER_TOP,
        &tile.subtitle,
        FontId::proportional(size),
        c(TEXT_DIM),
    );
    if let Some(f) = tile.progress {
        let track = Rect::from_min_size(
            Pos2::new(rect.min.x + 40.0 * s, rect.max.y - 26.0 * s),
            Vec2::new(rect.width() - 80.0 * s, 8.0 * s),
        );
        let radius = CornerRadius::same((4.0 * s) as u8);
        painter.rect_filled(track, radius, c(Color32::from_rgb(0x3a, 0x40, 0x55)));
        let mut bar = track;
        bar.set_width(track.width() * f.clamp(0.0, 1.0));
        painter.rect_filled(bar, radius, c(ACCENT));
    }
}

fn progress_text(label: &str, fraction: f32) -> String {
    format!("{label}… {}%", (fraction * 100.0).round() as u32)
}

/// Our own optical disc icon (not the Blu-ray logo): a shiny disc with a blue tint.
fn paint_disc_icon(painter: &egui::Painter, center: Pos2, r: f32, alpha: f32) {
    let c = |col: Color32| col.gamma_multiply(alpha);
    painter.circle_filled(center, r, c(Color32::from_rgb(0x1d, 0x5f, 0xd6)));
    painter.circle_filled(center, r * 0.93, c(Color32::from_rgb(0x5a, 0x9c, 0xf5)));
    painter.circle_filled(center, r * 0.75, c(Color32::from_rgb(0x8d, 0xbd, 0xfa)));
    painter.circle_filled(center, r * 0.55, c(Color32::from_rgb(0xb9, 0xd6, 0xfc)));
    painter.circle_filled(center, r * 0.32, c(Color32::from_rgb(0xdc, 0xe4, 0xf0)));
    painter.circle_filled(center, r * 0.12, c(TILE_BG));
    // highlight
    let hl = Color32::from_rgba_unmultiplied(255, 255, 255, 60).gamma_multiply(alpha);
    painter.circle_stroke(center, r * 0.85, Stroke::new(r * 0.04, hl));
}

/// Returns the index of a clicked option.
#[allow(clippy::too_many_arguments)]
fn paint_chooser(
    ui: &egui::Ui,
    painter: &egui::Painter,
    screen: Rect,
    s: f32,
    tile: &BlurayTile,
    list: &[Choice],
    selected: usize,
    scan_ready: bool,
) -> Option<usize> {
    const ROW_STEP: f32 = 105.0;
    painter.rect_filled(screen, 0.0, Color32::from_black_alpha(190));
    let height = 150.0 + list.len() as f32 * ROW_STEP + 30.0;
    let panel = Rect::from_center_size(screen.center(), Vec2::new(720.0 * s, height * s));
    painter.rect_filled(panel, CornerRadius::same((24.0 * s) as u8), TILE_BG);
    painter.text(
        Pos2::new(panel.center().x, panel.min.y + 50.0 * s),
        Align2::CENTER_TOP,
        elide(&tile.title, 32),
        FontId::proportional(40.0 * s),
        TEXT,
    );
    let mut clicked = None;
    let has_resume = list.iter().any(|c| matches!(c, Choice::Resume(_)));
    for (i, choice) in list.iter().enumerate() {
        let needs_scan = matches!(choice, Choice::Resume(_) | Choice::MainMovie);
        let waiting = needs_scan && !scan_ready;
        let (label, hint): (String, &str) = match choice {
            Choice::Resume(pos) => (
                format!("Resume from {}", resume::format_time(*pos)),
                "Continues where you stopped",
            ),
            Choice::MainMovie if has_resume => (
                "Play from beginning".into(),
                "Starts the movie from the start",
            ),
            Choice::MainMovie => ("Play movie".into(), "Starts the main movie right away"),
            Choice::DiscMenu => ("Disc menu".into(), "Starts the disc like a normal player"),
            Choice::Eject => ("Eject disc".into(), "Opens the tray"),
            Choice::Back => ("Back".into(), ""),
        };
        let hint = if waiting {
            tile.subtitle.as_str()
        } else {
            hint
        };
        let enabled = !waiting;
        let row = Rect::from_min_size(
            Pos2::new(
                panel.min.x + 60.0 * s,
                panel.min.y + (150.0 + i as f32 * ROW_STEP) * s,
            ),
            Vec2::new(panel.width() - 120.0 * s, (ROW_STEP - 15.0) * s),
        );
        let focused = i == selected;
        let fill = if focused {
            ACCENT
        } else {
            Color32::from_rgb(0x2e, 0x33, 0x45)
        };
        let alpha = if enabled { 1.0 } else { 0.45 };
        painter.rect_filled(
            row,
            CornerRadius::same((14.0 * s) as u8),
            fill.gamma_multiply(alpha),
        );
        painter.text(
            Pos2::new(
                row.min.x + 30.0 * s,
                row.center().y - if hint.is_empty() { 0.0 } else { 14.0 * s },
            ),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(32.0 * s),
            TEXT.gamma_multiply(alpha),
        );
        if !hint.is_empty() {
            painter.text(
                Pos2::new(row.min.x + 30.0 * s, row.center().y + 22.0 * s),
                Align2::LEFT_CENTER,
                hint,
                FontId::proportional(20.0 * s),
                TEXT.gamma_multiply(0.75 * alpha),
            );
        }
        if ui
            .interact(row, ui.make_persistent_id(("choice", i)), Sense::click())
            .clicked()
        {
            clicked = Some(i);
        }
    }
    clicked
}

fn paint_overlay(painter: &egui::Painter, screen: Rect, s: f32, title: &str, body: &str) {
    painter.rect_filled(screen, 0.0, Color32::from_black_alpha(200));
    painter.text(
        screen.center() - Vec2::new(0.0, 40.0 * s),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(44.0 * s),
        TEXT,
    );
    for (i, line) in body.lines().enumerate() {
        painter.text(
            screen.center() + Vec2::new(0.0, (30.0 + i as f32 * 34.0) * s),
            Align2::CENTER_CENTER,
            line,
            FontId::proportional(26.0 * s),
            TEXT_DIM,
        );
    }
}

fn elide(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max_chars - 1).collect();
        format!("{}…", cut.trim_end())
    }
}
