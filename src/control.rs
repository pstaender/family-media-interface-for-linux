//! Local control socket (`$XDG_RUNTIME_DIR/curator.sock`), used by the admin interface.
//!
//! Protocol: one JSON object per line in each direction.
//! - `{"command":"reload"}` → `{"ok":true}` or `{"ok":false,"error":"…"}`
//! - `{"command":"status","home_screen":"kids"}` → `{"ok":true,"config_version":2,
//!   "config_error":null,"home_screen":"kids","active_home_screen":"family","locked":false,
//!   "lock_reason":null,"remaining_secs":1200,"used_today_secs":2400,"used_week_secs":5400,
//!   "unlocked_until":null,…}` (without `home_screen`: the one shown on the TV)
//! - `{"command":"unlock","minutes":30}` / `{"command":"unlock","rest_of_day":true}`:
//!   lifts the limits (and a parent's lock) for that time
//! - `{"command":"end_unlock"}`: ends an unlock early; the time limits apply again
//! - `{"command":"lock"}`: locks the media center until it is unlocked
//! - unlock / end_unlock / lock act on `"home_screen"` if given, else the one shown
//! - `{"command":"switch_home_screen","home_screen":"kids"}`: show that home screen
//! - `{"command":"calibrate_remote"}`: shows the remote calibration on the TV
//! - `{"command":"restart"}`: stops what plays and starts the media center again (the newest
//!   installed version, e.g. after `scripts/install.sh --update`)

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::Local;
use serde_json::{Value, json};

use crate::config::{self, ConfigState};
use crate::limits::{self, Status};
use crate::remote::{self, RemoteMap, SharedDevices};
use crate::usage::SharedTracker;

pub type SharedConfig = Arc<Mutex<ConfigState>>;

/// Things the admin interface asks the UI to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRequest {
    CalibrateRemote,
    Restart,
}

/// Everything the control socket works on.
#[derive(Clone)]
pub struct Control {
    pub config: SharedConfig,
    pub config_path: PathBuf,
    pub tracker: SharedTracker,
    pub remote_devices: SharedDevices,
    pub ui_requests: std::sync::mpsc::Sender<UiRequest>,
}

/// The request's `home_screen`, if given (it must exist).
fn screen_of(request: &Value, config: &config::Config) -> Result<Option<String>, Value> {
    match request.get("home_screen") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) if config.home_screens.iter().any(|s| &s.id == id) => {
            Ok(Some(id.clone()))
        }
        Some(_) => Err(json!({"ok": false, "error": "unknown home screen"})),
    }
}

/// The home screen a command acts on (given, else the one shown), and how many days of
/// statistics to keep (`None`: statistics are off).
fn target(request: &Value, control: &Control) -> Result<(String, Option<u32>), Value> {
    let state = control.config.lock().unwrap_or_else(|p| p.into_inner());
    let given = screen_of(request, &state.config)?;
    let tracker = control.tracker.lock().unwrap_or_else(|p| p.into_inner());
    let id = given.unwrap_or_else(|| state.config.screen(tracker.active()).id.clone());
    let stats = &state.config.statistics;
    Ok((id, stats.enabled.then_some(stats.keep_days)))
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
            let active = state.config.screen(tracker.active()).id.clone();
            let screen = match screen_of(&request, &state.config) {
                Ok(s) => s,
                Err(e) => return e,
            };
            let id = screen.as_deref().unwrap_or(&active);
            // The one on the TV: as the UI last evaluated it; others: evaluated now.
            let status = if id == active {
                tracker.status.clone()
            } else {
                let now = Local::now().naive_local();
                let limits = &state.config.screen(Some(id)).limits;
                limits::evaluate(limits, &tracker.situation(id), now)
            };
            let (locked, reason, remaining) = match &status {
                Status::Locked(r) => (true, Some(limits::reason_text(r)), None),
                Status::Allowed { remaining_secs } => (false, None, *remaining_secs),
            };
            json!({
                "ok": true,
                "config_version": state.config.version,
                "config_error": state.error,
                "home_screen": id,
                "active_home_screen": active,
                "active_locked": matches!(tracker.status, Status::Locked(_)),
                "locked": locked,
                "lock_reason": reason,
                "remaining_secs": remaining,
                "used_today_secs": tracker.used_on(id, Local::now().date_naive()),
                "used_week_secs": tracker.used_this_week(id, Local::now().date_naive()),
                "unlocked_until": tracker.unlocked_until(id),
                "remote": remote_status(&control.remote_devices),
                // So the admin panel can tell this one from other (stray) media centers.
                "pid": std::process::id(),
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
            let (id, keep_days) = match target(&request, control) {
                Ok(t) => t,
                Err(e) => return e,
            };
            let mut tracker = control.tracker.lock().unwrap_or_else(|p| p.into_inner());
            tracker.unlock(&id, Local::now(), minutes);
            drop(tracker);
            if let Some(days) = keep_days {
                crate::stats::prune(days);
            }
            log::info!("{id} unlocked from the admin panel ({minutes:?} minutes)");
            json!({"ok": true})
        }
        Some("end_unlock") => {
            let (id, _) = match target(&request, control) {
                Ok(t) => t,
                Err(e) => return e,
            };
            control
                .tracker
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .end_unlock(&id);
            log::info!("{id}: unlock ended from the admin panel");
            json!({"ok": true})
        }
        Some("lock") => {
            let (id, _) = match target(&request, control) {
                Ok(t) => t,
                Err(e) => return e,
            };
            control
                .tracker
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .lock(&id);
            log::info!("{id} locked from the admin panel");
            json!({"ok": true})
        }
        Some("switch_home_screen") => {
            let state = control.config.lock().unwrap_or_else(|p| p.into_inner());
            let id = match screen_of(&request, &state.config) {
                Ok(Some(id)) => id,
                Ok(None) => return json!({"ok": false, "error": "home_screen is missing"}),
                Err(e) => return e,
            };
            drop(state);
            control
                .tracker
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .set_active(&id);
            log::info!("home screen switched to {id} from the admin panel");
            json!({"ok": true})
        }
        Some("calibrate_remote") => match control.ui_requests.send(UiRequest::CalibrateRemote) {
            Ok(()) => json!({"ok": true}),
            Err(_) => json!({"ok": false, "error": "the media center's screen isn't running"}),
        },
        Some("restart") => match control.ui_requests.send(UiRequest::Restart) {
            Ok(()) => json!({"ok": true}),
            Err(_) => json!({"ok": false, "error": "the media center's screen isn't running"}),
        },
        Some(other) => json!({"ok": false, "error": format!("unknown command: {other}")}),
        None => json!({"ok": false, "error": "expected {\"command\": \"…\"}"}),
    }
}

/// Readable remote devices and the learned buttons, for the admin panel.
fn remote_status(devices: &SharedDevices) -> Value {
    let devices: Vec<String> = devices
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .cloned()
        .collect();
    let map = RemoteMap::load();
    let buttons: serde_json::Map<String, Value> = remote::Action::ALL
        .iter()
        .map(|a| (a.label().to_string(), json!(map.describe(*a))))
        .collect();
    json!({
        "devices": devices,
        "calibrated": !map.buttons.is_empty(),
        "buttons": buttons,
    })
}

/// Start the socket server in a background thread. `notify` is called after every
/// request (e.g. to repaint the UI with a reloaded config or a new lock state).
pub fn spawn_server(socket: PathBuf, control: Control, notify: impl Fn() + Send + Sync + 'static) {
    if UnixStream::connect(&socket).is_ok() {
        log::error!(
            "{} is in use: another curator instance is running; control socket disabled",
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
        // One directory per test: tests run in parallel and would overwrite each other's file.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("curator-control-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, content).unwrap();
        let state = config::load_initial(Some(&path));
        Control {
            config: Arc::new(Mutex::new(state)),
            config_path: path,
            tracker: Tracker::in_memory().shared(),
            remote_devices: Arc::default(),
            // The receiver is dropped: calibrate_remote reports that no screen runs.
            ui_requests: std::sync::mpsc::channel().0,
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
        assert!(
            !control.config.lock().unwrap().config.home_screens[0]
                .discs
                .enabled
        );
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
        assert!(control.tracker.lock().unwrap().locked_by_parent("home"));
        assert_eq!(
            handle(r#"{"command":"unlock","minutes":30}"#, &control),
            json!({"ok": true})
        );
        let tracker = control.tracker.lock().unwrap();
        assert!(!tracker.locked_by_parent("home"));
        assert!(tracker.unlocked_until("home").is_some());
        drop(tracker);
        assert_eq!(
            handle(r#"{"command":"unlock","rest_of_day":true}"#, &control),
            json!({"ok": true})
        );
        assert_eq!(
            handle(r#"{"command":"end_unlock"}"#, &control),
            json!({"ok": true})
        );
        let tracker = control.tracker.lock().unwrap();
        assert_eq!(tracker.unlocked_until("home"), None);
        assert!(!tracker.locked_by_parent("home"));
        drop(tracker);
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
    fn home_screens_status_lock_and_switch() {
        let control = setup(
            r#"{"version":2,"home_screens":[
                {"id":"family","name":"Family","default":true},
                {"id":"kids","name":"Kids","limits":[{"type":"budget","minutes":1,"scope":{"kind":"every_day"}}]}]}"#,
        );
        let status = handle(r#"{"command":"status"}"#, &control);
        assert_eq!(status["home_screen"], json!("family"));
        assert_eq!(status["active_home_screen"], json!("family"));
        // Another home screen: evaluated with its own limits and usage.
        control
            .tracker
            .lock()
            .unwrap()
            .add_usage("kids", Local::now().date_naive(), 120);
        let kids = handle(r#"{"command":"status","home_screen":"kids"}"#, &control);
        assert_eq!(kids["home_screen"], json!("kids"));
        assert_eq!(kids["locked"], json!(true));
        assert_eq!(kids["used_today_secs"], json!(120));
        // Lock only that one.
        handle(r#"{"command":"lock","home_screen":"kids"}"#, &control);
        let tracker = control.tracker.lock().unwrap();
        assert!(tracker.locked_by_parent("kids") && !tracker.locked_by_parent("family"));
        drop(tracker);
        // Switch.
        assert_eq!(
            handle(
                r#"{"command":"switch_home_screen","home_screen":"kids"}"#,
                &control
            ),
            json!({"ok": true})
        );
        assert_eq!(control.tracker.lock().unwrap().active(), Some("kids"));
        for bad in [
            r#"{"command":"switch_home_screen"}"#,
            r#"{"command":"switch_home_screen","home_screen":"nope"}"#,
            r#"{"command":"lock","home_screen":"nope"}"#,
            r#"{"command":"status","home_screen":5}"#,
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
        assert!(!state.config.home_screens[0].discs.allow_disc_menu);
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
