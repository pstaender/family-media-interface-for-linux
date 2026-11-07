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
    pub discs: DiscSettings,
    #[serde(default)]
    pub folders: Vec<FolderSettings>,
    #[serde(default)]
    pub websites: Vec<WebsiteSettings>,
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
            discs: DiscSettings::default(),
            folders: Vec::new(),
            websites: Vec::new(),
        }
    }
}

impl Config {
    /// Used when the config exists but can't be read: everything that could be restricted is off.
    pub fn locked_down() -> Self {
        Self {
            version: VERSION,
            discs: DiscSettings {
                enabled: false,
                allow_disc_menu: false,
                ..DiscSettings::default()
            },
            folders: Vec::new(),
            websites: Vec::new(),
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
    let name_chars = name.chars().count();
    if name_chars == 0 || name_chars > MAX_NAME_CHARS {
        return Err(format!("name must have 1 to {MAX_NAME_CHARS} characters"));
    }
    if name.trim() != name || name.chars().any(char::is_control) {
        return Err("name must not start/end with spaces or contain control characters".into());
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
