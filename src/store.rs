//! Small JSON files in the XDG directories: disc info cache and resume positions.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

const APP: &str = "curator";

/// `$XDG_CACHE_HOME/curator/discs/<key>.json`: MakeMKV scan results.
pub fn disc_cache_path(key: &str) -> Option<PathBuf> {
    Some(
        dirs::cache_dir()?
            .join(APP)
            .join("discs")
            .join(file_name(key)),
    )
}

/// `$XDG_DATA_HOME/curator/resume/<key>.json`: where a movie was stopped.
pub fn resume_path(key: &str) -> Option<PathBuf> {
    Some(
        dirs::data_dir()?
            .join(APP)
            .join("resume")
            .join(file_name(key)),
    )
}

/// `$XDG_DATA_HOME/curator/usage.json`: usage time, unlocks, PIN tries.
pub fn usage_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join(APP).join("usage.json"))
}

/// `$XDG_DATA_HOME/curator/recent.json`: recently played files per folder.
pub fn recent_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join(APP).join("recent.json"))
}

/// `$XDG_DATA_HOME/curator/remote.json`: the remote's buttons (calibration).
pub fn remote_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join(APP).join("remote.json"))
}

/// `$XDG_CACHE_HOME/curator/player`: where players may write (screenshots, …),
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
            crate::player::MPV_INPUT_CONF.to_string(),
        ),
        (
            crate::player::MPV_HOME_SCRIPT_NAME,
            crate::player::mpv_home_script(),
        ),
        (
            crate::player::MPV_SAVER_INPUT_CONF_NAME,
            crate::player::MPV_SAVER_INPUT_CONF.to_string(),
        ),
    ] {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(content.as_str())
            && let Err(e) = std::fs::write(&path, &content)
        {
            log::warn!("could not write {}: {e}", path.display());
        }
    }
    Some(dir)
}

/// `$XDG_CACHE_HOME/curator/web`: cookies, local storage and cache of the website window.
pub fn web_data_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join(APP).join("web"))
}

/// `$XDG_CACHE_HOME/curator/logos/<website id>.img`: the website's logo, fetched by the
/// admin interface (`admin/src/logos.ts`); the media center only reads it.
pub fn logo_path(website_id: &str) -> Option<PathBuf> {
    Some(
        dirs::cache_dir()?
            .join(APP)
            .join("logos")
            .join(format!("{website_id}.img")),
    )
}

/// `$XDG_CACHE_HOME/curator/aerial`: the screensaver's aerial videos, downloaded by the
/// admin interface (`admin/src/aerial.ts`); the media center only reads them.
pub fn aerial_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join(APP).join("aerial"))
}

/// `$XDG_DATA_HOME/curator/statistics`: usage statistics, one file per day (`stats.rs`).
pub fn statistics_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join(APP).join("statistics"))
}

/// `$XDG_DATA_HOME/curator/highscores.json`: the games' high score lists.
pub fn highscores_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join(APP).join("highscores.json"))
}

/// `$XDG_CACHE_HOME/curator/music`: the games' tunes as WAV files (made by `music.rs`).
pub fn music_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join(APP).join("music"))
}

/// `$XDG_CACHE_HOME/curator/sounds`: the games' sound files (made by `sound.rs`).
pub fn sounds_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join(APP).join("sounds"))
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

/// Held (flock) by the running media center, so a second one for the same user exits.
pub fn instance_lock_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("{APP}.lock"))
}

/// The project's former name: its folders are taken over once (`migrate_old_folders`).
const OLD_NAME: &str = "familynovice";

/// curator was called "familynovice": rename its folders in the XDG config, data and cache
/// dirs (only our own folders, only if the new one doesn't exist yet), and turn the old
/// default title into the new one. Done by whichever part starts first (admin or media
/// center; renaming is atomic). Not for a config dir set with `CURATOR_CONFIG_DIR`.
pub fn migrate_old_folders() {
    let override_config = std::env::var_os("CURATOR_CONFIG_DIR").is_some_and(|d| !d.is_empty());
    let bases = [
        (!override_config).then(dirs::config_dir).flatten(),
        dirs::data_dir(),
        dirs::cache_dir(),
    ];
    for (i, base) in bases.into_iter().enumerate() {
        let Some(base) = base else { continue };
        if migrate_folder(&base) && i == 0 {
            rename_old_title(&base.join(APP).join("config.json"));
        }
    }
}

/// `<base>/familynovice` → `<base>/curator`. True if it was renamed.
fn migrate_folder(base: &Path) -> bool {
    let (old, new) = (base.join(OLD_NAME), base.join(APP));
    if !old.is_dir() || new.exists() {
        return false;
    }
    match std::fs::rename(&old, &new) {
        Ok(()) => {
            log::info!("renamed {} to {}", old.display(), new.display());
            true
        }
        Err(e) => {
            log::warn!("could not rename {}: {e}", old.display());
            false
        }
    }
}

/// The old default title ("familynovice") becomes the new one.
fn rename_old_title(config: &Path) {
    let Ok(text) = std::fs::read_to_string(config) else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    let Some(title) = value.pointer_mut("/home/title") else {
        return;
    };
    if title != OLD_NAME {
        return;
    }
    *title = serde_json::Value::from(APP);
    if let Err(e) = save(config, &value) {
        log::warn!("could not update the title in {}: {e}", config.display());
    }
}

/// The kiosk loop's process name (`Name` in /proc is cut to 15 characters; this fits).
const KIOSK_LOOP: &str = "curator-kiosk";

/// Were we started by a `curator-kiosk` loop whose session (cage) is gone? Such a loop
/// survives the logout (its parent is then init or a `systemd` manager) and would start the
/// media center into the next session a second time. `proc` is `/proc`.
pub fn started_by_orphaned_kiosk(proc: &Path, pid: u32) -> bool {
    let parent = |pid: &str| -> Option<(String, String)> {
        let status = std::fs::read_to_string(proc.join(pid).join("status")).ok()?;
        let field = |name: &str| {
            status
                .lines()
                .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
                .map(|v| v.trim().to_string())
        };
        Some((field("Name")?, field("PPid")?))
    };
    let Some((_, loop_pid)) = parent(&pid.to_string()) else {
        return false;
    };
    let Some((loop_name, session_pid)) = parent(&loop_pid) else {
        return false;
    };
    if loop_name != KIOSK_LOOP {
        return false;
    }
    if session_pid == "1" {
        return true;
    }
    matches!(parent(&session_pid), Some((name, _)) if name == "systemd")
}

/// Were we started by the kiosk session's `curator-kiosk` loop (which restarts us)?
pub fn started_by_kiosk(proc: &Path, pid: u32) -> bool {
    let status = |pid: &str| std::fs::read_to_string(proc.join(pid).join("status")).ok();
    let field = |status: &str, name: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
            .map(|v| v.trim().to_string())
    };
    let Some(parent) = status(&pid.to_string()).and_then(|s| field(&s, "PPid")) else {
        return false;
    };
    status(&parent)
        .and_then(|s| field(&s, "Name"))
        .is_some_and(|n| n == KIOSK_LOOP)
}

/// Take the single-instance lock. `Ok(None)`: another media center of this user runs.
/// The lock lasts as long as the returned file stays open (also released by `exec`, as
/// files are opened close-on-exec, so a restart takes it again).
pub fn lock_instance(path: &Path) -> std::io::Result<Option<std::fs::File>> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(path)?;
    // SAFETY: flock(2) on a file descriptor we own.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(Some(file));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        Ok(None)
    } else {
        Err(error)
    }
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
    fn takes_over_the_old_folders_once() {
        let base = std::env::temp_dir().join(format!("curator-migrate-{}", std::process::id()));
        let old = base.join(OLD_NAME);
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(
            old.join("config.json"),
            r#"{"version":1,"home":{"title":"familynovice","show_title":true}}"#,
        )
        .unwrap();
        assert!(migrate_folder(&base));
        rename_old_title(&base.join(APP).join("config.json"));
        assert!(!old.exists());
        let text = std::fs::read_to_string(base.join(APP).join("config.json")).unwrap();
        assert!(text.contains(r#""title": "curator""#), "{text}");
        // Both exist: nothing is touched (the new one wins).
        std::fs::create_dir_all(&old).unwrap();
        assert!(!migrate_folder(&base));
        assert!(old.exists());
        // A title the parents chose stays.
        let custom = base.join("custom.json");
        std::fs::write(&custom, r#"{"version":1,"home":{"title":"Kino"}}"#).unwrap();
        rename_old_title(&custom);
        assert!(std::fs::read_to_string(&custom).unwrap().contains("Kino"));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn detects_an_orphaned_kiosk_loop() {
        let proc = std::env::temp_dir().join(format!("curator-proc-{}", std::process::id()));
        let add = |pid: u32, name: &str, ppid: u32| {
            let dir = proc.join(pid.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("status"),
                format!("Name:\t{name}\nUmask:\t0002\nState:\tS\nPPid:\t{ppid}\n"),
            )
            .unwrap();
        };
        add(1669, "systemd", 1);
        // Live session: cage → kiosk loop → media center.
        add(21611, "cage", 21562);
        add(21712, "curator-kiosk", 21611);
        add(21714, "curator", 21712);
        // Orphaned loop (its cage is gone): under the user's systemd, or init.
        add(20196, "curator-kiosk", 1669);
        add(21785, "curator", 20196);
        add(30000, "curator-kiosk", 1);
        add(30001, "curator", 30000);
        // Development: started from a shell.
        add(500, "zsh", 400);
        add(501, "curator", 500);
        assert!(!started_by_orphaned_kiosk(&proc, 21714));
        assert!(started_by_orphaned_kiosk(&proc, 21785));
        assert!(started_by_orphaned_kiosk(&proc, 30001));
        assert!(!started_by_orphaned_kiosk(&proc, 501));
        assert!(!started_by_orphaned_kiosk(&proc, 99999));
        assert!(started_by_kiosk(&proc, 21714));
        assert!(started_by_kiosk(&proc, 21785));
        assert!(!started_by_kiosk(&proc, 501));
        std::fs::remove_dir_all(&proc).unwrap();
    }

    #[test]
    fn only_one_instance_gets_the_lock() {
        let path = std::env::temp_dir().join(format!("curator-lock-{}", std::process::id()));
        let first = lock_instance(&path).unwrap();
        assert!(first.is_some());
        assert!(lock_instance(&path).unwrap().is_none());
        drop(first);
        assert!(lock_instance(&path).unwrap().is_some());
        let _ = std::fs::remove_file(&path);
    }

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
        let dir = std::env::temp_dir().join(format!("curator-test-{}", std::process::id()));
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
