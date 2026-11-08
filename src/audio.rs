//! Music playback inside curator.
//!
//! The sound comes from an mpv process without any window (it plays every audio format);
//! curator shows the player screen and controls mpv over its JSON IPC socket.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{player, store};

/// What the player screen shows; updated from mpv's property events.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioState {
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub paused: bool,
    /// Index in the list mpv got (0 = the track the user chose).
    pub playlist_pos: Option<usize>,
}

pub struct AudioPlayer {
    child: Child,
    writer: Arc<Mutex<Option<UnixStream>>>,
    state: Arc<Mutex<AudioState>>,
}

impl AudioPlayer {
    /// Start playing `files[start..]`. `notify` is called on every state change (repaint).
    pub fn start(
        files: &[PathBuf],
        start: usize,
        notify: impl Fn() + Send + 'static,
    ) -> Result<Self, String> {
        let socket = store::audio_socket_path();
        store::remove(&socket); // stale socket from an earlier run
        let cache = store::prepare_player_cache();
        let mut cmd = player::audio_command(files, start, &socket, cache.as_deref())
            .ok_or("no track to play")?;
        log::info!("starting music: {cmd:?}");
        let child = cmd.spawn().map_err(|e| {
            crate::tr!("Could not start the music player (mpv): {error}", error = e)
        })?;
        let player = Self {
            child,
            writer: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(AudioState::default())),
        };
        let (writer, state) = (player.writer.clone(), player.state.clone());
        std::thread::Builder::new()
            .name("music-ipc".into())
            .spawn(move || {
                if let Err(e) = run_ipc(&socket, &writer, &state, &notify) {
                    log::warn!("music player connection: {e}");
                }
            })
            .expect("spawning a thread only fails if the OS is out of resources");
        Ok(player)
    }

    pub fn state(&self) -> AudioState {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn send(&self, command: Value) {
        let mut writer = self.writer.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(stream) = writer.as_mut()
            && let Err(e) = writeln!(stream, "{}", json!({ "command": command }))
        {
            log::warn!("music player command failed: {e}");
        }
    }

    pub fn toggle_pause(&self) {
        self.send(json!(["cycle", "pause"]));
    }

    pub fn next(&self) {
        self.send(json!(["playlist-next"]));
    }

    pub fn previous(&self) {
        // Like CD players: back to the start of the track first, unless it just started.
        if self.state().position.is_some_and(|p| p > 3.0) {
            self.send(json!(["seek", 0, "absolute"]));
        } else {
            self.send(json!(["playlist-prev"]));
        }
    }

    pub fn seek(&self, seconds: f64) {
        self.send(json!(["seek", seconds, "relative"]));
    }

    /// True once mpv has exited (end of the list or stopped).
    pub fn finished(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    pub fn stop(&mut self) {
        self.send(json!(["quit"]));
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline && !self.finished() {
            std::thread::sleep(Duration::from_millis(20));
        }
        if !self.finished() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        if !self.finished() {
            self.stop();
        }
    }
}

fn run_ipc(
    socket: &Path,
    writer: &Mutex<Option<UnixStream>>,
    state: &Mutex<AudioState>,
    notify: &dyn Fn(),
) -> std::io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let stream = loop {
        match UnixStream::connect(socket) {
            Ok(s) => break s,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e),
        }
    };
    let mut w = stream.try_clone()?;
    for (id, name) in [
        (1, "time-pos"),
        (2, "duration"),
        (3, "pause"),
        (4, "playlist-pos"),
    ] {
        writeln!(
            w,
            "{}",
            json!({ "command": ["observe_property", id, name] })
        )?;
    }
    *writer.lock().unwrap_or_else(|p| p.into_inner()) = Some(w);
    for line in BufReader::new(stream).lines() {
        let line = line?;
        let mut s = state.lock().unwrap_or_else(|p| p.into_inner());
        if apply_event(&mut s, &line) {
            drop(s);
            notify();
        }
    }
    Ok(())
}

/// Apply one mpv event line to the state. Returns true if something changed.
fn apply_event(state: &mut AudioState, line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    if v.get("event").and_then(Value::as_str) != Some("property-change") {
        return false;
    }
    let data = v.get("data");
    let before = state.clone();
    match v.get("name").and_then(Value::as_str) {
        Some("time-pos") => state.position = data.and_then(Value::as_f64),
        Some("duration") => state.duration = data.and_then(Value::as_f64),
        Some("pause") => state.paused = data.and_then(Value::as_bool).unwrap_or(false),
        Some("playlist-pos") => {
            state.playlist_pos = data
                .and_then(Value::as_i64)
                .and_then(|p| usize::try_from(p).ok())
        }
        _ => {}
    }
    *state != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_property_events() {
        let mut s = AudioState::default();
        assert!(apply_event(
            &mut s,
            r#"{"event":"property-change","id":1,"name":"time-pos","data":12.5}"#
        ));
        assert!(apply_event(
            &mut s,
            r#"{"event":"property-change","id":4,"name":"playlist-pos","data":2}"#
        ));
        assert!(apply_event(
            &mut s,
            r#"{"event":"property-change","id":3,"name":"pause","data":true}"#
        ));
        assert_eq!(
            s,
            AudioState {
                position: Some(12.5),
                duration: None,
                paused: true,
                playlist_pos: Some(2)
            }
        );
        // Same value again, other events, garbage: no change.
        assert!(!apply_event(
            &mut s,
            r#"{"event":"property-change","id":3,"name":"pause","data":true}"#
        ));
        assert!(!apply_event(&mut s, r#"{"event":"start-file"}"#));
        assert!(!apply_event(&mut s, "garbage"));
        // mpv reports -1 / null between tracks.
        apply_event(
            &mut s,
            r#"{"event":"property-change","id":4,"name":"playlist-pos","data":-1}"#,
        );
        assert_eq!(s.playlist_pos, None);
    }
}
