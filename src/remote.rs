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
use crate::volume::VolumeCommand;
use crate::{tr, tr_noop};

/// `KEY_BACK`, and the right click that is "return" on air-mouse remotes. Only devices that
/// were set up for us are read, so this is never an ordinary mouse.
pub const KEY_BACK: u16 = 158;
pub const BTN_RIGHT: u16 = 0x111;
/// `KEY_POWER`: logind doesn't act on it for remotes set up with `scripts/remote-access.sh`.
pub const KEY_POWER: u16 = 116;
/// `KEY_HOMEPAGE`: the home button of most remotes.
pub const KEY_HOMEPAGE: u16 = 172;
/// `KEY_OK` / `KEY_SELECT`: OK of remotes outside pointer mode; invisible to X11 windows.
pub const KEY_OK: u16 = 352;
pub const KEY_SELECT: u16 = 353;
/// `KEY_NEXT` / `KEY_PREVIOUS`: next/previous of some remotes; invisible to X11 windows and
/// so to the desktop's media keys.
pub const KEY_NEXT: u16 = 407;
pub const KEY_PREVIOUS: u16 = 412;
/// `KEY_MUTE` / `KEY_VOLUMEDOWN` / `KEY_VOLUMEUP`: a desktop handles these itself (see
/// `volume::desktop_handles_keys`). Our window gets them as `AudioVolume*` keys; while a
/// player or website is in front, they come from the device.
pub const KEY_MUTE: u16 = 113;
pub const KEY_VOLUMEDOWN: u16 = 114;
pub const KEY_VOLUMEUP: u16 = 115;

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
    VolumeUp,
    VolumeDown,
    Mute,
}

impl Action {
    /// The order the calibration asks for the buttons.
    pub const ALL: [Action; 13] = [
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
        Action::VolumeUp,
        Action::VolumeDown,
        Action::Mute,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::Ok => tr_noop!("OK"),
            Action::Back => tr_noop!("Back / Return"),
            Action::Home => tr_noop!("Home"),
            Action::Up => tr_noop!("Up"),
            Action::Down => tr_noop!("Down"),
            Action::Left => tr_noop!("Left"),
            Action::Right => tr_noop!("Right"),
            Action::PlayPause => tr_noop!("Play / Pause"),
            Action::Next => tr_noop!("Next"),
            Action::Previous => tr_noop!("Previous"),
            Action::VolumeUp => tr_noop!("Volume up"),
            Action::VolumeDown => tr_noop!("Volume down"),
            Action::Mute => tr_noop!("Mute"),
        }
    }

    /// The volume command of a volume button.
    pub fn volume(self) -> Option<VolumeCommand> {
        match self {
            Action::VolumeUp => Some(VolumeCommand::Up),
            Action::VolumeDown => Some(VolumeCommand::Down),
            Action::Mute => Some(VolumeCommand::Mute),
            _ => None,
        }
    }
}

/// One learned button.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Button {
    /// Key codes from the input device (linux/input-event-codes.h, e.g. 353 = KEY_SELECT).
    #[serde(default)]
    pub codes: Vec<u16>,
    /// What the window got for the same press (`key:Enter`, `mouse:secondary`, or
    /// `desktop:next` for a media key the desktop grabbed and sent via MPRIS). Empty: the
    /// window gets nothing, so the media center acts on `codes` itself.
    #[serde(default)]
    pub window: Vec<String>,
}

/// The learned buttons: `$XDG_DATA_HOME/curator/remote.json`.
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

    /// The action of a device code the window doesn't get. Without calibration, codes above
    /// 247 (invisible to X11 windows) have defaults: OK/Select = OK, Next/Previous.
    pub fn device_action(&self, code: u16) -> Option<Action> {
        let learned = self
            .buttons
            .iter()
            .find(|(_, b)| b.window.is_empty() && b.codes.contains(&code))
            .map(|(a, _)| *a);
        let known_elsewhere = self.buttons.values().any(|b| b.codes.contains(&code));
        learned.or(match code {
            _ if known_elsewhere => None,
            KEY_OK | KEY_SELECT => Some(Action::Ok),
            KEY_NEXT => Some(Action::Next),
            KEY_PREVIOUS => Some(Action::Previous),
            _ => None,
        })
    }

    /// The volume command of a device code. `focused`: our window is in front and gets the
    /// keys it can (then only buttons the window doesn't get count); otherwise one of our
    /// players or websites is in front, which doesn't change the system volume, so every
    /// volume button counts, learned or standard. `desktop_volume`: a desktop already
    /// handles the standard volume keys, so they never count.
    pub fn device_volume(
        &self,
        code: u16,
        focused: bool,
        desktop_volume: bool,
    ) -> Option<VolumeCommand> {
        if desktop_volume && matches!(code, KEY_VOLUMEUP | KEY_VOLUMEDOWN | KEY_MUTE) {
            return None;
        }
        if focused {
            return self.device_action(code).and_then(Action::volume);
        }
        let learned = self
            .buttons
            .iter()
            .find(|(_, b)| b.codes.contains(&code))
            .map(|(a, _)| *a);
        match learned {
            Some(action) => action.volume(),
            None => match code {
                KEY_VOLUMEUP => Some(VolumeCommand::Up),
                KEY_VOLUMEDOWN => Some(VolumeCommand::Down),
                KEY_MUTE => Some(VolumeCommand::Mute),
                _ => None,
            },
        }
    }

    /// Is this device code the home button (held: go to the home screen)?
    pub fn is_home(&self, code: u16) -> bool {
        match self.buttons.get(&Action::Home) {
            Some(b) if !b.codes.is_empty() => b.codes.contains(&code),
            _ => code == KEY_HOMEPAGE,
        }
    }

    /// Does this device code close a program? Return or home, also learned ones whose press
    /// reaches a window, because a program's window would just swallow it.
    pub fn closes_program(&self, code: u16) -> bool {
        if self.is_home(code) {
            return true;
        }
        let learned_as_back = self
            .buttons
            .get(&Action::Back)
            .is_some_and(|b| b.codes.contains(&code));
        let used_otherwise = self.buttons.values().any(|b| b.codes.contains(&code));
        learned_as_back || (!used_otherwise && matches!(code, KEY_BACK | BTN_RIGHT))
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

    /// For the admin panel (English): "Enter key, remote code 353".
    pub fn describe(&self, action: Action) -> String {
        self.describe_with(action, |t| t)
    }

    /// The same, with the texts translated by `t` (the TV: `i18n::tr`).
    pub fn describe_with(&self, action: Action, t: fn(&'static str) -> &'static str) -> String {
        let Some(b) = self.buttons.get(&action) else {
            return t(tr_noop!("not set up (skipped)")).into();
        };
        let fill = |text: &'static str, name: &str, value: &str| {
            t(text).replace(&format!("{{{name}}}"), value)
        };
        let mut parts: Vec<String> = b
            .window
            .iter()
            .map(|w| match w.split_once(':') {
                Some(("key", k)) => fill(tr_noop!("{key} key"), "key", k),
                Some(("mouse", m)) => fill(tr_noop!("{button} click"), "button", m),
                Some(("desktop", d)) => fill(
                    tr_noop!("{command} via the desktop (media key)"),
                    "command",
                    d,
                ),
                _ => w.clone(),
            })
            .collect();
        parts.extend(
            b.codes
                .iter()
                .map(|c| fill(tr_noop!("remote code {code}"), "code", &c.to_string())),
        );
        if b.window.is_empty() && !b.codes.is_empty() {
            parts.push(t(tr_noop!("read from the remote directly")).into());
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
const EV_ABS: u16 = 3;
/// `struct input_event`: struct timeval (two longs), u16 type, u16 code, i32 value.
const TIME_SIZE: usize = 2 * std::mem::size_of::<libc::c_long>();
const EVENT_SIZE: usize = TIME_SIZE + 8;

/// All events in a buffer of raw `input_event`s: (type, code, value).
fn parse_raw(buf: &[u8]) -> Vec<(u16, u16, i32)> {
    buf.as_chunks::<EVENT_SIZE>()
        .0
        .iter()
        .map(|e| {
            let field = |at: usize| [e[TIME_SIZE + at], e[TIME_SIZE + at + 1]];
            let value = i32::from_ne_bytes([
                e[TIME_SIZE + 4],
                e[TIME_SIZE + 5],
                e[TIME_SIZE + 6],
                e[TIME_SIZE + 7],
            ]);
            (
                u16::from_ne_bytes(field(0)),
                u16::from_ne_bytes(field(2)),
                value,
            )
        })
        .collect()
}

/// Key events in a buffer of raw `input_event`s (other event types are skipped).
#[cfg(test)]
fn parse_events(buf: &[u8]) -> Vec<KeyEvent> {
    parse_raw(buf)
        .into_iter()
        .filter(|(kind, _, _)| *kind == EV_KEY)
        .map(|(_, code, value)| KeyEvent { code, value })
        .collect()
}

// ------------------------------------------------------------------------------- gamepads

/// Joystick and gamepad buttons (`BTN_JOYSTICK` … `BTN_THUMBR`); keyboards have none.
pub const PAD_BUTTONS: std::ops::RangeInclusive<u16> = 0x120..=0x13f;

/// The axes we use: left stick and d-pad (the d-pad of simple pads is `X`/`Y`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadAxis {
    X,
    Y,
    HatX,
    HatY,
}

const PAD_AXES: [(u16, PadAxis); 4] = [
    (0x00, PadAxis::X),    // ABS_X
    (0x01, PadAxis::Y),    // ABS_Y
    (0x10, PadAxis::HatX), // ABS_HAT0X
    (0x11, PadAxis::HatY), // ABS_HAT0Y
];

#[derive(Debug, Clone, PartialEq)]
pub enum PadInput {
    Connected(String),
    Button {
        code: u16,
        pressed: bool,
    },
    /// -1 (left / up) … 1 (right / down).
    Axis(PadAxis, f32),
    Gone,
}

/// Something a gamepad did; `pad` is its event device number (`/dev/input/event<pad>`).
#[derive(Debug, Clone, PartialEq)]
pub struct PadEvent {
    pub pad: u32,
    pub input: PadInput,
}

/// A raw axis value in -1 … 1, from the device's range.
pub fn normalize(value: i32, min: i32, max: i32) -> f32 {
    if max <= min {
        return 0.0;
    }
    let f = (f64::from(value) - f64::from(min)) / (f64::from(max) - f64::from(min));
    (f * 2.0 - 1.0).clamp(-1.0, 1.0) as f32
}

/// `_IOC(_IOC_READ, 'E', nr, size)` (the generic Linux layout: x86, ARM, RISC-V).
const fn eviocg(nr: u32, size: u32) -> libc::c_ulong {
    ((2u32 << 30) | (size << 16) | ((b'E' as u32) << 8) | nr) as libc::c_ulong
}

/// The ranges of the axes we use, if the device is a gamepad / joystick.
fn pad_axes(fd: std::os::fd::RawFd) -> Option<Vec<(u16, PadAxis, i32, i32)>> {
    // EVIOCGBIT(EV_KEY): which keys / buttons the device has (KEY_MAX = 0x2ff).
    let mut keys = [0u8; 0x300 / 8];
    // SAFETY: the buffer is as long as the size encoded in the request.
    let n = unsafe {
        libc::ioctl(
            fd,
            eviocg(0x20 + u32::from(EV_KEY), keys.len() as u32),
            keys.as_mut_ptr(),
        )
    };
    let has = |code: u16| keys[usize::from(code / 8)] & (1 << (code % 8)) != 0;
    if n < 0 || !PAD_BUTTONS.into_iter().any(has) {
        return None;
    }
    let mut axes = Vec::new();
    for (code, axis) in PAD_AXES {
        // struct input_absinfo: value, minimum, maximum, fuzz, flat, resolution.
        let mut info = [0i32; 6];
        // SAFETY: as above, 24 bytes.
        let r = unsafe { libc::ioctl(fd, eviocg(0x40 + u32::from(code), 24), info.as_mut_ptr()) };
        if r >= 0 {
            axes.push((code, axis, info[1], info[2]));
        }
    }
    Some(axes)
}

/// Names of the input devices being read (for the admin panel).
pub type SharedDevices = Arc<Mutex<BTreeMap<PathBuf, String>>>;

/// Read every input device this user may read (normally only the remote, see the module
/// docs), now and when plugged in later. Events go to `events`; `notify` wakes the UI.
pub fn spawn(
    events: Sender<KeyEvent>,
    pads: Sender<PadEvent>,
    notify: impl Fn() + Send + Sync + 'static,
) -> SharedDevices {
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
                    use std::os::fd::AsRawFd;
                    let pad = pad_axes(file.as_raw_fd()).and_then(|axes| {
                        let number = node.to_str()?.strip_prefix("/dev/input/event")?;
                        log::info!("gamepad: {name}");
                        let pad = number.parse::<u32>().ok()?;
                        let _ = pads.send(PadEvent {
                            pad,
                            input: PadInput::Connected(name.clone()),
                        });
                        Some((pad, axes, pads.clone()))
                    });
                    devices.insert(node.clone(), name);
                    let (events, notify, open) = (events.clone(), notify.clone(), open.clone());
                    std::thread::spawn(move || {
                        read_device(file, &events, pad.as_ref(), &*notify);
                        if let Some((pad, _, pads)) = &pad {
                            let _ = pads.send(PadEvent {
                                pad: *pad,
                                input: PadInput::Gone,
                            });
                            notify();
                        }
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

type PadInfo = (u32, Vec<(u16, PadAxis, i32, i32)>, Sender<PadEvent>);

fn read_device(
    mut file: std::fs::File,
    events: &Sender<KeyEvent>,
    pad: Option<&PadInfo>,
    notify: &dyn Fn(),
) {
    let mut buf = vec![0u8; EVENT_SIZE * 64];
    loop {
        match file.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                let mut any = false;
                for (kind, code, value) in parse_raw(&buf[..n - n % EVENT_SIZE]) {
                    if kind == EV_KEY {
                        any = true;
                        if events.send(KeyEvent { code, value }).is_err() {
                            return;
                        }
                    }
                    let Some((number, axes, pads)) = pad else {
                        continue;
                    };
                    let input = match kind {
                        EV_KEY if PAD_BUTTONS.contains(&code) && value != 2 => {
                            Some(PadInput::Button {
                                code,
                                pressed: value != 0,
                            })
                        }
                        EV_ABS => {
                            axes.iter()
                                .find(|(c, ..)| *c == code)
                                .map(|(_, axis, min, max)| {
                                    PadInput::Axis(*axis, normalize(value, *min, *max))
                                })
                        }
                        _ => None,
                    };
                    if let Some(input) = input {
                        any = true;
                        let _ = pads.send(PadEvent {
                            pad: *number,
                            input,
                        });
                    }
                }
                if any {
                    notify();
                }
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
                self.message = Some(tr!(
                    "That button is already {action}. Press another one, or wait to skip.",
                    action = crate::i18n::tr(other.label())
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

    #[test]
    fn return_and_home_close_a_program() {
        let empty = RemoteMap::default();
        for code in [KEY_BACK, BTN_RIGHT, KEY_HOMEPAGE] {
            assert!(empty.closes_program(code));
        }
        assert!(!empty.closes_program(KEY_SELECT));
        let mut map = RemoteMap::default();
        let button = |codes: Vec<u16>, window: &str| Button {
            codes,
            window: vec![window.to_string()],
        };
        map.buttons
            .insert(Action::Back, button(vec![200], "mouse:secondary"));
        map.buttons
            .insert(Action::Ok, button(vec![KEY_BACK], "key:Enter"));
        assert!(map.closes_program(200), "learned, even if a window gets it");
        assert!(!map.closes_program(KEY_BACK), "this remote's OK button");
        assert!(map.closes_program(BTN_RIGHT), "pointer mode");
    }

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
    fn pad_axes_are_normalized() {
        // Simple USB pads: d-pad as 0…255 with the middle at 127/128.
        assert_eq!(normalize(0, 0, 255), -1.0);
        assert_eq!(normalize(255, 0, 255), 1.0);
        assert!(normalize(127, 0, 255).abs() < 0.01);
        // Xbox-style stick, hat switch.
        assert_eq!(normalize(-32768, -32768, 32767), -1.0);
        assert!(normalize(0, -32768, 32767).abs() < 0.001);
        assert_eq!(normalize(1, -1, 1), 1.0);
        assert_eq!(normalize(5, 3, 3), 0.0);
        assert_eq!(normalize(900, 0, 255), 1.0);
        // EVIOCGABS(ABS_Y) as in linux/input.h: 0x80184541.
        assert_eq!(eviocg(0x41, 24), 0x8018_4541);
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
        assert_eq!(empty.device_action(KEY_NEXT), Some(Action::Next));
        assert_eq!(empty.device_action(KEY_PREVIOUS), Some(Action::Previous));
        assert_eq!(empty.device_action(28), None); // Enter reaches the window
        // Player in front: the standard volume keys from the device.
        assert_eq!(
            empty.device_volume(KEY_VOLUMEUP, false, false),
            Some(VolumeCommand::Up)
        );
        assert_eq!(
            empty.device_volume(KEY_MUTE, false, false),
            Some(VolumeCommand::Mute)
        );
        // Our window in front: it gets them as AudioVolume* keys, the device doesn't count.
        assert_eq!(empty.device_volume(KEY_VOLUMEUP, true, false), None);
        // A desktop handles the standard keys itself: no second change.
        assert_eq!(empty.device_volume(KEY_VOLUMEUP, false, true), None);
        assert_eq!(empty.device_volume(KEY_SELECT, false, false), None);
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
        // A learned volume button only the device sees: always, also with a desktop.
        map.buttons.insert(
            Action::VolumeDown,
            Button {
                codes: vec![KEY_VOLUMEUP, 0x1a0],
                window: vec![],
            },
        );
        for focused in [true, false] {
            assert_eq!(
                map.device_volume(0x1a0, focused, true),
                Some(VolumeCommand::Down)
            );
        }
        assert_eq!(map.device_volume(KEY_VOLUMEUP, false, true), None);
        // Learned: KEY_VOLUMEUP now means "down" (the default is replaced).
        assert_eq!(
            map.device_volume(KEY_VOLUMEUP, false, false),
            Some(VolumeCommand::Down)
        );
        // A learned button the window gets too: the window's while focused, else the device's.
        map.buttons.insert(
            Action::Mute,
            Button {
                codes: vec![KEY_MUTE],
                window: vec!["key:AudioVolumeMute".into()],
            },
        );
        assert_eq!(map.device_volume(KEY_MUTE, true, false), None);
        assert_eq!(
            map.device_volume(KEY_MUTE, false, false),
            Some(VolumeCommand::Mute)
        );
        assert!(!map.is_home(KEY_HOMEPAGE)); // learned home button replaces the default
        let json = serde_json::to_string(&map).unwrap();
        assert_eq!(serde_json::from_str::<RemoteMap>(&json).unwrap(), map);
    }
}
