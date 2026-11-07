//! The home screen ("front row"): rows of large tiles, controlled with keyboard/remote or mouse.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Child;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

use crate::config::{Config, FolderSettings};
use crate::control::SharedConfig;
use crate::disc::{self, DriveReport, DriveState, MediaKind};
use crate::library::{self, Entry, EntryKind, Filter};
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
    /// Browsing a configured folder.
    Browse(Browse),
    Playing {
        child: Child,
        started: Instant,
        /// Where to return when the player exits (`None`: home screen).
        back: Option<Browse>,
        /// Started from a disc (for the error hint).
        disc: bool,
    },
    Message {
        text: String,
        back: Option<Browse>,
    },
}

fn message(text: impl Into<String>) -> Screen {
    Screen::Message {
        text: text.into(),
        back: None,
    }
}

/// State of the folder browser.
#[derive(Clone)]
struct Browse {
    folder_id: String,
    /// Folder names below the configured folder.
    sub: Vec<String>,
    entries: Vec<Entry>,
    selected: usize,
    error: Option<String>,
}

impl Browse {
    /// Read a directory (read-only). `select` picks an entry by name, e.g. the folder we
    /// just came back from.
    fn open(folder: &FolderSettings, sub: Vec<String>, select: Option<&str>) -> Self {
        let root = PathBuf::from(&folder.path);
        let (entries, error) = match library::list(&root, &sub, &Filter::from(folder)) {
            Ok(entries) => (entries, None),
            Err(e) => {
                log::warn!("cannot list folder {}: {e}", folder.id);
                (Vec::new(), Some(e))
            }
        };
        let selected = select
            .and_then(|name| entries.iter().position(|e| e.name == name))
            .unwrap_or(0);
        Self {
            folder_id: folder.id.clone(),
            sub,
            entries,
            selected,
            error,
        }
    }
}

/// Keys pressed this frame.
struct Keys {
    enter: bool,
    back: bool,
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    page_up: bool,
    page_down: bool,
    quit: bool,
}

impl Keys {
    fn read(ctx: &egui::Context) -> Self {
        ctx.input(|i| Keys {
            enter: i.key_pressed(Key::Enter) || i.key_pressed(Key::Space),
            back: i.key_pressed(Key::Escape) || i.key_pressed(Key::Backspace),
            up: i.key_pressed(Key::ArrowUp),
            down: i.key_pressed(Key::ArrowDown),
            left: i.key_pressed(Key::ArrowLeft),
            right: i.key_pressed(Key::ArrowRight),
            page_up: i.key_pressed(Key::PageUp),
            page_down: i.key_pressed(Key::PageDown),
            quit: i.modifiers.ctrl && i.key_pressed(Key::Q),
        })
    }

    /// For vertical lists: up/left = previous, down/right = next.
    fn prev(&self) -> bool {
        self.up || self.left
    }
    fn next(&self) -> bool {
        self.down || self.right
    }
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
    /// Focused tile on the home screen: (row, column). Row 0: discs, row 1: folders.
    focus: (usize, usize),
    /// Horizontal scroll of the folders row.
    folder_scroll: f32,
    /// Whether each folder (by id) can be read; refreshed every few seconds.
    folder_ok: HashMap<String, bool>,
    folder_checked: Option<Instant>,
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
            focus: (0, 0),
            folder_scroll: 0.0,
            folder_ok: HashMap::new(),
            folder_checked: None,
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
                Err(e) => message(e),
            };
        }
        // Folder availability (read-only metadata check), every 5 s.
        if self
            .folder_checked
            .is_none_or(|t| t.elapsed() > Duration::from_secs(5))
        {
            let (config, _) = self.config();
            self.folder_ok = config
                .folders
                .iter()
                .map(|f| {
                    let ok = std::fs::metadata(&f.path).is_ok_and(|m| m.is_dir())
                        && std::fs::read_dir(&f.path).is_ok();
                    (f.id.clone(), ok)
                })
                .collect();
            self.folder_checked = Some(Instant::now());
            self.ctx.request_repaint_after(Duration::from_secs(5));
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
        let mut next = None;
        if let Screen::Playing {
            child,
            started,
            back,
            disc,
        } = &mut self.screen
        {
            match child.try_wait() {
                Ok(None) => self.ctx.request_repaint_after(Duration::from_millis(500)),
                Ok(Some(status))
                    if !status.success() && started.elapsed() < Duration::from_secs(10) =>
                {
                    let text = if *disc {
                        format!(
                            "The player stopped right away ({status}).\n\
                             Is MakeMKV installed and registered? See scripts/setup.sh."
                        )
                    } else {
                        format!("This file could not be played ({status}).")
                    };
                    next = Some(Screen::Message {
                        text,
                        back: back.take(),
                    });
                }
                Ok(Some(_)) => next = Some(back.take().map_or(Screen::Home, Screen::Browse)),
                Err(e) => {
                    next = Some(Screen::Message {
                        text: format!("Lost track of the player: {e}"),
                        back: back.take(),
                    })
                }
            }
        }
        if let Some(screen) = next {
            self.screen = screen;
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
        let player_cache = store::prepare_player_cache();
        let socket = store::mpv_socket_path();
        if track.is_some() {
            store::remove(&socket); // stale socket from an earlier run
        }
        let options = MovieOptions {
            start_secs,
            ipc_socket: track.is_some().then_some(socket.as_path()),
            audio_languages: &config.discs.audio_languages,
            subtitle_languages: &config.discs.subtitle_languages,
            cache_dir: player_cache.as_deref(),
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
                    back: None,
                    disc: true,
                }
            }
            Err(e) => message(format!(
                "Could not start the player ({:?}): {e}.\nRun scripts/setup.sh to install it.",
                cmd.get_program()
            )),
        };
    }

    /// Open a file from the folder browser with the matching player/viewer.
    fn open_file(&mut self, browse: Browse, entry: Entry) {
        let files: Vec<PathBuf> = browse
            .entries
            .iter()
            .filter(|e| e.kind == entry.kind)
            .map(|e| e.path.clone())
            .collect();
        let start = files.iter().position(|p| *p == entry.path).unwrap_or(0);
        let cache = store::prepare_player_cache();
        let conf = match (entry.kind, &cache) {
            (EntryKind::Image, Some(dir)) => {
                store::write_cache_file(dir, "image-input.conf", player::IMAGE_INPUT_CONF)
            }
            _ => None,
        };
        let Some(mut cmd) =
            player::file_command(entry.kind, &files, start, cache.as_deref(), conf.as_deref())
        else {
            self.screen = Screen::Browse(browse);
            return;
        };
        log::info!("opening {}: {cmd:?}", entry.path.display());
        self.screen = match cmd.spawn() {
            Ok(child) => Screen::Playing {
                child,
                started: Instant::now(),
                back: Some(browse),
                disc: false,
            },
            Err(e) => Screen::Message {
                text: format!(
                    "Could not open \"{}\" ({:?}): {e}",
                    entry.name,
                    cmd.get_program()
                ),
                back: Some(browse),
            },
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

        let keys = Keys::read(&ctx);
        // TODO: protect leaving the media center with the PIN.
        if keys.quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if matches!(self.screen, Screen::Browse(_)) {
            self.browse_screen(ui, &painter, screen_rect, s, &keys);
        } else {
            self.home_screen(ui, &painter, screen_rect, s, &keys);
        }
    }
}

impl App {
    fn home_screen(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen_rect: Rect,
        s: f32,
        keys: &Keys,
    ) {
        let ctx = ui.ctx().clone();
        let (config, _) = self.config();
        let folders = &config.folders;
        let margin = 80.0 * s;
        let tile_size = Vec2::new(300.0 * s, 340.0 * s);
        let gap = 40.0 * s;
        let home = matches!(self.screen, Screen::Home);

        // Focus navigation between the rows and tiles.
        let row_lens = [1, folders.len()];
        let rows = if folders.is_empty() { 1 } else { 2 };
        if home {
            let (mut row, mut col) = self.focus;
            if keys.up && row > 0 {
                row -= 1;
            }
            if keys.down && row + 1 < rows {
                row += 1;
            }
            row = row.min(rows - 1);
            if keys.left {
                col = col.saturating_sub(1);
            }
            if keys.right {
                col += 1;
            }
            col = col.min(row_lens[row] - 1);
            self.focus = (row, col);
        }

        painter.text(
            screen_rect.min + Vec2::new(margin, 50.0 * s),
            Align2::LEFT_TOP,
            "familynovice",
            FontId::proportional(34.0 * s),
            TEXT_DIM,
        );

        // Row 0: discs
        let row_label = |y: f32, text: &str| {
            painter.text(
                screen_rect.min + Vec2::new(margin, y),
                Align2::LEFT_TOP,
                text,
                FontId::proportional(38.0 * s),
                TEXT,
            );
        };
        let mut activate = None;
        let discs_y = 130.0 * s;
        row_label(discs_y, "Discs");
        let tile = self.bluray_tile();
        let tile_rect = Rect::from_min_size(
            screen_rect.min + Vec2::new(margin, discs_y + 65.0 * s),
            tile_size,
        );
        let response = ui.interact(
            tile_rect,
            ui.make_persistent_id("tile-disc"),
            Sense::click(),
        );
        let focus = ctx.animate_bool_with_time(
            ui.make_persistent_id("focus-disc"),
            home && self.focus == (0, 0),
            0.15,
        );
        paint_bluray_tile(painter, tile_rect, s, &tile, focus);
        if home && response.clicked() {
            self.focus = (0, 0);
            activate = Some((0, 0));
        }

        // Row 1: folders
        if !folders.is_empty() {
            let y = discs_y + 65.0 * s + tile_size.y + 45.0 * s;
            row_label(y, "Folders");
            let step = tile_size.x + gap;
            let visible = screen_rect.width() - 2.0 * margin;
            if self.focus.0 == 1 {
                // Keep the focused tile on screen.
                let left = self.focus.1 as f32 * step;
                let right = left + tile_size.x;
                if left < self.folder_scroll {
                    self.folder_scroll = left;
                } else if right > self.folder_scroll + visible {
                    self.folder_scroll = right - visible;
                }
            }
            for (i, folder) in folders.iter().enumerate() {
                let x = margin + i as f32 * step - self.folder_scroll;
                if x + tile_size.x < 0.0 || x > screen_rect.width() {
                    continue;
                }
                let rect =
                    Rect::from_min_size(screen_rect.min + Vec2::new(x, y + 65.0 * s), tile_size);
                let available = self.folder_ok.get(&folder.id).copied().unwrap_or(false);
                let id = ui.make_persistent_id(("tile-folder", &folder.id));
                let response = ui.interact(rect, id, Sense::click());
                let focus = ctx.animate_bool_with_time(
                    ui.make_persistent_id(("focus-folder", &folder.id)),
                    home && self.focus == (1, i),
                    0.15,
                );
                let subtitle = if available {
                    folder_kinds(folder)
                } else {
                    "Not available".into()
                };
                paint_folder_tile(painter, rect, s, &folder.name, &subtitle, available, focus);
                if home && response.clicked() {
                    self.focus = (1, i);
                    activate = Some((1, i));
                }
            }
        }

        if home && keys.enter {
            activate = Some(self.focus);
        }
        if let Some((row, col)) = activate {
            match row {
                0 if tile.available => {
                    self.screen = Screen::Choose {
                        selected: 0,
                        resume: tile.disc_id.as_deref().and_then(resume::load),
                    };
                }
                1 => {
                    if let Some(folder) = folders.get(col)
                        && self.folder_ok.get(&folder.id).copied().unwrap_or(false)
                    {
                        self.screen = Screen::Browse(Browse::open(folder, Vec::new(), None));
                    }
                }
                _ => {}
            }
        }

        // Overlays
        match &self.screen {
            Screen::Home | Screen::Browse(_) => {}
            Screen::Choose { selected, resume } => {
                let (mut selected, resume) = (*selected, *resume);
                let list = choices(resume, config.discs.allow_disc_menu);
                if keys.prev() {
                    selected = selected.saturating_sub(1);
                }
                if keys.next() {
                    selected = (selected + 1).min(list.len() - 1);
                }
                let scan_ready = self.scan_done(tile.disc_key.as_ref());
                let clicked = paint_chooser(
                    ui,
                    painter,
                    screen_rect,
                    s,
                    &tile,
                    &list,
                    selected,
                    scan_ready,
                );
                let activated = clicked.or(keys.enter.then_some(selected));
                self.screen = Screen::Choose { selected, resume };
                if keys.back || !tile.available {
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
                    painter,
                    screen_rect,
                    s,
                    "Ejecting…",
                    "The tray opens in a moment.",
                );
            }
            Screen::Playing { .. } => {
                paint_overlay(
                    painter,
                    screen_rect,
                    s,
                    "Playing…",
                    "Close the player to come back.",
                );
            }
            Screen::Message { text, .. } => {
                paint_overlay(painter, screen_rect, s, "Something went wrong", text);
                if keys.enter || keys.back || ctx.input(|i| i.pointer.any_click()) {
                    let Screen::Message { back, .. } =
                        std::mem::replace(&mut self.screen, Screen::Home)
                    else {
                        unreachable!("matched Screen::Message above")
                    };
                    self.screen = back.map_or(Screen::Home, Screen::Browse);
                }
            }
        }
    }

    fn browse_screen(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen_rect: Rect,
        s: f32,
        keys: &Keys,
    ) {
        let Screen::Browse(mut b) = std::mem::replace(&mut self.screen, Screen::Home) else {
            return;
        };
        let (config, _) = self.config();
        // The folder may have been removed by the admin in the meantime.
        let Some(folder) = config.folders.iter().find(|f| f.id == b.folder_id) else {
            return; // back to the home screen
        };
        let margin = 80.0 * s;
        let top = 170.0 * s;
        let row_h = 78.0 * s;
        let visible = (((screen_rect.height() - top - 80.0 * s) / row_h).floor() as usize).max(1);
        let n = b.entries.len();

        // Navigation
        // Mouse wheel: one notch moves the selection by one entry.
        let wheel: f32 = ui.ctx().input(|i| {
            i.events
                .iter()
                .map(|e| match e {
                    egui::Event::MouseWheel { delta, .. } => delta.y,
                    _ => 0.0,
                })
                .sum()
        });
        if (keys.up || wheel > 0.0) && b.selected > 0 {
            b.selected -= 1;
        }
        if (keys.down || wheel < 0.0) && b.selected + 1 < n {
            b.selected += 1;
        }
        if keys.page_up {
            b.selected = b.selected.saturating_sub(visible);
        }
        if keys.page_down && n > 0 {
            b.selected = (b.selected + visible).min(n - 1);
        }

        // Header: folder name › subfolders
        let mut crumbs = vec![folder.name.as_str()];
        crumbs.extend(b.sub.iter().map(String::as_str));
        painter.text(
            screen_rect.min + Vec2::new(margin, 60.0 * s),
            Align2::LEFT_TOP,
            elide(&crumbs.join("  ›  "), 70),
            FontId::proportional(42.0 * s),
            TEXT,
        );
        if n > 0 {
            painter.text(
                Pos2::new(screen_rect.max.x - margin, screen_rect.min.y + 72.0 * s),
                Align2::RIGHT_TOP,
                format!("{} / {n}", b.selected + 1),
                FontId::proportional(26.0 * s),
                TEXT_DIM,
            );
        }

        let mut activate = keys.enter;
        if let Some(error) = &b.error {
            paint_overlay(
                painter,
                screen_rect,
                s,
                "This folder can't be opened right now",
                &elide(error, 90),
            );
        } else if n == 0 {
            painter.text(
                screen_rect.center(),
                Align2::CENTER_CENTER,
                "Nothing to show here",
                FontId::proportional(32.0 * s),
                TEXT_DIM,
            );
        }

        let first = b
            .selected
            .saturating_sub(visible / 2)
            .min(n.saturating_sub(visible));
        for (i, entry) in b.entries.iter().enumerate().skip(first).take(visible) {
            let y = top + (i - first) as f32 * row_h;
            let rect = Rect::from_min_size(
                Pos2::new(screen_rect.min.x + margin, screen_rect.min.y + y),
                Vec2::new(screen_rect.width() - 2.0 * margin, row_h - 10.0 * s),
            );
            let selected = i == b.selected;
            painter.rect_filled(
                rect,
                CornerRadius::same((12.0 * s) as u8),
                if selected { ACCENT } else { TILE_BG },
            );
            let icon_center = Pos2::new(rect.min.x + 45.0 * s, rect.center().y);
            paint_entry_icon(painter, entry.kind, icon_center, 20.0 * s);
            painter.text(
                Pos2::new(rect.min.x + 90.0 * s, rect.center().y),
                Align2::LEFT_CENTER,
                elide(&entry.name, 80),
                FontId::proportional(30.0 * s),
                TEXT,
            );
            if entry.kind == EntryKind::Folder {
                painter.text(
                    Pos2::new(rect.max.x - 30.0 * s, rect.center().y),
                    Align2::RIGHT_CENTER,
                    "›",
                    FontId::proportional(36.0 * s),
                    if selected { TEXT } else { TEXT_DIM },
                );
            }
            let id = ui.make_persistent_id(("entry", i));
            if ui.interact(rect, id, Sense::click()).clicked() {
                b.selected = i;
                activate = true;
            }
        }

        painter.text(
            Pos2::new(screen_rect.min.x + margin, screen_rect.max.y - 50.0 * s),
            Align2::LEFT_CENTER,
            "Enter: open     Esc / Backspace: back",
            FontId::proportional(22.0 * s),
            TEXT_DIM,
        );

        if keys.back {
            match b.sub.pop() {
                None => return, // leave the folder: home screen
                Some(last) => b = Browse::open(folder, b.sub, Some(&last)),
            }
        } else if activate && let Some(entry) = b.entries.get(b.selected).cloned() {
            if entry.kind == EntryKind::Folder {
                let mut sub = b.sub.clone();
                sub.push(entry.name);
                b = Browse::open(folder, sub, None);
            } else {
                self.open_file(b, entry);
                return;
            }
        }
        self.screen = Screen::Browse(b);
    }
}

/// "Videos · Pictures · PDF" for a folder tile.
fn folder_kinds(folder: &FolderSettings) -> String {
    let mut kinds: Vec<String> = Vec::new();
    for (on, name) in [
        (folder.video, "Videos"),
        (folder.audio, "Music"),
        (folder.images, "Pictures"),
    ] {
        if on {
            kinds.push(name.into());
        }
    }
    kinds.extend(folder.extensions.iter().map(|e| e.to_uppercase()));
    kinds.join(" · ")
}

fn paint_folder_tile(
    painter: &egui::Painter,
    rect: Rect,
    s: f32,
    title: &str,
    subtitle: &str,
    available: bool,
    focus: f32,
) {
    let alpha = if available { 1.0 } else { UNAVAILABLE_ALPHA };
    let c = |col: Color32| col.gamma_multiply(alpha);
    let rect = rect.expand(focus * 12.0 * s);
    let radius = CornerRadius::same((18.0 * s) as u8);
    painter.rect_filled(rect, radius, c(TILE_BG));
    if focus > 0.0 {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(4.0 * s, ACCENT.gamma_multiply(focus)),
            StrokeKind::Outside,
        );
    }
    let center = Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.38);
    paint_folder_icon(painter, center, rect.width() * 0.62, alpha);
    painter.text(
        Pos2::new(rect.center().x, rect.max.y - 110.0 * s),
        Align2::CENTER_TOP,
        elide(title, 20),
        FontId::proportional(30.0 * s),
        c(TEXT),
    );
    painter.text(
        Pos2::new(rect.center().x, rect.max.y - 65.0 * s),
        Align2::CENTER_TOP,
        elide(subtitle, 30),
        FontId::proportional(20.0 * s),
        c(TEXT_DIM),
    );
}

/// A folder: tab plus body, `width` wide, centered on `center`.
fn paint_folder_icon(painter: &egui::Painter, center: Pos2, width: f32, alpha: f32) {
    let c = |col: Color32| col.gamma_multiply(alpha);
    let h = width * 0.72;
    let body = Rect::from_center_size(
        center + Vec2::new(0.0, h * 0.08),
        Vec2::new(width, h * 0.84),
    );
    let tab = Rect::from_min_size(
        Pos2::new(body.min.x, body.min.y - h * 0.14),
        Vec2::new(width * 0.42, h * 0.24),
    );
    let r = CornerRadius::same((width * 0.06) as u8);
    painter.rect_filled(tab, r, c(Color32::from_rgb(0xd9, 0x9a, 0x2b)));
    painter.rect_filled(body, r, c(Color32::from_rgb(0xf2, 0xb5, 0x44)));
    painter.rect_filled(
        Rect::from_min_size(body.min, Vec2::new(body.width(), body.height() * 0.18)),
        r,
        c(Color32::from_rgb(0xf6, 0xc6, 0x66)),
    );
}

/// Small icon for a browser entry, `r` is roughly the half size.
fn paint_entry_icon(painter: &egui::Painter, kind: EntryKind, center: Pos2, r: f32) {
    let stroke = Stroke::new((r * 0.12).max(1.5), TEXT);
    match kind {
        EntryKind::Folder => paint_folder_icon(painter, center, r * 2.0, 1.0),
        EntryKind::Video => {
            let frame = Rect::from_center_size(center, Vec2::new(r * 2.0, r * 1.5));
            painter.rect_stroke(
                frame,
                CornerRadius::same((r * 0.2) as u8),
                stroke,
                StrokeKind::Middle,
            );
            let tri = vec![
                center + Vec2::new(-r * 0.3, -r * 0.4),
                center + Vec2::new(r * 0.45, 0.0),
                center + Vec2::new(-r * 0.3, r * 0.4),
            ];
            painter.add(egui::Shape::convex_polygon(tri, TEXT, Stroke::NONE));
        }
        EntryKind::Audio => {
            let head = center + Vec2::new(-r * 0.35, r * 0.55);
            painter.circle_filled(head, r * 0.35, TEXT);
            let top = center + Vec2::new(r * 0.0, -r * 0.8);
            painter.line_segment([head + Vec2::new(r * 0.33, 0.0), top], stroke);
            painter.line_segment([top, top + Vec2::new(r * 0.55, r * 0.3)], stroke);
        }
        EntryKind::Image => {
            let frame = Rect::from_center_size(center, Vec2::new(r * 2.0, r * 1.6));
            painter.rect_stroke(
                frame,
                CornerRadius::same((r * 0.15) as u8),
                stroke,
                StrokeKind::Middle,
            );
            painter.circle_filled(center + Vec2::new(r * 0.45, -r * 0.35), r * 0.2, TEXT);
            let hill = vec![
                Pos2::new(frame.min.x + r * 0.2, frame.max.y - r * 0.2),
                Pos2::new(center.x - r * 0.2, center.y - r * 0.2),
                Pos2::new(center.x + r * 0.5, frame.max.y - r * 0.2),
            ];
            painter.add(egui::Shape::convex_polygon(hill, TEXT, Stroke::NONE));
        }
        EntryKind::Other => {
            let page = Rect::from_center_size(center, Vec2::new(r * 1.5, r * 2.0));
            painter.rect_stroke(
                page,
                CornerRadius::same((r * 0.12) as u8),
                stroke,
                StrokeKind::Middle,
            );
            for i in 0..3 {
                let y = page.min.y + r * (0.6 + i as f32 * 0.4);
                painter.line_segment(
                    [
                        Pos2::new(page.min.x + r * 0.3, y),
                        Pos2::new(page.max.x - r * 0.3, y),
                    ],
                    stroke,
                );
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
