//! Local control socket (`$XDG_RUNTIME_DIR/familynovice.sock`), used by the admin interface.
//!
//! Protocol: one JSON object per line in each direction.
//! - `{"command":"reload"}` → `{"ok":true}` or `{"ok":false,"error":"…"}`
//! - `{"command":"status"}` → `{"ok":true,"config_version":1,"config_error":null,
//!   "locked":false,"lock_reason":null,"remaining_secs":1200,"used_today_secs":2400,
//!   "unlocked_until":null}`
//! - `{"command":"unlock","minutes":30}` / `{"command":"unlock","rest_of_day":true}`:
//!   lifts the limits (and a parent's lock) for that time
//! - `{"command":"lock"}`: locks the media center until it is unlocked
//! - `{"command":"restart"}` → not supported yet (`ok: false`)

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::Local;
use serde_json::{Value, json};

use crate::config::{self, ConfigState};
use crate::limits::{self, Status};
use crate::usage::SharedTracker;

pub type SharedConfig = Arc<Mutex<ConfigState>>;

/// Everything the control socket works on.
#[derive(Clone)]
pub struct Control {
    pub config: SharedConfig,
    pub config_path: PathBuf,
    pub tracker: SharedTracker,
}

/// Handle one request line and return the reply.
pub fn handle(line: &str, control: &Control) -> Value {
    let request = serde_json::from_str::<Value>(line).unwrap_or(Value::Null);
    let command = request.get("command").and_then(Value::as_str);
    // A poisoned lock means a panic elsewhere; the state itself is still usable.
    match command {
        Some("reload") => {
            let mut state = control.config.lock().unwrap_or_else(|p| p.into_inner());
            match config::reload(&mut state, &control.config_path) {
                Ok(()) => json!({"ok": true}),
                Err(e) => json!({"ok": false, "error": e}),
            }
        }
        Some("status") => {
            let state = control.config.lock().unwrap_or_else(|p| p.into_inner());
            let tracker = control.tracker.lock().unwrap_or_else(|p| p.into_inner());
            let (locked, reason, remaining) = match &tracker.status {
                Status::Locked(r) => (true, Some(limits::reason_text(r)), None),
                Status::Allowed { remaining_secs } => (false, None, *remaining_secs),
            };
            json!({
                "ok": true,
                "config_version": state.config.version,
                "config_error": state.error,
                "locked": locked,
                "lock_reason": reason,
                "remaining_secs": remaining,
                "used_today_secs": tracker.used_on(Local::now().date_naive()),
                "unlocked_until": tracker.stored.unlocked_until,
            })
        }
        Some("unlock") => {
            let minutes = request.get("minutes").and_then(Value::as_u64);
            let rest_of_day = request.get("rest_of_day").and_then(Value::as_bool) == Some(true);
            let minutes = match (minutes, rest_of_day) {
                (Some(m @ 1..=1440), false) => Some(m as u32),
                (None, true) => None,
                _ => {
                    return json!({"ok": false, "error": "unlock needs minutes (1-1440) or rest_of_day"});
                }
            };
            let mut tracker = control.tracker.lock().unwrap_or_else(|p| p.into_inner());
            tracker.unlock(Local::now(), minutes);
            log::info!("unlocked from the admin panel ({minutes:?} minutes)");
            json!({"ok": true})
        }
        Some("lock") => {
            control
                .tracker
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .lock();
            log::info!("locked from the admin panel");
            json!({"ok": true})
        }
        Some("restart") => json!({"ok": false, "error": "restart is not supported yet"}),
        Some(other) => json!({"ok": false, "error": format!("unknown command: {other}")}),
        None => json!({"ok": false, "error": "expected {\"command\": \"…\"}"}),
    }
}

/// Start the socket server in a background thread. `notify` is called after every
/// request (e.g. to repaint the UI with a reloaded config or a new lock state).
pub fn spawn_server(socket: PathBuf, control: Control, notify: impl Fn() + Send + Sync + 'static) {
    if UnixStream::connect(&socket).is_ok() {
        log::error!(
            "{} is in use: another familynovice instance is running; control socket disabled",
            socket.display()
        );
        return;
    }
    let _ = std::fs::remove_file(&socket); // stale socket from a crashed run
    let listener = match UnixListener::bind(&socket) {
        Ok(l) => l,
        Err(e) => {
            log::error!("could not open control socket {}: {e}", socket.display());
            return;
        }
    };
    // Only our own user may talk to the media center.
    if let Err(e) = std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)) {
        log::warn!("could not restrict control socket permissions: {e}");
    }
    log::info!("control socket: {}", socket.display());
    let notify = Arc::new(notify);
    std::thread::Builder::new()
        .name("control-socket".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let (control, notify) = (control.clone(), notify.clone());
                std::thread::spawn(move || {
                    if let Err(e) = serve(stream, &control, &*notify) {
                        log::debug!("control connection ended: {e}");
                    }
                });
            }
        })
        .expect("spawning a thread only fails if the OS is out of resources");
}

fn serve(stream: UnixStream, control: &Control, notify: &dyn Fn()) -> std::io::Result<()> {
    let mut writer = stream.try_clone()?;
    for line in BufReader::new(stream).lines() {
        let reply = handle(&line?, control);
        writeln!(writer, "{reply}")?;
        notify();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::usage::Tracker;

    fn setup(content: &str) -> Control {
        let dir = std::env::temp_dir().join(format!(
            "familynovice-control-{}-{}",
            std::process::id(),
            content.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, content).unwrap();
        let state = config::load_initial(Some(&path));
        Control {
            config: Arc::new(Mutex::new(state)),
            config_path: path,
            tracker: Tracker::in_memory().shared(),
        }
    }

    #[test]
    fn reload_and_status() {
        let control = setup(r#"{"version":1}"#);
        std::fs::write(
            &control.config_path,
            r#"{"version":1,"discs":{"enabled":false}}"#,
        )
        .unwrap();
        assert_eq!(
            handle(r#"{"command":"reload"}"#, &control),
            json!({"ok": true})
        );
        assert!(!control.config.lock().unwrap().config.discs.enabled);
        let status = handle(r#"{"command":"status"}"#, &control);
        assert_eq!(status["ok"], json!(true));
        assert_eq!(status["config_error"], Value::Null);
        assert_eq!(status["locked"], json!(false));
    }

    #[test]
    fn lock_and_unlock() {
        let control = setup(r#"{"version":1}"#);
        assert_eq!(
            handle(r#"{"command":"lock"}"#, &control),
            json!({"ok": true})
        );
        assert!(control.tracker.lock().unwrap().stored.locked_by_parent);
        assert_eq!(
            handle(r#"{"command":"unlock","minutes":30}"#, &control),
            json!({"ok": true})
        );
        let tracker = control.tracker.lock().unwrap();
        assert!(!tracker.stored.locked_by_parent);
        assert!(tracker.stored.unlocked_until.is_some());
        drop(tracker);
        assert_eq!(
            handle(r#"{"command":"unlock","rest_of_day":true}"#, &control),
            json!({"ok": true})
        );
        for bad in [
            r#"{"command":"unlock"}"#,
            r#"{"command":"unlock","minutes":0}"#,
            r#"{"command":"unlock","minutes":5000}"#,
            r#"{"command":"unlock","minutes":30,"rest_of_day":true}"#,
        ] {
            assert_eq!(handle(bad, &control)["ok"], json!(false), "{bad}");
        }
    }

    #[test]
    fn invalid_reload_reports_error_and_keeps_config() {
        let control = setup(r#"{"version":1,"discs":{"allow_disc_menu":false}}"#);
        std::fs::write(&control.config_path, "{").unwrap();
        let reply = handle(r#"{"command":"reload"}"#, &control);
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("invalid config"));
        let state = control.config.lock().unwrap();
        assert!(!state.config.discs.allow_disc_menu);
        assert_ne!(state.config, Config::locked_down());
    }

    #[test]
    fn rejects_garbage_and_unknown_commands() {
        let control = setup(r#"{"version":1}"#);
        assert_eq!(handle("hello", &control)["ok"], json!(false));
        assert_eq!(
            handle(r#"{"command":"rm -rf"}"#, &control)["ok"],
            json!(false)
        );
    }

    #[test]
    fn socket_roundtrip() {
        let control = setup(r#"{"version":1}"#);
        let socket = std::env::temp_dir().join(format!("fn-ctl-{}.sock", std::process::id()));
        spawn_server(socket.clone(), control, || {});
        let mut stream = UnixStream::connect(&socket).unwrap();
        stream.write_all(b"{\"command\":\"status\"}\n").unwrap();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&reply).unwrap()["ok"],
            json!(true)
        );
        let mode = std::fs::metadata(&socket).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_file(socket);
    }
}
