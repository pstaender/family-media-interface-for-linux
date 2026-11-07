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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub discs: DiscSettings,
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
    Ok(())
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
