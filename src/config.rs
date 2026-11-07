//! The media center config (`$XDG_CONFIG_HOME/familynovice/config.json`).
//!
//! Written by the admin interface (`admin/src/config.ts`); the format is documented in
//! `docs/config.schema.json`. Rules:
//! - missing file: defaults (fresh installation),
//! - invalid file at startup: fail closed (locked-down settings) and report the error,
//! - invalid file on reload: keep the last valid config and report the error.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
const MAX_LANGUAGES: usize = 10;
const MAX_FOLDERS: usize = 50;
const MAX_NAME_CHARS: usize = 60;
const MAX_PATH_BYTES: usize = 4096;
const MAX_WEBSITES: usize = 50;
const MAX_URL_BYTES: usize = 2048;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
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
    pub parental: ParentalSettings,
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
    pub limits: Vec<LimitRule>,
}

impl Default for ParentalSettings {
    fn default() -> Self {
        Self {
            pin_hash: None,
            pin_length: None,
            pin_backoff_doubling: true,
            limits: Vec::new(),
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
}

impl Default for HomeSettings {
    fn default() -> Self {
        Self {
            title: "familynovice".into(),
            show_title: true,
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
}

fn yes() -> bool {
    true
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
            home: HomeSettings::default(),
            discs: DiscSettings::default(),
            folders: Vec::new(),
            websites: Vec::new(),
            parental: ParentalSettings::default(),
        }
    }
}

impl Config {
    /// Used when the config exists but can't be read: everything that could be restricted is off.
    pub fn locked_down() -> Self {
        Self {
            version: VERSION,
            home: HomeSettings::default(),
            discs: DiscSettings {
                enabled: false,
                allow_disc_menu: false,
                ..DiscSettings::default()
            },
            folders: Vec::new(),
            websites: Vec::new(),
            parental: ParentalSettings::default(),
        }
    }
}

/// The current config plus the last error (shown to parents and reported over the socket).
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigState {
    pub config: Config,
    pub error: Option<String>,
}

/// `$FAMILYNOVICE_CONFIG_DIR`, or `$XDG_CONFIG_HOME/familynovice`.
pub fn config_dir() -> Option<PathBuf> {
    match std::env::var_os("FAMILYNOVICE_CONFIG_DIR") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => Some(dirs::config_dir()?.join("familynovice")),
    }
}

pub fn config_path() -> Option<PathBuf> {
    Some(config_dir()?.join("config.json"))
}

/// Parse and validate config JSON.
pub fn parse(text: &str) -> Result<Config, String> {
    let config: Config = serde_json::from_str(text).map_err(|e| format!("invalid config: {e}"))?;
    validate(&config)?;
    Ok(config)
}

fn validate(config: &Config) -> Result<(), String> {
    if config.version != VERSION {
        return Err(format!(
            "unsupported config version {} (expected {VERSION})",
            config.version
        ));
    }
    check_text(&config.home.title).map_err(|e| format!("home.title: {e}"))?;
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
    if config.parental.limits.len() > MAX_LIMITS {
        return Err(format!("parental.limits: at most {MAX_LIMITS} rules"));
    }
    for (i, rule) in config.parental.limits.iter().enumerate() {
        validate_limit(rule).map_err(|e| format!("parental.limits[{i}]: {e}"))?;
    }
    for (field, list) in [
        ("discs.audio_languages", &config.discs.audio_languages),
        ("discs.subtitle_languages", &config.discs.subtitle_languages),
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
    if config.folders.len() > MAX_FOLDERS {
        return Err(format!("folders: at most {MAX_FOLDERS} folders"));
    }
    if config.websites.len() > MAX_WEBSITES {
        return Err(format!("websites: at most {MAX_WEBSITES} websites"));
    }
    for (i, site) in config.websites.iter().enumerate() {
        validate_website(site).map_err(|e| format!("websites[{i}]: {e}"))?;
        if config.websites[..i].iter().any(|w| w.id == site.id) {
            return Err(format!("websites[{i}]: id \"{}\" is used twice", site.id));
        }
    }
    for (i, folder) in config.folders.iter().enumerate() {
        validate_folder(folder).map_err(|e| format!("folders[{i}]: {e}"))?;
        if config.folders[..i].iter().any(|f| f.id == folder.id) {
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
        let dir = std::env::temp_dir().join(format!("familynovice-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn example_config_is_valid() {
        let config = parse(include_str!("../docs/config.example.json")).unwrap();
        assert_eq!(config.discs.audio_languages, ["deu", "eng"]);
    }

    #[test]
    fn example_folders() {
        let config = parse(include_str!("../docs/config.example.json")).unwrap();
        assert_eq!(config.folders.len(), 3);
        assert!(config.folders[2].audio);
        assert!(!config.folders[2].video);
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
        assert_eq!(config.websites.len(), 2);
        assert!(config.websites[0].allow_subdomains);
        assert!(!config.websites[1].allow_subdomains);
        // Missing toggle: subdomains allowed.
        let c =
            parse(r#"{"version":1,"websites":[{"id":"k","name":"K","url":"https://kika.de/"}]}"#)
                .unwrap();
        assert!(c.websites[0].allow_subdomains);
    }

    #[test]
    fn home_title() {
        let config = parse(include_str!("../docs/config.example.json")).unwrap();
        assert_eq!(config.home.title, "Family Cinema");
        let c = parse(r#"{"version":1,"home":{"show_title":false}}"#).unwrap();
        assert_eq!(c.home.title, "familynovice");
        assert!(!c.home.show_title);
        assert!(parse(r#"{"version":1}"#).unwrap().home.show_title);
    }

    #[test]
    fn example_parental() {
        let config = parse(include_str!("../docs/config.example.json")).unwrap();
        assert!(config.parental.pin_backoff_doubling);
        assert_eq!(config.parental.limits.len(), 4);
        assert_eq!(
            config.parental.limits[1],
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
        assert!(!config.discs.enabled);
        assert!(config.discs.allow_disc_menu);
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
            r#"{"version":1,"websites":[{"id":"a","name":"A","url":"https://kika.de/","extra":1}]}"#,
            r#"{"version":1,"websites":[{"id":"a","name":"A","url":"https://a.de/"},{"id":"a","name":"B","url":"https://b.de/"}]}"#,
            r#"{"version":1,"home":{"title":""}}"#,
            r#"{"version":1,"home":{"title":" Kino"}}"#,
            r#"{"version":1,"home":{"title":"a\u0007b"}}"#,
            r#"{"version":1,"home":{"show_title":"no"}}"#,
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
            "not json",
        ] {
            assert!(parse(bad).is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn startup_fails_closed_on_broken_file() {
        let path = temp_file("broken.json", "{ broken");
        let state = load_initial(Some(&path));
        assert_eq!(state.config, Config::locked_down());
        assert!(!state.config.discs.enabled);
        assert!(state.error.is_some());
    }

    #[test]
    fn missing_file_means_defaults() {
        let path = std::env::temp_dir().join("familynovice-does-not-exist.json");
        let state = load_initial(Some(&path));
        assert_eq!(state.config, Config::default());
        assert_eq!(state.error, None);
    }

    #[test]
    fn reload_keeps_last_valid_config() {
        let path = temp_file("reload.json", r#"{"version":1,"discs":{"enabled":false}}"#);
        let mut state = load_initial(Some(&path));
        assert!(!state.config.discs.enabled);

        std::fs::write(&path, r#"{"version":1,"discs":{"audio_languages":["x"]}}"#).unwrap();
        assert!(reload(&mut state, &path).is_err());
        assert!(!state.config.discs.enabled, "old config must stay active");
        assert!(state.error.is_some());

        std::fs::write(&path, r#"{"version":1}"#).unwrap();
        assert!(reload(&mut state, &path).is_ok());
        assert!(state.config.discs.enabled);
        assert_eq!(state.error, None);
    }
}
