//! Remembering where a movie was stopped.
//!
//! While mpv plays, we connect to its JSON IPC socket (`--input-ipc-server`), observe
//! `time-pos` and `duration`, and save the position regularly and when mpv exits.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::store;

/// Don't offer to resume if the movie was stopped this early.
const MIN_RESUME_SECS: f64 = 60.0;
/// Within this much of the end (end credits), the movie counts as finished …
const FINISHED_REMAINING_SECS: f64 = 5.0 * 60.0;
/// … but at most this share of the duration, so short videos (episodes) work too.
const FINISHED_REMAINING_SHARE: f64 = 0.10;
const SAVE_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Resume {
    pub position_secs: f64,
    pub duration_secs: f64,
}

impl Resume {
    /// Position to offer for resuming, or `None` if barely started or finished.
    pub fn resume_at(&self) -> Option<f64> {
        let credits = FINISHED_REMAINING_SECS.min(self.duration_secs * FINISHED_REMAINING_SHARE);
        let finished =
            self.duration_secs > 0.0 && self.duration_secs - self.position_secs < credits;
        (self.position_secs >= MIN_RESUME_SECS && !finished).then_some(self.position_secs)
    }
}

/// Load the resume position for a disc.
pub fn load(disc_key: &str) -> Option<f64> {
    store::load::<Resume>(&store::resume_path(disc_key)?)?.resume_at()
}

/// Save (or clear, if not worth resuming) the position for a disc.
fn store_position(disc_key: &str, resume: Resume) {
    let Some(path) = store::resume_path(disc_key) else {
        return;
    };
    if resume.resume_at().is_some() {
        if let Err(e) = store::save(&path, &resume) {
            log::warn!("could not save resume position: {e}");
        }
    } else {
        store::remove(&path);
    }
}

/// Watch a running mpv through its IPC socket and keep the resume position up to date.
/// Ends when mpv closes the socket.
pub fn spawn_monitor(socket: PathBuf, disc_key: String) {
    std::thread::Builder::new()
        .name("resume-monitor".into())
        .spawn(move || {
            if let Err(e) = monitor(&socket, &disc_key) {
                log::warn!("resume tracking stopped: {e}");
            }
        })
        .expect("spawning a thread only fails if the OS is out of resources");
}

fn monitor(socket: &std::path::Path, disc_key: &str) -> std::io::Result<()> {
    // mpv creates the socket right after starting; give it some time.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut stream = loop {
        match UnixStream::connect(socket) {
            Ok(s) => break s,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(e),
        }
    };
    stream.write_all(
        b"{\"command\":[\"observe_property\",1,\"time-pos\"]}\n\
          {\"command\":[\"observe_property\",2,\"duration\"]}\n",
    )?;

    let mut position = None;
    let mut duration = 0.0;
    let mut last_save = Instant::now();
    for line in BufReader::new(stream).lines() {
        let Some((id, value)) = parse_property_change(&line?) else {
            continue;
        };
        match (id, value) {
            (1, Some(v)) => position = Some(v),
            (2, Some(v)) => duration = v,
            _ => {}
        }
        if let Some(position_secs) = position
            && last_save.elapsed() >= SAVE_INTERVAL
        {
            store_position(
                disc_key,
                Resume {
                    position_secs,
                    duration_secs: duration,
                },
            );
            last_save = Instant::now();
        }
    }
    // mpv has quit: store the final position.
    if let Some(position_secs) = position {
        store_position(
            disc_key,
            Resume {
                position_secs,
                duration_secs: duration,
            },
        );
    }
    Ok(())
}

/// Parse `{"event":"property-change","id":1,"name":"time-pos","data":12.3}`.
/// Returns the observer id and the value (`None` if mpv reports no value).
fn parse_property_change(line: &str) -> Option<(u64, Option<f64>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("event")?.as_str()? != "property-change" {
        return None;
    }
    Some((
        v.get("id")?.as_u64()?,
        v.get("data").and_then(|d| d.as_f64()),
    ))
}

/// Resume key for a video file: `file-<FNV-1a hash of the canonical path>`. Stable across
/// runs and Rust versions (unlike `DefaultHasher`).
pub fn file_key(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.as_os_str().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("file-{hash:016x}")
}

/// Seconds → `h:mm:ss`.
pub fn format_time(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    format!("{}:{:02}:{:02}", total / 3600, total / 60 % 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_rules() {
        let r = |position_secs, duration_secs| Resume {
            position_secs,
            duration_secs,
        };
        assert_eq!(r(30.0, 6120.0).resume_at(), None); // barely started
        assert_eq!(r(2533.0, 6120.0).resume_at(), Some(2533.0));
        assert_eq!(r(6000.0, 6120.0).resume_at(), None); // in the end credits
        assert_eq!(r(2533.0, 0.0).resume_at(), Some(2533.0)); // unknown duration
        // A 10-minute episode stopped at 5:30 is not finished; at 9:30 it is.
        assert_eq!(r(330.0, 600.0).resume_at(), Some(330.0));
        assert_eq!(r(570.0, 600.0).resume_at(), None);
    }

    #[test]
    fn parses_mpv_events() {
        assert_eq!(
            parse_property_change(
                r#"{"event":"property-change","id":1,"name":"time-pos","data":12.5}"#
            ),
            Some((1, Some(12.5)))
        );
        assert_eq!(
            parse_property_change(r#"{"event":"property-change","id":1,"name":"time-pos"}"#),
            Some((1, None))
        );
        assert_eq!(
            parse_property_change(r#"{"data":null,"request_id":0,"error":"success"}"#),
            None
        );
        assert_eq!(parse_property_change("garbage"), None);
    }

    #[test]
    fn file_keys_are_stable() {
        let key = file_key(std::path::Path::new("/srv/movies/a.mkv"));
        assert_eq!(key, file_key(std::path::Path::new("/srv/movies/a.mkv")));
        assert_ne!(key, file_key(std::path::Path::new("/srv/movies/b.mkv")));
        assert_eq!(file_key(std::path::Path::new("")), "file-cbf29ce484222325");
    }

    #[test]
    fn formats_times() {
        assert_eq!(format_time(2533.4), "0:42:13");
        assert_eq!(format_time(6120.0), "1:42:00");
        assert_eq!(format_time(-5.0), "0:00:00");
    }
}
