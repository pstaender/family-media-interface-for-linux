//! System volume: the remote's volume buttons change the default audio output (e.g. HDMI to
//! a TV or receiver) through PipeWire (`wpctl`) or, as fallback, PulseAudio (`pactl`, also
//! works with pipewire-pulse). Software volume: the computer scales the sound before sending
//! it, so the receiver itself stays at a fixed, fairly high level.
//!
//! Commands run in a worker thread (they take a few ms each, never in the UI thread), one
//! after the other, with absolute values, so quick presses never race. The volume never goes
//! above 100 % (no distorting boost).
//!
//! A desktop (Cinnamon, GNOME, KDE, …) handles the standard volume keys itself; then the
//! media center leaves them alone, so a press doesn't change the volume twice. In the kiosk
//! session (cage) nobody else does, so the media center does it.

use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// One press, in percent.
pub const STEP: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeCommand {
    Up,
    Down,
    Mute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub percent: u32,
    pub muted: bool,
}

/// What the volume bar shows: the level after the last press, or that no volume control
/// was found (neither `wpctl` nor `pactl` worked).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    Level(Level),
    Unavailable,
}

/// The new level after a command. Up/down unmute (like a TV) and snap to the step grid.
pub fn apply(level: Level, command: VolumeCommand) -> Level {
    let snapped = (level.percent + STEP / 2) / STEP * STEP;
    match command {
        VolumeCommand::Up => Level {
            percent: (snapped + STEP).min(100),
            muted: false,
        },
        VolumeCommand::Down => Level {
            percent: snapped.saturating_sub(STEP).min(100),
            muted: false,
        },
        VolumeCommand::Mute => Level {
            muted: !level.muted,
            ..level
        },
    }
}

/// `wpctl get-volume @DEFAULT_AUDIO_SINK@`: `Volume: 0.40` or `Volume: 0.40 [MUTED]`.
pub fn parse_wpctl(out: &str) -> Option<Level> {
    let rest = out.trim().strip_prefix("Volume:")?.trim();
    let number = rest.split_whitespace().next()?;
    let value: f64 = number.parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some(Level {
        percent: (value * 100.0).round() as u32,
        muted: rest.contains("[MUTED]"),
    })
}

/// `pactl get-sink-volume @DEFAULT_SINK@`:
/// `Volume: front-left: 26214 /  40% / -23.87 dB,   front-right: …` → 40 (first channel).
pub fn parse_pactl_volume(out: &str) -> Option<u32> {
    let percent = out.split('%').next()?;
    percent
        .rsplit(['/', ' '])
        .find(|s| !s.is_empty())?
        .parse()
        .ok()
}

/// `pactl get-sink-mute @DEFAULT_SINK@`: `Mute: yes` / `Mute: no`.
pub fn parse_pactl_mute(out: &str) -> Option<bool> {
    match out.trim().strip_prefix("Mute:")?.trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// Does a desktop handle the standard volume keys (`KEY_VOLUMEUP` …)? Not in our kiosk
/// session (`CURATOR_KIOSK=1`, set by the session script) nor without any desktop.
pub fn desktop_handles_keys() -> bool {
    std::env::var_os("CURATOR_KIOSK").is_none_or(|v| v.is_empty())
        && std::env::var_os("XDG_CURRENT_DESKTOP").is_some_and(|v| !v.is_empty())
}

pub struct Volume {
    tx: Sender<VolumeCommand>,
    shown: Arc<Mutex<Option<(Shown, Instant)>>>,
}

impl Volume {
    /// Start the worker; `notify` wakes the UI after a change.
    pub fn spawn(notify: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let shown: Arc<Mutex<Option<(Shown, Instant)>>> = Arc::default();
        let out = shown.clone();
        let spawned = std::thread::Builder::new()
            .name("volume".into())
            .spawn(move || worker(&rx, &out, &notify));
        if let Err(e) = spawned {
            log::warn!("volume: no worker thread: {e}");
        }
        Self { tx, shown }
    }

    pub fn send(&self, command: VolumeCommand) {
        let _ = self.tx.send(command);
    }

    /// What the volume bar shows, and since when.
    pub fn shown(&self) -> Option<(Shown, Instant)> {
        *self.shown.lock().unwrap_or_else(|p| p.into_inner())
    }
}

fn worker(
    rx: &Receiver<VolumeCommand>,
    shown: &Mutex<Option<(Shown, Instant)>>,
    notify: &dyn Fn(),
) {
    while let Ok(first) = rx.recv() {
        let mut commands = vec![first];
        commands.extend(rx.try_iter()); // presses that queued up meanwhile
        let result = change(&commands);
        if let Err(e) = &result {
            log::warn!("volume: {e}");
        }
        let now = match result {
            Ok(level) => Shown::Level(level),
            Err(_) => Shown::Unavailable,
        };
        *shown.lock().unwrap_or_else(|p| p.into_inner()) = Some((now, Instant::now()));
        notify();
    }
}

/// Read the level, apply the commands, write it back. PipeWire first, then PulseAudio.
fn change(commands: &[VolumeCommand]) -> Result<Level, String> {
    let wpctl_error = match wpctl_get() {
        Ok(before) => {
            let after = commands.iter().fold(before, |l, c| apply(l, *c));
            let volume = format!("{:.2}", f64::from(after.percent) / 100.0);
            run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &volume])?;
            run(
                "wpctl",
                &["set-mute", "@DEFAULT_AUDIO_SINK@", mute_arg(after.muted)],
            )?;
            return Ok(after);
        }
        Err(e) => e,
    };
    let before =
        pactl_get().map_err(|e| format!("no volume control (wpctl: {wpctl_error}; pactl: {e})"))?;
    let after = commands.iter().fold(before, |l, c| apply(l, *c));
    run(
        "pactl",
        &[
            "set-sink-volume",
            "@DEFAULT_SINK@",
            &format!("{}%", after.percent),
        ],
    )?;
    run(
        "pactl",
        &["set-sink-mute", "@DEFAULT_SINK@", mute_arg(after.muted)],
    )?;
    Ok(after)
}

fn mute_arg(muted: bool) -> &'static str {
    if muted { "1" } else { "0" }
}

fn wpctl_get() -> Result<Level, String> {
    let out = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])?;
    parse_wpctl(&out).ok_or_else(|| format!("unexpected output: {}", out.trim()))
}

fn pactl_get() -> Result<Level, String> {
    let volume = run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])?;
    let mute = run("pactl", &["get-sink-mute", "@DEFAULT_SINK@"])?;
    Ok(Level {
        percent: parse_pactl_volume(&volume)
            .ok_or_else(|| format!("unexpected output: {}", volume.trim()))?,
        muted: parse_pactl_mute(&mute)
            .ok_or_else(|| format!("unexpected output: {}", mute.trim()))?,
    })
}

/// Run a program (fixed argument list, no shell) and return its output.
fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C") // pactl translates its output
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(percent: u32, muted: bool) -> Level {
        Level { percent, muted }
    }

    #[test]
    fn steps_stay_between_0_and_100_and_unmute() {
        assert_eq!(apply(level(40, false), VolumeCommand::Up), level(45, false));
        assert_eq!(
            apply(level(40, false), VolumeCommand::Down),
            level(35, false)
        );
        assert_eq!(
            apply(level(98, false), VolumeCommand::Up),
            level(100, false)
        );
        assert_eq!(
            apply(level(100, false), VolumeCommand::Up),
            level(100, false)
        );
        assert_eq!(apply(level(2, false), VolumeCommand::Down), level(0, false));
        assert_eq!(apply(level(0, false), VolumeCommand::Down), level(0, false));
        // Off-grid (set elsewhere): snapped to the grid first.
        assert_eq!(apply(level(43, false), VolumeCommand::Up), level(50, false));
        assert_eq!(
            apply(level(41, false), VolumeCommand::Down),
            level(35, false)
        );
        // Boosted above 100 % elsewhere: back to at most 100 %.
        assert_eq!(
            apply(level(150, false), VolumeCommand::Up),
            level(100, false)
        );
        assert_eq!(
            apply(level(150, false), VolumeCommand::Down),
            level(100, false)
        );
        // Up/down unmute, mute toggles and keeps the level.
        assert_eq!(apply(level(40, true), VolumeCommand::Up), level(45, false));
        assert_eq!(
            apply(level(40, false), VolumeCommand::Mute),
            level(40, true)
        );
        assert_eq!(
            apply(level(40, true), VolumeCommand::Mute),
            level(40, false)
        );
    }

    #[test]
    fn parses_wpctl() {
        assert_eq!(parse_wpctl("Volume: 0.40\n"), Some(level(40, false)));
        assert_eq!(parse_wpctl("Volume: 0.55 [MUTED]\n"), Some(level(55, true)));
        assert_eq!(parse_wpctl("Volume: 1.00"), Some(level(100, false)));
        assert_eq!(parse_wpctl("Volume: 1.50"), Some(level(150, false)));
        assert_eq!(parse_wpctl("Volume: -1"), None);
        assert_eq!(parse_wpctl("Volume: nan"), None);
        assert_eq!(parse_wpctl(""), None);
        assert_eq!(parse_wpctl("Translate ID error: …"), None);
    }

    #[test]
    fn parses_pactl() {
        let out = "Volume: front-left: 26214 /  40% / -23.87 dB,   front-right: 26214 /  40% / -23.87 dB\n        balance 0.00\n";
        assert_eq!(parse_pactl_volume(out), Some(40));
        assert_eq!(
            parse_pactl_volume("Volume: mono: 65536 / 100% / 0.00 dB"),
            Some(100)
        );
        assert_eq!(parse_pactl_volume("Volume: x"), None);
        assert_eq!(parse_pactl_mute("Mute: yes\n"), Some(true));
        assert_eq!(parse_pactl_mute("Mute: no\n"), Some(false));
        assert_eq!(parse_pactl_mute("Stummschaltung: ja"), None);
    }
}
