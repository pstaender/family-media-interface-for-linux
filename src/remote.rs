//! Remote controls, read directly from their input device (`/dev/input/event*`, read-only,
//! never grabbed), plus the calibration that learns which button sends what.
//!
//! Why: some buttons never reach any window. X11 can't deliver key codes above 247 (e.g.
//! `KEY_SELECT` = 353, the OK button of many remotes outside pointer mode), and egui ignores
//! keys it doesn't know. Access: `scripts/remote-access.sh` adds a udev rule that lets the
//! user at the screen read only the remote (systemd-logind "uaccess"); without it, no device
//! is readable and only the window's keys work.
//!
//! No double actions: a learned button whose press also reached the window (as a key or
//! click) is handled by the window as before; the media center only acts on device codes for
//! buttons the window doesn't get. The home button is the exception: holding it is a state,
//! and it must also work while a player or website is in front.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::store;

/// `KEY_HOMEPAGE`: the home button of most remotes.
pub const KEY_HOMEPAGE: u16 = 172;
/// `KEY_OK` / `KEY_SELECT`: OK of remotes outside pointer mode; invisible to X11 windows.
pub const KEY_OK: u16 = 352;
pub const KEY_SELECT: u16 = 353;

/// What a remote button does in the media center.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Ok,
    Back,
    Home,
    Up,
    Down,
    Left,
    Right,
    PlayPause,
    Next,
    Previous,
}

impl Action {
    /// The order the calibration asks for the buttons.
    pub const ALL: [Action; 10] = [
        Action::Ok,
        Action::Back,
        Action::Home,
        Action::Up,
        Action::Down,
        Action::Left,
        Action::Right,
        Action::PlayPause,
        Action::Next,
        Action::Previous,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::Ok => "OK",
            Action::Back => "Back / Return",
            Action::Home => "Home",
            Action::Up => "Up",
            Action::Down => "Down",
            Action::Left => "Left",
            Action::Right => "Right",
            Action::PlayPause => "Play / Pause",
            Action::Next => "Next",
            Action::Previous => "Previous",
        }
    }
}

/// One learned button.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Button {
    /// Key codes from the input device (linux/input-event-codes.h, e.g. 353 = KEY_SELECT).
    #[serde(default)]
    pub codes: Vec<u16>,
    /// What the window got for the same press (`key:Enter`, `mouse:secondary`). Empty: the
    /// window gets nothing, so the media center acts on `codes` itself.
    #[serde(default)]
    pub window: Vec<String>,
}

/// The learned buttons: `$XDG_DATA_HOME/familynovice/remote.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteMap {
    #[serde(default)]
    pub buttons: BTreeMap<Action, Button>,
}

impl RemoteMap {
    pub fn load() -> Self {
        store::remote_path()
            .and_then(|p| store::load(&p))
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path =
            store::remote_path().ok_or_else(|| std::io::Error::other("no data directory"))?;
        store::save(&path, self)
    }

    /// The action of a device code the window doesn't get. Without calibration: OK/Select
    /// (invisible to X11 windows) mean OK.
    pub fn device_action(&self, code: u16) -> Option<Action> {
        let learned = self
            .buttons
            .iter()
            .find(|(_, b)| b.window.is_empty() && b.codes.contains(&code))
            .map(|(a, _)| *a);
        let known_elsewhere = self.buttons.values().any(|b| b.codes.contains(&code));
        learned.or(match code {
            KEY_OK | KEY_SELECT if !known_elsewhere => Some(Action::Ok),
            _ => None,
        })
    }

    /// Is this device code the home button (held: go to the home screen)?
    pub fn is_home(&self, code: u16) -> bool {
        match self.buttons.get(&Action::Home) {
            Some(b) if !b.codes.is_empty() => b.codes.contains(&code),
            _ => code == KEY_HOMEPAGE,
        }
    }

    /// Learned window keys (`key:…`) besides the built-in ones, e.g. a remote whose back
    /// button sends F5. Clicks aren't included: the left click already works on its own.
    pub fn window_keys(&self) -> impl Iterator<Item = (&str, Action)> {
        self.buttons.iter().flat_map(|(action, b)| {
            b.window
                .iter()
                .filter_map(|w| w.strip_prefix("key:"))
                .map(move |name| (name, *action))
        })
    }

    /// For the summary screen and the admin panel: "OK: Enter key, remote code 353".
    pub fn describe(&self, action: Action) -> String {
        let Some(b) = self.buttons.get(&action) else {
            return "not set up (skipped)".into();
        };
        let mut parts: Vec<String> = b
            .window
            .iter()
            .map(|w| match w.split_once(':') {
                Some(("key", k)) => format!("{k} key"),
                Some(("mouse", m)) => format!("{m} click"),
                _ => w.clone(),
            })
            .collect();
        parts.extend(b.codes.iter().map(|c| format!("remote code {c}")));
        if b.window.is_empty() && !b.codes.is_empty() {
            parts.push("read from the remote directly".into());
        }
        parts.join(", ")
    }
}

// ------------------------------------------------------------------------------- devices

/// A key or button of an input device: `value` 1 = pressed, 2 = repeated, 0 = released.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub code: u16,
    pub value: i32,
}

const EV_KEY: u16 = 1;
/// `struct input_event`: struct timeval (two longs), u16 type, u16 code, i32 value.
const TIME_SIZE: usize = 2 * std::mem::size_of::<libc::c_long>();
const EVENT_SIZE: usize = TIME_SIZE + 8;

/// Key events in a buffer of raw `input_event`s (other event types are skipped).
pub fn parse_events(buf: &[u8]) -> Vec<KeyEvent> {
    buf.as_chunks::<EVENT_SIZE>()
        .0
        .iter()
        .filter_map(|e| {
            let field = |at: usize| [e[TIME_SIZE + at], e[TIME_SIZE + at + 1]];
            let kind = u16::from_ne_bytes(field(0));
            let code = u16::from_ne_bytes(field(2));
            let value = i32::from_ne_bytes([
                e[TIME_SIZE + 4],
                e[TIME_SIZE + 5],
                e[TIME_SIZE + 6],
                e[TIME_SIZE + 7],
            ]);
            (kind == EV_KEY).then_some(KeyEvent { code, value })
        })
        .collect()
}

/// Names of the input devices being read (for the admin panel).
pub type SharedDevices = Arc<Mutex<BTreeMap<PathBuf, String>>>;

/// Read every input device this user may read (normally only the remote, see the module
/// docs), now and when plugged in later. Events go to `events`; `notify` wakes the UI.
pub fn spawn(events: Sender<KeyEvent>, notify: impl Fn() + Send + Sync + 'static) -> SharedDevices {
    let devices: SharedDevices = Arc::default();
    let notify = Arc::new(notify);
    let open = devices.clone();
    std::thread::Builder::new()
        .name("remote-devices".into())
        .spawn(move || {
            loop {
                for (node, name) in readable_devices(Path::new("/sys/class/input")) {
                    let mut devices = open.lock().unwrap_or_else(|p| p.into_inner());
                    if devices.contains_key(&node) {
                        continue;
                    }
                    let Ok(file) = std::fs::File::open(&node) else {
                        continue;
                    };
                    log::info!("remote: reading {} ({name})", node.display());
                    devices.insert(node.clone(), name);
                    let (events, notify, open) = (events.clone(), notify.clone(), open.clone());
                    std::thread::spawn(move || {
                        read_device(file, &events, &*notify);
                        log::info!("remote: {} is gone", node.display());
                        open.lock().unwrap_or_else(|p| p.into_inner()).remove(&node);
                    });
                }
                std::thread::sleep(Duration::from_secs(3));
            }
        })
        .expect("spawning a thread only fails if the OS is out of resources");
    devices
}

/// `/dev/input/eventN` nodes we may open for reading, with the device name.
fn readable_devices(sys: &Path) -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(sys) else {
        return Vec::new();
    };
    let mut out: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            let file_name = e.file_name().to_string_lossy().into_owned();
            if !file_name.starts_with("event") {
                return None;
            }
            let node = Path::new("/dev/input").join(&file_name);
            // R_OK check without opening (opening is the real test, done by the caller).
            let c = std::ffi::CString::new(node.as_os_str().as_encoded_bytes()).ok()?;
            // SAFETY: `c` is a valid NUL-terminated path; access() only reads it.
            if unsafe { libc::access(c.as_ptr(), libc::R_OK) } != 0 {
                return None;
            }
            let name = std::fs::read_to_string(e.path().join("device/name"))
                .map(|n| n.trim().to_string())
                .unwrap_or_default();
            Some((node, name))
        })
        .collect();
    out.sort();
    out
}

fn read_device(mut file: std::fs::File, events: &Sender<KeyEvent>, notify: &dyn Fn()) {
    let mut buf = vec![0u8; EVENT_SIZE * 64];
    loop {
        match file.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                let parsed = parse_events(&buf[..n - n % EVENT_SIZE]);
                if parsed.is_empty() {
                    continue;
                }
                for event in parsed {
                    if events.send(event).is_err() {
                        return;
                    }
                }
                notify();
            }
        }
    }
}

// ------------------------------------------------------------------------------ calibration

/// How long a press may take to show up on both ways (device and window).
const GATHER: Duration = Duration::from_millis(300);
/// Pause after a learned button, so its release / key repeat isn't taken for the next one.
const COOLDOWN: Duration = Duration::from_millis(700);
/// Without a press for this long, the button is skipped (the remote may not have it).
pub const SKIP_AFTER: Duration = Duration::from_secs(10);

/// The calibration screen's state: asks for one button after the other.
#[derive(Debug, Clone)]
pub struct Calibration {
    step: usize,
    step_started: Instant,
    pending: Option<(Instant, Button)>,
    cooldown_until: Option<Instant>,
    /// Shown below the question, e.g. "already used for OK".
    pub message: Option<String>,
    pub map: RemoteMap,
}

impl Calibration {
    pub fn new(now: Instant) -> Self {
        Self {
            step: 0,
            step_started: now,
            pending: None,
            cooldown_until: None,
            message: None,
            map: RemoteMap::default(),
        }
    }

    /// The button asked for, `None` when done.
    pub fn current(&self) -> Option<Action> {
        Action::ALL.get(self.step).copied()
    }

    pub fn step(&self) -> usize {
        self.step
    }

    /// Seconds until this button is skipped.
    pub fn seconds_left(&self, now: Instant) -> u64 {
        SKIP_AFTER
            .saturating_sub(now.duration_since(self.step_started))
            .as_secs()
            + 1
    }

    fn accepting(&self, now: Instant) -> bool {
        self.current().is_some() && self.cooldown_until.is_none_or(|t| now >= t)
    }

    fn pending(&mut self, now: Instant) -> &mut Button {
        &mut self
            .pending
            .get_or_insert_with(|| (now, Button::default()))
            .1
    }

    /// A key press from the input device (`value` 1).
    pub fn device_press(&mut self, code: u16, now: Instant) {
        if self.accepting(now) {
            let b = self.pending(now);
            if !b.codes.contains(&code) {
                b.codes.push(code);
            }
        }
    }

    /// A key press or click the window got (`key:Enter`, `mouse:secondary`).
    pub fn window_press(&mut self, what: String, now: Instant) {
        if self.accepting(now) {
            let b = self.pending(now);
            if !b.window.contains(&what) {
                b.window.push(what);
            }
        }
    }

    /// Advance: take a gathered press, or skip after `SKIP_AFTER`. True when done.
    pub fn tick(&mut self, now: Instant) -> bool {
        let Some(action) = self.current() else {
            return true;
        };
        if let Some((since, _)) = &self.pending
            && now.duration_since(*since) >= GATHER
        {
            let (_, button) = self.pending.take().expect("checked above");
            let used_by = self.map.buttons.iter().find(|(_, b)| {
                b.codes.iter().any(|c| button.codes.contains(c))
                    || b.window.iter().any(|w| button.window.contains(w))
            });
            self.cooldown_until = Some(now + COOLDOWN);
            if let Some((other, _)) = used_by {
                self.message = Some(format!(
                    "That button is already {}. Press another one, or wait to skip.",
                    other.label()
                ));
                self.step_started = now;
                return false;
            }
            self.map.buttons.insert(action, button);
            self.next(now);
        } else if self.pending.is_none() && now.duration_since(self.step_started) >= SKIP_AFTER {
            self.next(now);
        }
        self.current().is_none()
    }

    fn next(&mut self, now: Instant) {
        self.step += 1;
        self.step_started = now;
        self.message = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(kind: u16, code: u16, value: i32) -> Vec<u8> {
        let mut e = vec![0u8; TIME_SIZE];
        e.extend(kind.to_ne_bytes());
        e.extend(code.to_ne_bytes());
        e.extend(value.to_ne_bytes());
        e
    }

    #[test]
    fn parses_key_events_only() {
        let mut buf = raw(4, 4, 0x70028); // EV_MSC scan code
        buf.extend(raw(EV_KEY, KEY_SELECT, 1));
        buf.extend(raw(0, 0, 0)); // EV_SYN
        buf.extend(raw(EV_KEY, KEY_SELECT, 0));
        assert_eq!(
            parse_events(&buf),
            [
                KeyEvent {
                    code: KEY_SELECT,
                    value: 1
                },
                KeyEvent {
                    code: KEY_SELECT,
                    value: 0
                }
            ]
        );
        assert_eq!(parse_events(&buf[..EVENT_SIZE - 1]), []);
    }

    #[test]
    fn calibration_learns_skips_and_rejects_duplicates() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut c = Calibration::new(t0);
        // OK: only the device sees it (KEY_SELECT outside pointer mode).
        c.device_press(KEY_SELECT, at(100));
        assert!(!c.tick(at(200)));
        assert_eq!(c.current(), Some(Action::Ok)); // still gathering
        c.tick(at(450));
        assert_eq!(c.current(), Some(Action::Back));
        // Presses during the cooldown (release, repeat) are ignored.
        c.device_press(KEY_SELECT, at(500));
        c.tick(at(900));
        assert_eq!(c.current(), Some(Action::Back));
        // Back: window and device both see it.
        c.window_press("mouse:secondary".into(), at(1300));
        c.device_press(273, at(1310));
        c.tick(at(1700));
        assert_eq!(c.current(), Some(Action::Home));
        // Home: the OK button again → rejected with a message, still asking for Home.
        c.device_press(KEY_SELECT, at(2500));
        c.tick(at(2900));
        assert_eq!(c.current(), Some(Action::Home));
        assert!(c.message.as_deref().unwrap().contains("already OK"));
        // No home button: skipped after 10 s.
        c.tick(at(2900 + 10_000));
        assert_eq!(c.current(), Some(Action::Up));
        let mut now = 13_000;
        while !c.tick(at(now)) {
            now += 11_000;
        }
        let map = c.map;
        assert_eq!(map.buttons.len(), 2);
        assert_eq!(map.device_action(KEY_SELECT), Some(Action::Ok));
        assert_eq!(map.device_action(273), None); // the window handles the right click
        assert!(map.describe(Action::Ok).contains("remote code 353"));
        assert_eq!(map.describe(Action::Home), "not set up (skipped)");
        // No learned home button: KEY_HOMEPAGE.
        assert!(map.is_home(KEY_HOMEPAGE));
    }

    #[test]
    fn defaults_and_learned_window_keys() {
        let empty = RemoteMap::default();
        assert_eq!(empty.device_action(KEY_SELECT), Some(Action::Ok));
        assert_eq!(empty.device_action(KEY_OK), Some(Action::Ok));
        assert_eq!(empty.device_action(28), None); // Enter reaches the window
        let mut map = RemoteMap::default();
        map.buttons.insert(
            Action::Back,
            Button {
                codes: vec![63],
                window: vec!["key:F5".into(), "mouse:middle".into()],
            },
        );
        map.buttons.insert(
            Action::Home,
            Button {
                codes: vec![102],
                window: vec![],
            },
        );
        assert_eq!(
            map.window_keys().collect::<Vec<_>>(),
            [("F5", Action::Back)]
        );
        assert!(map.is_home(102));
        assert!(!map.is_home(KEY_HOMEPAGE)); // learned home button replaces the default
        let json = serde_json::to_string(&map).unwrap();
        assert_eq!(serde_json::from_str::<RemoteMap>(&json).unwrap(), map);
    }
}
