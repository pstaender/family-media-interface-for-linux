//! The home screen ("front row"): rows of large tiles, controlled with keyboard/remote or mouse.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Child;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

use crate::audio::AudioPlayer;
use crate::config::{Config, FolderSettings};
use crate::control::SharedConfig;
use crate::control::UiRequest;
use crate::disc::{self, DriveReport, DriveState, MediaKind};
use crate::library::{self, Entry, EntryKind, Filter};
use crate::limits::{self, Status};
use crate::lock::{LockAction, LockInfo, LockView, Nav};
use crate::makemkv::{self, DiscInfo, ScanProgress};
use crate::mpris::{MediaCommand, Mpris, Status as PlayStatus};
use crate::pictures::{self, Picture};
use crate::player::{self, MovieOptions, PlayMode};
use crate::recent;
use crate::remote::{self, Action, Calibration, RemoteMap};
use crate::tr;
use crate::usage::SharedTracker;
use crate::volume::{self, Volume, VolumeCommand};
use crate::{cover, resume, sites, store};

/// How long the remote's home button must be held to go to the home screen.
const HOME_HOLD: Duration = Duration::from_secs(3);
const BG: Color32 = Color32::from_rgb(0x12, 0x14, 0x1c);
const TILE_BG: Color32 = Color32::from_rgb(0x22, 0x26, 0x34);
pub const TEXT: Color32 = Color32::from_rgb(0xf2, 0xf4, 0xf8);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x9a, 0xa1, 0xb3);
pub const ACCENT: Color32 = Color32::from_rgb(0x3d, 0x8b, 0xff);
/// Tile text: fixed sizes (never shrunk to fit; long text gets "…" instead), and their
/// distance from the tile's bottom edge.
const TILE_TITLE_FONT: f32 = 32.0;
const TILE_SUBTITLE_FONT: f32 = 24.0;
const TILE_TITLE_Y: f32 = 122.0;
const TILE_SUBTITLE_Y: f32 = 76.0;

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
            ScanState::Running { progress: None, .. } => Some((tr!("Reading disc"), 0.0)),
            _ => None,
        }
    }
}

enum ScanEvent {
    Progress(String, ScanProgress),
    Done(String, Result<DiscInfo, String>),
}

enum Screen {
    /// Learning the remote's buttons (started from the admin panel).
    Calibrate(Box<CalibrateView>),
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
    /// Built-in picture viewer.
    Picture(PictureView),
    /// Built-in music player.
    Music(MusicView),
    /// A video that was played before: resume or start over?
    VideoChoose {
        back: Browse,
        entry: Entry,
        position: f64,
        selected: usize,
    },
    Playing {
        child: Child,
        started: Instant,
        /// Where to return when the player exits (`None`: home screen).
        back: Option<Browse>,
        /// What is playing (for the error hint).
        kind: PlayKind,
        /// When the user asked to stop it (Esc / back): SIGTERM sent, SIGKILL after 2 s.
        stopping: Option<Instant>,
    },
    Message {
        text: String,
        back: Option<Browse>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlayKind {
    Disc,
    File,
    Website,
}

/// The rows of the home screen (only rows with content are shown, except discs).
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Discs = 0,
    Folders = 1,
    Websites = 2,
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
    /// Only on the folder's first page: the last played files, after `entries`.
    recent: Vec<RecentEntry>,
    /// Index into `entries`, then `recent`.
    selected: usize,
    error: Option<String>,
}

/// A recently played file, checked again when the folder page opens.
#[derive(Clone)]
struct RecentEntry {
    /// Its folder below the configured folder.
    sub: Vec<String>,
    entry: Entry,
}

/// The last played files of a folder that still exist and are still allowed.
fn recent_entries(folder: &FolderSettings) -> Vec<RecentEntry> {
    let root = PathBuf::from(&folder.path);
    let filter = Filter::from(folder);
    recent::Recent::load()
        .items(&folder.id)
        .iter()
        .filter_map(|item| {
            let entry = library::file_entry(&root, &item.sub, &item.name, &filter)?;
            matches!(entry.kind, EntryKind::Video | EntryKind::Audio).then(|| RecentEntry {
                sub: item.sub.clone(),
                entry,
            })
        })
        .take(recent::SHOWN)
        .collect()
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
        let recent = if sub.is_empty() && error.is_none() {
            recent_entries(folder)
        } else {
            Vec::new()
        };
        Self {
            folder_id: folder.id.clone(),
            sub,
            entries,
            recent,
            selected,
            error,
        }
    }
}

struct PictureView {
    back: Browse,
    /// The folder's pictures.
    files: Vec<Entry>,
    index: usize,
    texture: Option<egui::TextureHandle>,
    /// Index of the picture in `texture`.
    shown: Option<usize>,
    /// Index of the picture being loaded.
    requested: Option<usize>,
    error: Option<String>,
}

struct MusicView {
    back: Browse,
    player: AudioPlayer,
    /// The folder's music files; playback started at `start`.
    tracks: Vec<Entry>,
    start: usize,
    /// The configured folder (cover files must be inside it).
    root: PathBuf,
    /// Cover art of the current track (embedded or a cover file in its folder).
    cover: Option<(cover::Source, egui::TextureHandle)>,
    /// The track whose cover was asked for last, and where the answer arrives.
    cover_track: Option<PathBuf>,
    cover_rx: Option<Receiver<Option<cover::Cover>>>,
    /// The track last put on the recently played list.
    recorded: Option<PathBuf>,
}

impl MusicView {
    /// Show the cover of `track`: ask a thread for it when the track changes, take the
    /// answer when it's there. The old cover stays until then (no flicker).
    fn update_cover(&mut self, ctx: &egui::Context, track: Option<&std::path::Path>) {
        if let Some(result) = self.cover_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.cover_rx = None;
            self.cover = match result {
                None => None,
                Some(cover::Cover {
                    source,
                    picture: Some(p),
                }) => {
                    let image =
                        egui::ColorImage::from_rgba_unmultiplied([p.width, p.height], &p.rgba);
                    let texture = ctx.load_texture("cover", image, egui::TextureOptions::LINEAR);
                    Some((source, texture))
                }
                // Same folder image as before.
                Some(cover::Cover { picture: None, .. }) => self.cover.take(),
            };
        }
        let Some(track) = track else { return };
        if self.cover_track.as_deref() == Some(track) {
            return;
        }
        self.cover_track = Some(track.to_path_buf());
        let (tx, rx) = mpsc::channel();
        self.cover_rx = Some(rx); // an older request's answer goes nowhere
        let (track, root, ctx) = (track.to_path_buf(), self.root.clone(), ctx.clone());
        let current = self.cover.as_ref().map(|(source, _)| source.clone());
        std::thread::spawn(move || {
            let _ = tx.send(cover::load(&track, &root, current.as_ref(), 640));
            ctx.request_repaint();
        });
    }
}

/// Select the entry with this path (when coming back to the browser).
fn select(mut browse: Browse, path: &std::path::Path) -> Browse {
    if let Some(i) = browse.entries.iter().position(|e| e.path == path) {
        browse.selected = i;
    }
    browse
}

struct CalibrateView {
    calibration: Calibration,
    /// Set when all buttons are through: the summary shows until a press or 20 s.
    done: Option<Instant>,
}

/// Channels from the background threads into the UI.
pub struct Inputs {
    /// Keys of remotes read directly from their device (`remote::spawn`).
    pub remote: Receiver<remote::KeyEvent>,
    /// Requests from the admin interface (control socket).
    pub requests: Receiver<UiRequest>,
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
    /// Media keys (next, previous, play/pause, stop): from the window or via MPRIS.
    media: Vec<MediaCommand>,
    /// Volume keys the window got (built-in `AudioVolume*` or learned keys).
    volume: Vec<VolumeCommand>,
}

impl Keys {
    fn read(ctx: &egui::Context) -> Self {
        ctx.input(|i| Keys {
            enter: i.key_pressed(Key::Enter) || i.key_pressed(Key::Space),
            // Right click: the "return" button of air-mouse remotes.
            back: i.key_pressed(Key::Escape)
                || i.key_pressed(Key::Backspace)
                || i.key_pressed(Key::BrowserBack)
                || i.pointer.secondary_clicked(),
            up: i.key_pressed(Key::ArrowUp),
            down: i.key_pressed(Key::ArrowDown),
            left: i.key_pressed(Key::ArrowLeft),
            right: i.key_pressed(Key::ArrowRight),
            page_up: i.key_pressed(Key::PageUp),
            page_down: i.key_pressed(Key::PageDown),
            quit: i.modifiers.ctrl && i.key_pressed(Key::Q),
            media: [
                (Key::MediaTrackNext, MediaCommand::Next),
                (Key::MediaTrackPrevious, MediaCommand::Previous),
                (Key::MediaPlayPause, MediaCommand::PlayPause),
                (Key::MediaStop, MediaCommand::Stop),
            ]
            .into_iter()
            .filter(|(key, _)| i.key_pressed(*key))
            .map(|(_, command)| command)
            .collect(),
            // Up/down repeat while held; mute doesn't.
            volume: i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat,
                        ..
                    } => match key {
                        Key::AudioVolumeUp => Some(VolumeCommand::Up),
                        Key::AudioVolumeDown => Some(VolumeCommand::Down),
                        Key::AudioVolumeMute if !repeat => Some(VolumeCommand::Mute),
                        _ => None,
                    },
                    _ => None,
                })
                .collect(),
        })
    }

    fn media(&self, command: MediaCommand) -> bool {
        self.media.contains(&command)
    }

    /// A remote button learned in the calibration (or read from the device).
    fn apply(&mut self, action: Action) {
        match action {
            Action::Ok => self.enter = true,
            Action::Back => self.back = true,
            Action::Up => self.up = true,
            Action::Down => self.down = true,
            Action::Left => self.left = true,
            Action::Right => self.right = true,
            Action::PlayPause => self.media.push(MediaCommand::PlayPause),
            Action::Next => self.media.push(MediaCommand::Next),
            Action::Previous => self.media.push(MediaCommand::Previous),
            Action::VolumeUp | Action::VolumeDown | Action::Mute => {
                // A learned key may be a built-in one too (AudioVolumeUp): count it once.
                if let Some(command) = action.volume()
                    && !self.volume.contains(&command)
                {
                    self.volume.push(command);
                }
            }
            Action::Home => {} // held, see App::home_button
        }
    }

    /// For vertical lists: up/left = previous, down/right = next.
    fn prev(&self) -> bool {
        self.up || self.left
    }
    fn next(&self) -> bool {
        self.down || self.right
    }
}

/// "Volume 45 %", "Muted" or that there is no volume control.
fn volume_text(shown: volume::Shown) -> String {
    match shown {
        volume::Shown::Level(l) if l.muted => tr!("Muted").into(),
        volume::Shown::Level(l) => tr!("Volume {percent} %", percent = l.percent),
        volume::Shown::Unavailable => tr!("No volume control found").into(),
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
    /// Media keys from the desktop (MPRIS); also tells the desktop whether something plays.
    mpris: Mpris,
    media_rx: Receiver<MediaCommand>,
    inputs: Inputs,
    /// The remote's learned buttons (`remote.json`).
    remote_map: RemoteMap,
    /// Device key codes currently held down (for the home button).
    remote_down: std::collections::BTreeSet<u16>,
    /// The admin panel asked for the calibration; starts on the home screen.
    calibrate_requested: bool,
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
    /// The lock button (top right) has the focus instead of a tile.
    lock_button_focused: bool,
    /// Horizontal scroll per row kind, vertical scroll of the rows.
    row_scroll_x: [f32; 3],
    rows_scroll_y: f32,
    /// The browser window program (`familynovice-web`), if installed.
    web_helper: Option<PathBuf>,
    /// Whether each folder (by id) can be read; refreshed every few seconds.
    folder_ok: HashMap<String, bool>,
    folder_checked: Option<Instant>,
    /// Since when the remote's home button is held; `home_done` once it went home (until
    /// the button is released).
    home_since: Option<Instant>,
    home_done: bool,
    /// Website logos by website id: file time of the loaded logo and its texture.
    logos: HashMap<String, (std::time::SystemTime, Option<egui::TextureHandle>)>,
    logo_tx: Sender<(String, std::time::SystemTime, Result<Picture, String>)>,
    logo_rx: Receiver<(String, std::time::SystemTime, Result<Picture, String>)>,
    picture_tx: Sender<(PathBuf, Result<Picture, String>)>,
    picture_rx: Receiver<(PathBuf, Result<Picture, String>)>,
    /// Whether our window is currently pushed below a running player.
    window_below_player: bool,
    /// Usage time and lock state (shared with the control socket).
    tracker: SharedTracker,
    /// The lock screen, while locked.
    lock: Option<LockView>,
    last_tick: Instant,
    /// Fractions of a second not yet added to the usage.
    usage_carry: f64,
    last_input: Instant,
    pin_tx: Sender<bool>,
    pin_rx: Receiver<bool>,
    /// System volume (remote's volume buttons) and the last change already passed to mpv.
    volume: Volume,
    volume_seen: Option<Instant>,
    /// A desktop handles the standard volume keys (not in the kiosk session).
    desktop_volume: bool,
}

impl App {
    pub fn new(
        ctx: egui::Context,
        config: SharedConfig,
        tracker: SharedTracker,
        inputs: Inputs,
    ) -> Self {
        let (drives_tx, drives_rx) = mpsc::channel();
        let repaint = ctx.clone();
        disc::spawn_watcher(Duration::from_secs(2), drives_tx, move || {
            repaint.request_repaint()
        });
        let (scan_tx, scan_rx) = mpsc::channel();
        let (eject_tx, eject_rx) = mpsc::channel();
        let (picture_tx, picture_rx) = mpsc::channel();
        let (pin_tx, pin_rx) = mpsc::channel();
        let (media_tx, media_rx) = mpsc::channel();
        let (logo_tx, logo_rx) = mpsc::channel();
        let repaint = ctx.clone();
        let mpris = Mpris::start(media_tx, move || repaint.request_repaint());
        let repaint = ctx.clone();
        let volume = Volume::spawn(move || repaint.request_repaint());
        let desktop_volume = volume::desktop_handles_keys();
        log::info!(
            "volume keys: {}",
            if desktop_volume {
                "handled by the desktop (learned non-standard buttons: by us)"
            } else {
                "handled by the media center"
            }
        );
        Self {
            ctx,
            mpris,
            media_rx,
            inputs,
            remote_map: RemoteMap::load(),
            remote_down: Default::default(),
            calibrate_requested: false,
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
            lock_button_focused: false,
            row_scroll_x: [0.0; 3],
            rows_scroll_y: 0.0,
            web_helper: find_web_helper(),
            folder_ok: HashMap::new(),
            folder_checked: None,
            home_since: None,
            home_done: false,
            logos: HashMap::new(),
            logo_tx,
            logo_rx,
            picture_tx,
            picture_rx,
            window_below_player: false,
            tracker,
            lock: None,
            last_tick: Instant::now(),
            usage_carry: 0.0,
            last_input: Instant::now(),
            pin_tx,
            pin_rx,
            volume,
            volume_seen: None,
            desktop_volume,
        }
    }

    /// The current config (may change at any time through a reload).
    fn config(&self) -> (Config, Option<String>) {
        let state = self.config.lock().unwrap_or_else(|p| p.into_inner());
        (state.config.clone(), state.error.clone())
    }

    /// Pull updates from background threads.
    /// The remote calibration: asks for one button after the other (see `remote::Calibration`).
    fn calibrate_screen(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        screen_rect: Rect,
        s: f32,
        remote_events: &[remote::KeyEvent],
        desktop_media: &[MediaCommand],
    ) {
        let Screen::Calibrate(view) = &mut self.screen else {
            return;
        };
        let now = Instant::now();
        let mut window_presses: Vec<String> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        ..
                    } => Some(format!("key:{}", key.name())),
                    egui::Event::PointerButton {
                        button,
                        pressed: true,
                        ..
                    } => Some(format!("mouse:{button:?}").to_lowercase()),
                    _ => None,
                })
                .collect()
        });
        // Desktops grab media keys (they never reach the window) and pass them on via MPRIS.
        window_presses.extend(
            desktop_media
                .iter()
                .map(|c| format!("desktop:{c:?}").to_lowercase()),
        );
        let center = screen_rect.center();
        let font = |size: f32| FontId::proportional(size * s);
        painter.text(
            Pos2::new(center.x, screen_rect.min.y + 110.0 * s),
            Align2::CENTER_CENTER,
            tr!("Remote setup"),
            font(52.0),
            TEXT,
        );

        if let Some(done) = view.done {
            // Summary; any button (or 20 s) closes it.
            let mut lines = vec![
                tr!("Saved. Your remote's buttons:").to_string(),
                String::new(),
            ];
            for action in Action::ALL {
                lines.push(format!(
                    "{}:  {}",
                    crate::i18n::tr(action.label()),
                    view.calibration.map.describe_with(action, crate::i18n::tr)
                ));
            }
            painter.text(
                center,
                Align2::CENTER_CENTER,
                lines.join("\n"),
                font(26.0),
                TEXT,
            );
            painter.text(
                Pos2::new(center.x, screen_rect.max.y - 70.0 * s),
                Align2::CENTER_CENTER,
                tr!("Press any button to continue"),
                font(24.0),
                TEXT_DIM,
            );
            let pressed = !window_presses.is_empty() || remote_events.iter().any(|e| e.value == 1);
            if done.elapsed() > Duration::from_secs(20)
                || (done.elapsed() > Duration::from_secs(1) && pressed)
            {
                self.screen = Screen::Home;
            }
            ctx.request_repaint_after(Duration::from_millis(500));
            return;
        }

        for e in remote_events.iter().filter(|e| e.value == 1) {
            view.calibration.device_press(e.code, now);
        }
        for w in window_presses {
            view.calibration.window_press(w, now);
        }
        if view.calibration.tick(now) {
            let map = view.calibration.map.clone();
            if let Err(e) = map.save() {
                log::warn!("could not save the remote's buttons: {e}");
            }
            log::info!("remote calibrated: {map:?}");
            self.remote_map = map;
            view.done = Some(now);
            ctx.request_repaint();
            return;
        }
        let Some(action) = view.calibration.current() else {
            return;
        };
        // In mouse mode many remotes send other codes (clicks, desktop media keys).
        painter.text(
            Pos2::new(center.x, screen_rect.min.y + 175.0 * s),
            Align2::CENTER_CENTER,
            tr!("Switch the remote's mouse / pointer mode off first."),
            font(28.0),
            Color32::from_rgb(0xff, 0xb4, 0x5c),
        );
        painter.text(
            Pos2::new(center.x, center.y - 150.0 * s),
            Align2::CENTER_CENTER,
            tr!(
                "Button {n} of {total}",
                n = view.calibration.step() + 1,
                total = Action::ALL.len()
            ),
            font(26.0),
            TEXT_DIM,
        );
        painter.text(
            Pos2::new(center.x, center.y - 60.0 * s),
            Align2::CENTER_CENTER,
            tr!("Press this button on the remote:"),
            font(32.0),
            TEXT,
        );
        painter.text(
            Pos2::new(center.x, center.y + 20.0 * s),
            Align2::CENTER_CENTER,
            crate::i18n::tr(action.label()),
            font(72.0),
            ACCENT,
        );
        painter.text(
            Pos2::new(center.x, center.y + 110.0 * s),
            Align2::CENTER_CENTER,
            tr!(
                "No such button? It's skipped in {secs} s.",
                secs = view.calibration.seconds_left(now)
            ),
            font(24.0),
            TEXT_DIM,
        );
        if let Some(message) = &view.calibration.message {
            painter.text(
                Pos2::new(center.x, center.y + 170.0 * s),
                Align2::CENTER_CENTER,
                message,
                font(26.0),
                Color32::from_rgb(0xff, 0xb4, 0x5c),
            );
        }
        let learned: Vec<String> = Action::ALL
            .iter()
            .take(view.calibration.step())
            .map(|a| match view.calibration.map.buttons.get(a) {
                Some(_) => format!("{} ✓", crate::i18n::tr(a.label())),
                None => format!("{} –", crate::i18n::tr(a.label())),
            })
            .collect();
        painter.text(
            Pos2::new(center.x, screen_rect.max.y - 70.0 * s),
            Align2::CENTER_CENTER,
            learned.join("    "),
            font(22.0),
            TEXT_DIM,
        );
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    /// Holding the remote's home button (XF86HomePage) for 3 s stops what is playing and
    /// shows the home screen. While a player or website is in front, they handle it
    /// themselves (`player::MPV_HOME_SCRIPT`, `familynovice-web`).
    fn home_button(&mut self, ui: &egui::Ui, screen_rect: Rect, s: f32) {
        let ctx = ui.ctx();
        let window_home = ctx.input(|i| {
            i.key_down(Key::BrowserHome)
                || self.remote_map.window_keys().any(|(name, action)| {
                    action == Action::Home && Key::from_name(name).is_some_and(|k| i.key_down(k))
                })
        });
        let device_home = self.remote_down.iter().any(|c| self.remote_map.is_home(*c));
        if !window_home && !device_home {
            self.home_since = None;
            self.home_done = false;
            return;
        }
        if self.home_done {
            return;
        }
        let since = *self.home_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= HOME_HOLD {
            self.home_done = true;
            self.go_home();
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            ui.make_persistent_id("home-hint"),
        ));
        let rect = Rect::from_center_size(
            Pos2::new(screen_rect.center().x, screen_rect.max.y - 120.0 * s),
            Vec2::new(620.0 * s, 64.0 * s),
        );
        painter.rect_filled(
            rect,
            CornerRadius::same((14.0 * s) as u8),
            Color32::from_black_alpha(210),
        );
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            tr!("Keep holding to go to the home screen…"),
            FontId::proportional(26.0 * s),
            TEXT,
        );
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    /// The remote's volume buttons change the system volume (`volume`): from the window
    /// while it's in front, from the device while one of our players or websites is (they
    /// don't change the system volume themselves). Not during the calibration.
    fn volume_keys(&mut self, ctx: &egui::Context, keys: &Keys, events: &[remote::KeyEvent]) {
        if matches!(self.screen, Screen::Calibrate(_)) {
            return;
        }
        let focused = ctx.input(|i| i.focused);
        let mut commands = keys.volume.clone();
        // Not focused and nothing of ours in front: someone uses the desktop (development).
        if focused || matches!(self.screen, Screen::Playing { .. }) {
            for e in events.iter().filter(|e| e.value == 1 || e.value == 2) {
                match self
                    .remote_map
                    .device_volume(e.code, focused, self.desktop_volume)
                {
                    Some(VolumeCommand::Mute) if e.value == 2 => {}
                    Some(command) => commands.push(command),
                    None => {}
                }
            }
        }
        for command in commands {
            self.volume.send(command);
        }
        // A new level: show it in mpv too (in front of our window during a video).
        let Some((shown, at)) = self.volume.shown() else {
            return;
        };
        if self.volume_seen == Some(at) {
            return;
        }
        self.volume_seen = Some(at);
        if let Screen::Playing {
            kind,
            stopping: None,
            ..
        } = &self.screen
            && *kind != PlayKind::Website
        {
            player::mpv_show_text(&store::mpv_socket_path(), &volume_text(shown));
        }
    }

    /// The volume bar (bottom center) for 2 s after a change.
    fn paint_volume(&self, ui: &egui::Ui, screen_rect: Rect, s: f32) {
        const SHOW: Duration = Duration::from_secs(2);
        let Some((shown, at)) = self.volume.shown() else {
            return;
        };
        let age = at.elapsed();
        if age >= SHOW {
            return;
        }
        let ctx = ui.ctx();
        ctx.request_repaint_after(SHOW - age);
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            ui.make_persistent_id("volume"),
        ));
        let rect = Rect::from_center_size(
            Pos2::new(screen_rect.center().x, screen_rect.max.y - 220.0 * s),
            Vec2::new(560.0 * s, 110.0 * s),
        );
        painter.rect_filled(
            rect,
            CornerRadius::same((16.0 * s) as u8),
            Color32::from_black_alpha(220),
        );
        painter.text(
            Pos2::new(rect.center().x, rect.min.y + 32.0 * s),
            Align2::CENTER_CENTER,
            volume_text(shown),
            FontId::proportional(28.0 * s),
            TEXT,
        );
        if let volume::Shown::Level(level) = shown {
            let bar = Rect::from_min_size(
                Pos2::new(rect.min.x + 36.0 * s, rect.max.y - 38.0 * s),
                Vec2::new(rect.width() - 72.0 * s, 14.0 * s),
            );
            let radius = CornerRadius::same((7.0 * s) as u8);
            painter.rect_filled(bar, radius, Color32::from_white_alpha(40));
            let mut filled = bar;
            filled.set_width(bar.width() * level.percent.min(100) as f32 / 100.0);
            let color = if level.muted { TEXT_DIM } else { ACCENT };
            painter.rect_filled(filled, radius, color);
        }
    }

    /// Stop whatever runs (music, player, website) and show the home screen.
    fn go_home(&mut self) {
        self.screen = match std::mem::replace(&mut self.screen, Screen::Home) {
            Screen::Music(mut view) => {
                view.player.stop();
                Screen::Home
            }
            // Stop the player like Esc does; once it's gone, the home screen follows.
            Screen::Playing {
                child,
                started,
                kind,
                stopping,
                ..
            } => {
                if stopping.is_none() {
                    stop_player(child.id());
                }
                Screen::Playing {
                    child,
                    started,
                    back: None,
                    kind,
                    stopping: Some(stopping.unwrap_or_else(Instant::now)),
                }
            }
            _ => Screen::Home,
        };
    }

    /// Load website logos that are new or changed (the admin interface writes them), drop
    /// those that are gone or turned off. Read-only; decoding happens in a thread.
    fn check_logos(&mut self, config: &Config) {
        let wanted: Vec<_> = config.websites.iter().filter(|w| w.show_logo).collect();
        self.logos
            .retain(|id, _| wanted.iter().any(|w| &w.id == id));
        for site in wanted {
            let Some(path) = store::logo_path(&site.id) else {
                continue;
            };
            let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
                self.logos.remove(&site.id);
                continue;
            };
            if self
                .logos
                .get(&site.id)
                .is_some_and(|(t, _)| *t == modified)
            {
                continue;
            }
            // Remember the time now, so the file isn't decoded twice; keep the old texture
            // until the new one is there.
            let old = self.logos.remove(&site.id).and_then(|(_, t)| t);
            self.logos.insert(site.id.clone(), (modified, old));
            let (tx, id, ctx) = (self.logo_tx.clone(), site.id.clone(), self.ctx.clone());
            std::thread::spawn(move || {
                let _ = tx.send((id, modified, pictures::load(&path, 384, 384)));
                ctx.request_repaint();
            });
        }
    }

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
        while let Ok((path, result)) = self.picture_rx.try_recv() {
            if let Screen::Picture(view) = &mut self.screen
                && let Some(index) = view.files.iter().position(|e| e.path == path)
                && view.requested == Some(index)
            {
                match result {
                    Ok(pic) => {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [pic.width, pic.height],
                            &pic.rgba,
                        );
                        view.texture = Some(self.ctx.load_texture(
                            "picture",
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                        view.shown = Some(index);
                        view.error = None;
                    }
                    Err(e) => {
                        log::warn!("cannot show {}: {e}", path.display());
                        view.error = Some(e);
                        view.shown = None;
                    }
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
            self.check_logos(&config);
        }
        while let Ok((id, modified, result)) = self.logo_rx.try_recv() {
            match result {
                Ok(p) => {
                    let image =
                        egui::ColorImage::from_rgba_unmultiplied([p.width, p.height], &p.rgba);
                    let texture = self.ctx.load_texture(
                        format!("logo-{id}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.logos.insert(id, (modified, Some(texture)));
                }
                Err(e) => {
                    log::warn!("logo of website {id}: {e}");
                    self.logos.insert(id, (modified, None));
                }
            }
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
            kind,
            stopping,
        } = &mut self.screen
        {
            match child.try_wait() {
                // The home button was held in the player / website window.
                Ok(Some(status)) if status.code() == Some(player::EXIT_GO_HOME) => {
                    next = Some(Screen::Home)
                }
                Ok(None) => {
                    if stopping.is_some_and(|t| t.elapsed() > Duration::from_secs(2)) {
                        log::warn!("player did not quit, killing it");
                        let _ = child.kill();
                    }
                    self.ctx.request_repaint_after(Duration::from_millis(250));
                }
                // Stopped by the user: back without an error message.
                Ok(Some(_)) if stopping.is_some() => {
                    next = Some(back.take().map_or(Screen::Home, Screen::Browse))
                }
                Ok(Some(status))
                    if !status.success() && started.elapsed() < Duration::from_secs(10) =>
                {
                    let text = match kind {
                        PlayKind::Disc => tr!(
                            "The player stopped right away ({status}).\nIs MakeMKV installed and registered? See scripts/setup.sh.",
                            status = status
                        ),
                        PlayKind::File => {
                            tr!("This file could not be played ({status}).", status = status)
                        }
                        PlayKind::Website => {
                            tr!(
                                "The website could not be opened ({status}).",
                                status = status
                            )
                        }
                    };
                    next = Some(Screen::Message {
                        text,
                        back: back.take(),
                    });
                }
                Ok(Some(_)) => next = Some(back.take().map_or(Screen::Home, Screen::Browse)),
                Err(e) => {
                    next = Some(Screen::Message {
                        text: tr!("Lost track of the player: {error}", error = e),
                        back: back.take(),
                    })
                }
            }
        }
        if let Some(screen) = next {
            self.screen = screen;
        }
        self.track_usage_and_limits();
        self.keep_player_in_front();
    }

    /// Count usage time and lock the media center when a limit is reached.
    ///
    /// Usage counts while something is open (not the home screen) or within 60 s after the
    /// last key press / click, so a media center left alone on the home screen doesn't use
    /// up the time.
    fn track_usage_and_limits(&mut self) {
        let now = chrono::Local::now();
        let input = self.ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::PointerButton { pressed: true, .. }
                        | egui::Event::MouseWheel { .. }
                )
            })
        });
        if input {
            self.last_input = Instant::now();
        }
        // Long gaps (suspend, hung frame) don't count.
        let dt = self.last_tick.elapsed().as_secs_f64().min(5.0);
        self.last_tick = Instant::now();
        let active = self.lock.is_none()
            && (!matches!(self.screen, Screen::Home)
                || self.last_input.elapsed() < Duration::from_secs(60));

        let (config, _) = self.config();
        let mut tracker = self.tracker.lock().unwrap_or_else(|p| p.into_inner());
        if active {
            self.usage_carry += dt;
            let whole = self.usage_carry.floor();
            if whole >= 1.0 {
                tracker.add_usage(now.date_naive(), whole as u64);
                self.usage_carry -= whole;
            }
        }
        let status = limits::evaluate(
            &config.parental.limits,
            &tracker.situation(),
            now.naive_local(),
        );
        tracker.status = status.clone();
        tracker.save_if_due();
        drop(tracker);

        match status {
            Status::Locked(reason) => match &mut self.lock {
                Some(view) => view.reason = reason,
                None => {
                    log::info!("locking: {}", limits::reason_text(&reason));
                    self.stop_everything();
                    self.lock = Some(LockView::new(reason));
                }
            },
            Status::Allowed { .. } => {
                if self.lock.take().is_some() {
                    log::info!("unlocked");
                }
            }
        }
        // Keep counting and checking even when nothing else repaints.
        self.ctx.request_repaint_after(Duration::from_secs(1));
    }

    /// Time is up: stop players, music and websites, close viewers.
    fn stop_everything(&mut self) {
        match std::mem::replace(&mut self.screen, Screen::Home) {
            Screen::Playing {
                child,
                started,
                kind,
                stopping,
                ..
            } => {
                if stopping.is_none() {
                    stop_player(child.id());
                }
                // Keep watching it until it has exited (killed after 2 s if needed).
                self.screen = Screen::Playing {
                    child,
                    started,
                    back: None,
                    kind,
                    stopping: Some(stopping.unwrap_or_else(Instant::now)),
                };
            }
            Screen::Music(mut view) => view.player.stop(),
            _ => {}
        }
    }

    fn lock_screen(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen_rect: Rect,
        s: f32,
        keys: &Keys,
    ) {
        let (config, _) = self.config();
        let now = chrono::Local::now();
        let doubling = config.parental.pin_backoff_doubling;
        while let Ok(ok) = self.pin_rx.try_recv() {
            let mut tracker = self.tracker.lock().unwrap_or_else(|p| p.into_inner());
            if ok {
                tracker.pin_ok();
            } else {
                let wait = tracker.pin_wrong(now.timestamp(), doubling);
                log::info!("wrong PIN, waiting {wait} s");
            }
            drop(tracker);
            if let Some(view) = &mut self.lock {
                view.pin_checked(ok);
            }
        }
        let info = LockInfo {
            has_pin: config.parental.pin_hash.is_some(),
            pin_length: config.parental.pin_length.map(usize::from),
            wait_secs: self
                .tracker
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .pin_wait(now.timestamp()),
        };
        let Some(view) = &mut self.lock else { return };
        let nav = Nav {
            enter: keys.enter,
            back: keys.back,
            up: keys.up,
            down: keys.down,
            left: keys.left,
            right: keys.right,
        };
        match view.show(ui, painter, screen_rect, s, &info, &nav) {
            LockAction::None => {}
            LockAction::CheckPin(pin) => match config.parental.pin_hash.clone() {
                Some(hash) => {
                    let (tx, ctx) = (self.pin_tx.clone(), self.ctx.clone());
                    std::thread::spawn(move || {
                        let _ = tx.send(crate::pin::verify(&pin, &hash));
                        ctx.request_repaint();
                    });
                }
                None => view.pin_checked(false),
            },
            LockAction::Unlock(minutes) => {
                log::info!("unlocked with the PIN ({minutes:?} minutes)");
                self.tracker
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .unlock(now, minutes);
                self.lock = None;
            }
        }
    }

    /// Countdown in the top-right corner during the last 5 minutes.
    fn paint_time_warning(&self, painter: &egui::Painter, screen_rect: Rect, s: f32) {
        let status = self
            .tracker
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .status
            .clone();
        let Status::Allowed {
            remaining_secs: Some(left),
        } = status
        else {
            return;
        };
        if left > 5 * 60 {
            return;
        }
        // Countdown m:ss; refreshed several times a second so no second is skipped.
        self.ctx.request_repaint_after(Duration::from_millis(250));
        let left = left.max(0);
        let text = tr!(
            "Time left {time}",
            time = format!("{}:{:02}", left / 60, left % 60)
        );
        let font = FontId::proportional(30.0 * s);
        // Width from a fixed template, so the box doesn't change size every second.
        let size = painter
            .layout_no_wrap(tr!("Time left {time}", time = "0:00"), font.clone(), TEXT)
            .size();
        let box_size = size + Vec2::new(44.0 * s, 22.0 * s);
        let rect = Rect::from_min_size(
            Pos2::new(
                // Left of the lock button.
                screen_rect.max.x - 190.0 * s - box_size.x,
                screen_rect.min.y + 38.0 * s,
            ),
            box_size,
        );
        // Orange, red in the last minute.
        let color = if left <= 60 {
            Color32::from_rgb(0xd6, 0x3a, 0x3a)
        } else {
            Color32::from_rgb(0xd9, 0x7a, 0x1e)
        };
        painter.rect_filled(rect, CornerRadius::same((rect.height() / 2.0) as u8), color);
        painter.text(rect.center(), Align2::CENTER_CENTER, text, font, TEXT);
    }

    /// While an external player (mpv/VLC) runs, keep our window at the bottom so the player
    /// is always visible in front of it (window managers may open it behind our fullscreen
    /// window). Afterwards: normal level and keyboard focus back to us.
    fn keep_player_in_front(&mut self) {
        let playing = matches!(self.screen, Screen::Playing { .. });
        if playing == self.window_below_player {
            return;
        }
        self.window_below_player = playing;
        let level = if playing {
            egui::WindowLevel::AlwaysOnBottom
        } else {
            egui::WindowLevel::Normal
        };
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        if !playing {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
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
                Some(_) => tile(false, "Blu-ray", tr!("Settings could not be loaded")),
                None => tile(false, "Blu-ray", tr!("Discs are turned off")),
            };
        }
        if self.drives.is_empty() {
            return tile(false, "Blu-ray", tr!("No disc drive found"));
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
                (_, Some((label, _))) => (fallback, format!("{label}…")),
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
                tile(false, "Blu-ray", tr!("DVDs are not supported yet"))
            }
            DriveState::Loaded(_) => tile(false, "Blu-ray", tr!("This disc is not a Blu-ray")),
            DriveState::NotReady => tile(false, "Blu-ray", tr!("Reading disc…")),
            DriveState::TrayOpen => tile(false, "Blu-ray", tr!("Tray is open")),
            DriveState::Empty => tile(false, "Blu-ray", tr!("Insert a Blu-ray")),
            DriveState::Error(e) => {
                log::debug!("drive error: {e}");
                tile(false, "Blu-ray", tr!("Drive not accessible"))
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
                    kind: PlayKind::Disc,
                    stopping: None,
                }
            }
            Err(e) => message(tr!(
                "Could not start the player ({player}): {error}.\nRun scripts/setup.sh to install it.",
                player = cmd.get_program().to_string_lossy(),
                error = e
            )),
        };
    }

    /// Open a file from the folder browser: pictures and music inside familynovice, videos
    /// with the same mpv setup as the disc's main movie.
    fn open_file(&mut self, browse: Browse, entry: Entry) {
        let same_kind: Vec<Entry> = browse
            .entries
            .iter()
            .filter(|e| e.kind == entry.kind)
            .cloned()
            .collect();
        let index = same_kind
            .iter()
            .position(|e| e.path == entry.path)
            .unwrap_or(0);
        self.screen = match entry.kind {
            EntryKind::Folder => Screen::Browse(browse),
            EntryKind::Image => Screen::Picture(PictureView {
                back: browse,
                files: same_kind,
                index,
                texture: None,
                shown: None,
                requested: None,
                error: None,
            }),
            EntryKind::Audio => {
                let paths: Vec<PathBuf> = same_kind.iter().map(|e| e.path.clone()).collect();
                let root = self
                    .config()
                    .0
                    .folders
                    .iter()
                    .find(|f| f.id == browse.folder_id)
                    .map(|f| PathBuf::from(&f.path))
                    .unwrap_or_default();
                let ctx = self.ctx.clone();
                match AudioPlayer::start(&paths, index, move || ctx.request_repaint()) {
                    Ok(player) => Screen::Music(MusicView {
                        back: browse,
                        player,
                        tracks: same_kind,
                        start: index,
                        root,
                        cover: None,
                        cover_track: None,
                        cover_rx: None,
                        recorded: None,
                    }),
                    Err(e) => Screen::Message {
                        text: e,
                        back: Some(browse),
                    },
                }
            }
            EntryKind::Video => match resume::load(&resume::file_key(&entry.path)) {
                Some(position) => Screen::VideoChoose {
                    back: browse,
                    entry,
                    position,
                    selected: 0,
                },
                None => {
                    self.start_video(browse, entry, None);
                    return;
                }
            },
        };
    }

    /// Open a website in the locked-down browser window (`familynovice-web`).
    fn open_website(&mut self, site: &crate::config::WebsiteSettings) {
        let Some(helper) = self.web_helper.clone() else {
            return;
        };
        let rules = match sites::rules_for(&site.url, site.allow_subdomains) {
            Ok(rules) => rules,
            Err(e) => {
                self.screen = message(tr!("This website can't be opened: {error}", error = e));
                return;
            }
        };
        let mut cmd = std::process::Command::new(helper);
        cmd.arg("--url")
            .arg(&site.url)
            .arg("--title")
            .arg(&site.name)
            .arg("--lang")
            .arg(crate::i18n::current().code());
        for rule in &rules {
            cmd.arg("--allow").arg(rule.to_arg());
        }
        if let Some(dir) = store::web_data_dir() {
            cmd.arg("--data-dir").arg(dir);
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null());
        log::info!("opening website {}: {cmd:?}", site.id);
        self.screen = match cmd.spawn() {
            Ok(child) => Screen::Playing {
                child,
                started: Instant::now(),
                back: None,
                kind: PlayKind::Website,
                stopping: None,
            },
            Err(e) => message(tr!("Could not open the website: {error}", error = e)),
        };
    }

    /// Play a video file like the disc's main movie: languages, resume tracking, no writes.
    fn start_video(&mut self, browse: Browse, entry: Entry, start_secs: Option<f64>) {
        let (config, _) = self.config();
        let cache = store::prepare_player_cache();
        let socket = store::mpv_socket_path();
        store::remove(&socket); // stale socket from an earlier run
        let options = MovieOptions {
            start_secs,
            ipc_socket: Some(&socket),
            audio_languages: &config.discs.audio_languages,
            subtitle_languages: &config.discs.subtitle_languages,
            cache_dir: cache.as_deref(),
        };
        let mut cmd = player::video_file_command(&entry.path, &options);
        log::info!("playing {}: {cmd:?}", entry.path.display());
        self.screen = match cmd.spawn() {
            Ok(child) => {
                resume::spawn_monitor(socket.clone(), resume::file_key(&entry.path));
                recent::record(
                    &browse.folder_id,
                    &browse.sub,
                    &entry.name,
                    recent::Kind::Video,
                );
                Screen::Playing {
                    child,
                    started: Instant::now(),
                    back: Some(select(browse, &entry.path)),
                    kind: PlayKind::File,
                    stopping: None,
                }
            }
            Err(e) => Screen::Message {
                text: tr!("Could not start the player (mpv): {error}", error = e),
                back: Some(browse),
            },
        };
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The language follows the config (also after a reload).
        crate::i18n::set(crate::i18n::Lang::from_setting(
            &self.config().0.home.language,
        ));
        self.poll_background();
        let ctx = ui.ctx().clone();
        let screen_rect = ui.max_rect();
        let s = (screen_rect.height() / 1080.0).max(0.5); // scale factor
        let painter = ui.painter().clone();
        painter.rect_filled(screen_rect, 0.0, BG);

        let mut keys = Keys::read(&ctx);
        // Media keys the desktop grabbed and passed on via MPRIS (Cinnamon, GNOME, KDE, …).
        let desktop_media: Vec<MediaCommand> = self.media_rx.try_iter().collect();
        keys.media.extend(&desktop_media);
        for (name, action) in self.remote_map.window_keys() {
            if Key::from_name(name).is_some_and(|k| ctx.input(|i| i.key_pressed(k))) {
                keys.apply(action);
            }
        }
        let remote_events: Vec<remote::KeyEvent> = self.inputs.remote.try_iter().collect();
        for e in &remote_events {
            match e.value {
                0 => {
                    self.remote_down.remove(&e.code);
                }
                _ => {
                    self.remote_down.insert(e.code);
                    self.last_input = Instant::now();
                }
            }
        }
        if self
            .inputs
            .requests
            .try_iter()
            .any(|r| r == UiRequest::CalibrateRemote)
        {
            self.calibrate_requested = true;
            self.go_home();
        }
        self.mpris.set_status(match &self.screen {
            Screen::Music(view) if view.player.state().paused => PlayStatus::Paused,
            Screen::Music(_) => PlayStatus::Playing,
            Screen::Playing { kind, .. } if *kind != PlayKind::Website => PlayStatus::Playing,
            _ => PlayStatus::Stopped,
        });
        // TODO: protect leaving the media center with the PIN.
        if keys.quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Buttons only the device reports (e.g. OK = KEY_SELECT), while our window is in
        // front (not while a player or website is, or the desktop is used in development).
        if ctx.input(|i| i.focused) {
            for e in &remote_events {
                let repeat_ok = e.value == 2;
                if (e.value == 1 || repeat_ok)
                    && let Some(action) = self.remote_map.device_action(e.code)
                    && action.volume().is_none() // see App::volume_keys
                    && (!repeat_ok
                        || matches!(
                            action,
                            Action::Up | Action::Down | Action::Left | Action::Right
                        ))
                {
                    keys.apply(action);
                }
            }
        }
        self.volume_keys(&ctx, &keys, &remote_events);
        self.paint_volume(ui, screen_rect, s);

        if self.lock.is_some() {
            self.lock_screen(ui, &painter, screen_rect, s, &keys);
            return;
        }
        self.home_button(ui, screen_rect, s);
        if self.calibrate_requested && matches!(self.screen, Screen::Home) {
            self.calibrate_requested = false;
            self.screen = Screen::Calibrate(Box::new(CalibrateView {
                calibration: Calibration::new(Instant::now()),
                done: None,
            }));
        }
        if matches!(self.screen, Screen::Calibrate(_)) {
            self.calibrate_screen(
                &ctx,
                &painter,
                screen_rect,
                s,
                &remote_events,
                &desktop_media,
            );
            return;
        }

        match self.screen {
            Screen::Browse(_) => self.browse_screen(ui, &painter, screen_rect, s, &keys),
            Screen::Picture(_) => self.picture_screen(ui, &painter, screen_rect, s, &keys),
            Screen::Music(_) => self.music_screen(ui, &painter, screen_rect, s, &keys),
            Screen::VideoChoose { .. } => {
                self.video_choose_screen(ui, &painter, screen_rect, s, &keys)
            }
            _ => self.home_screen(ui, &painter, screen_rect, s, &keys),
        }
        self.paint_time_warning(&painter, screen_rect, s);
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
        let tile_size = Vec2::new(340.0 * s, 380.0 * s);
        let gap = 40.0 * s;
        let home = matches!(self.screen, Screen::Home);

        let websites = &config.websites;
        let mut rows = vec![RowKind::Discs];
        if !folders.is_empty() {
            rows.push(RowKind::Folders);
        }
        if !websites.is_empty() {
            rows.push(RowKind::Websites);
        }
        let row_len = |kind: RowKind| match kind {
            RowKind::Discs => 1,
            RowKind::Folders => folders.len(),
            RowKind::Websites => websites.len(),
        };

        // The lock button (only with a PIN, so nobody gets locked out): "up" from the top row.
        let has_pin = config.parental.pin_hash.is_some();
        if !has_pin {
            self.lock_button_focused = false;
        }
        if home && self.lock_button_focused {
            if keys.down {
                self.lock_button_focused = false;
            }
        } else if home && keys.up && self.focus.0 == 0 && has_pin {
            self.lock_button_focused = true;
        } else if home {
            let (mut row, mut col) = self.focus;
            if keys.up && row > 0 {
                row -= 1;
            }
            if keys.down && row + 1 < rows.len() {
                row += 1;
            }
            row = row.min(rows.len() - 1);
            if keys.left {
                col = col.saturating_sub(1);
            }
            if keys.right {
                col += 1;
            }
            col = col.min(row_len(rows[row]) - 1);
            self.focus = (row, col);
        }

        // Vertical layout: rows scroll so that the focused row is fully visible.
        let top = 115.0 * s;
        let row_h = 65.0 * s + tile_size.y + 30.0 * s;
        let focused_y = self.focus.0 as f32 * row_h;
        let visible_h = screen_rect.height() - top;
        if focused_y < self.rows_scroll_y {
            self.rows_scroll_y = focused_y;
        } else if focused_y + row_h > self.rows_scroll_y + visible_h {
            self.rows_scroll_y = focused_y + row_h - visible_h;
        }
        let scroll_y = ctx.animate_value_with_time(
            ui.make_persistent_id("rows-scroll"),
            self.rows_scroll_y,
            0.2,
        );
        let painter = &painter.with_clip_rect(Rect::from_min_max(
            Pos2::new(screen_rect.min.x, screen_rect.min.y + top - 10.0 * s),
            screen_rect.max,
        ));

        // Header (above the scrolling rows)
        if config.home.show_title {
            let font = FontId::proportional(34.0 * s);
            ui.painter().text(
                screen_rect.min + Vec2::new(margin, 50.0 * s),
                Align2::LEFT_TOP,
                fit_text(
                    painter,
                    &config.home.title,
                    &font,
                    screen_rect.width() - 2.0 * margin,
                ),
                font,
                TEXT_DIM,
            );
        }

        let tile = self.bluray_tile();
        let mut activate = None;
        let step = tile_size.x + gap;
        let visible_w = screen_rect.width() - 2.0 * margin;
        for (r, &kind) in rows.iter().enumerate() {
            let y = top + r as f32 * row_h - scroll_y;
            if y > screen_rect.height() || y + row_h < 0.0 {
                continue;
            }
            let label = match kind {
                RowKind::Discs => tr!("Discs"),
                RowKind::Folders => tr!("Folders"),
                RowKind::Websites => tr!("Websites"),
            };
            painter.text(
                screen_rect.min + Vec2::new(margin, y),
                Align2::LEFT_TOP,
                label,
                FontId::proportional(38.0 * s),
                TEXT,
            );
            // Horizontal scroll: keep the focused tile on screen.
            let sx = &mut self.row_scroll_x[kind as usize];
            if self.focus.0 == r {
                let left = self.focus.1 as f32 * step;
                let right = left + tile_size.x;
                if left < *sx {
                    *sx = left;
                } else if right > *sx + visible_w {
                    *sx = right - visible_w;
                }
            }
            let sx = *sx;
            for i in 0..row_len(kind) {
                let x = margin + i as f32 * step - sx;
                if x + tile_size.x < 0.0 || x > screen_rect.width() {
                    continue;
                }
                let rect =
                    Rect::from_min_size(screen_rect.min + Vec2::new(x, y + 65.0 * s), tile_size);
                let id = ui.make_persistent_id(("tile", kind as usize, i));
                let response = ui.interact(rect, id, Sense::click());
                let focus = ctx.animate_bool_with_time(
                    ui.make_persistent_id(("focus", kind as usize, i)),
                    home && !self.lock_button_focused && self.focus == (r, i),
                    0.15,
                );
                match kind {
                    RowKind::Discs => paint_bluray_tile(painter, rect, s, &tile, focus),
                    RowKind::Folders => {
                        let folder = &folders[i];
                        let available = self.folder_ok.get(&folder.id).copied().unwrap_or(false);
                        let subtitle = if available {
                            folder_kinds(folder)
                        } else {
                            tr!("Not available").into()
                        };
                        paint_folder_tile(
                            painter,
                            rect,
                            s,
                            &folder.name,
                            &subtitle,
                            available,
                            focus,
                        );
                    }
                    RowKind::Websites => {
                        let site = &websites[i];
                        let available = self.web_helper.is_some();
                        let subtitle = if available {
                            website_domain(site)
                        } else {
                            tr!("Not installed").into()
                        };
                        let logo = site
                            .show_logo
                            .then(|| self.logos.get(&site.id).and_then(|(_, t)| t.as_ref()))
                            .flatten();
                        let tile = WebsiteTile {
                            title: &site.name,
                            subtitle: &subtitle,
                            available,
                            logo,
                        };
                        paint_website_tile(painter, rect, s, &tile, focus);
                    }
                }
                if home && response.clicked() {
                    self.focus = (r, i);
                    activate = Some((r, i));
                }
            }
        }

        if has_pin {
            let size = 76.0 * s;
            let rect = Rect::from_min_size(
                Pos2::new(
                    screen_rect.max.x - margin - size,
                    screen_rect.min.y + 34.0 * s,
                ),
                Vec2::splat(size),
            );
            let response = ui.interact(rect, ui.make_persistent_id("lock-button"), Sense::click());
            let focused = home && (self.lock_button_focused || response.hovered());
            paint_lock_button(ui.painter(), rect, focused, s);
            let pressed = response.clicked() || (home && self.lock_button_focused && keys.enter);
            if home && pressed {
                log::info!("locked with the lock button");
                self.lock_button_focused = false;
                self.tracker
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .lock();
                self.ctx.request_repaint();
                return;
            }
        }

        if home && keys.enter && !self.lock_button_focused {
            activate = Some(self.focus);
        }
        if let Some((row, col)) = activate {
            match rows.get(row) {
                Some(RowKind::Discs) if tile.available => {
                    self.screen = Screen::Choose {
                        selected: 0,
                        resume: tile.disc_id.as_deref().and_then(resume::load),
                    };
                }
                Some(RowKind::Folders) => {
                    if let Some(folder) = folders.get(col)
                        && self.folder_ok.get(&folder.id).copied().unwrap_or(false)
                    {
                        self.screen = Screen::Browse(Browse::open(folder, Vec::new(), None));
                    }
                }
                Some(RowKind::Websites) => {
                    if let Some(site) = websites.get(col) {
                        self.open_website(site);
                    }
                }
                _ => {}
            }
        }

        // Overlays
        match &self.screen {
            Screen::Home
            | Screen::Calibrate(_)
            | Screen::Browse(_)
            | Screen::Picture(_)
            | Screen::Music(_)
            | Screen::VideoChoose { .. } => {}
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
                    tr!("Ejecting…"),
                    tr!("The tray opens in a moment."),
                );
            }
            Screen::Playing {
                child,
                stopping,
                started,
                kind,
                ..
            } => {
                // Ignore input for a moment after starting: the click / OK press that opened
                // the player (or website) must not stop it again right away.
                let settled = started.elapsed() > Duration::from_millis(1000);
                let stop_now = settled
                    && stopping.is_none()
                    && (keys.back
                        || keys.media(MediaCommand::Stop)
                        || ctx.input(|i| i.pointer.any_click()));
                // Media keys for the video (mpv; nothing happens for VLC or websites).
                if stopping.is_none() && *kind != PlayKind::Website {
                    for command in &keys.media {
                        player::mpv_media_key(&store::mpv_socket_path(), *command);
                    }
                }
                let pid = child.id();
                let body = if stopping.is_some() {
                    tr!("Stopping…")
                } else {
                    tr!("Esc / Back: stop")
                };
                paint_overlay(painter, screen_rect, s, tr!("Playing…"), body);
                if stop_now {
                    stop_player(pid);
                    if let Screen::Playing { stopping, .. } = &mut self.screen {
                        *stopping = Some(Instant::now());
                    }
                }
            }
            Screen::Message { text, .. } => {
                paint_overlay(painter, screen_rect, s, tr!("Something went wrong"), text);
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
        // Selectable: the entries, then the recently played files.
        let total = n + b.recent.len();

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
        if (keys.down || wheel < 0.0) && b.selected + 1 < total {
            b.selected += 1;
        }
        if keys.page_up {
            b.selected = b.selected.saturating_sub(visible);
        }
        if keys.page_down && total > 0 {
            b.selected = (b.selected + visible).min(total - 1);
        }

        // Header: folder name › subfolders
        let mut crumbs = vec![folder.name.as_str()];
        crumbs.extend(b.sub.iter().map(String::as_str));
        let (back_rect, back_response) = back_button(ui, screen_rect, s);
        paint_back_button(painter, back_rect, back_response.hovered(), s);
        let go_back = keys.back || back_response.clicked();
        let title_x = back_rect.max.x - screen_rect.min.x + 30.0 * s;
        painter.text(
            screen_rect.min + Vec2::new(title_x, 56.0 * s),
            Align2::LEFT_TOP,
            fit_text(
                painter,
                &crumbs.join("  ›  "),
                &FontId::proportional(42.0 * s),
                screen_rect.width() - title_x - margin - 160.0 * s, // room for "12 / 140"
            ),
            FontId::proportional(42.0 * s),
            TEXT,
        );
        if b.selected < n {
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
                tr!("This folder can't be opened right now"),
                &elide(error, 90),
            );
        } else if total == 0 {
            painter.text(
                screen_rect.center(),
                Align2::CENTER_CENTER,
                tr!("Nothing to show here"),
                FontId::proportional(32.0 * s),
                TEXT_DIM,
            );
        }

        // Rows: the entries, then a "Recently played" heading and those files.
        enum Row {
            Entry(usize),
            Heading,
            Recent(usize),
        }
        let mut rows: Vec<Row> = (0..n).map(Row::Entry).collect();
        if !b.recent.is_empty() {
            rows.push(Row::Heading);
            rows.extend((0..b.recent.len()).map(Row::Recent));
        }
        let selected_row = if b.selected < n {
            b.selected
        } else {
            b.selected + 1 // after the heading
        };
        let first = selected_row
            .saturating_sub(visible / 2)
            .min(rows.len().saturating_sub(visible));
        for (r, row) in rows.iter().enumerate().skip(first).take(visible) {
            let y = top + (r - first) as f32 * row_h;
            let (i, entry, label) = match row {
                Row::Heading => {
                    painter.text(
                        Pos2::new(
                            screen_rect.min.x + margin + 10.0 * s,
                            screen_rect.min.y + y + (row_h - 10.0 * s) / 2.0 + 8.0 * s,
                        ),
                        Align2::LEFT_CENTER,
                        tr!("Recently played"),
                        FontId::proportional(28.0 * s),
                        TEXT_DIM,
                    );
                    continue;
                }
                Row::Entry(i) => (*i, &b.entries[*i], b.entries[*i].name.clone()),
                Row::Recent(j) => {
                    let item = &b.recent[*j];
                    let mut parts: Vec<&str> = item.sub.iter().map(String::as_str).collect();
                    parts.push(&item.entry.name);
                    (n + j, &item.entry, parts.join("  ›  "))
                }
            };
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
                fit_text(
                    painter,
                    &label,
                    &FontId::proportional(30.0 * s),
                    rect.width() - 160.0 * s, // icon on the left, "›" on the right
                ),
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
            tr!("Enter: open     Esc / Backspace: back"),
            FontId::proportional(22.0 * s),
            TEXT_DIM,
        );

        if go_back {
            match b.sub.pop() {
                None => return, // leave the folder: home screen
                Some(last) => b = Browse::open(folder, b.sub, Some(&last)),
            }
        } else if activate
            && let Some(item) = b.selected.checked_sub(n).and_then(|j| b.recent.get(j))
        {
            // Open it from its own folder, so "back" and next/previous track work there.
            let item = item.clone();
            let target = Browse::open(folder, item.sub, Some(&item.entry.name));
            let entry = target
                .entries
                .iter()
                .find(|e| e.path == item.entry.path)
                .cloned()
                .unwrap_or(item.entry);
            self.open_file(target, entry);
            return;
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

impl App {
    fn picture_screen(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen_rect: Rect,
        s: f32,
        keys: &Keys,
    ) {
        let Screen::Picture(mut view) = std::mem::replace(&mut self.screen, Screen::Home) else {
            return;
        };
        painter.rect_filled(screen_rect, 0.0, Color32::BLACK);
        let n = view.files.len();

        // Mouse: click on the left/right half = previous/next.
        let click = ui
            .interact(
                screen_rect,
                ui.make_persistent_id("picture"),
                Sense::click(),
            )
            .interact_pointer_pos()
            .filter(|_| ui.ctx().input(|i| i.pointer.any_click()));
        // On top of the click area above.
        let (back_rect, back_response) = back_button(ui, screen_rect, s);
        let prev = keys.left
            || keys.up
            || keys.media(MediaCommand::Previous)
            || click.is_some_and(|p| p.x < screen_rect.center().x);
        let next = keys.right
            || keys.down
            || keys.media(MediaCommand::Next)
            || keys.enter
            || click.is_some_and(|p| p.x >= screen_rect.center().x);
        if keys.back || back_response.clicked() {
            let path = view.files[view.index].path.clone();
            self.screen = Screen::Browse(select(view.back, &path));
            return;
        }
        if prev && view.index > 0 {
            view.index -= 1;
        }
        if next && view.index + 1 < n {
            view.index += 1;
        }

        // Load the current picture in the background, scaled to the screen.
        if view.requested != Some(view.index) {
            view.requested = Some(view.index);
            let ppp = ui.ctx().pixels_per_point();
            let (w, h) = (
                (screen_rect.width() * ppp) as u32,
                (screen_rect.height() * ppp) as u32,
            );
            let (tx, ctx, path) = (
                self.picture_tx.clone(),
                ui.ctx().clone(),
                view.files[view.index].path.clone(),
            );
            std::thread::spawn(move || {
                let result = pictures::load(&path, w, h);
                let _ = tx.send((path, result));
                ctx.request_repaint();
            });
        }

        if let Some(texture) = &view.texture {
            let size = texture.size_vec2();
            let scale = (screen_rect.width() / size.x).min(screen_rect.height() / size.y);
            let rect = Rect::from_center_size(screen_rect.center(), size * scale);
            let tint = if view.shown == Some(view.index) {
                Color32::WHITE
            } else {
                Color32::from_gray(90) // the next picture is still loading
            };
            painter.image(
                texture.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                tint,
            );
        }
        if let Some(error) = &view.error {
            paint_overlay(
                painter,
                screen_rect,
                s,
                tr!("This picture can't be shown"),
                &elide(error, 90),
            );
        } else if view.shown != Some(view.index) {
            painter.text(
                screen_rect.center(),
                Align2::CENTER_CENTER,
                tr!("Loading…"),
                FontId::proportional(32.0 * s),
                TEXT_DIM,
            );
        }

        // Caption
        let band = Rect::from_min_max(
            Pos2::new(screen_rect.min.x, screen_rect.max.y - 70.0 * s),
            screen_rect.max,
        );
        painter.rect_filled(band, 0.0, Color32::from_black_alpha(150));
        painter.text(
            Pos2::new(band.min.x + 40.0 * s, band.center().y),
            Align2::LEFT_CENTER,
            fit_text(
                painter,
                &view.files[view.index].name,
                &FontId::proportional(24.0 * s),
                band.width() * 0.6,
            ),
            FontId::proportional(24.0 * s),
            TEXT,
        );
        painter.text(
            Pos2::new(band.max.x - 40.0 * s, band.center().y),
            Align2::RIGHT_CENTER,
            tr!(
                "{n} / {total}     ← →  ·  Esc: back",
                n = view.index + 1,
                total = n
            ),
            FontId::proportional(22.0 * s),
            TEXT_DIM,
        );
        paint_back_button(painter, back_rect, back_response.hovered(), s);
        self.screen = Screen::Picture(view);
    }

    fn music_screen(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen_rect: Rect,
        s: f32,
        keys: &Keys,
    ) {
        let Screen::Music(mut view) = std::mem::replace(&mut self.screen, Screen::Home) else {
            return;
        };
        let state = view.player.state();
        let current = view.start + state.playlist_pos.unwrap_or(0);
        let current_path = view.tracks.get(current).map(|t| t.path.clone());
        view.update_cover(ui.ctx(), current_path.as_deref());
        if let Some(track) = view.tracks.get(current)
            && view.recorded.as_ref() != Some(&track.path)
        {
            recent::record(
                &view.back.folder_id,
                &view.back.sub,
                &track.name,
                recent::Kind::Audio,
            );
            view.recorded = Some(track.path.clone());
        }
        let back_to_browser = |view: MusicView| -> Screen {
            let browse = view.back.clone();
            Screen::Browse(match &current_path {
                Some(p) => select(browse, p),
                None => browse,
            })
        };
        let (back_rect, back_response) = back_button(ui, screen_rect, s);
        paint_back_button(painter, back_rect, back_response.hovered(), s);
        if keys.back || keys.media(MediaCommand::Stop) || back_response.clicked() {
            view.player.stop();
            self.screen = back_to_browser(view);
            return;
        }
        if view.player.finished() {
            self.screen = back_to_browser(view);
            return;
        }
        let paused = state.paused;
        if keys.enter
            || keys.media(MediaCommand::PlayPause)
            || (keys.media(MediaCommand::Play) && paused)
            || (keys.media(MediaCommand::Pause) && !paused)
        {
            view.player.toggle_pause();
        }
        if keys.left || keys.media(MediaCommand::Previous) {
            view.player.previous();
        }
        if keys.right || keys.media(MediaCommand::Next) {
            view.player.next();
        }
        if keys.up {
            view.player.seek(30.0);
        }
        if keys.down {
            view.player.seek(-30.0);
        }

        let center = screen_rect.center();
        match &view.cover {
            Some((_, texture)) => {
                // Fit into a 300 × 300 square above the title, keeping the aspect ratio.
                let size = texture.size_vec2();
                let scale = (300.0 * s / size.x).min(300.0 * s / size.y);
                let rect = Rect::from_center_size(center - Vec2::new(0.0, 210.0 * s), size * scale);
                painter.image(
                    texture.id(),
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => paint_entry_icon(
                painter,
                EntryKind::Audio,
                center - Vec2::new(0.0, 200.0 * s),
                90.0 * s,
            ),
        }
        let title = view
            .tracks
            .get(current)
            .map(|t| {
                std::path::Path::new(&t.name)
                    .file_stem()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| t.name.clone())
            })
            .unwrap_or_default();
        painter.text(
            center + Vec2::new(0.0, -20.0 * s),
            Align2::CENTER_CENTER,
            fit_text(
                painter,
                &title,
                &FontId::proportional(44.0 * s),
                screen_rect.width() - 160.0 * s,
            ),
            FontId::proportional(44.0 * s),
            TEXT,
        );
        let folder_name = self
            .config()
            .0
            .folders
            .iter()
            .find(|f| f.id == view.back.folder_id)
            .map(|f| f.name.clone())
            .unwrap_or_default();
        painter.text(
            center + Vec2::new(0.0, 35.0 * s),
            Align2::CENTER_CENTER,
            tr!(
                "{folder}  ·  Track {n} of {total}",
                folder = folder_name,
                n = current + 1,
                total = view.tracks.len()
            ),
            FontId::proportional(24.0 * s),
            TEXT_DIM,
        );

        // Progress
        let bar = Rect::from_center_size(
            center + Vec2::new(0.0, 110.0 * s),
            Vec2::new(900.0 * s, 10.0 * s),
        );
        let radius = CornerRadius::same((5.0 * s) as u8);
        painter.rect_filled(bar, radius, Color32::from_rgb(0x3a, 0x40, 0x55));
        if let (Some(pos), Some(dur)) = (state.position, state.duration)
            && dur > 0.0
        {
            let mut done = bar;
            done.set_width(bar.width() * (pos / dur).clamp(0.0, 1.0) as f32);
            painter.rect_filled(done, radius, ACCENT);
        }
        let time = |t: Option<f64>| t.map(resume::format_time).unwrap_or_else(|| "–".into());
        painter.text(
            Pos2::new(bar.min.x, bar.max.y + 14.0 * s),
            Align2::LEFT_TOP,
            time(state.position),
            FontId::proportional(22.0 * s),
            TEXT_DIM,
        );
        painter.text(
            Pos2::new(bar.max.x, bar.max.y + 14.0 * s),
            Align2::RIGHT_TOP,
            time(state.duration),
            FontId::proportional(22.0 * s),
            TEXT_DIM,
        );
        if state.paused {
            painter.text(
                center + Vec2::new(0.0, 200.0 * s),
                Align2::CENTER_CENTER,
                tr!("Paused"),
                FontId::proportional(32.0 * s),
                TEXT,
            );
        }
        painter.text(
            Pos2::new(center.x, screen_rect.max.y - 60.0 * s),
            Align2::CENTER_CENTER,
            tr!("Enter: pause   ← →: previous / next   ↑ ↓: ±30 s   Esc: stop"),
            FontId::proportional(22.0 * s),
            TEXT_DIM,
        );
        ui.ctx().request_repaint_after(Duration::from_millis(500));
        self.screen = Screen::Music(view);
    }

    fn video_choose_screen(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen_rect: Rect,
        s: f32,
        keys: &Keys,
    ) {
        let Screen::VideoChoose {
            back,
            entry,
            position,
            mut selected,
        } = std::mem::replace(&mut self.screen, Screen::Home)
        else {
            return;
        };
        let items = [
            MenuItem::new(
                tr!("Resume from {time}", time = resume::format_time(position)),
                tr!("Continues where you stopped"),
            ),
            MenuItem::new(
                tr!("Play from beginning"),
                tr!("Starts the video from the start"),
            ),
            MenuItem::new(tr!("Back"), ""),
        ];
        if keys.prev() {
            selected = selected.saturating_sub(1);
        }
        if keys.next() {
            selected = (selected + 1).min(items.len() - 1);
        }
        let title = entry.name.clone();
        let clicked = paint_menu(ui, painter, screen_rect, s, &title, &items, selected);
        match clicked.or(keys.enter.then_some(selected)) {
            _ if keys.back => self.screen = Screen::Browse(back),
            Some(0) => self.start_video(back, entry, Some(position)),
            Some(1) => self.start_video(back, entry, None),
            Some(_) => self.screen = Screen::Browse(back),
            None => {
                self.screen = Screen::VideoChoose {
                    back,
                    entry,
                    position,
                    selected,
                }
            }
        }
    }
}

/// `familynovice-web` next to our own binary (development and installed layout), else in PATH.
fn find_web_helper() -> Option<PathBuf> {
    let name = "familynovice-web";
    let next_to_us = std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name(name))
        .filter(|p| p.is_file());
    let found = next_to_us.or_else(|| {
        std::env::var_os("PATH")?
            .to_str()?
            .split(':')
            .map(|dir| PathBuf::from(dir).join(name))
            .find(|p| p.is_file())
    });
    if found.is_none() {
        log::warn!("{name} not found: websites can't be opened (cargo build -p familynovice-web)");
    }
    found
}

/// "kika.de" for a website tile.
fn website_domain(site: &crate::config::WebsiteSettings) -> String {
    match sites::rules_for(&site.url, site.allow_subdomains) {
        Ok(rules) => rules
            .first()
            .map(|r| r.to_arg().trim_start_matches("*.").to_string())
            .unwrap_or_default(),
        Err(_) => tr!("Invalid address").into(),
    }
}

/// What a website tile shows.
struct WebsiteTile<'a> {
    title: &'a str,
    subtitle: &'a str,
    available: bool,
    /// The website's logo; `None`: our globe icon.
    logo: Option<&'a egui::TextureHandle>,
}

fn paint_website_tile(painter: &egui::Painter, rect: Rect, s: f32, tile: &WebsiteTile, focus: f32) {
    let WebsiteTile {
        title,
        subtitle,
        available,
        logo,
    } = *tile;
    let alpha = if available { 1.0 } else { UNAVAILABLE_ALPHA };
    let rect = rect.expand(focus * 12.0 * s);
    let radius = CornerRadius::same((18.0 * s) as u8);
    painter.rect_filled(rect, radius, TILE_BG.gamma_multiply(alpha));
    if focus > 0.0 {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(4.0 * s, ACCENT.gamma_multiply(focus)),
            StrokeKind::Outside,
        );
    }
    let center = Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.36);
    match logo {
        Some(texture) => {
            // On a light rounded plate: many logos are dark or transparent.
            let edge = rect.width() * 0.5;
            let plate = Rect::from_center_size(center, Vec2::splat(edge + 20.0 * s));
            painter.rect_filled(
                plate,
                CornerRadius::same((14.0 * s) as u8),
                Color32::from_rgb(0xf4, 0xf5, 0xf8).gamma_multiply(alpha),
            );
            let size = texture.size_vec2();
            let scale = (edge / size.x).min(edge / size.y);
            painter.image(
                texture.id(),
                Rect::from_center_size(center, size * scale),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE.gamma_multiply(alpha),
            );
        }
        None => paint_globe_icon(painter, center, rect.width() * 0.27, alpha),
    }
    paint_tile_texts(painter, rect, s, title, subtitle, alpha);
}

/// Our own globe icon: a circle with meridians and parallels.
fn paint_globe_icon(painter: &egui::Painter, center: Pos2, r: f32, alpha: f32) {
    let fill = Color32::from_rgb(0x2f, 0xa3, 0x8a).gamma_multiply(alpha);
    let line = Stroke::new(
        r * 0.06,
        Color32::from_rgb(0xd8, 0xf5, 0xee).gamma_multiply(alpha),
    );
    painter.circle_filled(center, r, fill);
    painter.circle_stroke(center, r, line);
    // Meridians: ellipses of decreasing width.
    for w in [0.35, 0.75] {
        let points: Vec<Pos2> = (0..=48)
            .map(|k| {
                let t = k as f32 / 48.0 * std::f32::consts::TAU;
                center + Vec2::new(t.cos() * r * w, t.sin() * r)
            })
            .collect();
        painter.add(egui::Shape::line(points, line));
    }
    painter.line_segment(
        [center - Vec2::new(0.0, r), center + Vec2::new(0.0, r)],
        line,
    );
    // Parallels
    for dy in [-0.5f32, 0.0, 0.5] {
        let half = r * (1.0 - dy * dy).sqrt();
        let y = center.y + dy * r;
        painter.line_segment(
            [Pos2::new(center.x - half, y), Pos2::new(center.x + half, y)],
            line,
        );
    }
}

/// Round button with our own padlock icon.
fn paint_lock_button(painter: &egui::Painter, rect: Rect, focused: bool, s: f32) {
    let r = rect.width() / 2.0;
    let c = rect.center();
    painter.circle_filled(c, r, if focused { ACCENT } else { TILE_BG });
    if focused {
        painter.circle_stroke(c, r, Stroke::new(3.0 * s, TEXT));
    }
    // Body
    let body = Rect::from_center_size(c + Vec2::new(0.0, r * 0.18), Vec2::new(r * 0.9, r * 0.7));
    painter.rect_filled(body, CornerRadius::same((r * 0.12) as u8), TEXT);
    // Shackle: half circle above the body
    let sr = r * 0.28;
    let sc = Pos2::new(c.x, body.min.y);
    let points: Vec<Pos2> = (0..=20)
        .map(|k| {
            let t = std::f32::consts::PI * (1.0 + k as f32 / 20.0);
            sc + Vec2::new(t.cos() * sr, t.sin() * sr * 1.25)
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(r * 0.13, TEXT)));
    // Keyhole
    painter.circle_filled(
        body.center(),
        r * 0.09,
        if focused { ACCENT } else { TILE_BG },
    );
}

/// A large "‹ Back" button for mouse/touch (no keyboard needed). Register it (`back_button`)
/// after other click areas so it lies on top of them; paint it (`paint_back_button`) last.
fn back_button(ui: &egui::Ui, screen_rect: Rect, s: f32) -> (Rect, egui::Response) {
    let rect = Rect::from_min_size(
        screen_rect.min + Vec2::new(60.0 * s, 44.0 * s),
        Vec2::new(160.0 * s, 68.0 * s),
    );
    let response = ui.interact(rect, ui.make_persistent_id("back-button"), Sense::click());
    (rect, response)
}

fn paint_back_button(painter: &egui::Painter, rect: Rect, hovered: bool, s: f32) {
    let fill = if hovered {
        ACCENT
    } else {
        Color32::from_rgba_unmultiplied(0x2e, 0x33, 0x45, 230)
    };
    painter.rect_filled(rect, CornerRadius::same((rect.height() / 2.0) as u8), fill);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        tr!("‹  Back"),
        FontId::proportional(28.0 * s),
        TEXT,
    );
}

/// Ask a player to quit (SIGTERM): mpv/VLC exit cleanly, so mpv still reports its last
/// position to the resume monitor. `poll_background` kills it if it doesn't exit in time.
fn stop_player(pid: u32) {
    log::info!("stopping player (pid {pid})");
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        // SAFETY: plain kill(2) on our own child process.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
}

/// "Videos · Pictures" for a folder tile.
fn folder_kinds(folder: &FolderSettings) -> String {
    let mut kinds: Vec<String> = Vec::new();
    for (on, name) in [
        (folder.video, tr!("Videos")),
        (folder.audio, tr!("Music")),
        (folder.images, tr!("Pictures")),
    ] {
        if on {
            kinds.push(name.into());
        }
    }
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
    let center = Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.36);
    paint_folder_icon(painter, center, rect.width() * 0.58, alpha);
    paint_tile_texts(painter, rect, s, title, subtitle, alpha);
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
    let center = Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.36);
    paint_disc_icon(painter, center, rect.width() * 0.30, alpha);
    paint_tile_texts(painter, rect, s, &tile.title, &tile.subtitle, alpha);
    if let Some(f) = tile.progress {
        // Bar plus a fixed spot for the percentage, so the text above never moves.
        let pct_width = 62.0 * s;
        let track = Rect::from_min_size(
            Pos2::new(rect.min.x + 30.0 * s, rect.max.y - 32.0 * s),
            Vec2::new(rect.width() - 60.0 * s - pct_width, 8.0 * s),
        );
        let radius = CornerRadius::same((4.0 * s) as u8);
        painter.rect_filled(track, radius, c(Color32::from_rgb(0x3a, 0x40, 0x55)));
        let mut bar = track;
        bar.set_width(track.width() * f.clamp(0.0, 1.0));
        painter.rect_filled(bar, radius, c(ACCENT));
        painter.text(
            Pos2::new(rect.max.x - 30.0 * s, track.center().y),
            Align2::RIGHT_CENTER,
            percent(f),
            FontId::proportional(20.0 * s),
            c(TEXT_DIM),
        );
    }
}

/// Title and subtitle of a tile, in fixed sizes; too long text ends with "…".
fn paint_tile_texts(
    painter: &egui::Painter,
    rect: Rect,
    s: f32,
    title: &str,
    subtitle: &str,
    alpha: f32,
) {
    let max_width = rect.width() - 30.0 * s;
    for (text, size, y, color) in [
        (title, TILE_TITLE_FONT, TILE_TITLE_Y, TEXT),
        (subtitle, TILE_SUBTITLE_FONT, TILE_SUBTITLE_Y, TEXT_DIM),
    ] {
        let font = FontId::proportional(size * s);
        painter.text(
            Pos2::new(rect.center().x, rect.max.y - y * s),
            Align2::CENTER_TOP,
            fit_text(painter, text, &font, max_width),
            font,
            color.gamma_multiply(alpha),
        );
    }
}

fn percent(fraction: f32) -> String {
    format!("{}%", (fraction.clamp(0.0, 1.0) * 100.0).round() as u32)
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

/// One row of a menu.
pub struct MenuItem {
    label: String,
    hint: String,
    enabled: bool,
}

impl MenuItem {
    pub fn new(label: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            hint: hint.into(),
            enabled: true,
        }
    }
}

/// The disc start menu. Returns the index of a clicked option.
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
    let has_resume = list.iter().any(|c| matches!(c, Choice::Resume(_)));
    let items: Vec<MenuItem> = list
        .iter()
        .map(|choice| {
            let waiting = matches!(choice, Choice::Resume(_) | Choice::MainMovie) && !scan_ready;
            let mut item = match choice {
                Choice::Resume(pos) => MenuItem::new(
                    tr!("Resume from {time}", time = resume::format_time(*pos)),
                    tr!("Continues where you stopped"),
                ),
                Choice::MainMovie if has_resume => MenuItem::new(
                    tr!("Play from beginning"),
                    tr!("Starts the movie from the start"),
                ),
                Choice::MainMovie => {
                    MenuItem::new(tr!("Play movie"), tr!("Starts the main movie right away"))
                }
                Choice::DiscMenu => MenuItem::new(
                    tr!("Disc menu"),
                    tr!("Starts the disc like a normal player"),
                ),
                Choice::Eject => MenuItem::new(tr!("Eject disc"), tr!("Opens the tray")),
                Choice::Back => MenuItem::new(tr!("Back"), ""),
            };
            if waiting {
                item.hint = match tile.progress {
                    Some(f) => format!("{} {}", tile.subtitle, percent(f)),
                    None => tile.subtitle.clone(),
                };
                item.enabled = false;
            }
            item
        })
        .collect();
    paint_menu(ui, painter, screen, s, &tile.title, &items, selected)
}

/// A centered menu panel. Returns the index of a clicked option.
pub fn paint_menu(
    ui: &egui::Ui,
    painter: &egui::Painter,
    screen: Rect,
    s: f32,
    title: &str,
    items: &[MenuItem],
    selected: usize,
) -> Option<usize> {
    const ROW_STEP: f32 = 105.0;
    painter.rect_filled(screen, 0.0, Color32::from_black_alpha(190));
    let height = 150.0 + items.len() as f32 * ROW_STEP + 30.0;
    let panel = Rect::from_center_size(screen.center(), Vec2::new(720.0 * s, height * s));
    painter.rect_filled(panel, CornerRadius::same((24.0 * s) as u8), TILE_BG);
    painter.text(
        Pos2::new(panel.center().x, panel.min.y + 50.0 * s),
        Align2::CENTER_TOP,
        fit_text(
            painter,
            title,
            &FontId::proportional(40.0 * s),
            panel.width() - 60.0 * s,
        ),
        FontId::proportional(40.0 * s),
        TEXT,
    );
    let mut clicked = None;
    for (i, item) in items.iter().enumerate() {
        let row = Rect::from_min_size(
            Pos2::new(
                panel.min.x + 60.0 * s,
                panel.min.y + (150.0 + i as f32 * ROW_STEP) * s,
            ),
            Vec2::new(panel.width() - 120.0 * s, (ROW_STEP - 15.0) * s),
        );
        let fill = if i == selected {
            ACCENT
        } else {
            Color32::from_rgb(0x2e, 0x33, 0x45)
        };
        let alpha = if item.enabled { 1.0 } else { 0.45 };
        painter.rect_filled(
            row,
            CornerRadius::same((14.0 * s) as u8),
            fill.gamma_multiply(alpha),
        );
        let has_hint = !item.hint.is_empty();
        painter.text(
            Pos2::new(
                row.min.x + 30.0 * s,
                row.center().y - if has_hint { 14.0 * s } else { 0.0 },
            ),
            Align2::LEFT_CENTER,
            &item.label,
            FontId::proportional(32.0 * s),
            TEXT.gamma_multiply(alpha),
        );
        if has_hint {
            painter.text(
                Pos2::new(row.min.x + 30.0 * s, row.center().y + 22.0 * s),
                Align2::LEFT_CENTER,
                &item.hint,
                FontId::proportional(20.0 * s),
                TEXT.gamma_multiply(0.75 * alpha),
            );
        }
        if ui
            .interact(row, ui.make_persistent_id(("menu", i)), Sense::click())
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

/// Shorten `text` with "…" so that it fits into `max_width` (measured with the real font).
fn fit_text(painter: &egui::Painter, text: &str, font: &FontId, max_width: f32) -> String {
    let width = |t: String| painter.layout_no_wrap(t, font.clone(), TEXT).size().x;
    if width(text.to_string()) <= max_width {
        return text.to_string();
    }
    // Binary search for the longest prefix that still fits together with "…".
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect();
        if width(format!("{}…", candidate.trim_end())) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let cut: String = chars[..lo].iter().collect();
    format!("{}…", cut.trim_end())
}

fn elide(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max_chars - 1).collect();
        format!("{}…", cut.trim_end())
    }
}
