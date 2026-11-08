//! Small JSON files in the XDG directories: disc info cache and resume positions.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

const APP: &str = "familynovice";

/// `$XDG_CACHE_HOME/familynovice/discs/<key>.json`: MakeMKV scan results.
pub fn disc_cache_path(key: &str) -> Option<PathBuf> {
    Some(
        dirs::cache_dir()?
            .join(APP)
            .join("discs")
            .join(file_name(key)),
    )
}

/// `$XDG_DATA_HOME/familynovice/resume/<key>.json`: where a movie was stopped.
pub fn resume_path(key: &str) -> Option<PathBuf> {
    Some(
        dirs::data_dir()?
            .join(APP)
            .join("resume")
            .join(file_name(key)),
    )
}

/// `$XDG_DATA_HOME/familynovice/usage.json`: usage time, unlocks, PIN tries.
pub fn usage_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join(APP).join("usage.json"))
}

/// `$XDG_DATA_HOME/familynovice/recent.json`: recently played files per folder.
pub fn recent_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join(APP).join("recent.json"))
}

/// `$XDG_DATA_HOME/familynovice/remote.json`: the remote's buttons (calibration).
pub fn remote_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join(APP).join("remote.json"))
}

/// `$XDG_CACHE_HOME/familynovice/player`: where players may write (screenshots, …),
/// so they never write next to the media.
pub fn player_cache_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join(APP).join("player"))
}

/// Create the player cache dir (and its `screenshots` folder) and return it.
/// Also writes mpv's key bindings (`player::MPV_INPUT_CONF`) there.
pub fn prepare_player_cache() -> Option<PathBuf> {
    let dir = player_cache_dir()?;
    if let Err(e) = std::fs::create_dir_all(dir.join("screenshots")) {
        log::warn!("could not create {}: {e}", dir.display());
    }
    for (name, content) in [
        (
            crate::player::MPV_INPUT_CONF_NAME,
            crate::player::MPV_INPUT_CONF,
        ),
        (
            crate::player::MPV_HOME_SCRIPT_NAME,
            crate::player::MPV_HOME_SCRIPT,
        ),
    ] {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(content)
            && let Err(e) = std::fs::write(&path, content)
        {
            log::warn!("could not write {}: {e}", path.display());
        }
    }
    Some(dir)
}

/// `$XDG_CACHE_HOME/familynovice/web`: cookies, local storage and cache of the website window.
pub fn web_data_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join(APP).join("web"))
}

/// `$XDG_CACHE_HOME/familynovice/logos/<website id>.img`: the website's logo, fetched by the
/// admin interface (`admin/src/logos.ts`); the media center only reads it.
pub fn logo_path(website_id: &str) -> Option<PathBuf> {
    Some(
        dirs::cache_dir()?
            .join(APP)
            .join("logos")
            .join(format!("{website_id}.img")),
    )
}

/// Socket for talking to the running mpv (`$XDG_RUNTIME_DIR`, falls back to the temp dir).
pub fn mpv_socket_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("{APP}-mpv.sock"))
}

/// Socket of the background music player.
pub fn audio_socket_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("{APP}-audio.sock"))
}

/// The media center's control socket (see `control.rs`), used by the admin interface.
pub fn control_socket_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("{APP}.sock"))
}

/// Turn an arbitrary key into a safe file name.
fn file_name(key: &str) -> String {
    let safe: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}.json")
}

/// Read a JSON file. Missing or broken files count as "nothing stored".
pub fn load<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("ignoring unreadable {}: {e}", path.display());
            None
        }
    }
}

/// Write a JSON file atomically (temp file + rename), creating directories as needed.
pub fn save<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("path has no parent directory"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(serde_json::to_string_pretty(value)?.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)
}

pub fn remove(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("could not remove {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe() {
        assert_eq!(
            file_name("8d3f12ee17357a03_KUBO_TWO_STRINGS"),
            "8d3f12ee17357a03_KUBO_TWO_STRINGS.json"
        );
        assert_eq!(file_name("../etc/passwd"), "___etc_passwd.json");
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("familynovice-test-{}", std::process::id()));
        let path = dir.join("sub").join("x.json");
        save(&path, &vec![1, 2, 3]).unwrap();
        assert_eq!(load::<Vec<i32>>(&path), Some(vec![1, 2, 3]));
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load::<Vec<i32>>(&path), None);
        remove(&path);
        assert_eq!(load::<Vec<i32>>(&path), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
