//! Local control socket (`$XDG_RUNTIME_DIR/familynovice.sock`), used by the admin interface.
//!
//! Protocol: one JSON object per line in each direction.
//! - `{"command":"reload"}` → `{"ok":true}` or `{"ok":false,"error":"…"}`
//! - `{"command":"status"}` → `{"ok":true,"config_version":1,"config_error":null}`
//! - `{"command":"restart"}` → not supported yet (`ok: false`)

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::config::{self, ConfigState};

pub type SharedConfig = Arc<Mutex<ConfigState>>;

/// Handle one request line and return the reply.
pub fn handle(line: &str, shared: &SharedConfig, config_path: &Path) -> Value {
    let command = serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|v| v.get("command")?.as_str().map(String::from));
    // A poisoned lock means a panic elsewhere; the state itself is still usable.
    let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
    match command.as_deref() {
        Some("reload") => match config::reload(&mut state, config_path) {
            Ok(()) => json!({"ok": true}),
            Err(e) => json!({"ok": false, "error": e}),
        },
        Some("status") => json!({
            "ok": true,
            "config_version": state.config.version,
            "config_error": state.error,
        }),
        Some("restart") => json!({"ok": false, "error": "restart is not supported yet"}),
        Some(other) => json!({"ok": false, "error": format!("unknown command: {other}")}),
        None => json!({"ok": false, "error": "expected {\"command\": \"…\"}"}),
    }
}

/// Start the socket server in a background thread. `notify` is called after every
/// request (e.g. to repaint the UI with a reloaded config).
pub fn spawn_server(
    socket: PathBuf,
    config_path: PathBuf,
    shared: SharedConfig,
    notify: impl Fn() + Send + Sync + 'static,
) {
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
                let (shared, path, notify) = (shared.clone(), config_path.clone(), notify.clone());
                std::thread::spawn(move || {
                    if let Err(e) = serve(stream, &shared, &path, &*notify) {
                        log::debug!("control connection ended: {e}");
                    }
                });
            }
        })
        .expect("spawning a thread only fails if the OS is out of resources");
}

fn serve(
    stream: UnixStream,
    shared: &SharedConfig,
    config_path: &Path,
    notify: &dyn Fn(),
) -> std::io::Result<()> {
    let mut writer = stream.try_clone()?;
    for line in BufReader::new(stream).lines() {
        let reply = handle(&line?, shared, config_path);
        writeln!(writer, "{reply}")?;
        notify();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn setup(content: &str) -> (SharedConfig, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "familynovice-control-{}-{}",
            std::process::id(),
            content.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, content).unwrap();
        let state = config::load_initial(Some(&path));
        (Arc::new(Mutex::new(state)), path)
    }

    #[test]
    fn reload_and_status() {
        let (shared, path) = setup(r#"{"version":1}"#);
        std::fs::write(&path, r#"{"version":1,"discs":{"enabled":false}}"#).unwrap();
        assert_eq!(
            handle(r#"{"command":"reload"}"#, &shared, &path),
            json!({"ok": true})
        );
        assert!(!shared.lock().unwrap().config.discs.enabled);
        assert_eq!(
            handle(r#"{"command":"status"}"#, &shared, &path),
            json!({"ok": true, "config_version": 1, "config_error": null})
        );
    }

    #[test]
    fn invalid_reload_reports_error_and_keeps_config() {
        let (shared, path) = setup(r#"{"version":1,"discs":{"allow_disc_menu":false}}"#);
        std::fs::write(&path, "{").unwrap();
        let reply = handle(r#"{"command":"reload"}"#, &shared, &path);
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("invalid config"));
        let state = shared.lock().unwrap();
        assert!(!state.config.discs.allow_disc_menu);
        assert_ne!(state.config, Config::locked_down());
    }

    #[test]
    fn rejects_garbage_and_unknown_commands() {
        let (shared, path) = setup(r#"{"version":1}"#);
        assert_eq!(handle("hello", &shared, &path)["ok"], json!(false));
        assert_eq!(
            handle(r#"{"command":"rm -rf"}"#, &shared, &path)["ok"],
            json!(false)
        );
    }

    #[test]
    fn socket_roundtrip() {
        let (shared, path) = setup(r#"{"version":1}"#);
        let socket = std::env::temp_dir().join(format!("fn-ctl-{}.sock", std::process::id()));
        spawn_server(socket.clone(), path, shared, || {});
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
