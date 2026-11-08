//! The media center config (`$XDG_CONFIG_HOME/curator/config.json`).
//!
//! Written by the admin interface (`admin/src/config.ts`); the format is documented in
//! `docs/config.schema.json`. Rules:
//! - missing file: defaults (fresh installation),
//! - invalid file at startup: fail closed (locked-down settings) and report the error,
//! - invalid file on reload: keep the last valid config and report the error.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 2: home screens (`home_screens`), each with its own items and time limits. Files of
/// version 1 (one home screen) are still read and converted (`parse`).
pub const VERSION: u32 = 2;
/// At most this many home screens.
pub const MAX_HOME_SCREENS: usize = 10;
/// Home screen names are short (shown in menus on the TV).
const MAX_SCREEN_NAME_CHARS: usize = 30;
/// The home screen a version-1 config becomes (its usage data moves there too).
pub const V1_SCREEN_ID: &str = "home";
const MAX_LANGUAGES: usize = 10;
const MAX_FOLDERS: usize = 50;
const MAX_NAME_CHARS: usize = 60;
const MAX_PATH_BYTES: usize = 4096;
const MAX_WEBSITES: usize = 50;
const MAX_PROGRAMS: usize = 50;
const MAX_EMULATORS: usize = 50;
const MAX_DRIVES: usize = 8;
const MAX_AUTOSTART_CHARS: usize = 260;
const MAX_STARTUP_SCRIPT_CHARS: usize = 4000;
const MAX_PROGRAM_ARGS: usize = 64;
const MAX_COMMAND_CHARS: usize = 1024;
const MAX_ARG_CHARS: usize = 4096;
const MAX_URL_BYTES: usize = 2048;

/// The whole config (version 2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    /// At least one; exactly one is the default (★): shown at the start and preselected
    /// in the admin panel.
    pub home_screens: Vec<HomeScreen>,
    #[serde(default)]
    pub parental: ParentalSettings,
    /// For all home screens.
    #[serde(default)]
    pub screensaver: ScreensaverSettings,
    #[serde(default)]
    pub statistics: StatisticsSettings,
    /// For all home screens.
    #[serde(default)]
    pub power: PowerSettings,
}

/// One home screen: its own items and time limits (like a profile).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeScreen {
    pub id: String,
    /// Short title (menus on the TV, admin panel).
    pub name: String,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub home: HomeSettings,
    #[serde(default)]
    pub discs: DiscSettings,
    #[serde(default)]
    pub folders: Vec<FolderSettings>,
    #[serde(default)]
    pub websites: Vec<WebsiteSettings>,
    #[serde(default)]
    pub programs: Vec<ProgramSettings>,
    #[serde(default)]
    pub emulators: Vec<EmulatorSettings>,
    #[serde(default)]
    pub games: GameSettings,
    #[serde(default)]
    pub limits: Vec<LimitRule>,
}

impl HomeScreen {
    fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            default: true,
            home: HomeSettings::default(),
            discs: DiscSettings::default(),
            folders: Vec::new(),
            websites: Vec::new(),
            programs: Vec::new(),
            emulators: Vec::new(),
            games: GameSettings::default(),
            limits: Vec::new(),
        }
    }
}

/// Usage statistics (`src/stats.rs`): what was used when, kept for `keep_days`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct StatisticsSettings {
    pub enabled: bool,
    pub keep_days: u32,
}

impl Default for StatisticsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            keep_days: 90,
        }
    }
}

pub const MAX_KEEP_DAYS: u32 = 3650;

/// Turning the computer off from the TV. Off: no power button, Ctrl+Q and the remote's power
/// key do nothing, only the button on the computer is left (`src/power.rs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PowerSettings {
    pub enabled: bool,
}

impl Default for PowerSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// What the media center works with: the active home screen's settings plus the shared
/// ones (field names as before home screens existed).
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub screen_id: String,
    pub screen_name: String,
    /// All home screens: id, name, default.
    pub screens: Vec<(String, String, bool)>,
    pub home: HomeSettings,
    pub discs: DiscSettings,
    pub folders: Vec<FolderSettings>,
    pub websites: Vec<WebsiteSettings>,
    pub programs: Vec<ProgramSettings>,
    pub emulators: Vec<EmulatorSettings>,
    pub games: GameSettings,
    pub limits: Vec<LimitRule>,
    pub parental: ParentalSettings,
    pub screensaver: ScreensaverSettings,
    pub statistics: StatisticsSettings,
    pub power: PowerSettings,
}

impl Config {
    /// The default (★) home screen.
    pub fn default_screen(&self) -> &HomeScreen {
        self.home_screens
            .iter()
            .find(|s| s.default)
            .or(self.home_screens.first())
            .expect("a valid config has at least one home screen")
    }

    /// The home screen with this id, else the default one.
    pub fn screen(&self, id: Option<&str>) -> &HomeScreen {
        id.and_then(|id| self.home_screens.iter().find(|s| s.id == id))
            .unwrap_or_else(|| self.default_screen())
    }

    pub fn view(&self, active: Option<&str>) -> View {
        let s = self.screen(active);
        View {
            screen_id: s.id.clone(),
            screen_name: s.name.clone(),
            screens: self
                .home_screens
                .iter()
                .map(|s| (s.id.clone(), s.name.clone(), s.default))
                .collect(),
            home: s.home.clone(),
            discs: s.discs.clone(),
            folders: s.folders.clone(),
            websites: s.websites.clone(),
            programs: s.programs.clone(),
            emulators: s.emulators.clone(),
            games: s.games.clone(),
            limits: s.limits.clone(),
            parental: self.parental.clone(),
            screensaver: self.screensaver.clone(),
            statistics: self.statistics.clone(),
            power: self.power.clone(),
        }
    }
}

/// A version-1 config (one home screen); only read, then converted.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigV1 {
    pub version: u32,
    #[serde(default)]
    pub home: HomeSettings,
    #[serde(default)]
    pub discs: DiscSettings,
    #[serde(default)]
    pub folders: Vec<FolderSettings>,
    #[serde(default)]
    pub websites: Vec<WebsiteSettings>,
    #[serde(default)]
    pub parental: ParentalV1,
    #[serde(default)]
    pub screensaver: ScreensaverSettings,
    #[serde(default)]
    pub games: GameSettings,
    /// Where the games were before they got their own "Games" tab (`programs.pong`): still
    /// read and moved to `games`. "Programs" (registered programs) will be a different thing.
    #[serde(default, rename = "programs")]
    pub legacy_programs: Option<GameSettings>,
}

impl ConfigV1 {
    /// One home screen ("Home", ★) with everything; the limits move into it.
    fn convert(self, had_games: bool) -> Config {
        let games = match self.legacy_programs {
            Some(old) if !had_games => old,
            _ => self.games,
        };
        Config {
            version: VERSION,
            home_screens: vec![HomeScreen {
                home: self.home,
                discs: self.discs,
                folders: self.folders,
                websites: self.websites,
                games,
                limits: self.parental.limits,
                ..HomeScreen::new(V1_SCREEN_ID, "Home")
            }],
            parental: ParentalSettings {
                pin_hash: self.parental.pin_hash,
                pin_length: self.parental.pin_length,
                pin_backoff_doubling: self.parental.pin_backoff_doubling,
            },
            screensaver: self.screensaver,
            statistics: StatisticsSettings::default(),
            power: PowerSettings::default(),
        }
    }
}

/// `parental` of a version-1 config (with the time limits).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct ParentalV1 {
    pin_hash: Option<String>,
    pin_length: Option<u8>,
    pin_backoff_doubling: bool,
    limits: Vec<LimitRule>,
}

impl Default for ParentalV1 {
    fn default() -> Self {
        Self {
            pin_hash: None,
            pin_length: None,
            pin_backoff_doubling: true,
            limits: Vec::new(),
        }
    }
}

/// The "Games" row: built-in games.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GameSettings {
    pub pong: PongSettings,
    pub bricks: BricksSettings,
    pub blocks: BlocksSettings,
    pub dodger: DodgerSettings,
}

/// Star Dodger (`src/dodger.rs`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DodgerSettings {
    pub enabled: bool,
    /// Sound on at the start (can be switched in the game's menu).
    pub sound: bool,
    /// Music on at the start (can be switched in the game's menu).
    pub music: bool,
}

/// Block Stacker (`src/blocks.rs`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BlocksSettings {
    pub enabled: bool,
    /// Level the menu starts with (0–9; can be changed in the game's menu).
    pub start_level: u32,
    /// Sound on at the start (can be switched in the game's menu).
    pub sound: bool,
}

/// Brick Breaker (`src/bricks.rs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BricksSettings {
    pub enabled: bool,
    /// Paddles at the start of a game (1–99).
    pub lives: u32,
    /// Sound on at the start (can be switched in the game's menu).
    pub sound: bool,
}

impl Default for BricksSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            lives: 5,
            sound: false,
        }
    }
}

/// The built-in Ping Pong game (`src/pong.rs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PongSettings {
    pub enabled: bool,
    /// How well the computer plays (1-player game).
    pub computer_level: PongLevel,
    pub points_to_win: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PongLevel {
    Easy,
    Normal,
    Hard,
}

impl Default for PongSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            computer_level: PongLevel::Normal,
            points_to_win: 7,
        }
    }
}

/// The screensaver (`src/screensaver.rs`), started after `idle_minutes` without input on the
/// media center's own screens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ScreensaverSettings {
    pub mode: ScreensaverMode,
    pub idle_minutes: u32,
    /// Which file of the aerial videos is downloaded and played.
    pub aerial_quality: AerialQuality,
    /// Ids of the chosen aerial videos (AerialWindows/Aerial's `videos.json`), downloaded by
    /// the admin interface into `$XDG_CACHE_HOME/curator/aerial/<id>.<quality>.mov`.
    pub aerial_videos: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreensaverMode {
    Off,
    /// The title moving slowly over a very dark, slowly changing background.
    Title,
    /// Aerial videos (Apple TV style), played by mpv in random order.
    Aerial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AerialQuality {
    #[serde(rename = "1080p_h264")]
    Hd264,
    #[serde(rename = "1080p_hevc")]
    HdHevc,
    #[serde(rename = "4k_hevc")]
    UhdHevc,
}

impl AerialQuality {
    pub const ALL: [AerialQuality; 3] = [Self::Hd264, Self::HdHevc, Self::UhdHevc];

    /// As in the config and the downloaded file names.
    pub fn name(self) -> &'static str {
        match self {
            Self::Hd264 => "1080p_h264",
            Self::HdHevc => "1080p_hevc",
            Self::UhdHevc => "4k_hevc",
        }
    }
}

pub const MAX_AERIAL_VIDEOS: usize = 10;
pub const MAX_IDLE_MINUTES: u32 = 240;

impl Default for ScreensaverSettings {
    fn default() -> Self {
        Self {
            mode: ScreensaverMode::Title,
            idle_minutes: 10,
            aerial_quality: AerialQuality::Hd264,
            aerial_videos: Vec::new(),
        }
    }
}

/// Aerial video ids (UUIDs in the catalog): letters, digits and `-`, so they are safe in
/// file names.
pub fn is_aerial_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn validate_screensaver(s: &ScreensaverSettings) -> Result<(), String> {
    if !(1..=MAX_IDLE_MINUTES).contains(&s.idle_minutes) {
        return Err(format!("idle_minutes must be 1 to {MAX_IDLE_MINUTES}"));
    }
    if s.aerial_videos.len() > MAX_AERIAL_VIDEOS {
        return Err(format!("aerial_videos: at most {MAX_AERIAL_VIDEOS} videos"));
    }
    for (i, id) in s.aerial_videos.iter().enumerate() {
        if !is_aerial_id(id) {
            return Err(format!(
                "aerial_videos: \"{id}\" must be letters, digits and -"
            ));
        }
        if s.aerial_videos[..i].contains(id) {
            return Err(format!("aerial_videos: \"{id}\" is listed twice"));
        }
    }
    Ok(())
}

/// PIN and time limits (evaluated in `src/limits.rs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ParentalSettings {
    /// argon2 PHC string of the PIN (set by the admin interface, never the PIN itself).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_hash: Option<String>,
    /// Number of digits of the PIN (4–8), so the lock screen can check it as soon as the last
    /// digit is entered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_length: Option<u8>,
    /// Wrong PIN: wait 5 s, and with this option double the wait after each further try.
    pub pin_backoff_doubling: bool,
}

impl Default for ParentalSettings {
    fn default() -> Self {
        Self {
            pin_hash: None,
            pin_length: None,
            pin_backoff_doubling: true,
        }
    }
}

/// One time limit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LimitRule {
    /// A time budget in minutes (per day; for `this_week` for the whole week).
    Budget { minutes: u32, scope: Scope },
    /// Usable only from … until (`HH:MM`, `from` optional = from midnight).
    Window {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        until: String,
        scope: Scope,
    },
}

/// Which days a limit applies to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Scope {
    /// Only on this date (`YYYY-MM-DD`).
    Today {
        date: String,
    },
    /// On these weekdays (`mon` … `sun`).
    Weekdays {
        days: Vec<String>,
    },
    /// The week starting on this Monday (`YYYY-MM-DD`).
    ThisWeek {
        week_of: String,
    },
    EveryDay {},
}

pub const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const MAX_LIMITS: usize = 50;

/// `HH:MM` → time of day.
pub fn parse_time(text: &str) -> Option<chrono::NaiveTime> {
    let (h, m) = text.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    chrono::NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

/// `YYYY-MM-DD` → date.
pub fn parse_date(text: &str) -> Option<chrono::NaiveDate> {
    (text.len() == 10)
        .then(|| chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").ok())
        .flatten()
}

fn validate_scope(scope: &Scope) -> Result<(), String> {
    use chrono::Datelike;
    match scope {
        Scope::Today { date } => {
            parse_date(date).ok_or(format!("date \"{date}\" must be YYYY-MM-DD"))?;
        }
        Scope::Weekdays { days } => {
            if days.is_empty() {
                return Err("choose at least one weekday".into());
            }
            for (i, d) in days.iter().enumerate() {
                if !WEEKDAYS.contains(&d.as_str()) {
                    return Err(format!("\"{d}\" is not a weekday (mon … sun)"));
                }
                if days[..i].contains(d) {
                    return Err(format!("\"{d}\" is listed twice"));
                }
            }
        }
        Scope::ThisWeek { week_of } => {
            let monday =
                parse_date(week_of).ok_or(format!("week_of \"{week_of}\" must be YYYY-MM-DD"))?;
            if monday.weekday() != chrono::Weekday::Mon {
                return Err(format!("week_of \"{week_of}\" must be a Monday"));
            }
        }
        Scope::EveryDay {} => {}
    }
    Ok(())
}

fn validate_limit(rule: &LimitRule) -> Result<(), String> {
    match rule {
        LimitRule::Budget { minutes, scope } => {
            let max = if matches!(scope, Scope::ThisWeek { .. }) {
                7 * 24 * 60
            } else {
                24 * 60
            };
            if *minutes == 0 || *minutes > max {
                return Err(format!("minutes must be 1 to {max}"));
            }
            validate_scope(scope)
        }
        LimitRule::Window { from, until, scope } => {
            let until_t = parse_time(until).ok_or(format!("until \"{until}\" must be HH:MM"))?;
            if let Some(from) = from {
                let from_t = parse_time(from).ok_or(format!("from \"{from}\" must be HH:MM"))?;
                if from_t >= until_t {
                    return Err("from must be earlier than until".into());
                }
            }
            validate_scope(scope)
        }
    }
}

/// The home screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HomeSettings {
    /// Shown in the top-left corner.
    pub title: String,
    pub show_title: bool,
    /// Interface language: `auto` (the computer's language, English if it isn't
    /// available), or one of `LANGUAGES`.
    pub language: String,
}

/// Values of `home.language`.
pub const LANGUAGES: &[&str] = &["auto", "en", "de"];

impl Default for HomeSettings {
    fn default() -> Self {
        Self {
            title: "curator".into(),
            show_title: true,
            language: "auto".into(),
        }
    }
}

/// A website shown in the "Websites" row (rules: `src/sites.rs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebsiteSettings {
    pub id: String,
    pub name: String,
    /// Start page.
    pub url: String,
    #[serde(default = "yes")]
    pub allow_subdomains: bool,
    /// Show the website's own logo on the tile (fetched by the admin interface, see
    /// `logos`); otherwise the generic website icon.
    #[serde(default = "yes")]
    pub show_logo: bool,
}

fn yes() -> bool {
    true
}

/// The picture on a program's tile (painted by us, `ui::paint_program_icon`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramIcon {
    #[default]
    Gamepad,
    Joystick,
    Computer,
    Handheld,
    Floppy,
    App,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmulatorSystem {
    Dos,
    Snes,
}

/// A DOS drive: a folder or a disk image. Z: is DOSBox's own drive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveMapping {
    pub letter: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DosOptions {
    pub drives: Vec<DriveMapping>,
    /// What to start, e.g. `GAME\\START.EXE -sound`; on the first drive unless it names one.
    pub autostart: String,
    /// DOS commands after mounting and before the game.
    pub startup_script: String,
    pub correct_aspect: bool,
    /// Keep what the game saved between starts; off: fresh from the original every time.
    pub keep_saves: bool,
}

impl Default for DosOptions {
    fn default() -> Self {
        Self {
            drives: Vec::new(),
            autostart: String::new(),
            startup_script: String::new(),
            correct_aspect: true,
            keep_saves: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnesFilter {
    None,
    #[default]
    CrtEasymode,
    ZfastCrt,
    CrtGeom,
    CrtLottes,
    CrtRoyale,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnesAspect {
    /// As on the TV the games were made for.
    #[default]
    #[serde(rename = "4_3")]
    Ratio4x3,
    /// Square pixels.
    #[serde(rename = "8_7")]
    Ratio8x7,
    #[serde(rename = "fill")]
    Fill,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SnesOptions {
    pub rom: String,
    pub filter: SnesFilter,
    pub aspect: SnesAspect,
    pub integer_scale: bool,
    /// Continue exactly where the game was left (battery saves are always kept).
    pub resume: bool,
}

impl Default for SnesOptions {
    fn default() -> Self {
        Self {
            rom: String::new(),
            filter: SnesFilter::default(),
            aspect: SnesAspect::default(),
            integer_scale: false,
            resume: true,
        }
    }
}

/// A game in the "Emulators" row (`src/emulator.rs`). The options of the other system are
/// kept, so switching back and forth in the admin panel loses nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmulatorSettings {
    pub id: String,
    pub name: String,
    pub system: EmulatorSystem,
    #[serde(default)]
    pub dos: DosOptions,
    #[serde(default)]
    pub snes: SnesOptions,
}

/// A program in the "Programs" row (e.g. an emulator). Started exactly as registered: the
/// program and each argument as they are, never through a shell (`player::program_command`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramSettings {
    pub id: String,
    pub name: String,
    /// An absolute path, or a program name looked up in `PATH`.
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub icon: ProgramIcon,
}

/// A folder shown in the "Folders" row. Always accessed read-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderSettings {
    pub id: String,
    pub name: String,
    /// Absolute, normalized path.
    pub path: String,
    #[serde(default)]
    pub video: bool,
    #[serde(default)]
    pub audio: bool,
    #[serde(default)]
    pub images: bool,
    /// List the files played last ("Recently played") when the folder is opened.
    #[serde(default = "yes")]
    pub remember_recent: bool,
    /// Removed option (extra file types). Still accepted in older config files, but ignored and
    /// never written: only media files are shown.
    #[serde(default, rename = "extensions", skip_serializing)]
    pub legacy_extensions: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DiscSettings {
    pub enabled: bool,
    pub allow_disc_menu: bool,
    /// ISO 639-2 codes, most preferred first.
    pub audio_languages: Vec<String>,
    pub subtitle_languages: Vec<String>,
}

impl Default for DiscSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_disc_menu: true,
            audio_languages: Vec::new(),
            subtitle_languages: Vec::new(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: VERSION,
            home_screens: vec![HomeScreen::new(V1_SCREEN_ID, "Home")],
            parental: ParentalSettings::default(),
            screensaver: ScreensaverSettings::default(),
            statistics: StatisticsSettings::default(),
            power: PowerSettings::default(),
        }
    }
}

impl Config {
    /// Used when the config exists but can't be read: everything that could be restricted is off.
    pub fn locked_down() -> Self {
        let mut config = Self::default();
        config.home_screens[0].discs = DiscSettings {
            enabled: false,
            allow_disc_menu: false,
            ..DiscSettings::default()
        };
        config.power.enabled = false;
        config
    }
}

/// The current config plus the last error (shown to parents and reported over the socket).
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigState {
    pub config: Config,
    pub error: Option<String>,
}

/// `$CURATOR_CONFIG_DIR`, or `$XDG_CONFIG_HOME/curator`.
pub fn config_dir() -> Option<PathBuf> {
    match std::env::var_os("CURATOR_CONFIG_DIR") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => Some(dirs::config_dir()?.join("curator")),
    }
}

pub fn config_path() -> Option<PathBuf> {
    Some(config_dir()?.join("config.json"))
}

/// Parse and validate config JSON (version 2, or version 1, converted).
pub fn parse(text: &str) -> Result<Config, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("invalid config: {e}"))?;
    let config = match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(1) => {
            settings_are_objects_v1(&value)?;
            let had_games = value.get("games").is_some();
            let v1: ConfigV1 =
                serde_json::from_value(value).map_err(|e| format!("invalid config: {e}"))?;
            v1.convert(had_games)
        }
        _ => {
            settings_are_objects(&value)?;
            serde_json::from_value(value).map_err(|e| format!("invalid config: {e}"))?
        }
    };
    validate(&config)?;
    Ok(config)
}

fn object(v: Option<&serde_json::Value>, path: &str) -> Result<(), String> {
    match v {
        None | Some(serde_json::Value::Object(_)) => Ok(()),
        Some(_) => Err(format!("invalid config: {path} must be an object")),
    }
}

/// Inside one home screen (or a version-1 config): settings sections must be objects.
fn screen_sections_are_objects(value: &serde_json::Value, at: &str) -> Result<(), String> {
    use serde_json::Value;
    for key in ["home", "discs", "games"] {
        object(value.get(key), &format!("{at}{key}"))?;
    }
    for path in ["games/pong", "games/bricks", "games/blocks", "games/dodger"] {
        object(
            value.pointer(&format!("/{path}")),
            &format!("{at}{}", path.replace('/', ".")),
        )?;
    }
    for list in ["folders", "websites", "programs", "emulators"] {
        if let Some(Value::Array(items)) = value.get(list) {
            for (i, item) in items.iter().enumerate() {
                object(Some(item), &format!("{at}{list}[{i}]"))?;
            }
        }
    }
    if let Some(Value::Array(items)) = value.get("emulators") {
        for (i, item) in items.iter().enumerate() {
            let at = format!("{at}emulators[{i}]");
            object(item.get("dos"), &format!("{at}.dos"))?;
            object(item.get("snes"), &format!("{at}.snes"))?;
            if let Some(Value::Array(drives)) = item.pointer("/dos/drives") {
                for (d, drive) in drives.iter().enumerate() {
                    object(Some(drive), &format!("{at}.dos.drives[{d}]"))?;
                }
            }
        }
    }
    Ok(())
}

fn limits_are_objects(rules: Option<&serde_json::Value>, at: &str) -> Result<(), String> {
    if let Some(serde_json::Value::Array(rules)) = rules {
        for (i, rule) in rules.iter().enumerate() {
            object(Some(rule), &format!("{at}[{i}]"))?;
            object(rule.get("scope"), &format!("{at}[{i}].scope"))?;
        }
    }
    Ok(())
}

/// serde also builds a settings struct from a list (`"home": []`); the admin interface (and
/// the schema) only accept objects, so lists are refused here, at every level.
fn settings_are_objects(value: &serde_json::Value) -> Result<(), String> {
    use serde_json::Value;
    object(Some(value), "the config")?;
    for key in ["parental", "screensaver", "statistics", "power"] {
        object(value.get(key), key)?;
    }
    if let Some(Value::Array(screens)) = value.get("home_screens") {
        for (i, screen) in screens.iter().enumerate() {
            let at = format!("home_screens[{i}]");
            object(Some(screen), &at)?;
            screen_sections_are_objects(screen, &format!("{at}."))?;
            limits_are_objects(screen.get("limits"), &format!("{at}.limits"))?;
        }
    }
    Ok(())
}

/// The same for a version-1 config.
fn settings_are_objects_v1(value: &serde_json::Value) -> Result<(), String> {
    object(Some(value), "the config")?;
    for key in ["parental", "screensaver", "programs"] {
        object(value.get(key), key)?;
    }
    for path in ["programs/pong", "programs/bricks"] {
        object(value.pointer(&format!("/{path}")), &path.replace('/', "."))?;
    }
    screen_sections_are_objects(value, "")?;
    limits_are_objects(value.pointer("/parental/limits"), "parental.limits")
}

fn validate(config: &Config) -> Result<(), String> {
    if config.version != VERSION {
        return Err(format!(
            "unsupported config version {} (expected {VERSION})",
            config.version
        ));
    }
    let screens = &config.home_screens;
    if screens.is_empty() || screens.len() > MAX_HOME_SCREENS {
        return Err(format!(
            "home_screens: 1 to {MAX_HOME_SCREENS} home screens"
        ));
    }
    if screens.iter().filter(|s| s.default).count() != 1 {
        return Err("home_screens: exactly one must be the default".into());
    }
    for (i, screen) in screens.iter().enumerate() {
        let at = format!("home_screens[{i}]");
        validate_screen(screen).map_err(|e| format!("{at}.{e}"))?;
        if screens[..i].iter().any(|s| s.id == screen.id) {
            return Err(format!("{at}: id \"{}\" is used twice", screen.id));
        }
        // Folder and website ids are keys of our own files (recently played, logos):
        // unique across all home screens.
        for f in &screen.folders {
            if screens[..i]
                .iter()
                .any(|s| s.folders.iter().any(|o| o.id == f.id))
            {
                return Err(format!(
                    "{at}.folders: id \"{}\" is used on another home screen",
                    f.id
                ));
            }
        }
        for e in &screen.emulators {
            if screens[..i]
                .iter()
                .any(|s| s.emulators.iter().any(|o| o.id == e.id))
            {
                return Err(format!(
                    "{at}.emulators: id \"{}\" is used on another home screen",
                    e.id
                ));
            }
        }
        for p in &screen.programs {
            if screens[..i]
                .iter()
                .any(|s| s.programs.iter().any(|o| o.id == p.id))
            {
                return Err(format!(
                    "{at}.programs: id \"{}\" is used on another home screen",
                    p.id
                ));
            }
        }
        for w in &screen.websites {
            if screens[..i]
                .iter()
                .any(|s| s.websites.iter().any(|o| o.id == w.id))
            {
                return Err(format!(
                    "{at}.websites: id \"{}\" is used on another home screen",
                    w.id
                ));
            }
        }
    }
    if let Some(hash) = &config.parental.pin_hash
        && (!hash.starts_with("$argon2") || hash.len() > 512)
    {
        return Err("parental.pin_hash must be an argon2 hash".into());
    }
    if let Some(len) = config.parental.pin_length {
        if !(4..=8).contains(&len) {
            return Err("parental.pin_length must be 4 to 8".into());
        }
        if config.parental.pin_hash.is_none() {
            return Err("parental.pin_length needs a pin_hash".into());
        }
    }
    validate_screensaver(&config.screensaver).map_err(|e| format!("screensaver.{e}"))?;
    if !(1..=MAX_KEEP_DAYS).contains(&config.statistics.keep_days) {
        return Err(format!("statistics.keep_days must be 1 to {MAX_KEEP_DAYS}"));
    }
    Ok(())
}

/// One home screen (errors start with the field, e.g. `folders[0]: …`).
fn validate_screen(screen: &HomeScreen) -> Result<(), String> {
    validate_id_and_name(&screen.id, "x").map_err(|e| format!("id: {e}"))?;
    let chars = screen.name.chars().count();
    if chars > MAX_SCREEN_NAME_CHARS {
        return Err(format!("name: at most {MAX_SCREEN_NAME_CHARS} characters"));
    }
    check_text(&screen.name).map_err(|e| format!("name {e}"))?;
    check_text(&screen.home.title).map_err(|e| format!("home.title: {e}"))?;
    if !LANGUAGES.contains(&screen.home.language.as_str()) {
        return Err(format!(
            "home.language must be one of {}",
            LANGUAGES.join(", ")
        ));
    }
    if screen.limits.len() > MAX_LIMITS {
        return Err(format!("limits: at most {MAX_LIMITS} rules"));
    }
    for (i, rule) in screen.limits.iter().enumerate() {
        validate_limit(rule).map_err(|e| format!("limits[{i}]: {e}"))?;
    }
    for (field, list) in [
        ("discs.audio_languages", &screen.discs.audio_languages),
        ("discs.subtitle_languages", &screen.discs.subtitle_languages),
    ] {
        if list.len() > MAX_LANGUAGES {
            return Err(format!("{field}: at most {MAX_LANGUAGES} languages"));
        }
        for (i, lang) in list.iter().enumerate() {
            if lang.len() != 3 || !lang.bytes().all(|b| b.is_ascii_lowercase()) {
                return Err(format!(
                    "{field}: \"{lang}\" is not a 3-letter language code like \"deu\""
                ));
            }
            if list[..i].contains(lang) {
                return Err(format!("{field}: \"{lang}\" is listed twice"));
            }
        }
    }
    if !(3..=21).contains(&screen.games.pong.points_to_win) {
        return Err("games.pong.points_to_win must be 3 to 21".into());
    }
    if !(1..=99).contains(&screen.games.bricks.lives) {
        return Err("games.bricks.lives must be 1 to 99".into());
    }
    if screen.games.blocks.start_level > 9 {
        return Err("games.blocks.start_level must be 0 to 9".into());
    }
    if screen.folders.len() > MAX_FOLDERS {
        return Err(format!("folders: at most {MAX_FOLDERS} folders"));
    }
    if screen.websites.len() > MAX_WEBSITES {
        return Err(format!("websites: at most {MAX_WEBSITES} websites"));
    }
    if screen.programs.len() > MAX_PROGRAMS {
        return Err(format!("programs: at most {MAX_PROGRAMS} programs"));
    }
    if screen.emulators.len() > MAX_EMULATORS {
        return Err(format!("emulators: at most {MAX_EMULATORS}"));
    }
    for (i, emulator) in screen.emulators.iter().enumerate() {
        validate_emulator(emulator).map_err(|e| format!("emulators[{i}]: {e}"))?;
        if screen.emulators[..i].iter().any(|o| o.id == emulator.id) {
            return Err(format!(
                "emulators[{i}]: id \"{}\" is used twice",
                emulator.id
            ));
        }
    }
    for (i, program) in screen.programs.iter().enumerate() {
        validate_program(program).map_err(|e| format!("programs[{i}]: {e}"))?;
        if screen.programs[..i].iter().any(|p| p.id == program.id) {
            return Err(format!(
                "programs[{i}]: id \"{}\" is used twice",
                program.id
            ));
        }
    }
    for (i, site) in screen.websites.iter().enumerate() {
        validate_website(site).map_err(|e| format!("websites[{i}]: {e}"))?;
        if screen.websites[..i].iter().any(|w| w.id == site.id) {
            return Err(format!("websites[{i}]: id \"{}\" is used twice", site.id));
        }
    }
    for (i, folder) in screen.folders.iter().enumerate() {
        validate_folder(folder).map_err(|e| format!("folders[{i}]: {e}"))?;
        if screen.folders[..i].iter().any(|f| f.id == folder.id) {
            return Err(format!("folders[{i}]: id \"{}\" is used twice", folder.id));
        }
    }
    Ok(())
}

fn validate_id_and_name(id: &str, name: &str) -> Result<(), String> {
    let id_ok = (1..=40).contains(&id.len())
        && id
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_lowercase() || b.is_ascii_digit() || (b == b'-' && i > 0));
    if !id_ok {
        return Err(format!(
            "id \"{id}\" must be lowercase letters, digits and -"
        ));
    }
    check_text(name).map_err(|e| format!("name {e}"))
}

/// Names and titles: 1–60 characters, no control characters, no leading/trailing spaces.
fn check_text(text: &str) -> Result<(), String> {
    let chars = text.chars().count();
    if chars == 0 || chars > MAX_NAME_CHARS {
        return Err(format!("must have 1 to {MAX_NAME_CHARS} characters"));
    }
    if text.trim() != text || text.chars().any(char::is_control) {
        return Err("must not start/end with spaces or contain control characters".into());
    }
    Ok(())
}

fn validate_website(w: &WebsiteSettings) -> Result<(), String> {
    validate_id_and_name(&w.id, &w.name)?;
    if w.url.len() > MAX_URL_BYTES {
        return Err(format!("url: at most {MAX_URL_BYTES} characters"));
    }
    let url = url::Url::parse(&w.url).map_err(|e| format!("url \"{}\": {e}", w.url))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("url must not contain a user name or password".into());
    }
    crate::sites::rules_for(&w.url, w.allow_subdomains)
        .map_err(|e| format!("url \"{}\": {e}", w.url))?;
    Ok(())
}

fn has_control_chars(text: &str) -> bool {
    text.chars().any(|c| c < ' ' || c == '\x7f')
}

fn validate_program(p: &ProgramSettings) -> Result<(), String> {
    validate_id_and_name(&p.id, &p.name)?;
    let command = &p.command;
    if command.is_empty() || command.chars().count() > MAX_COMMAND_CHARS {
        return Err(format!("command: 1 to {MAX_COMMAND_CHARS} characters"));
    }
    if has_control_chars(command) || command.starts_with('-') {
        return Err("command: not a program".into());
    }
    if command.starts_with('/') {
        if !is_normalized_absolute_path(command) {
            return Err("command: the path must not contain '.', '..' or empty parts".into());
        }
    } else if command.contains('/') {
        return Err("command: an absolute path or a program name".into());
    } else if command.contains(' ') {
        return Err("command: put arguments into the list of arguments".into());
    }
    if p.args.len() > MAX_PROGRAM_ARGS {
        return Err(format!("args: at most {MAX_PROGRAM_ARGS}"));
    }
    for arg in &p.args {
        if arg.chars().count() > MAX_ARG_CHARS || has_control_chars(arg) {
            return Err(format!(
                "args: at most {MAX_ARG_CHARS} characters, no control characters"
            ));
        }
    }
    if let Some(dir) = &p.working_dir
        && (!is_normalized_absolute_path(dir) || has_control_chars(dir))
    {
        return Err("working_dir: must be an absolute path".into());
    }
    Ok(())
}

const SNES_ROM_EXTENSIONS: [&str; 5] = ["sfc", "smc", "swc", "fig", "zip"];

/// A path we hand to DOSBox or RetroArch, in double quotes in their configuration.
fn game_path_ok(path: &str) -> bool {
    is_normalized_absolute_path(path) && !has_control_chars(path) && !path.contains('"')
}

fn validate_emulator(e: &EmulatorSettings) -> Result<(), String> {
    validate_id_and_name(&e.id, &e.name)?;
    let dos = &e.dos;
    if dos.drives.len() > MAX_DRIVES {
        return Err(format!("dos.drives: at most {MAX_DRIVES}"));
    }
    if e.system == EmulatorSystem::Dos && dos.drives.is_empty() {
        return Err("dos.drives: add at least one drive".into());
    }
    for (i, drive) in dos.drives.iter().enumerate() {
        let letter_ok =
            drive.letter.len() == 1 && matches!(drive.letter.as_bytes()[0], b'A'..=b'Y');
        if !letter_ok {
            return Err(format!("dos.drives[{i}]: the drive letter must be A to Y"));
        }
        if dos.drives[..i].iter().any(|d| d.letter == drive.letter) {
            return Err(format!(
                "dos.drives[{i}]: drive {} is used twice",
                drive.letter
            ));
        }
        if !game_path_ok(&drive.path) {
            return Err(format!("dos.drives[{i}]: must be an absolute path"));
        }
    }
    if dos.autostart.chars().count() > MAX_AUTOSTART_CHARS
        || has_control_chars(&dos.autostart)
        || dos.autostart.contains('"')
        || dos.autostart.contains("..")
    {
        return Err("dos.autostart: not a file inside a drive".into());
    }
    if dos.startup_script.chars().count() > MAX_STARTUP_SCRIPT_CHARS {
        return Err(format!(
            "dos.startup_script: at most {MAX_STARTUP_SCRIPT_CHARS} characters"
        ));
    }
    for line in dos.startup_script.split('\n') {
        // A line starting with [ would begin a new section of DOSBox's configuration.
        if has_control_chars(line) || line.trim_start().starts_with('[') {
            return Err("dos.startup_script: a line is not a DOS command".into());
        }
    }
    let rom = &e.snes.rom;
    if rom.is_empty() {
        if e.system == EmulatorSystem::Snes {
            return Err("snes.rom: choose a ROM file".into());
        }
    } else {
        let extension = rom.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase());
        if !game_path_ok(rom)
            || !extension.is_some_and(|x| SNES_ROM_EXTENSIONS.contains(&x.as_str()))
        {
            return Err(format!(
                "snes.rom: an absolute path to a {} file",
                SNES_ROM_EXTENSIONS.join(", ")
            ));
        }
    }
    Ok(())
}

fn validate_folder(f: &FolderSettings) -> Result<(), String> {
    validate_id_and_name(&f.id, &f.name)?;
    if !is_normalized_absolute_path(&f.path) {
        return Err(format!(
            "path \"{}\" must be an absolute path without '.', '..' or empty parts",
            f.path
        ));
    }
    if !(f.video || f.audio || f.images) {
        return Err("choose at least one kind of file to show".into());
    }
    Ok(())
}

/// `/a/b` yes; `a/b`, `/a/../b`, `/a/./b`, `/a//b`, `/a/` no; `/` yes.
pub fn is_normalized_absolute_path(path: &str) -> bool {
    if path == "/" {
        return true;
    }
    path.len() <= MAX_PATH_BYTES
        && !path.contains('\0')
        && path.starts_with('/')
        && path[1..]
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Load the config file. `Ok(None)` if there is none yet.
pub fn load(path: &Path) -> Result<Option<Config>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

/// Config at startup.
pub fn load_initial(path: Option<&Path>) -> ConfigState {
    let Some(path) = path else {
        return ConfigState {
            config: Config::locked_down(),
            error: Some("no config directory (HOME not set?)".into()),
        };
    };
    match load(path) {
        Ok(Some(config)) => ConfigState {
            config,
            error: None,
        },
        Ok(None) => ConfigState {
            config: Config::default(),
            error: None,
        },
        Err(e) => {
            log::error!("{e}; using locked-down settings");
            ConfigState {
                config: Config::locked_down(),
                error: Some(e),
            }
        }
    }
}

/// Reload the config. On error the current config stays active.
pub fn reload(state: &mut ConfigState, path: &Path) -> Result<(), String> {
    match load(path) {
        Ok(config) => {
            state.config = config.unwrap_or_default();
            state.error = None;
            log::info!("config reloaded");
            Ok(())
        }
        Err(e) => {
            log::warn!("config reload failed, keeping the current config: {e}");
            state.error = Some(e.clone());
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("curator-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn home_screens() {
        let config = parse(include_str!("../docs/config.example.json")).unwrap();
        assert_eq!(config.home_screens.len(), 2);
        assert_eq!(config.default_screen().id, "family");
        assert_eq!(config.screen(Some("kids")).name, "Kids");
        assert_eq!(
            config.screen(Some("gone")).id,
            "family",
            "unknown: the default"
        );
        assert!(config.statistics.enabled);
        assert_eq!(config.statistics.keep_days, 90);
        assert!(config.power.enabled);
        let dos = &config.home_screens[1].programs[0];
        assert_eq!(dos.command, "dosbox");
        assert_eq!(dos.args.len(), 3);
        assert_eq!(dos.icon, ProgramIcon::Computer);
        assert_eq!(view_programs(&config), 1);
        let games = &config.home_screens[1].emulators;
        assert_eq!(games[0].system, EmulatorSystem::Dos);
        assert_eq!(games[0].dos.drives[1].letter, "A");
        assert_eq!(games[1].snes.filter, SnesFilter::CrtGeom);
        assert_eq!(games[1].snes.aspect, SnesAspect::Ratio4x3);
        assert!(games[1].snes.resume, "on by default");
        assert!(games[0].dos.keep_saves, "on by default");
        assert!(
            config.home_screens[0].folders[0].remember_recent,
            "on by default"
        );
        assert!(!Config::locked_down().power.enabled, "fails closed");
        let view = config.view(Some("kids"));
        assert_eq!(view.screen_id, "kids");
        assert_eq!(view.limits.len(), 1);
        assert_eq!(view.folders[0].id, "kids-stories");
        assert_eq!(view.screens.len(), 2);
        assert!(view.parental.pin_backoff_doubling);
        // Missing file: one home screen with the star.
        let d = Config::default();
        assert_eq!(d.home_screens.len(), 1);
        assert!(d.home_screens[0].default);
        assert!(!d.statistics.enabled);
        assert!(d.power.enabled, "as before the setting existed");
    }

    #[test]
    fn version_1_becomes_one_home_screen() {
        let c = parse(
            r#"{"version":1,"home":{"title":"Kino"},"folders":[{"id":"m","name":"M","path":"/m","video":true}],
               "parental":{"pin_backoff_doubling":false,"limits":[{"type":"budget","minutes":60,"scope":{"kind":"every_day"}}]},
               "programs":{"pong":{"enabled":true}}}"#,
        )
        .unwrap();
        assert_eq!(c.version, VERSION);
        assert_eq!(c.home_screens.len(), 1);
        let s = &c.home_screens[0];
        assert_eq!(
            (s.id.as_str(), s.name.as_str(), s.default),
            (V1_SCREEN_ID, "Home", true)
        );
        assert_eq!(s.home.title, "Kino");
        assert_eq!(s.folders[0].id, "m");
        assert_eq!(s.limits.len(), 1, "the limits move into the home screen");
        assert!(s.games.pong.enabled, "games from their old place too");
        assert!(!c.parental.pin_backoff_doubling);
        // Written as version 2.
        let written = serde_json::to_value(&c).unwrap();
        assert_eq!(written["version"], 2);
        assert!(written.get("folders").is_none() && written["parental"].get("limits").is_none());
        assert_eq!(parse(&written.to_string()).unwrap(), c);
    }

    #[test]
    fn example_config_is_valid() {
        let config = parse(include_str!("../docs/config.example.json")).unwrap();
        assert_eq!(config.home_screens[0].discs.audio_languages, ["deu", "eng"]);
    }

    #[test]
    fn example_folders() {
        let config = parse(include_str!("../docs/config.example.json")).unwrap();
        assert_eq!(config.home_screens[0].folders.len(), 3);
        assert!(config.home_screens[0].folders[2].audio);
        assert!(!config.home_screens[0].folders[2].video);
    }

    #[test]
    fn old_extensions_field_is_ignored() {
        let config = parse(
            r#"{"version":1,"folders":[{"id":"a","name":"A","path":"/x","video":true,"extensions":["pdf"]}]}"#,
        )
        .unwrap();
        let written = serde_json::to_string(&config).unwrap();
        assert!(!written.contains("extensions"));
        // Extensions alone no longer make a folder valid.
        assert!(parse(r#"{"version":1,"folders":[{"id":"a","name":"A","path":"/x","extensions":["pdf"]}]}"#).is_err());
    }

    #[test]
    fn example_websites() {
        let config = parse(include_str!("../docs/config.example.json")).unwrap();
        assert_eq!(config.home_screens[0].websites.len(), 2);
        assert!(config.home_screens[0].websites[0].allow_subdomains);
        assert!(!config.home_screens[0].websites[1].allow_subdomains);
        // Missing toggle: subdomains allowed.
        let c =
            parse(r#"{"version":1,"websites":[{"id":"k","name":"K","url":"https://kika.de/"}]}"#)
                .unwrap();
        assert!(c.home_screens[0].websites[0].allow_subdomains);
        assert!(c.home_screens[0].websites[0].show_logo); // logo on by default
        assert!(!config.home_screens[0].websites[1].show_logo);
    }

    #[test]
    fn home_title() {
        let config = parse(include_str!("../docs/config.example.json")).unwrap();
        assert_eq!(config.home_screens[0].home.title, "Family Cinema");
        let c = parse(r#"{"version":1,"home":{"show_title":false}}"#).unwrap();
        assert_eq!(c.home_screens[0].home.title, "curator");
        assert!(!c.home_screens[0].home.show_title);
        assert!(
            parse(r#"{"version":1}"#).unwrap().home_screens[0]
                .home
                .show_title
        );
        assert_eq!(c.home_screens[0].home.language, "auto");
        assert_eq!(config.home_screens[0].home.language, "de");
    }

    #[test]
    fn example_parental() {
        let config = parse(include_str!("../docs/config.example.json")).unwrap();
        assert!(config.parental.pin_backoff_doubling);
        assert_eq!(config.home_screens[0].limits.len(), 4);
        assert_eq!(
            config.home_screens[0].limits[1],
            LimitRule::Window {
                from: Some("07:00".into()),
                until: "19:30".into(),
                scope: Scope::Weekdays {
                    days: ["mon", "tue", "wed", "thu", "fri"]
                        .map(String::from)
                        .to_vec()
                }
            }
        );
        // Week budgets may be larger than a day.
        assert!(parse(r#"{"version":1,"parental":{"limits":[{"type":"budget","minutes":2000,"scope":{"kind":"this_week","week_of":"2026-11-02"}}]}}"#).is_ok());
        // The PIN hash is never written back out when absent.
        let written = serde_json::to_string(&Config::default()).unwrap();
        assert!(!written.contains("pin_hash"));
    }

    fn view_programs(config: &Config) -> usize {
        config.view(Some("kids")).programs.len()
    }

    #[test]
    fn screensaver() {
        let c = parse(r#"{"version":1}"#).unwrap();
        assert_eq!(c.screensaver.mode, ScreensaverMode::Title);
        assert_eq!(c.screensaver.idle_minutes, 10);
        let config = parse(include_str!("../docs/config.example.json")).unwrap();
        assert_eq!(config.screensaver.mode, ScreensaverMode::Aerial);
        assert_eq!(config.screensaver.aerial_quality, AerialQuality::HdHevc);
        assert_eq!(config.screensaver.aerial_videos.len(), 2);
        let written = serde_json::to_string(&config.screensaver).unwrap();
        assert!(written.contains(r#""aerial_quality":"1080p_hevc""#));
        assert!(written.contains(r#""mode":"aerial""#));
    }

    #[test]
    fn games() {
        let c = parse(r#"{"version":1}"#).unwrap();
        assert!(!c.home_screens[0].games.pong.enabled && !c.home_screens[0].games.bricks.enabled);
        assert_eq!(
            c.home_screens[0].games.pong.computer_level,
            PongLevel::Normal
        );
        assert_eq!(c.home_screens[0].games.pong.points_to_win, 7);
        let config = parse(include_str!("../docs/config.example.json")).unwrap();
        assert!(
            config.home_screens[0].games.pong.enabled
                && config.home_screens[0].games.bricks.enabled
        );
        assert_eq!(config.home_screens[0].games.bricks.lives, 3);
        assert!(config.home_screens[0].games.bricks.sound);
        assert_eq!(c.home_screens[0].games.bricks.lives, 5);
        assert!(!c.home_screens[0].games.bricks.sound);
        assert_eq!(
            config.home_screens[0].games.pong.computer_level,
            PongLevel::Easy
        );
        assert_eq!(config.home_screens[0].games.pong.points_to_win, 5);
        // The old place (`programs`) is read, moved to `games` and not written back.
        let old = parse(r#"{"version":1,"programs":{"pong":{"enabled":true,"points_to_win":9}}}"#)
            .unwrap();
        assert!(old.home_screens[0].games.pong.enabled);
        assert_eq!(old.home_screens[0].games.pong.points_to_win, 9);
        let written = serde_json::to_string(&old).unwrap();
        assert!(!written.contains(r#""programs":{"#) && written.contains(r#""games""#));
        // Both: `games` wins.
        let both = parse(
            r#"{"version":1,"programs":{"pong":{"enabled":true}},"games":{"pong":{"enabled":false}}}"#,
        )
        .unwrap();
        assert!(!both.home_screens[0].games.pong.enabled);
    }

    #[test]
    fn paths() {
        for ok in ["/", "/a", "/srv/media movies/x.y", "/a/.hidden"] {
            assert!(is_normalized_absolute_path(ok), "{ok}");
        }
        for bad in ["", "a", "/a/", "//a", "/a/./b", "/a/..", "/a/../b", "/a\0b"] {
            assert!(!is_normalized_absolute_path(bad), "{bad}");
        }
    }

    #[test]
    fn missing_fields_take_defaults() {
        let config = parse(r#"{"version":1}"#).unwrap();
        assert_eq!(config, Config::default());
        let config = parse(r#"{"version":1,"discs":{"enabled":false}}"#).unwrap();
        assert!(!config.home_screens[0].discs.enabled);
        assert!(config.home_screens[0].discs.allow_disc_menu);
    }

    #[test]
    fn rejects_bad_configs() {
        for bad in [
            r#"{"version":2}"#,
            r#"{}"#,
            r#"{"version":1,"unknown":true}"#,
            r#"{"version":1,"discs":{"enabeld":true}}"#,
            r#"{"version":1,"discs":{"audio_languages":["de"]}}"#,
            r#"{"version":1,"discs":{"audio_languages":["DEU"]}}"#,
            r#"{"version":1,"discs":{"audio_languages":["deu","deu"]}}"#,
            r#"{"version":1,"discs":{"enabled":"yes"}}"#,
            r#"{"version":1,"folders":[{"id":"a","name":"A","path":"/x"}]}"#,
            r#"{"version":1,"folders":[{"id":"A","name":"A","path":"/x","video":true}]}"#,
            r#"{"version":1,"folders":[{"id":"-a","name":"A","path":"/x","video":true}]}"#,
            r#"{"version":1,"folders":[{"id":"a","name":"","path":"/x","video":true}]}"#,
            r#"{"version":1,"folders":[{"id":"a","name":" A","path":"/x","video":true}]}"#,
            r#"{"version":1,"folders":[{"id":"a","name":"A","path":"x","video":true}]}"#,
            r#"{"version":1,"folders":[{"id":"a","name":"A","path":"/x/../etc","video":true}]}"#,
            r#"{"version":1,"folders":[{"id":"a","name":"A","path":"/x/","video":true}]}"#,
            r#"{"version":1,"folders":[{"id":"a","name":"A","path":"/x//y","video":true}]}"#,
            r#"{"version":1,"folders":[{"id":"a","name":"A","path":"/x","video":true,"write":true}]}"#,
            r#"{"version":1,"folders":[{"id":"a","name":"A","path":"/x","video":true},{"id":"a","name":"B","path":"/y","audio":true}]}"#,
            r#"{"version":1,"websites":[{"id":"a","name":"A"}]}"#,
            r#"{"version":1,"websites":[{"id":"a","name":"A","url":"http://localhost:8787/"}]}"#,
            r#"{"version":1,"websites":[{"id":"a","name":"A","url":"https://192.168.1.1/"}]}"#,
            r#"{"version":1,"websites":[{"id":"a","name":"A","url":"https://box.local/"}]}"#,
            r#"{"version":1,"websites":[{"id":"a","name":"A","url":"file:///etc/passwd"}]}"#,
            r#"{"version":1,"websites":[{"id":"a","name":"A","url":"kika.de"}]}"#,
            r#"{"version":1,"websites":[{"id":"a","name":"A","url":"https://user:pw@kika.de/"}]}"#,
            r#"{"version":1,"websites":[{"id":"a","name":"A","url":"https://kika.de/","allow_subdomains":"yes"}]}"#,
            r#"{"version":1,"websites":[{"id":"a","name":"A","url":"https://kika.de/","show_logo":1}]}"#,
            r#"{"version":1,"websites":[{"id":"a","name":"A","url":"https://kika.de/","extra":1}]}"#,
            r#"{"version":1,"websites":[{"id":"a","name":"A","url":"https://a.de/"},{"id":"a","name":"B","url":"https://b.de/"}]}"#,
            r#"{"version":1,"home":{"title":""}}"#,
            r#"{"version":1,"home":{"title":" Kino"}}"#,
            r#"{"version":1,"home":{"title":"a\u0007b"}}"#,
            r#"{"version":1,"home":{"show_title":"no"}}"#,
            r#"{"version":1,"home":{"language":"fr"}}"#,
            r#"{"version":1,"home":{"language":"DE"}}"#,
            r#"{"version":1,"home":{"language":1}}"#,
            r#"{"version":1,"home":{"logo":"x"}}"#,
            r#"{"version":1,"parental":{"pin_hash":"1234"}}"#,
            r#"{"version":1,"parental":{"pin_hash":"$argon2id$x","pin_length":3}}"#,
            r#"{"version":1,"parental":{"pin_hash":"$argon2id$x","pin_length":9}}"#,
            r#"{"version":1,"parental":{"pin_length":4}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"budget","minutes":0,"scope":{"kind":"every_day"}}]}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"budget","minutes":1441,"scope":{"kind":"every_day"}}]}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"budget","minutes":60}]}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"budget","minutes":60,"scope":{"kind":"today","date":"2026-13-01"}}]}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"budget","minutes":60,"scope":{"kind":"weekdays","days":[]}}]}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"budget","minutes":60,"scope":{"kind":"weekdays","days":["mon","mon"]}}]}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"budget","minutes":60,"scope":{"kind":"weekdays","days":["monday"]}}]}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"budget","minutes":60,"scope":{"kind":"this_week","week_of":"2026-11-03"}}]}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"window","until":"7pm","scope":{"kind":"every_day"}}]}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"window","from":"19:00","until":"18:00","scope":{"kind":"every_day"}}]}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"window","until":"24:00","scope":{"kind":"every_day"}}]}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"nap","scope":{"kind":"every_day"}}]}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"budget","minutes":60,"scope":{"kind":"every_day","x":1}}]}}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true}],"power":{"enabled":"no"}}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true}],"power":{"x":1}}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true}],"power":[]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P","command":""}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P","command":"bin/x"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P","command":"./x"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P","command":"-x"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P","command":"dosbox -fullscreen"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P","command":"/a/../x"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P","command":"x\ny"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P","command":"x","args":"-a"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P","command":"x","args":["a\nb"]}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P","command":"x","working_dir":"games"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P","command":"x","icon":"dragon"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P","command":"x","shell":true}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"programs":[{"id":"p","name":"P","command":"x"},{"id":"p","name":"Q","command":"y"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"c64"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":{"drives":[]}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":{"drives":[{"letter":"Z","path":"/g"}]}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":{"drives":[{"letter":"c","path":"/g"}]}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":{"drives":[{"letter":"CC","path":"/g"}]}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":{"drives":[{"letter":"C","path":"g"}]}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":{"drives":[{"letter":"C","path":"/a\"b"}]}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":{"drives":[{"letter":"C","path":"/g"},{"letter":"C","path":"/g"}]}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":{"drives":[{"letter":"C","path":"/g"}],"autostart":"..\\\\x.exe"}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":{"drives":[{"letter":"C","path":"/g"}],"autostart":"a\"b"}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":{"drives":[{"letter":"C","path":"/g"}],"startup_script":"[sdl]"}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":{"drives":[{"letter":"C","path":"/g"}],"startup_script":"a\tb"}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":{"drives":[{"letter":"C","path":"/g"}],"x":1}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":[]}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"snes"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"snes","snes":{"rom":"/a.exe"}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"snes","snes":{"rom":"a.sfc"}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"snes","snes":{"rom":"/a.sfc","filter":"blur"}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"snes","snes":{"rom":"/a.sfc","aspect":"21_9"}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"snes","snes":{"rom":"/a.sfc","integer_scale":"yes"}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"snes","snes":[]}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"dos","dos":{"drives":[{"letter":"C","path":"/g"}],"keep_saves":"yes"}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"emulators":[{"id":"e","name":"E","system":"snes","snes":{"rom":"/a.sfc","resume":1}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"folders":[{"id":"f","name":"F","path":"/m","video":true,"remember_recent":"no"}]}]}"#,
            r#"{"version":1,"screensaver":{"mode":"matrix"}}"#,
            r#"{"version":1,"screensaver":{"mode":"Title"}}"#,
            r#"{"version":1,"screensaver":{"idle_minutes":0}}"#,
            r#"{"version":1,"screensaver":{"idle_minutes":241}}"#,
            r#"{"version":1,"screensaver":{"idle_minutes":1.5}}"#,
            r#"{"version":1,"screensaver":{"aerial_quality":"8k"}}"#,
            r#"{"version":1,"screensaver":{"aerial_videos":["../x"]}}"#,
            r#"{"version":1,"screensaver":{"aerial_videos":["a.b"]}}"#,
            r#"{"version":1,"screensaver":{"aerial_videos":[""]}}"#,
            r#"{"version":1,"screensaver":{"aerial_videos":["a","a"]}}"#,
            r#"{"version":1,"screensaver":{"aerial_videos":["1","2","3","4","5","6","7","8","9","10","11"]}}"#,
            r#"{"version":1,"screensaver":{"aerial_videos":[1]}}"#,
            r#"{"version":1,"screensaver":{"urls":[]}}"#,
            r#"{"version":1,"games":{"pong":{"enabled":"yes"}}}"#,
            r#"{"version":1,"games":{"pong":{"computer_level":"expert"}}}"#,
            r#"{"version":1,"games":{"pong":{"points_to_win":2}}}"#,
            r#"{"version":1,"games":{"pong":{"points_to_win":22}}}"#,
            r#"{"version":1,"games":{"pong":{"sound":true}}}"#,
            r#"{"version":1,"games":{"chess":{}}}"#,
            r#"{"version":1,"games":[]}"#,
            r#"{"version":1,"games":{"pong":[]}}"#,
            r#"{"version":1,"games":{"bricks":{"enabled":1}}}"#,
            r#"{"version":1,"games":{"bricks":{"lives":0}}}"#,
            r#"{"version":1,"games":{"bricks":{"lives":100}}}"#,
            r#"{"version":1,"games":{"bricks":{"sound":"on"}}}"#,
            r#"{"version":1,"games":{"bricks":{"speed":2}}}"#,
            r#"{"version":1,"games":{"bricks":[]}}"#,
            r#"{"version":1,"programs":[]}"#,
            r#"{"version":1,"programs":{"pong":{"points_to_win":2}}}"#,
            r#"{"version":1,"programs":{"chess":{}}}"#,
            r#"{"version":1,"home":[]}"#,
            r#"{"version":1,"discs":[true]}"#,
            r#"{"version":1,"parental":[]}"#,
            r#"{"version":1,"screensaver":["title"]}"#,
            r#"{"version":1,"folders":[["a","A","/x",true]]}"#,
            r#"{"version":1,"websites":[[]]}"#,
            r#"{"version":1,"parental":{"limits":[["budget",60]]}}"#,
            r#"{"version":1,"parental":{"limits":[{"type":"budget","minutes":60,"scope":["every_day"]}]}}"#,
            "[1]",
            "not json",
            // Version 2: home screens.
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"games":{"blocks":{"start_level":10}}}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"games":{"blocks":{"enabled":1}}}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"games":{"blocks":{"speed":1}}}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"games":{"blocks":[]}}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"games":{"dodger":[]}}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"games":{"dodger":{"enabled":1}}}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"games":{"dodger":{"sound":"on"}}}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"games":{"dodger":{"music":"on"}}}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"games":{"dodger":{"speed":1}}}]}"#,
            r#"{"version":2}"#,
            r#"{"version":2,"home_screens":[]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A"}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true},{"id":"b","name":"B","default":true}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true},{"id":"a","name":"B"}]}"#,
            r#"{"version":2,"home_screens":[{"id":"A","name":"A","default":true}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"","default":true}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A name that is much too long for it","default":true}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"pin":"1"}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"folders":[{"id":"f","name":"F","path":"/f","video":true}]},{"id":"b","name":"B","folders":[{"id":"f","name":"F","path":"/g","video":true}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"websites":[{"id":"w","name":"W","url":"https://a.de/"}]},{"id":"b","name":"B","websites":[{"id":"w","name":"W","url":"https://b.de/"}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"limits":[{"type":"budget","minutes":0,"scope":{"kind":"every_day"}}]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"discs":[]}]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true,"limits":[["budget"]]}]}"#,
            r#"{"version":2,"home_screens":[["a"]]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true}],"statistics":{"keep_days":0}}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true}],"statistics":{"keep_days":3651}}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true}],"statistics":{"enabled":"yes"}}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true}],"statistics":[]}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true}],"parental":{"limits":[]}}"#,
            r#"{"version":2,"home_screens":[{"id":"a","name":"A","default":true}],"folders":[]}"#,
            r#"{"version":3,"home_screens":[{"id":"a","name":"A","default":true}]}"#,
        ] {
            assert!(parse(bad).is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn startup_fails_closed_on_broken_file() {
        let path = temp_file("broken.json", "{ broken");
        let state = load_initial(Some(&path));
        assert_eq!(state.config, Config::locked_down());
        assert!(!state.config.home_screens[0].discs.enabled);
        assert!(state.error.is_some());
    }

    #[test]
    fn missing_file_means_defaults() {
        let path = std::env::temp_dir().join("curator-does-not-exist.json");
        let state = load_initial(Some(&path));
        assert_eq!(state.config, Config::default());
        assert_eq!(state.error, None);
    }

    #[test]
    fn reload_keeps_last_valid_config() {
        let path = temp_file("reload.json", r#"{"version":1,"discs":{"enabled":false}}"#);
        let mut state = load_initial(Some(&path));
        assert!(!state.config.home_screens[0].discs.enabled);

        std::fs::write(&path, r#"{"version":1,"discs":{"audio_languages":["x"]}}"#).unwrap();
        assert!(reload(&mut state, &path).is_err());
        assert!(
            !state.config.home_screens[0].discs.enabled,
            "old config must stay active"
        );
        assert!(state.error.is_some());

        std::fs::write(&path, r#"{"version":1}"#).unwrap();
        assert!(reload(&mut state, &path).is_ok());
        assert!(state.config.home_screens[0].discs.enabled);
        assert_eq!(state.error, None);
    }
}
