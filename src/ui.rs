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
use crate::config::{FolderSettings, View};
use crate::control::SharedConfig;
use crate::control::UiRequest;
use crate::disc::{self, DriveReport, DriveState, MediaKind};
use crate::gamepad::{PadNav, Pads};
use crate::library::{self, Entry, EntryKind, Filter};
use crate::limits::{self, Status};
use crate::lock::{LockAction, LockInfo, LockView, Nav, Purpose};
use crate::makemkv::{self, DiscInfo, ScanProgress};
use crate::mpris::{MediaCommand, Mpris, Status as PlayStatus};
use crate::pictures::{self, Picture};
use crate::player::{self, MovieOptions, PlayMode};
use crate::pong::{PongAction, PongInput, PongState, PongView};
use crate::recent;
use crate::remote::{self, Action, Calibration, RemoteMap};
use crate::stats::{Activity, Category};
use crate::tr;
use crate::usage::SharedTracker;
use crate::volume::{self, Volume, VolumeCommand};
use crate::{cover, resume, sites, store};
use crate::{inhibit, screensaver};

/// How long the remote's home button must be held to go to the home screen.
const HOME_HOLD: Duration = Duration::from_secs(3);
/// See `App::just_opened`.
const JUST_OPENED: Duration = Duration::from_millis(350);
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
    /// A DOS game's private copy is being made; then DOSBox starts.
    PreparingEmulator {
        job: Receiver<Result<crate::emulator::Prepared, String>>,
        emulator: Box<crate::config::EmulatorSettings>,
    },
    /// The built-in Ping Pong game ("Games" row).
    Pong(Box<PongView>),
    /// Brick Breaker ("Games" row).
    Bricks(Box<crate::bricks::BricksView>),
    /// Block Stacker ("Games" row).
    Blocks(Box<crate::blocks::BlocksView>),
    /// Star Dodger ("Games" row).
    Dodger(Box<crate::dodger::DodgerView>),
    /// "Exit the media center?" (exit button / Ctrl+Q).
    ConfirmExit {
        selected: usize,
    },
    /// Choosing another home screen (asks for the PIN, if one is set).
    SwitchHome {
        selected: usize,
    },
}

/// The round buttons at the top right of the home screen ("up" from the top row).
#[derive(Clone, Copy, PartialEq, Eq)]
enum TopButton {
    /// Switch the home screen (only with more than one).
    Screens,
    Exit,
    Lock,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlayKind {
    Disc,
    File,
    Website,
    Program,
}

impl PlayKind {
    /// Played by one of our media players (not a website or program window).
    fn is_media(self) -> bool {
        matches!(self, PlayKind::Disc | PlayKind::File)
    }
}

/// The rows of the home screen (only rows with content are shown, except discs).
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Discs = 0,
    Folders = 1,
    Websites = 2,
    Games = 3,
    Programs = 4,
    Emulators = 5,
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

/// Should files played from this folder be listed under "Recently played"?
fn remembers_files(config: &View, folder_id: &str) -> bool {
    config
        .folders
        .iter()
        .find(|f| f.id == folder_id)
        .is_none_or(|f| f.remember_recent)
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
        let recent = if sub.is_empty() && error.is_none() && folder.remember_recent {
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
    /// Gamepads (also read in `remote::spawn`).
    pub pads: Receiver<remote::PadEvent>,
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

/// The running screensaver (see `screensaver`).
enum Saver {
    /// The moving title, painted by us.
    Title { started: Instant },
    /// mpv with the aerial videos, in front of our window.
    Aerial { child: Child, started: Instant },
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
    /// Cancel flags of running disc scans (by disc key).
    scan_cancel: HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    scan_rx: Receiver<ScanEvent>,
    scans: HashMap<String, ScanState>,
    eject_tx: Sender<Result<(), String>>,
    eject_rx: Receiver<Result<(), String>>,
    config: SharedConfig,
    screen: Screen,
    /// Focused tile on the home screen: (row, column). Row 0: discs, row 1: folders.
    focus: (usize, usize),
    /// The lock button (top right) has the focus instead of a tile.
    top_focus: Option<TopButton>,
    /// Asking for the PIN to exit the media center.
    exit_view: Option<LockView>,
    /// Since when a gamepad holds select + start (closes a program).
    pad_close_since: Option<Instant>,
    /// The power button being held (log out / shut down).
    power_hold: crate::power::PowerHold,
    /// Horizontal scroll per row kind, vertical scroll of the rows.
    row_scroll_x: [f32; 4],
    rows_scroll_y: f32,
    /// The browser window program (`curator-web`), if installed.
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
    /// The screensaver, while it runs, and the last input (any key, click, pointer move,
    /// remote button) it waits for.
    saver: Option<Saver>,
    last_activity: Instant,
    /// Keeps the computer from blanking the screen while we have our own screensaver.
    inhibitor: inhibit::Inhibitor,
    /// Connected gamepads, and when the game was last advanced.
    pads: Pads,
    pong_tick: Instant,
    /// When the mouse was last used (the cursor hides in games).
    last_mouse: Instant,
    /// The home screen shown (to notice a switch), the usage statistics, and what the
    /// running player / website is (for them).
    shown_screen: String,
    stats: crate::stats::Recorder,
    playing_activity: Option<Activity>,
    /// The current kind of screen and since when (see `just_opened`).
    screen_kind: std::mem::Discriminant<Screen>,
    screen_since: Instant,
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
            scan_cancel: HashMap::new(),
            scan_rx,
            scans: HashMap::new(),
            eject_tx,
            eject_rx,
            config,
            screen: Screen::Home,
            focus: (0, 0),
            top_focus: None,
            exit_view: None,
            pad_close_since: None,
            power_hold: Default::default(),
            row_scroll_x: [0.0; 4],
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
            saver: None,
            last_activity: Instant::now(),
            inhibitor: inhibit::Inhibitor::spawn(),
            pads: Pads::default(),
            pong_tick: Instant::now(),
            last_mouse: Instant::now(),
            shown_screen: String::new(),
            stats: Default::default(),
            playing_activity: None,
            screen_kind: std::mem::discriminant(&Screen::Home),
            screen_since: Instant::now(),
        }
    }

    /// In a game with a gamepad connected: no mouse cursor while the mouse isn't used.
    fn hide_cursor_in_games(&mut self, ctx: &egui::Context) {
        let moved = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::PointerMoved(_)
                        | egui::Event::PointerButton { .. }
                        | egui::Event::MouseWheel { .. }
                )
            })
        });
        if moved {
            self.last_mouse = Instant::now();
        }
        let in_game = matches!(
            self.screen,
            Screen::Pong(_) | Screen::Bricks(_) | Screen::Blocks(_) | Screen::Dodger(_)
        );
        let idle = self.last_mouse.elapsed();
        if crate::gamepad::hide_cursor(in_game, self.pads.count(), idle) {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        } else if in_game && self.pads.count() > 0 {
            // Menus don't repaint by themselves: be there when it's time to hide it.
            ctx.request_repaint_after(crate::gamepad::CURSOR_HIDES_AFTER.saturating_sub(idle));
        }
    }

    /// Did the current screen open less than `JUST_OPENED` ago? Then it takes no OK / clicks.
    fn just_opened(&self) -> bool {
        self.screen_since.elapsed() < JUST_OPENED
    }

    /// The current config (may change at any time through a reload).
    /// The active home screen's settings plus the shared ones, and the config's error.
    fn config(&self) -> (View, Option<String>) {
        let active = self
            .tracker
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .active()
            .map(str::to_string);
        let state = self.config.lock().unwrap_or_else(|p| p.into_inner());
        (state.config.view(active.as_deref()), state.error.clone())
    }

    /// The websites of all home screens (for their logos).
    fn all_websites(&self) -> Vec<crate::config::WebsiteSettings> {
        let state = self.config.lock().unwrap_or_else(|p| p.into_inner());
        state
            .config
            .home_screens
            .iter()
            .flat_map(|s| s.websites.iter().cloned())
            .collect()
    }

    /// What is used right now, for the statistics (`None`: menus, home screen, lock screen).
    fn current_activity(&self, config: &View) -> Option<Activity> {
        if self.lock.is_some() {
            return None;
        }
        let folder_name = |id: &str| {
            config
                .folders
                .iter()
                .find(|f| f.id == id)
                .map_or_else(|| id.to_string(), |f| f.name.clone())
        };
        match &self.screen {
            Screen::Playing { stopping: None, .. } => self.playing_activity.clone(),
            Screen::Music(view) => {
                let index = view.start + view.player.state().playlist_pos.unwrap_or(0);
                let track = view.tracks.get(index)?;
                let detail = track
                    .path
                    .strip_prefix(&view.root)
                    .map_or_else(|_| track.name.clone(), |p| p.to_string_lossy().into_owned());
                Some(Activity::new(
                    Category::Music,
                    folder_name(&view.back.folder_id),
                    detail,
                ))
            }
            Screen::Picture(view) => Some(Activity::new(
                Category::Pictures,
                folder_name(&view.back.folder_id),
                view.back.sub.join("/"),
            )),
            Screen::Pong(_) => Some(Activity::new(Category::Game, "Ping Pong", "")),
            Screen::Bricks(_) => Some(Activity::new(Category::Game, "Brick Breaker", "")),
            Screen::Blocks(_) => Some(Activity::new(Category::Game, "Block Stacker", "")),
            Screen::Dodger(_) => Some(Activity::new(Category::Game, "Star Dodger", "")),
            _ => None,
        }
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
    /// themselves (`player::MPV_HOME_SCRIPT`, `curator-web`).
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

    /// Start, run and end the screensaver. True while it runs (or this frame's input ended
    /// it): then nothing else handles input or paints.
    fn screensaver(
        &mut self,
        painter: &egui::Painter,
        screen_rect: Rect,
        s: f32,
        activity: bool,
    ) -> bool {
        let (config, _) = self.config();
        let settings = &config.screensaver;
        self.inhibitor
            .set(settings.mode != crate::config::ScreensaverMode::Off);
        // mpv ended by itself: a key it got, or it failed (then the title for this round).
        if let Some(Saver::Aerial { child, started }) = &mut self.saver
            && let Ok(Some(status)) = child.try_wait()
        {
            let failed = !status.success() && started.elapsed() < Duration::from_secs(5);
            if failed {
                log::warn!("screensaver: mpv failed ({status}); showing the title instead");
                self.saver = Some(Saver::Title {
                    started: Instant::now(),
                });
            } else {
                self.saver = None;
                self.last_activity = Instant::now();
                return true;
            }
        }
        // Never over a player, website or the remote setup; only if turned on.
        let blocked = settings.mode == crate::config::ScreensaverMode::Off
            || matches!(self.screen, Screen::Playing { .. } | Screen::Calibrate(_))
            || matches!(&self.screen, Screen::Pong(v) if matches!(v.state, PongState::Playing(_)))
            || matches!(&self.screen, Screen::Bricks(v)
                if matches!(v.state, crate::bricks::BricksState::Playing(_)))
            || matches!(&self.screen, Screen::Blocks(v)
                if matches!(v.state, crate::blocks::BlocksState::Playing(_)))
            || matches!(&self.screen, Screen::Dodger(v)
                if matches!(v.state, crate::dodger::DodgerState::Playing(_)));
        if activity || blocked {
            self.last_activity = Instant::now();
            if self.saver.is_some() {
                self.stop_screensaver();
                return true; // the waking input does nothing else
            }
            return false;
        }
        if self.saver.is_none() {
            let idle = self.last_activity.elapsed();
            if !screensaver::due(settings, idle) {
                let wait =
                    Duration::from_secs(u64::from(settings.idle_minutes) * 60).saturating_sub(idle);
                self.ctx
                    .request_repaint_after(wait + Duration::from_millis(50));
                return false;
            }
            self.saver = Some(self.start_screensaver(settings));
        }
        match &self.saver {
            Some(Saver::Title { started }) => {
                let t = started.elapsed().as_secs_f64();
                let [r, g, b] = screensaver::background(t);
                painter.rect_filled(screen_rect, 0.0, Color32::from_rgb(r, g, b));
                let title = if config.home.title.is_empty() {
                    "curator".to_string()
                } else {
                    config.home.title.clone()
                };
                let galley = painter.layout_no_wrap(
                    title,
                    FontId::proportional(84.0 * s),
                    TEXT.gamma_multiply(0.55),
                );
                let size = galley.size();
                let [x, y] = screensaver::title_position(
                    t as f32,
                    [screen_rect.width(), screen_rect.height()],
                    [size.x, size.y],
                );
                painter.galley(screen_rect.min + Vec2::new(x, y), galley, TEXT);
                self.ctx.request_repaint_after(Duration::from_millis(33));
            }
            Some(Saver::Aerial { .. }) => {
                // mpv is in front; our window stays dark behind it.
                painter.rect_filled(screen_rect, 0.0, Color32::BLACK);
                self.ctx.request_repaint_after(Duration::from_millis(500));
            }
            None => {}
        }
        true
    }

    fn start_screensaver(&self, settings: &crate::config::ScreensaverSettings) -> Saver {
        let started = Instant::now();
        if settings.mode == crate::config::ScreensaverMode::Aerial {
            let files = store::aerial_dir()
                .map(|dir| screensaver::aerial_files(&dir, settings))
                .unwrap_or_default();
            if files.is_empty() {
                log::info!("screensaver: no aerial video downloaded yet; showing the title");
            } else if let Some(mut cmd) =
                player::screensaver_command(&files, store::prepare_player_cache().as_deref())
            {
                match cmd.spawn() {
                    Ok(child) => {
                        log::info!("screensaver: {} aerial videos", files.len());
                        return Saver::Aerial { child, started };
                    }
                    Err(e) => log::warn!("screensaver: could not start mpv: {e}"),
                }
            }
        }
        log::info!("screensaver: title");
        Saver::Title { started }
    }

    fn stop_screensaver(&mut self) {
        if let Some(Saver::Aerial { mut child, .. }) = self.saver.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        log::info!("screensaver: ended");
    }

    /// The Ping Pong game: paddles from gamepads, keyboard (W/S, arrows) and the remote.
    fn pong_screen(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen_rect: Rect,
        s: f32,
        keys: &Keys,
        nav: PadNav,
    ) {
        let dt = self.pong_tick.elapsed().as_secs_f32();
        self.pong_tick = Instant::now();
        let held = |up: Key, down: Key| {
            ui.ctx().input(|i| {
                f32::from(u8::from(i.key_down(down))) - f32::from(u8::from(i.key_down(up)))
            })
        };
        let (keys_left, keys_right) = (held(Key::W, Key::S), held(Key::ArrowUp, Key::ArrowDown));
        // Remote arrows only its input device reports (held down).
        let remote: f32 = self
            .remote_down
            .iter()
            .filter_map(|c| self.remote_map.device_action(*c))
            .map(|a| match a {
                Action::Up => -1.0,
                Action::Down => 1.0,
                _ => 0.0,
            })
            .sum();
        let pads = self.pads.vertical();
        let gamepads = self.pads.count();
        let just_opened = self.just_opened();
        let Screen::Pong(view) = &mut self.screen else {
            return;
        };
        let two_players = match &view.state {
            PongState::Playing(g) | PongState::Paused { game: g, .. } => g.computer.is_none(),
            _ => false,
        };
        // 2 players: 1st gamepad + W/S left, 2nd gamepad + arrows / remote right.
        // 1 player: everything moves the left paddle.
        let (left, right) = if two_players {
            (
                pads.first().copied().unwrap_or(0.0) + keys_left,
                pads.get(1).copied().unwrap_or(0.0) + keys_right + remote,
            )
        } else {
            (
                pads.iter().sum::<f32>() + keys_left + keys_right + remote,
                0.0,
            )
        };
        let input = PongInput {
            left,
            right,
            up: keys.up || nav.up,
            down: keys.down || nav.down,
            enter: keys.enter || nav.enter || nav.pause,
            back: keys.back || nav.back,
            pause: nav.pause,
            gamepads,
            clicks: !just_opened,
        };
        if let PongAction::Quit = view.show(ui, painter, screen_rect, s, dt, &input) {
            self.screen = Screen::Home;
        }
    }

    /// Star Dodger: left / right from gamepads, keyboard (arrows, A/D) and the remote.
    fn dodger_screen(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen_rect: Rect,
        s: f32,
        keys: &Keys,
        nav: PadNav,
    ) {
        let dt = self.pong_tick.elapsed().as_secs_f32();
        self.pong_tick = Instant::now();
        let keys_x = ui.ctx().input(|i| {
            let held = |k: Key| f32::from(u8::from(i.key_down(k)));
            held(Key::ArrowRight) + held(Key::D) - held(Key::ArrowLeft) - held(Key::A)
        });
        let remote: f32 = self
            .remote_down
            .iter()
            .filter_map(|c| self.remote_map.device_action(*c))
            .map(|a| match a {
                Action::Left => -1.0,
                Action::Right => 1.0,
                _ => 0.0,
            })
            .sum();
        let (typed, erase, space) = typed_letters(ui.ctx());
        let naming = matches!(&self.screen, Screen::Dodger(v)
            if matches!(v.state, crate::dodger::DodgerState::EnterName { .. }));
        let input = crate::dodger::DodgerInput {
            x: self.pads.horizontal() + keys_x + remote,
            up: keys.up || nav.up,
            down: keys.down || nav.down,
            // While typing a name, Space is a letter.
            enter: (keys.enter && !(naming && space)) || nav.enter || nav.pause,
            back: keys.back || nav.back,
            pause: nav.pause,
            clicks: !self.just_opened(),
            left: keys.left || nav.left,
            right: keys.right || nav.right,
            typed,
            erase,
        };
        let Screen::Dodger(view) = &mut self.screen else {
            return;
        };
        if view.show(ui, painter, screen_rect, s, dt, &input) {
            self.screen = Screen::Home;
        }
    }

    /// Brick Breaker: paddle from gamepads (stick / d-pad, any button held = faster), the
    /// keyboard (arrows or A/D, Space = faster) and the remote (arrows, OK held = faster).
    fn bricks_screen(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen_rect: Rect,
        s: f32,
        keys: &Keys,
        nav: PadNav,
    ) {
        let dt = self.pong_tick.elapsed().as_secs_f32();
        self.pong_tick = Instant::now();
        let (keys_x, keys_fast) = ui.ctx().input(|i| {
            let held = |k: Key| f32::from(u8::from(i.key_down(k)));
            (
                held(Key::ArrowRight) + held(Key::D) - held(Key::ArrowLeft) - held(Key::A),
                i.key_down(Key::Space) || i.key_down(Key::Enter) || i.modifiers.shift,
            )
        });
        let remote: f32 = self
            .remote_down
            .iter()
            .filter_map(|c| self.remote_map.device_action(*c))
            .map(|a| match a {
                Action::Left => -1.0,
                Action::Right => 1.0,
                _ => 0.0,
            })
            .sum();
        let (typed, erase, space) = typed_letters(ui.ctx());
        let naming = matches!(&self.screen, Screen::Bricks(v)
            if matches!(v.state, crate::bricks::BricksState::EnterName { .. }));
        let input = crate::bricks::BricksInput {
            x: self.pads.horizontal() + keys_x + remote,
            fast: keys_fast || self.pads.button_held() || self.ok_held(ui.ctx()),
            up: keys.up || nav.up,
            down: keys.down || nav.down,
            // While typing a name, Space is a letter.
            enter: (keys.enter && !(naming && space)) || nav.enter || nav.pause,
            back: keys.back || nav.back,
            pause: nav.pause,
            clicks: !self.just_opened(),
            left: keys.left || nav.left,
            right: keys.right || nav.right,
            typed,
            erase,
        };
        let Screen::Bricks(view) = &mut self.screen else {
            return;
        };
        if view.show(ui, painter, screen_rect, s, dt, &input) {
            self.screen = Screen::Home;
        }
    }

    /// Block Stacker. 1 player: everything controls the field. 2 players: 1st gamepad and
    /// W/A/S/D on the left field, 2nd gamepad, the arrow keys and the remote on the right one.
    fn blocks_screen(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen_rect: Rect,
        s: f32,
        keys: &Keys,
        nav: PadNav,
    ) {
        use crate::blocks::{BlocksState, Held, Mode};
        let dt = self.pong_tick.elapsed().as_secs_f32();
        self.pong_tick = Instant::now();
        let ctx = ui.ctx();
        let (typed, erase, space) = typed_letters(ctx);
        let remote = |wanted: Action| {
            self.remote_down
                .iter()
                .any(|c| self.remote_map.device_action(*c) == Some(wanted))
        };
        // Arrow keys, the remote, OK / up to rotate.
        let arrows = ctx.input(|i| Held {
            left: i.key_down(Key::ArrowLeft) || remote(Action::Left),
            right: i.key_down(Key::ArrowRight) || remote(Action::Right),
            down: i.key_down(Key::ArrowDown) || remote(Action::Down),
            rotate_right: keys.up || keys.enter,
            rotate_left: i.key_pressed(Key::Z),
        });
        let letters = ctx.input(|i| Held {
            left: i.key_down(Key::A),
            right: i.key_down(Key::D),
            down: i.key_down(Key::S),
            rotate_right: i.key_pressed(Key::W),
            rotate_left: i.key_pressed(Key::Q),
        });
        let or = |a: Held, b: Held| Held {
            left: a.left || b.left,
            right: a.right || b.right,
            down: a.down || b.down,
            rotate_right: a.rotate_right || b.rotate_right,
            rotate_left: a.rotate_left || b.rotate_left,
        };
        let pads: Vec<Held> = self
            .pads
            .players()
            .iter()
            .map(|p| Held {
                left: p.x <= -0.5,
                right: p.x >= 0.5,
                down: p.y >= 0.5,
                rotate_right: p.rotate_right,
                rotate_left: p.rotate_left,
            })
            .collect();
        let clicks = !self.just_opened();
        let Screen::Blocks(view) = &mut self.screen else {
            return;
        };
        let versus = matches!(&view.state, BlocksState::Playing(g) if g.mode == Mode::Versus);
        let held = if versus {
            [
                or(pads.first().copied().unwrap_or_default(), letters),
                or(pads.get(1).copied().unwrap_or_default(), arrows),
            ]
        } else {
            [
                pads.iter().fold(or(arrows, letters), |a, b| or(a, *b)),
                Held::default(),
            ]
        };
        let playing = matches!(view.state, BlocksState::Playing(_));
        let naming = matches!(view.state, BlocksState::EnterName { .. });
        let input = crate::blocks::BlocksInput {
            held,
            up: keys.up || nav.up,
            down: keys.down || nav.down,
            left: keys.left || nav.left,
            right: keys.right || nav.right,
            enter: (keys.enter && !(naming && space)) || nav.enter || nav.pause,
            // While playing, B / circle rotates; only Start / Select pause on a gamepad.
            back: keys.back || (nav.back && !playing),
            pause: nav.pause,
            clicks,
            typed,
            erase,
        };
        let Screen::Blocks(view) = &mut self.screen else {
            return;
        };
        if view.show(ui, painter, screen_rect, s, dt, &input) {
            self.screen = Screen::Home;
        }
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
            && kind.is_media()
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
                mut child,
                started,
                kind,
                stopping,
                ..
            } => {
                if stopping.is_none() {
                    stop_child(&mut child);
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
    fn check_logos(&mut self, websites: &[crate::config::WebsiteSettings]) {
        let wanted: Vec<_> = websites.iter().filter(|w| w.show_logo).collect();
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
            // The disc was taken out (or the tray opened) while it plays: the player would
            // hang on read errors and cover the screen, so stop it.
            let disc_in = self
                .drives
                .iter()
                .any(|r| matches!(&r.state, DriveState::Loaded(d) if d.kind == MediaKind::BluRay));
            // Scans of discs that aren't in the drive any more: end them (makemkvcon would
            // hang and keep the drive busy) and forget them, so the disc is read again
            // when it comes back. Finished scans stay (they are cached anyway).
            let loaded: Vec<String> = self
                .drives
                .iter()
                .filter_map(|r| match &r.state {
                    DriveState::Loaded(d) => Some(d.key(&r.device)),
                    _ => None,
                })
                .collect();
            self.scans.retain(|key, state| {
                let keep = matches!(state, ScanState::Done(_)) || loaded.contains(key);
                if !keep && let Some(cancel) = self.scan_cancel.remove(key) {
                    log::info!("disc {key} was taken out: stopping its scan");
                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                keep
            });
            if !disc_in
                && let Screen::Playing {
                    child,
                    kind: PlayKind::Disc,
                    stopping: stopping @ None,
                    ..
                } = &mut self.screen
            {
                log::info!("the disc is gone: stopping the player");
                stop_child(child);
                *stopping = Some(Instant::now());
            }
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
                // Results of scans that were forgotten (disc taken out) are dropped.
                ScanEvent::Done(key, Ok(info)) => {
                    self.scan_cancel.remove(&key);
                    if let Some(state) = self.scans.get_mut(&key) {
                        *state = ScanState::Done(info);
                    }
                }
                ScanEvent::Done(key, Err(e)) => {
                    log::warn!("disc scan of {key} failed: {e}");
                    self.scan_cancel.remove(&key);
                    if let Some(state) = self.scans.get_mut(&key) {
                        *state = ScanState::Failed;
                    }
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
            let websites = self.all_websites();
            self.check_logos(&websites);
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
                    player::stop_leftover_makemkv_servers();
                    log::info!("reading disc {key} with makemkvcon");
                    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    self.scan_cancel.insert(key.clone(), cancel.clone());
                    let (tx, device, ctx) = (
                        self.scan_tx.clone(),
                        report.device.clone(),
                        self.ctx.clone(),
                    );
                    std::thread::spawn(move || {
                        let result = makemkv::scan(&device, &cancel, |p| {
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
                        player::signal_group(child.id(), libc::SIGKILL);
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
                        PlayKind::Program => {
                            tr!(
                                "This program stopped right away ({status}).",
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
            // A program's main process is gone; don't leave anything it started behind.
            if let Screen::Playing {
                kind: PlayKind::Program,
                child,
                ..
            } = &self.screen
            {
                let pid = child.id();
                std::thread::spawn(move || player::end_group(pid));
            }
            // A disc player is gone: its makemkvcon helper may still hold the drive.
            if matches!(
                self.screen,
                Screen::Playing {
                    kind: PlayKind::Disc,
                    ..
                }
            ) {
                player::stop_leftover_makemkv_servers();
            }
            self.screen = screen;
        }
        if let Screen::PreparingEmulator { job, emulator } = &self.screen {
            match job.try_recv() {
                Ok(Ok(prepared)) => {
                    let emulator = emulator.clone();
                    match prepared {
                        crate::emulator::Prepared::Dos(mounts) => {
                            self.start_dos(&emulator, &mounts)
                        }
                        crate::emulator::Prepared::Snes(rom) => self.start_snes(&emulator, &rom),
                    }
                }
                Ok(Err(text)) => self.screen = message(text),
                Err(mpsc::TryRecvError::Empty) => {
                    self.ctx.request_repaint_after(Duration::from_millis(200));
                }
                Err(mpsc::TryRecvError::Disconnected) => self.screen = Screen::Home,
            }
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
            && (self.saver.is_none() || matches!(self.screen, Screen::Music(_)))
            && (!matches!(self.screen, Screen::Home)
                || self.last_input.elapsed() < Duration::from_secs(60));

        let (config, _) = self.config();
        // Another home screen (switched from the admin panel or on the TV): start on its
        // home screen.
        if config.screen_id != self.shown_screen {
            if !self.shown_screen.is_empty() {
                log::info!("home screen: {}", config.screen_id);
                self.go_home();
                self.focus = (0, 0);
                self.top_focus = None;
                self.row_scroll_x = [0.0; 4];
                self.rows_scroll_y = 0.0;
                self.folder_checked = None; // its folders are checked right away
            }
            self.shown_screen = config.screen_id.clone();
        }
        let activity = self.current_activity(&config);
        self.stats.update(
            now.timestamp(),
            activity,
            &config.screen_id,
            config.statistics.enabled,
        );
        let mut tracker = self.tracker.lock().unwrap_or_else(|p| p.into_inner());
        if active {
            self.usage_carry += dt;
            let whole = self.usage_carry.floor();
            if whole >= 1.0 {
                tracker.add_usage(&config.screen_id, now.date_naive(), whole as u64);
                self.usage_carry -= whole;
            }
        }
        let status = limits::evaluate(
            &config.limits,
            &tracker.situation(&config.screen_id),
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

    /// Restart (asked by the admin panel): stop everything that plays, save the usage, then
    /// replace this process with the installed program (`exec`, so a kiosk session keeps
    /// running; the screen is dark for a moment). Returns only if that failed.
    fn restart(&mut self) {
        log::info!("restarting (asked by the admin panel)");
        self.stop_all_now();
        let error = exec_self();
        log::error!("restart failed: {error}");
        // In the kiosk session its loop starts us again; elsewhere keep running.
        if store::started_by_kiosk(std::path::Path::new("/proc"), std::process::id()) {
            log::error!("exiting, so the kiosk session starts the media center again");
            std::process::exit(1);
        }
    }

    /// Exit the media center (exit button / Ctrl+Q, after asking): in the kiosk session this
    /// ends the session, back to the login screen.
    fn exit_now(&mut self) {
        log::info!("exiting (exit button)");
        self.stop_all_now();
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Shut the computer down (power button held 8 s, or the menu; with a PIN, after it).
    fn shut_down(&mut self) {
        log::info!("shutting the computer down (power button)");
        self.stop_all_now();
        if let Err(e) = crate::power::power_off() {
            log::warn!("shutting down failed: {e}");
            self.screen = Screen::Message {
                text: tr!("The computer could not be shut down: {error}", error = e),
                back: None,
            };
        }
    }

    /// Log out / shut down from the power button: with a PIN set, only after the PIN.
    fn power_action(&mut self, shutdown: bool, has_pin: bool) {
        self.top_focus = None;
        if has_pin {
            self.screen = Screen::Home;
            self.exit_view = Some(LockView::for_purpose(if shutdown {
                Purpose::ShutDown
            } else {
                Purpose::Exit
            }));
        } else if shutdown {
            self.shut_down();
        } else {
            self.exit_now();
        }
    }

    /// The remote's power key: shut down at once, like a TV remote (with a PIN, after it).
    /// Whatever plays stops first, because the PIN question is on our own window.
    fn remote_power_key(&mut self, has_pin: bool) {
        if has_pin {
            self.go_home();
            self.top_focus = None;
            self.exit_view = Some(LockView::for_purpose(Purpose::ShutDown));
        } else {
            self.shut_down();
        }
    }

    /// Is OK held down right now (Enter / Space, a learned key, or the remote's OK that only
    /// its input device reports)?
    fn ok_held(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| {
            i.key_down(Key::Enter)
                || i.key_down(Key::Space)
                || self.remote_map.window_keys().any(|(name, action)| {
                    action == Action::Ok && Key::from_name(name).is_some_and(|k| i.key_down(k))
                })
        }) || self
            .remote_down
            .iter()
            .any(|c| self.remote_map.device_action(*c) == Some(Action::Ok))
    }

    /// Show another home screen (its settings, items and time limits apply from now on).
    fn switch_home_screen(&mut self, id: &str) {
        log::info!("switching to home screen {id} on the TV");
        self.tracker
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .set_active(id);
        self.ctx.request_repaint();
    }

    /// Before restarting / exiting: end the screensaver, players and music (waiting for
    /// them, so none is left behind) and save the usage.
    fn stop_all_now(&mut self) {
        self.stats.finish(chrono::Local::now().timestamp());
        self.stop_screensaver();
        match std::mem::replace(&mut self.screen, Screen::Home) {
            Screen::Playing { mut child, .. } => {
                stop_child(&mut child);
                let deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < deadline && matches!(child.try_wait(), Ok(None)) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                let _ = child.kill();
                let _ = child.wait();
                player::end_group(child.id());
            }
            Screen::Music(mut view) => view.player.stop(),
            _ => {}
        }
        self.tracker
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .save_now();
    }

    /// Time is up: stop players, music and websites, close viewers.
    fn stop_everything(&mut self) {
        match std::mem::replace(&mut self.screen, Screen::Home) {
            Screen::Playing {
                mut child,
                started,
                kind,
                stopping,
                ..
            } => {
                if stopping.is_none() {
                    stop_child(&mut child);
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

    /// The PIN pad: the lock screen (`exit` false), or asking for the PIN to exit.
    fn lock_screen(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen_rect: Rect,
        s: f32,
        keys: &Keys,
        exit: bool,
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
            let view = if exit {
                &mut self.exit_view
            } else {
                &mut self.lock
            };
            if let Some(view) = view {
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
        let slot = if exit {
            &mut self.exit_view
        } else {
            &mut self.lock
        };
        let Some(view) = slot else { return };
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
                    .unlock(&config.screen_id, now, minutes);
                if config.statistics.enabled {
                    crate::stats::prune(config.statistics.keep_days);
                }
                self.lock = None;
            }
            LockAction::Confirmed(purpose) => {
                self.exit_view = None;
                match purpose {
                    Purpose::Exit => self.exit_now(),
                    Purpose::ShutDown => self.shut_down(),
                    Purpose::Switch { id, .. } => self.switch_home_screen(&id),
                }
            }
            LockAction::Cancel => self.exit_view = None,
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
        let (config, _) = self.config();
        let buttons = 1.0
            + f32::from(u8::from(config.parental.pin_hash.is_some()))
            + f32::from(u8::from(config.screens.len() > 1));
        let rect = Rect::from_min_size(
            Pos2::new(
                // Left of the round buttons (margin 80, 76 wide, 24 apart).
                screen_rect.max.x - (80.0 + 100.0 * buttons) * s - box_size.x,
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
        let playing = matches!(self.screen, Screen::Playing { .. })
            || matches!(self.saver, Some(Saver::Aerial { .. }));
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
        self.playing_activity = Some(Activity::new(
            Category::Disc,
            tile.title.clone(),
            match mode {
                PlayMode::DiscMenu => "disc menu",
                _ => "movie",
            },
        ));
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

    /// Open a file from the folder browser: pictures and music inside curator, videos
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

    /// Open a website in the locked-down browser window (`curator-web`).
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
        self.playing_activity = Some(Activity::new(
            Category::Website,
            site.name.clone(),
            website_domain(site),
        ));
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

    /// Start an emulator game once its files are in our folder (in a thread: copying can
    /// take a while).
    fn open_emulator(&mut self, emulator: &crate::config::EmulatorSettings) {
        use crate::config::EmulatorSystem;
        use crate::emulator::{self as emu, Prepared};
        let (Some(data), Some(library)) =
            (store::emulator_data_dir(), store::emulator_library_dir())
        else {
            self.screen = message(tr!(
                "The emulator could not be prepared: {error}",
                error = "no data directory"
            ));
            return;
        };
        let (tx, job) = mpsc::channel();
        let (settings, ctx) = (emulator.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let result = match settings.system {
                EmulatorSystem::Dos => emu::prepare_dos(
                    &settings.dos.drives,
                    &data.join("dos").join(&settings.id),
                    settings.dos.keep_saves,
                )
                .map(Prepared::Dos),
                EmulatorSystem::Snes => {
                    emu::prepare_snes(&settings.snes.rom, &library, &data.join("snes/roms"))
                        .map(Prepared::Snes)
                }
            };
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.screen = Screen::PreparingEmulator {
            job,
            emulator: Box::new(emulator.clone()),
        };
    }

    /// The copy is ready: DOSBox starts here, on the UI thread (the parent-death signal is
    /// tied to the thread that starts it).
    fn start_dos(
        &mut self,
        emulator: &crate::config::EmulatorSettings,
        mounts: &[crate::emulator::Mount],
    ) {
        let written = store::emulator_cache_dir()
            .ok_or_else(|| std::io::Error::other("no cache directory"))
            .and_then(|dir| {
                std::fs::create_dir_all(dir.join("capture"))?;
                let conf = dir.join(format!("{}.conf", emulator.id));
                let text = crate::emulator::dosbox_conf(&emulator.dos, mounts, &dir);
                std::fs::write(&conf, text)?;
                Ok(conf)
            });
        match written {
            Ok(conf) => {
                self.run_emulator(emulator, crate::emulator::dosbox_command(&conf), "DOSBox")
            }
            Err(e) => {
                self.screen = message(tr!(
                    "The emulator could not be prepared: {error}",
                    error = e
                ))
            }
        }
    }

    fn start_snes(&mut self, emulator: &crate::config::EmulatorSettings, rom: &std::path::Path) {
        use crate::emulator as emu;
        let Some(core) = emu::find_core() else {
            self.screen = message(tr!(
                "{name} is not installed. Run scripts/setup.sh on this computer.",
                name = "Snes9x"
            ));
            return;
        };
        let (Some(data), Some(cache)) = (store::emulator_data_dir(), store::emulator_cache_dir())
        else {
            self.screen = message(tr!(
                "The emulator could not be prepared: {error}",
                error = "no data directory"
            ));
            return;
        };
        let dirs = emu::RetroArchDirs {
            core_dir: core.parent().map(PathBuf::from).unwrap_or_default(),
            data: data.join("snes"),
            cache: cache.join("snes"),
        };
        let shader = emu::shader_path(emulator.snes.filter);
        let cfg = dirs.cache.join(format!("{}.cfg", emulator.id));
        let text = emu::retroarch_cfg(&emulator.snes, &dirs, shader.is_some());
        let written = ["saves", "system"]
            .iter()
            .map(|d| dirs.data.join(d))
            .chain(
                ["screenshots", "tmp", "playlists"]
                    .iter()
                    .map(|d| dirs.cache.join(d)),
            )
            .try_for_each(std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&cfg, text));
        if let Err(e) = written {
            self.screen = message(tr!(
                "The emulator could not be prepared: {error}",
                error = e
            ));
            return;
        }
        let cmd = emu::retroarch_command(&cfg, &core, shader.as_deref(), rom);
        self.run_emulator(emulator, cmd, "RetroArch");
    }

    fn run_emulator(
        &mut self,
        emulator: &crate::config::EmulatorSettings,
        mut cmd: std::process::Command,
        tool: &str,
    ) {
        log::info!("starting emulator {}: {cmd:?}", emulator.id);
        self.playing_activity = Some(Activity::new(
            Category::Emulator,
            emulator.name.clone(),
            tool.to_string(),
        ));
        self.screen = match cmd.spawn() {
            Ok(child) => Screen::Playing {
                child,
                started: Instant::now(),
                back: None,
                kind: PlayKind::Program,
                stopping: None,
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => message(tr!(
                "{name} is not installed. Run scripts/setup.sh on this computer.",
                name = tool
            )),
            Err(e) => message(tr!(
                "Could not start {name}: {error}",
                name = emulator.name,
                error = e
            )),
        };
    }

    /// Start a registered program; the "Playing" screen supervises it like a player.
    fn open_program(&mut self, program: &crate::config::ProgramSettings) {
        let mut cmd = player::program_command(program);
        log::info!("starting program {}: {cmd:?}", program.id);
        self.playing_activity = Some(Activity::new(
            Category::Program,
            program.name.clone(),
            String::new(),
        ));
        self.screen = match cmd.spawn() {
            Ok(child) => Screen::Playing {
                child,
                started: Instant::now(),
                back: None,
                kind: PlayKind::Program,
                stopping: None,
            },
            Err(e) => message(tr!(
                "Could not start {name}: {error}",
                name = program.name,
                error = e
            )),
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
        let folder = config
            .folders
            .iter()
            .find(|f| f.id == browse.folder_id)
            .map_or_else(|| browse.folder_id.clone(), |f| f.name.clone());
        let mut detail = browse.sub.join("/");
        if !detail.is_empty() {
            detail.push('/');
        }
        detail.push_str(&entry.name);
        self.playing_activity = Some(Activity::new(Category::Video, folder, detail));
        self.screen = match cmd.spawn() {
            Ok(child) => {
                resume::spawn_monitor(socket.clone(), resume::file_key(&entry.path));
                if remembers_files(&config, &browse.folder_id) {
                    recent::record(
                        &browse.folder_id,
                        &browse.sub,
                        &entry.name,
                        recent::Kind::Video,
                    );
                }
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
        // A screen that just opened ignores OK and clicks for a moment: the press that opened
        // it (its release, a key repeat) must not choose something on it.
        let kind = std::mem::discriminant(&self.screen);
        if kind != self.screen_kind {
            self.screen_kind = kind;
            self.screen_since = Instant::now();
        }
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
        // A program has the focus and keeps its keys, so return / home come from the device.
        // (Not in the first second: that press may have been meant for the menu.)
        if matches!(
            &self.screen,
            Screen::Playing {
                kind: PlayKind::Program,
                stopping: None,
                started,
                ..
            } if started.elapsed() >= Duration::from_secs(1)
        ) && remote_events
            .iter()
            .any(|e| e.value == 1 && self.remote_map.closes_program(e.code))
        {
            log::info!("closing the program (remote)");
            self.go_home();
        }
        // Gamepads: menu navigation while the media center itself is in front (not over a
        // player or website); the game reads them itself.
        let pad_events: Vec<remote::PadEvent> = self.inputs.pads.try_iter().collect();
        let pad_activity = !pad_events.is_empty();
        for e in pad_events {
            self.pads.apply(e);
        }
        let nav = self.pads.frame();
        let program_runs = matches!(
            self.screen,
            Screen::Playing {
                kind: PlayKind::Program,
                stopping: None,
                ..
            }
        );
        if program_runs && self.pads.select_and_start_held() {
            let since = *self.pad_close_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= crate::gamepad::CLOSE_COMBO_HOLD {
                log::info!("closing the program (gamepad)");
                self.pad_close_since = None;
                self.go_home();
            } else {
                self.ctx.request_repaint_after(Duration::from_millis(50));
            }
        } else {
            self.pad_close_since = None;
        }
        if ctx.input(|i| i.focused)
            && !matches!(
                self.screen,
                Screen::Pong(_) | Screen::Bricks(_) | Screen::Blocks(_) | Screen::Dodger(_)
            )
        {
            keys.up |= nav.up;
            keys.down |= nav.down;
            keys.left |= nav.left;
            keys.right |= nav.right;
            keys.enter |= nav.enter || nav.pause;
            keys.back |= nav.back;
        }
        let requests: Vec<UiRequest> = self.inputs.requests.try_iter().collect();
        if requests.contains(&UiRequest::Restart) {
            self.restart();
        }
        if requests.contains(&UiRequest::CalibrateRemote) {
            self.calibrate_requested = true;
            self.go_home();
        }
        self.mpris.set_status(match &self.screen {
            Screen::Music(view) if view.player.state().paused => PlayStatus::Paused,
            Screen::Music(_) => PlayStatus::Playing,
            Screen::Playing { kind, .. } if kind.is_media() => PlayStatus::Playing,
            _ => PlayStatus::Stopped,
        });
        let (power_enabled, pin_set) = {
            let (config, _) = self.config();
            (config.power.enabled, config.parental.pin_hash.is_some())
        };
        if power_enabled {
            // Ctrl+Q: like the exit button (asks first; with a PIN, the PIN).
            if keys.quit && matches!(self.screen, Screen::Home) {
                self.screen = Screen::ConfirmExit { selected: 1 };
            }
            if remote_events
                .iter()
                .any(|e| e.code == remote::KEY_POWER && e.value == 1)
                && crate::power::remote_key_acts(
                    power_enabled,
                    self.lock.is_some(),
                    matches!(self.screen, Screen::Calibrate(_)),
                )
            {
                self.remote_power_key(pin_set);
            }
        } else {
            // Switched off in the admin panel while something of it was open.
            self.power_hold = Default::default();
            if self.exit_view.as_ref().is_some_and(LockView::is_for_power) {
                self.exit_view = None;
            }
            if matches!(self.screen, Screen::ConfirmExit { .. }) {
                self.screen = Screen::Home;
            }
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
        // Any input (also a remote button only the device reports) ends the screensaver and
        // does nothing else.
        if self.just_opened() {
            keys.enter = false;
            ctx.request_repaint_after(JUST_OPENED);
        }
        let activity = !desktop_media.is_empty()
            || pad_activity
            || remote_events.iter().any(|e| e.value == 1)
            || ctx.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key { pressed: true, .. }
                            | egui::Event::PointerButton { pressed: true, .. }
                            | egui::Event::PointerMoved(_)
                            | egui::Event::MouseWheel { .. }
                            | egui::Event::Touch { .. }
                    )
                })
            });
        if self.screensaver(&painter, screen_rect, s, activity) {
            return;
        }
        self.volume_keys(&ctx, &keys, &remote_events);
        self.paint_volume(ui, screen_rect, s);

        if self.lock.is_some() {
            self.lock_screen(ui, &painter, screen_rect, s, &keys, false);
            return;
        }
        if self.exit_view.is_some() {
            self.lock_screen(ui, &painter, screen_rect, s, &keys, true);
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

        self.hide_cursor_in_games(&ctx);

        match self.screen {
            Screen::Browse(_) => self.browse_screen(ui, &painter, screen_rect, s, &keys),
            Screen::Picture(_) => self.picture_screen(ui, &painter, screen_rect, s, &keys),
            Screen::Music(_) => self.music_screen(ui, &painter, screen_rect, s, &keys),
            Screen::VideoChoose { .. } => {
                self.video_choose_screen(ui, &painter, screen_rect, s, &keys)
            }
            Screen::Pong(_) => self.pong_screen(ui, &painter, screen_rect, s, &keys, nav),
            Screen::Bricks(_) => self.bricks_screen(ui, &painter, screen_rect, s, &keys, nav),
            Screen::Blocks(_) => self.blocks_screen(ui, &painter, screen_rect, s, &keys, nav),
            Screen::Dodger(_) => self.dodger_screen(ui, &painter, screen_rect, s, &keys, nav),
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
        let pong = &config.games.pong;
        let games = game_tiles(&config.games);
        if !games.is_empty() {
            rows.push(RowKind::Games);
        }
        let emulators = &config.emulators;
        if !emulators.is_empty() {
            rows.push(RowKind::Emulators);
        }
        let programs = &config.programs;
        if !programs.is_empty() {
            rows.push(RowKind::Programs);
        }
        let row_len = |kind: RowKind| match kind {
            RowKind::Discs => 1,
            RowKind::Folders => folders.len(),
            RowKind::Websites => websites.len(),
            RowKind::Games => games.len(),
            RowKind::Programs => programs.len(),
            RowKind::Emulators => emulators.len(),
        };

        // The buttons at the top right ("up" from the top row), left to right: home screens
        // (only with more than one), exit, and lock (only with a PIN, so nobody gets locked
        // out).
        let has_pin = config.parental.pin_hash.is_some();
        let several = config.screens.len() > 1;
        let buttons: Vec<TopButton> = [
            several.then_some(TopButton::Screens),
            config.power.enabled.then_some(TopButton::Exit),
            has_pin.then_some(TopButton::Lock),
        ]
        .into_iter()
        .flatten()
        .collect();
        if self.top_focus.is_some_and(|b| !buttons.contains(&b)) {
            self.top_focus = buttons.last().copied();
        }
        if home && let Some(focused) = self.top_focus {
            let at = buttons.iter().position(|b| *b == focused).unwrap_or(0);
            if keys.down {
                self.top_focus = None;
            } else if keys.left && at > 0 {
                self.top_focus = Some(buttons[at - 1]);
            } else if keys.right && at + 1 < buttons.len() {
                self.top_focus = Some(buttons[at + 1]);
            }
        } else if home && keys.up && self.focus.0 == 0 {
            self.top_focus = buttons.last().copied();
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
                RowKind::Games => tr!("Games"),
                RowKind::Programs => tr!("Programs"),
                RowKind::Emulators => tr!("Emulators"),
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
                    home && self.top_focus.is_none() && self.focus == (r, i),
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
                    RowKind::Games => match games[i] {
                        GameTile::Pong => paint_pong_tile(painter, rect, s, focus),
                        GameTile::Bricks => paint_bricks_tile(painter, rect, s, focus),
                        GameTile::Blocks => paint_blocks_tile(painter, rect, s, focus),
                        GameTile::Dodger => paint_dodger_tile(painter, rect, s, focus),
                    },
                    RowKind::Programs => paint_program_tile(
                        painter,
                        rect,
                        s,
                        &programs[i].name,
                        programs[i].icon,
                        focus,
                    ),
                    RowKind::Emulators => {
                        use crate::config::{EmulatorSystem, ProgramIcon};
                        let icon = match emulators[i].system {
                            EmulatorSystem::Dos => ProgramIcon::Computer,
                            EmulatorSystem::Snes => ProgramIcon::Gamepad,
                        };
                        paint_program_tile(painter, rect, s, &emulators[i].name, icon, focus)
                    }
                }
                if home && response.clicked() {
                    self.focus = (r, i);
                    activate = Some((r, i));
                }
            }
        }

        let size = 76.0 * s;
        let button = |right: f32| {
            Rect::from_min_size(
                Pos2::new(
                    screen_rect.max.x - margin - right - size,
                    screen_rect.min.y + 34.0 * s,
                ),
                Vec2::splat(size),
            )
        };
        // Home screens: left of the exit button.
        if several {
            let rect =
                button((size + 24.0 * s) * (has_pin as u8 + config.power.enabled as u8) as f32);
            let response = ui.interact(
                rect,
                ui.make_persistent_id("screens-button"),
                Sense::click(),
            );
            let focused =
                home && (self.top_focus == Some(TopButton::Screens) || response.hovered());
            paint_screens_button(ui.painter(), rect, focused, s);
            if home
                && (response.clicked()
                    || (self.top_focus == Some(TopButton::Screens) && keys.enter))
            {
                let current = config
                    .screens
                    .iter()
                    .position(|(id, ..)| *id == config.screen_id)
                    .unwrap_or(0);
                self.screen = Screen::SwitchHome { selected: current };
                self.ctx.request_repaint();
                return;
            }
        }
        // Exit: left of the lock button (or in its place without a PIN).
        if config.power.enabled {
            let rect = button(if has_pin { size + 24.0 * s } else { 0.0 });
            let response = ui.interact(rect, ui.make_persistent_id("exit-button"), Sense::click());
            let focused = home && (self.top_focus == Some(TopButton::Exit) || response.hovered());
            paint_power_button(ui.painter(), rect, focused, s);
            // Short press: ask; held 3 s: log out; held 8 s: shut down.
            let down = home
                && ((self.top_focus == Some(TopButton::Exit) && self.ok_held(&ctx))
                    || response.is_pointer_button_down_on());
            match self.power_hold.update(down, Instant::now()) {
                crate::power::Hold::Nothing => {}
                crate::power::Hold::Holding(held) => {
                    paint_power_hold(&ctx, ui, rect, screen_rect, s, held);
                    ctx.request_repaint_after(Duration::from_millis(30));
                }
                crate::power::Hold::Ask => {
                    self.screen = Screen::ConfirmExit { selected: 2 };
                    self.ctx.request_repaint();
                    return;
                }
                crate::power::Hold::LogOut => {
                    self.power_action(false, has_pin);
                    return;
                }
                crate::power::Hold::ShutDown => {
                    self.power_action(true, has_pin);
                    return;
                }
            }
        }
        if has_pin {
            let rect = button(0.0);
            let response = ui.interact(rect, ui.make_persistent_id("lock-button"), Sense::click());
            let focused = home && (self.top_focus == Some(TopButton::Lock) || response.hovered());
            paint_lock_button(ui.painter(), rect, focused, s);
            let pressed = response.clicked()
                || (home && self.top_focus == Some(TopButton::Lock) && keys.enter);
            if home && pressed {
                log::info!("locked with the lock button");
                self.top_focus = None;
                self.tracker
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .lock(&config.screen_id);
                self.ctx.request_repaint();
                return;
            }
        }

        if home && keys.enter && self.top_focus.is_none() {
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
                Some(RowKind::Emulators) => {
                    if let Some(emulator) = emulators.get(col) {
                        self.open_emulator(emulator);
                    }
                }
                Some(RowKind::Programs) => {
                    if let Some(program) = programs.get(col) {
                        self.open_program(program);
                    }
                }
                Some(RowKind::Games) => {
                    self.pong_tick = Instant::now();
                    match games.get(col) {
                        Some(GameTile::Pong) => {
                            let seed = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(1, |d| d.as_nanos() as u64);
                            log::info!("starting Ping Pong");
                            self.screen = Screen::Pong(Box::new(PongView::new(
                                pong.computer_level,
                                pong.points_to_win,
                                seed,
                            )));
                        }
                        Some(GameTile::Bricks) => {
                            log::info!("starting Brick Breaker");
                            let b = &config.games.bricks;
                            self.screen = Screen::Bricks(Box::new(crate::bricks::BricksView::new(
                                b.lives, b.sound,
                            )));
                        }
                        Some(GameTile::Dodger) => {
                            let seed = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(1, |d| d.as_nanos() as u64);
                            log::info!("starting Star Dodger");
                            self.screen = Screen::Dodger(Box::new(crate::dodger::DodgerView::new(
                                seed,
                                config.games.dodger.sound,
                                config.games.dodger.music,
                            )));
                        }
                        Some(GameTile::Blocks) => {
                            log::info!("starting Block Stacker");
                            let b = &config.games.blocks;
                            let seed = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(1, |d| d.as_nanos() as u64);
                            self.screen = Screen::Blocks(Box::new(crate::blocks::BlocksView::new(
                                b.start_level,
                                b.sound,
                                seed,
                            )));
                        }
                        None => {}
                    }
                }
                _ => {}
            }
            // The click / OK that opened something must not also act on it (e.g. pick
            // the first of the disc's start options): the new screen starts next frame.
            if !matches!(self.screen, Screen::Home) {
                ctx.request_repaint();
                return;
            }
        }

        // Overlays
        match &self.screen {
            Screen::Home
            | Screen::Calibrate(_)
            | Screen::Browse(_)
            | Screen::Picture(_)
            | Screen::Music(_)
            | Screen::Pong(_)
            | Screen::Bricks(_)
            | Screen::Blocks(_)
            | Screen::Dodger(_)
            | Screen::VideoChoose { .. } => {}
            Screen::SwitchHome { selected } => {
                let count = config.screens.len() + 1;
                let mut selected = (*selected).min(count - 1);
                if keys.prev() {
                    selected = selected.saturating_sub(1);
                }
                if keys.next() {
                    selected = (selected + 1).min(count - 1);
                }
                let mut items: Vec<MenuItem> = config
                    .screens
                    .iter()
                    .map(|(id, name, default)| {
                        let mark = if *default { "★ " } else { "" };
                        let hint = if *id == config.screen_id {
                            tr!("Shown now")
                        } else {
                            ""
                        };
                        MenuItem::new(format!("{mark}{name}"), hint)
                    })
                    .collect();
                items.push(MenuItem::new(tr!("Cancel"), ""));
                let clicked = paint_menu(
                    ui,
                    painter,
                    screen_rect,
                    s,
                    tr!("Home screens"),
                    &items,
                    selected,
                )
                .filter(|_| !self.just_opened());
                match clicked.or(keys.enter.then_some(selected)) {
                    _ if keys.back => self.screen = Screen::Home,
                    Some(i) if i < config.screens.len() => {
                        let (id, name, _) = config.screens[i].clone();
                        self.screen = Screen::Home;
                        self.top_focus = None;
                        if id != config.screen_id {
                            if has_pin {
                                // Only a parent may switch.
                                self.exit_view =
                                    Some(LockView::for_purpose(Purpose::Switch { id, name }));
                            } else {
                                self.switch_home_screen(&id);
                            }
                        }
                    }
                    Some(_) => self.screen = Screen::Home,
                    None => self.screen = Screen::SwitchHome { selected },
                }
            }
            Screen::ConfirmExit { selected } => {
                let mut selected = *selected;
                if keys.prev() {
                    selected = selected.saturating_sub(1);
                }
                if keys.next() {
                    selected = (selected + 1).min(2);
                }
                let items = [
                    MenuItem::new(
                        tr!("Exit"),
                        tr!("Ends the media center and goes back to the login screen"),
                    ),
                    MenuItem::new(
                        tr!("Shut down the computer"),
                        tr!("Tip: hold the power button for 8 seconds"),
                    ),
                    MenuItem::new(tr!("Cancel"), ""),
                ];
                let clicked = paint_menu(
                    ui,
                    painter,
                    screen_rect,
                    s,
                    tr!("Exit the media center?"),
                    &items,
                    selected,
                )
                .filter(|_| !self.just_opened());
                match clicked.or(keys.enter.then_some(selected)) {
                    _ if keys.back => self.screen = Screen::Home,
                    // With a PIN, only a parent may leave or shut down.
                    Some(0) => self.power_action(false, has_pin),
                    Some(1) => self.power_action(true, has_pin),
                    Some(_) => self.screen = Screen::Home,
                    None => self.screen = Screen::ConfirmExit { selected },
                }
            }
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
                let clicked = clicked.filter(|_| !self.just_opened());
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
                                    player::stop_leftover_makemkv_servers();
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
                if stopping.is_none() && kind.is_media() {
                    for command in &keys.media {
                        player::mpv_media_key(&store::mpv_socket_path(), *command);
                    }
                }
                let body = if stopping.is_some() {
                    tr!("Stopping…")
                } else {
                    tr!("Esc / Back: stop")
                };
                paint_overlay(painter, screen_rect, s, tr!("Playing…"), body);
                if stop_now
                    && let Screen::Playing {
                        stopping, child, ..
                    } = &mut self.screen
                {
                    stop_child(child);
                    *stopping = Some(Instant::now());
                }
            }
            Screen::PreparingEmulator { emulator, .. } => {
                paint_overlay(
                    painter,
                    screen_rect,
                    s,
                    emulator.name.as_str(),
                    tr!("Preparing the game…"),
                );
                if keys.back {
                    self.screen = Screen::Home;
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
        let (config, _) = self.config();
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
            if remembers_files(&config, &view.back.folder_id) {
                recent::record(
                    &view.back.folder_id,
                    &view.back.sub,
                    &track.name,
                    recent::Kind::Audio,
                );
            }
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
        let clicked = paint_menu(ui, painter, screen_rect, s, &title, &items, selected)
            .filter(|_| !self.just_opened());
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

/// `curator-web` next to our own binary (development and installed layout), else in PATH.
fn find_web_helper() -> Option<PathBuf> {
    let name = "curator-web";
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
        log::warn!("{name} not found: websites can't be opened (cargo build -p curator-web)");
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

fn paint_program_tile(
    painter: &egui::Painter,
    rect: Rect,
    s: f32,
    name: &str,
    icon: crate::config::ProgramIcon,
    focus: f32,
) {
    let rect = rect.expand(focus * 12.0 * s);
    let radius = CornerRadius::same((18.0 * s) as u8);
    painter.rect_filled(rect, radius, TILE_BG);
    if focus > 0.0 {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(4.0 * s, ACCENT.gamma_multiply(focus)),
            StrokeKind::Outside,
        );
    }
    let center = Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.36);
    paint_program_icon(painter, center, rect.width() * 0.27, icon);
    paint_tile_texts(painter, rect, s, name, "", 1.0);
}

/// Our own simple icons, drawn from rectangles and circles around `center` (`r`: half size).
fn paint_program_icon(
    painter: &egui::Painter,
    center: Pos2,
    r: f32,
    icon: crate::config::ProgramIcon,
) {
    use crate::config::ProgramIcon as I;
    let at = |x: f32, y: f32| center + Vec2::new(x, y) * r;
    let block = |x: f32, y: f32, w: f32, h: f32, round: f32, color: Color32| {
        painter.rect_filled(
            Rect::from_center_size(at(x, y), Vec2::new(w, h) * r),
            CornerRadius::same((round * r) as u8),
            color,
        );
    };
    let dot = |x: f32, y: f32, size: f32, color: Color32| {
        painter.circle_filled(at(x, y), size * r, color);
    };
    let cross = |x: f32, y: f32, size: f32, color: Color32| {
        block(x, y, size, size * 0.34, 0.02, color);
        block(x, y, size * 0.34, size, 0.02, color);
    };
    let light = Color32::from_rgb(0xe8, 0xec, 0xf4);
    let dark = Color32::from_rgb(0x1d, 0x22, 0x30);
    let red = Color32::from_rgb(0xe5, 0x4b, 0x4b);
    let yellow = Color32::from_rgb(0xf2, 0xb9, 0x3b);
    let blue = Color32::from_rgb(0x3f, 0x8c, 0xe0);
    let green = Color32::from_rgb(0x4c, 0xc2, 0x8a);
    match icon {
        I::Gamepad => {
            block(0.0, 0.0, 2.0, 1.1, 0.42, light);
            cross(-0.5, 0.0, 0.6, dark);
            dot(0.4, 0.13, 0.15, red);
            dot(0.72, -0.13, 0.15, blue);
        }
        I::Joystick => {
            block(0.0, 0.72, 1.7, 0.5, 0.12, light);
            dot(0.55, 0.62, 0.12, red);
            block(0.0, 0.1, 0.18, 1.0, 0.04, dark);
            dot(0.0, -0.6, 0.38, red);
        }
        I::Computer => {
            block(0.0, -0.2, 1.9, 1.3, 0.12, light);
            block(0.0, -0.2, 1.6, 1.0, 0.04, dark);
            block(-0.55, -0.4, 0.3, 0.1, 0.0, green);
            block(-0.45, -0.15, 0.5, 0.1, 0.0, green);
            block(-0.6, 0.1, 0.12, 0.2, 0.0, green);
            block(0.0, 0.72, 1.3, 0.3, 0.06, light);
        }
        I::Handheld => {
            block(0.0, 0.0, 1.3, 2.0, 0.14, blue);
            block(0.0, -0.42, 0.98, 0.84, 0.05, dark);
            cross(-0.3, 0.55, 0.42, dark);
            dot(0.18, 0.62, 0.1, yellow);
            dot(0.42, 0.46, 0.1, yellow);
        }
        I::Floppy => {
            block(0.0, 0.0, 1.9, 1.9, 0.08, blue);
            block(0.05, -0.62, 1.0, 0.66, 0.02, light);
            block(0.3, -0.62, 0.22, 0.48, 0.0, blue);
            block(0.0, 0.42, 1.4, 1.0, 0.02, light);
            block(0.0, 0.25, 1.05, 0.06, 0.0, dark);
            block(0.0, 0.5, 1.05, 0.06, 0.0, dark);
        }
        I::App => {
            block(0.0, 0.0, 1.9, 1.9, 0.36, green);
            painter.add(egui::Shape::convex_polygon(
                vec![at(-0.3, -0.5), at(0.55, 0.0), at(-0.3, 0.5)],
                light,
                Stroke::NONE,
            ));
        }
    }
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

/// Holding the power button: a ring fills around it until "log out" (3 s); then a hint with
/// a progress bar until "shut down" (8 s).
fn paint_power_hold(
    ctx: &egui::Context,
    ui: &egui::Ui,
    button: Rect,
    screen_rect: Rect,
    s: f32,
    held: Duration,
) {
    use crate::power::{LOG_OUT_AFTER, SHUT_DOWN_AFTER};
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        ui.make_persistent_id("power-hold"),
    ));
    let t = held.as_secs_f32();
    let log_out = LOG_OUT_AFTER.as_secs_f32();
    let ring = (t / log_out).min(1.0);
    let r = button.width() / 2.0 + 8.0 * s;
    let points: Vec<Pos2> = (0..=60)
        .map(|k| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * ring * k as f32 / 60.0;
            button.center() + Vec2::new(a.cos() * r, a.sin() * r)
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(6.0 * s, ACCENT)));
    if held < LOG_OUT_AFTER {
        return;
    }
    let progress = ((t - log_out) / (SHUT_DOWN_AFTER.as_secs_f32() - log_out)).clamp(0.0, 1.0);
    let rect = Rect::from_center_size(
        Pos2::new(screen_rect.center().x, screen_rect.max.y - 150.0 * s),
        Vec2::new(900.0 * s, 150.0 * s),
    );
    painter.rect_filled(
        rect,
        CornerRadius::same((16.0 * s) as u8),
        Color32::from_black_alpha(225),
    );
    painter.text(
        Pos2::new(rect.center().x, rect.min.y + 36.0 * s),
        Align2::CENTER_CENTER,
        tr!("Release now: log out"),
        FontId::proportional(30.0 * s),
        TEXT,
    );
    painter.text(
        Pos2::new(rect.center().x, rect.min.y + 76.0 * s),
        Align2::CENTER_CENTER,
        tr!("Keep holding: the computer will be shut down"),
        FontId::proportional(26.0 * s),
        Color32::from_rgb(0xff, 0xb4, 0x5c),
    );
    let bar = Rect::from_min_size(
        Pos2::new(rect.min.x + 40.0 * s, rect.max.y - 38.0 * s),
        Vec2::new(rect.width() - 80.0 * s, 14.0 * s),
    );
    let radius = CornerRadius::same((7.0 * s) as u8);
    painter.rect_filled(bar, radius, Color32::from_white_alpha(40));
    let mut filled = bar;
    filled.set_width(bar.width() * progress);
    painter.rect_filled(filled, radius, Color32::from_rgb(0xff, 0x6b, 0x5c));
}

/// The home screens button: two people.
fn paint_screens_button(painter: &egui::Painter, rect: Rect, focused: bool, s: f32) {
    let r = rect.width() / 2.0;
    let c = rect.center();
    painter.circle_filled(c, r, if focused { ACCENT } else { TILE_BG });
    if focused {
        painter.circle_stroke(c, r, Stroke::new(3.0 * s, TEXT));
    }
    for (dx, size) in [(-0.22, 0.85), (0.22, 1.0)] {
        let head = c + Vec2::new(dx * r, -0.2 * r * size);
        painter.circle_filled(head, 0.16 * r * size, TEXT);
        let body = Rect::from_center_size(
            c + Vec2::new(dx * r, 0.28 * r),
            Vec2::new(0.42 * r * size, 0.32 * r * size),
        );
        painter.rect_filled(
            body,
            CornerRadius {
                nw: (0.2 * r) as u8,
                ne: (0.2 * r) as u8,
                sw: 2,
                se: 2,
            },
            TEXT,
        );
    }
}

/// The exit button: the power-off symbol (a ring open at the top, a line through the gap).
fn paint_power_button(painter: &egui::Painter, rect: Rect, focused: bool, s: f32) {
    let r = rect.width() / 2.0;
    let c = rect.center();
    painter.circle_filled(c, r, if focused { ACCENT } else { TILE_BG });
    if focused {
        painter.circle_stroke(c, r, Stroke::new(3.0 * s, TEXT));
    }
    let stroke = Stroke::new(r * 0.13, TEXT);
    let ring = r * 0.45;
    // From 1 o'clock clockwise round to 11 o'clock (the gap is at the top).
    let gap = 0.75_f32;
    let points: Vec<Pos2> = (0..=40)
        .map(|k| {
            let t = -std::f32::consts::FRAC_PI_2
                + gap
                + (std::f32::consts::TAU - 2.0 * gap) * k as f32 / 40.0;
            c + Vec2::new(t.cos() * ring, t.sin() * ring)
        })
        .collect();
    painter.add(egui::Shape::line(points, stroke));
    painter.line_segment(
        [
            c + Vec2::new(0.0, -ring * 1.25),
            c + Vec2::new(0.0, -ring * 0.05),
        ],
        stroke,
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
/// Replace this process with the program we were started as, same arguments.
fn exec_self() -> std::io::Error {
    use std::os::unix::process::CommandExt;
    let program = match restart_program(std::env::args_os().next(), std::env::current_exe().ok()) {
        Some(p) => p,
        None => return std::io::Error::other("don't know which program to start"),
    };
    log::info!("restarting as {}", program.to_string_lossy());
    std::process::Command::new(program)
        .args(std::env::args_os().skip(1))
        .exec()
}

/// The program to start again: how we were started (`argv[0]`: the kiosk uses the installed
/// path; a bare name is looked up in `PATH` again). Not `/proc/self/exe`: after an update
/// it names the deleted old file, in its old folder (`/opt/curator.old/… (deleted)`).
fn restart_program(
    argv0: Option<std::ffi::OsString>,
    current_exe: Option<PathBuf>,
) -> Option<std::ffi::OsString> {
    argv0
        .filter(|a| !a.is_empty())
        .or_else(|| current_exe.map(PathBuf::into_os_string))
}

/// Stop a player: RetroArch quits when told to, so that it can save; others get a signal.
fn stop_child(child: &mut Child) {
    if !player::ask_to_stop(child) {
        stop_player(child.id());
    }
}

fn stop_player(pid: u32) {
    log::info!("stopping player (pid {pid})");
    // A program is its own process group: also what its script started.
    if player::signal_group(pid, libc::SIGTERM) {
        return;
    }
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

/// The tiles of the "Games" row, in order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GameTile {
    Pong,
    Bricks,
    Blocks,
    Dodger,
}

fn game_tiles(games: &crate::config::GameSettings) -> Vec<GameTile> {
    let mut tiles = Vec::new();
    if games.pong.enabled {
        tiles.push(GameTile::Pong);
    }
    if games.bricks.enabled {
        tiles.push(GameTile::Bricks);
    }
    if games.blocks.enabled {
        tiles.push(GameTile::Blocks);
    }
    if games.dodger.enabled {
        tiles.push(GameTile::Dodger);
    }
    tiles
}

/// Letters typed this frame (for a player's name), Backspace, and whether Space was pressed.
fn typed_letters(ctx: &egui::Context) -> (Vec<char>, bool, bool) {
    ctx.input(|i| {
        let typed = i
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Text(t) => Some(t.chars()),
                _ => None,
            })
            .flatten()
            .collect();
        (
            typed,
            i.key_pressed(Key::Backspace),
            i.key_pressed(Key::Space),
        )
    })
}

/// The Block Stacker tile: a stack of round tiles and a falling piece.
/// The Star Dodger tile: stars, a falling ball and the striped ball.
fn paint_dodger_tile(painter: &egui::Painter, rect: Rect, s: f32, focus: f32) {
    let rect = rect.expand(focus * 12.0 * s);
    let radius = CornerRadius::same((18.0 * s) as u8);
    painter.rect_filled(rect, radius, TILE_BG);
    if focus > 0.0 {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(4.0 * s, ACCENT.gamma_multiply(focus)),
            StrokeKind::Outside,
        );
    }
    let w = rect.width() * 0.62;
    let field = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.36),
        Vec2::new(w, w * 3.0 / 4.0),
    );
    painter.rect_filled(field, 0.0, Color32::from_rgb(0x0d, 0x06, 0x24));
    let u = field.width() / 32.0;
    let dot = |x: f32, y: f32, color| {
        painter.rect_filled(
            Rect::from_min_size(field.min + Vec2::new(x * u, y * u), Vec2::splat(u)),
            0.0,
            color,
        );
    };
    for (x, y) in [
        (3.0, 3.0),
        (27.0, 5.0),
        (12.0, 9.0),
        (29.0, 15.0),
        (5.0, 18.0),
        (18.0, 2.0),
    ] {
        dot(x, y, Color32::from_rgb(0xff, 0xf6, 0xe0));
    }
    let ball = |cx: f32, cy: f32, r: i32, color: &dyn Fn(i32) -> Color32| {
        for dy in -r..=r {
            let half = crate::dodger::row_half(r, dy);
            painter.rect_filled(
                Rect::from_min_size(
                    field.min + Vec2::new((cx - half as f32) * u, (cy + dy as f32) * u),
                    Vec2::new((2 * half + 1) as f32 * u, u),
                ),
                0.0,
                color(dy),
            );
        }
    };
    ball(20.0, 8.0, 3, &|_| Color32::from_rgb(0xff, 0x4f, 0xa8));
    let stripes = [
        Color32::WHITE,
        Color32::from_rgb(0xff, 0xe6, 0x4a),
        Color32::from_rgb(0x35, 0xd8, 0xff),
    ];
    ball(11.0, 19.0, 4, &|dy| stripes[((dy + 8) / 2) as usize % 3]);
    paint_tile_texts(
        painter,
        rect,
        s,
        tr!("Star Dodger"),
        tr!("Dodge the falling balls"),
        1.0,
    );
}

fn paint_blocks_tile(painter: &egui::Painter, rect: Rect, s: f32, focus: f32) {
    let rect = rect.expand(focus * 12.0 * s);
    let radius = CornerRadius::same((18.0 * s) as u8);
    painter.rect_filled(rect, radius, TILE_BG);
    if focus > 0.0 {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(4.0 * s, ACCENT.gamma_multiply(focus)),
            StrokeKind::Outside,
        );
    }
    let w = rect.width() * 0.62;
    let field = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.36),
        Vec2::new(w, w * 9.0 / 16.0),
    );
    painter.rect_filled(
        field,
        CornerRadius::same((8.0 * s) as u8),
        Color32::from_rgb(0x1d, 0x1a, 0x30),
    );
    let unit = field.height() / 7.0;
    let (teal, gold, violet) = (
        Color32::from_rgb(0x4f, 0xd1, 0xc5),
        Color32::from_rgb(0xf6, 0xc4, 0x5a),
        Color32::from_rgb(0xb3, 0x88, 0xff),
    );
    let cells = [
        (0.0, 6.0, teal),
        (1.0, 6.0, teal),
        (2.0, 6.0, violet),
        (3.0, 6.0, violet),
        (5.0, 6.0, gold),
        (6.0, 6.0, gold),
        (7.0, 6.0, teal),
        (2.0, 5.0, violet),
        (6.0, 5.0, gold),
        (4.0, 1.0, gold),
        (4.0, 2.0, gold),
        (5.0, 2.0, gold),
        (4.0, 3.0, gold),
    ];
    let left = field.center().x - 4.0 * unit;
    for (x, y, color) in cells {
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(left + x * unit, field.min.y + y * unit),
                Vec2::splat(unit),
            )
            .shrink(unit * 0.08),
            CornerRadius::same((unit * 0.28) as u8),
            color,
        );
    }
    paint_tile_texts(
        painter,
        rect,
        s,
        tr!("Block Stacker"),
        tr!("Game for 1 or 2 players"),
        1.0,
    );
}

/// The Brick Breaker tile: rows of bricks, a paddle and the ball.
fn paint_bricks_tile(painter: &egui::Painter, rect: Rect, s: f32, focus: f32) {
    let rect = rect.expand(focus * 12.0 * s);
    let radius = CornerRadius::same((18.0 * s) as u8);
    painter.rect_filled(rect, radius, TILE_BG);
    if focus > 0.0 {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(4.0 * s, ACCENT.gamma_multiply(focus)),
            StrokeKind::Outside,
        );
    }
    let w = rect.width() * 0.62;
    let field = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.36),
        Vec2::new(w, w * 9.0 / 16.0),
    );
    painter.rect_filled(
        field,
        CornerRadius::same((8.0 * s) as u8),
        Color32::from_rgb(0x12, 0x18, 0x28),
    );
    let colors = [
        Color32::from_rgb(0x1d, 0x4f, 0xb8),
        Color32::from_rgb(0x3d, 0x8b, 0xff),
        Color32::from_rgb(0x9a, 0xd8, 0xff),
    ];
    let bw = field.width() / 7.0;
    for (row, color) in colors.iter().enumerate() {
        for col in 0..7 {
            if row == 2 && col % 3 == 1 {
                continue;
            }
            let min = field.min + Vec2::new(col as f32 * bw, (row as f32 + 0.6) * bw * 0.45);
            painter.rect_filled(
                Rect::from_min_size(min, Vec2::new(bw, bw * 0.45)).shrink(1.5 * s),
                CornerRadius::same(2),
                *color,
            );
        }
    }
    painter.rect_filled(
        Rect::from_center_size(
            Pos2::new(field.center().x - bw * 0.6, field.max.y - bw * 0.35),
            Vec2::new(bw * 1.6, bw * 0.22),
        ),
        CornerRadius::same(3),
        TEXT,
    );
    painter.circle_filled(
        Pos2::new(field.center().x + bw * 0.4, field.center().y + bw * 0.5),
        bw * 0.16,
        Color32::from_rgb(0xff, 0xe0, 0x6a),
    );
    paint_tile_texts(
        painter,
        rect,
        s,
        tr!("Brick Breaker"),
        tr!("Clear the walls of bricks"),
        1.0,
    );
}

/// The Ping Pong tile: a small field with paddles and ball.
fn paint_pong_tile(painter: &egui::Painter, rect: Rect, s: f32, focus: f32) {
    let rect = rect.expand(focus * 12.0 * s);
    let radius = CornerRadius::same((18.0 * s) as u8);
    painter.rect_filled(rect, radius, TILE_BG);
    if focus > 0.0 {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(4.0 * s, ACCENT.gamma_multiply(focus)),
            StrokeKind::Outside,
        );
    }
    let w = rect.width() * 0.62;
    let field = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.36),
        Vec2::new(w, w * 9.0 / 16.0),
    );
    painter.rect_filled(
        field,
        CornerRadius::same((8.0 * s) as u8),
        Color32::from_rgb(0x10, 0x24, 0x3a),
    );
    let u = field.width() / 16.0;
    for i in 0..5 {
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(
                    field.center().x - 0.05 * u,
                    field.min.y + (i as f32 * 2.0 + 0.5) * u * 0.9,
                ),
                Vec2::new(0.1 * u, 0.9 * u),
            ),
            0.0,
            Color32::from_white_alpha(60),
        );
    }
    let paddle = |x: f32, y: f32, color| {
        painter.rect_filled(
            Rect::from_center_size(
                field.min + Vec2::new(x * u, y * u),
                Vec2::new(0.4 * u, 2.2 * u),
            ),
            CornerRadius::same((0.2 * u) as u8),
            color,
        );
    };
    paddle(1.0, 3.2, Color32::from_rgb(0x5c, 0xc8, 0xff));
    paddle(15.0, 5.6, Color32::from_rgb(0xff, 0xa0, 0x5c));
    painter.circle_filled(field.min + Vec2::new(10.5 * u, 3.6 * u), 0.35 * u, TEXT);
    paint_tile_texts(
        painter,
        rect,
        s,
        tr!("Ping Pong"),
        tr!("Game for 1 or 2 players"),
        1.0,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_uses_how_we_were_started() {
        let installed = std::ffi::OsString::from("/opt/curator/bin/curator");
        let deleted = PathBuf::from("/opt/curator.old/bin/curator (deleted)");
        assert_eq!(
            restart_program(Some(installed.clone()), Some(deleted.clone())),
            Some(installed)
        );
        // Started by name: found in PATH again.
        assert_eq!(
            restart_program(Some("curator".into()), Some(deleted.clone())),
            Some("curator".into())
        );
        assert_eq!(
            restart_program(Some("".into()), Some(deleted.clone())),
            Some(deleted.into_os_string())
        );
        assert_eq!(restart_program(None, None), None);
    }
}
