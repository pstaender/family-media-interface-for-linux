//! Which button of a gamepad is which SNES button (for RetroArch), and the setup on the TV
//! that learns it.
//!
//! Cheap pads identify themselves badly: RetroArch's collection has several profiles with
//! the name and USB id of the common `USB Gamepad ` (a NES pad, a Saturn pad, …) and uses
//! whichever it finds first. So `emulator::write_autoconfig` leaves out every stock profile
//! for a pad we know better, and puts in ours: learned here, or built in (`active_profiles`).
//!
//! The numbers in a profile are RetroArch's own (`remote::PadIdentity`), not Linux codes.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::remote::{PadAxis, PadEvent, PadIdentity, PadInput, SKIP_AFTER};
use crate::store;
use crate::{tr, tr_noop};

/// How long the setup waits for the first press before it gives up.
const WAIT_FOR_PAD: Duration = Duration::from_secs(30);
/// Pause after a press, so a bouncing or still-held button isn't taken for the next one.
const COOLDOWN: Duration = Duration::from_millis(500);
/// How far a stick / d-pad axis must go to count as a press.
const PRESSED: f32 = 0.5;

/// The buttons of a SNES controller, in the order the setup asks for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnesButton {
    Up,
    Down,
    Left,
    Right,
    B,
    A,
    Y,
    X,
    L,
    R,
    Select,
    Start,
}

impl SnesButton {
    pub const ALL: [SnesButton; 12] = [
        SnesButton::Up,
        SnesButton::Down,
        SnesButton::Left,
        SnesButton::Right,
        SnesButton::B,
        SnesButton::A,
        SnesButton::Y,
        SnesButton::X,
        SnesButton::L,
        SnesButton::R,
        SnesButton::Select,
        SnesButton::Start,
    ];

    /// By position, as on the original controller, whatever this pad prints on them.
    pub fn label(self) -> &'static str {
        match self {
            SnesButton::Up => tr_noop!("Up"),
            SnesButton::Down => tr_noop!("Down"),
            SnesButton::Left => tr_noop!("Left"),
            SnesButton::Right => tr_noop!("Right"),
            SnesButton::B => tr_noop!("B (bottom button)"),
            SnesButton::A => tr_noop!("A (right button)"),
            SnesButton::Y => tr_noop!("Y (left button)"),
            SnesButton::X => tr_noop!("X (top button)"),
            SnesButton::L => tr_noop!("L (left shoulder)"),
            SnesButton::R => tr_noop!("R (right shoulder)"),
            SnesButton::Select => tr_noop!("Select"),
            SnesButton::Start => tr_noop!("Start"),
        }
    }

    fn cfg_name(self) -> &'static str {
        match self {
            SnesButton::Up => "up",
            SnesButton::Down => "down",
            SnesButton::Left => "left",
            SnesButton::Right => "right",
            SnesButton::B => "b",
            SnesButton::A => "a",
            SnesButton::Y => "y",
            SnesButton::X => "x",
            SnesButton::L => "l",
            SnesButton::R => "r",
            SnesButton::Select => "select",
            SnesButton::Start => "start",
        }
    }

    fn is_direction(self) -> bool {
        matches!(
            self,
            SnesButton::Up | SnesButton::Down | SnesButton::Left | SnesButton::Right
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HatDirection {
    Up,
    Down,
    Left,
    Right,
}

/// What a SNES button is on the pad, in RetroArch's numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadBind {
    Button(u16),
    Axis { axis: u16, positive: bool },
    Hat(HatDirection),
}

impl PadBind {
    fn cfg_line(self, button: SnesButton) -> String {
        let name = button.cfg_name();
        match self {
            PadBind::Button(index) => format!("input_{name}_btn = \"{index}\"\n"),
            PadBind::Axis { axis, positive } => {
                let sign = if positive { '+' } else { '-' };
                format!("input_{name}_axis = \"{sign}{axis}\"\n")
            }
            PadBind::Hat(direction) => {
                let direction = match direction {
                    HatDirection::Up => "up",
                    HatDirection::Down => "down",
                    HatDirection::Left => "left",
                    HatDirection::Right => "right",
                };
                format!("input_{name}_btn = \"h0{direction}\"\n")
            }
        }
    }
}

/// The SNES layout of one kind of gamepad.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadProfile {
    /// Exactly as the device reports it (RetroArch compares it, trailing space included).
    pub name: String,
    pub vendor: u16,
    pub product: u16,
    pub binds: BTreeMap<SnesButton, PadBind>,
}

impl PadProfile {
    fn same_pad(&self, other: &PadProfile) -> bool {
        (&self.name, self.vendor, self.product) == (&other.name, other.vendor, other.product)
    }

    /// As a RetroArch autoconfig file. `None` for a name that can't be written safely: the
    /// name comes from the USB device, and a quote or line break would add settings.
    pub fn cfg(&self) -> Option<String> {
        if self.name.is_empty() || self.name.chars().any(|c| c == '"' || c.is_control()) {
            return None;
        }
        let mut text = format!(
            "input_driver = \"udev\"\ninput_device = \"{}\"\n\
             input_vendor_id = \"{}\"\ninput_product_id = \"{}\"\n",
            self.name, self.vendor, self.product
        );
        for (button, bind) in &self.binds {
            text.push_str(&bind.cfg_line(*button));
        }
        Some(text)
    }

    /// Does this stock profile compete with ours (same name and USB id, RetroArch's perfect
    /// match, where the first one found wins)?
    pub fn replaces(&self, stock: &str) -> bool {
        let value = |key: &str| {
            stock.lines().find_map(|line| {
                let (k, v) = line.trim().split_once('=')?;
                (k.trim() == key).then(|| {
                    let v = v.trim();
                    v.strip_prefix('"')
                        .and_then(|v| v.strip_suffix('"'))
                        .unwrap_or(v)
                        .to_string()
                })
            })
        };
        let number = |key: &str| {
            let text = value(key)?;
            match text.strip_prefix("0x") {
                Some(hex) => u16::from_str_radix(hex, 16).ok(),
                None => text.parse::<u16>().ok(),
            }
        };
        value("input_device").as_deref() == Some(self.name.as_str())
            && number("input_vendor_id") == Some(self.vendor)
            && number("input_product_id") == Some(self.product)
    }
}

/// The common SNES-style USB pad (`0079:0011`), by far the most sold.
fn builtin() -> Vec<PadProfile> {
    use PadBind::{Axis, Button};
    use SnesButton as S;
    vec![PadProfile {
        name: "USB Gamepad ".into(),
        vendor: 0x0079,
        product: 0x0011,
        binds: BTreeMap::from([
            (
                S::Up,
                Axis {
                    axis: 1,
                    positive: false,
                },
            ),
            (
                S::Down,
                Axis {
                    axis: 1,
                    positive: true,
                },
            ),
            (
                S::Left,
                Axis {
                    axis: 0,
                    positive: false,
                },
            ),
            (
                S::Right,
                Axis {
                    axis: 0,
                    positive: true,
                },
            ),
            (S::X, Button(0)),
            (S::A, Button(1)),
            (S::B, Button(2)),
            (S::Y, Button(3)),
            (S::L, Button(4)),
            (S::R, Button(5)),
            (S::Select, Button(8)),
            (S::Start, Button(9)),
        ]),
    }]
}

/// What was learned, plus the built-in ones for pads nobody has set up.
fn with_builtin(saved: Vec<PadProfile>) -> Vec<PadProfile> {
    let mut all = saved;
    for profile in builtin() {
        if !all.iter().any(|p| p.same_pad(&profile)) {
            all.push(profile);
        }
    }
    all
}

fn load_saved() -> Vec<PadProfile> {
    store::gamepads_path()
        .and_then(|p| store::load(&p))
        .unwrap_or_default()
}

pub fn active_profiles() -> Vec<PadProfile> {
    with_builtin(load_saved())
}

fn merged(mut saved: Vec<PadProfile>, new: PadProfile) -> Vec<PadProfile> {
    saved.retain(|p| !p.same_pad(&new));
    saved.push(new);
    saved
}

/// Keep what the setup learned (replaces the same kind of pad).
pub fn remember(profile: PadProfile) -> std::io::Result<()> {
    let path = store::gamepads_path().ok_or_else(|| std::io::Error::other("no data directory"))?;
    store::save(&path, &merged(load_saved(), profile))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Press {
    Button(u16),
    Axis(PadAxis, i8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupEnd {
    Finished,
    /// Nobody pressed anything on a gamepad.
    NoPad,
}

/// The setup screen's state: asks for one SNES button after the other, on one pad.
#[derive(Debug, Clone)]
pub struct PadSetup {
    pad: Option<(u32, PadIdentity)>,
    step: usize,
    step_started: Instant,
    cooldown_until: Option<Instant>,
    axes: HashMap<(u32, PadAxis), i8>,
    binds: BTreeMap<SnesButton, PadBind>,
    /// Shown below the question, e.g. "already used for B".
    pub message: Option<String>,
}

impl PadSetup {
    pub fn new(now: Instant) -> Self {
        Self {
            pad: None,
            step: 0,
            step_started: now,
            cooldown_until: None,
            axes: HashMap::new(),
            binds: BTreeMap::new(),
            message: None,
        }
    }

    pub fn waiting_for_pad(&self) -> bool {
        self.pad.is_none()
    }

    /// The button asked for; `None` before the pad is chosen and when done.
    pub fn current(&self) -> Option<SnesButton> {
        self.pad.as_ref()?;
        SnesButton::ALL.get(self.step).copied()
    }

    pub fn step(&self) -> usize {
        self.step
    }

    /// Seconds until this step gives up.
    pub fn seconds_left(&self, now: Instant) -> u64 {
        let limit = if self.waiting_for_pad() {
            WAIT_FOR_PAD
        } else {
            SKIP_AFTER
        };
        limit
            .saturating_sub(now.duration_since(self.step_started))
            .as_secs()
            + 1
    }

    /// A button going down or an axis leaving the middle, once per push.
    fn press(&mut self, event: &PadEvent) -> Option<Press> {
        match event.input {
            PadInput::Button {
                code,
                pressed: true,
            } => Some(Press::Button(code)),
            PadInput::Axis(axis, value) => {
                let direction = if value >= PRESSED {
                    1
                } else if value <= -PRESSED {
                    -1
                } else {
                    0
                };
                let before = self.axes.insert((event.pad, axis), direction).unwrap_or(0);
                (direction != 0 && direction != before).then_some(Press::Axis(axis, direction))
            }
            _ => None,
        }
    }

    /// Something a gamepad did; `identity` is what that pad reported when connected.
    pub fn event(&mut self, event: &PadEvent, identity: Option<&PadIdentity>, now: Instant) {
        let Some(press) = self.press(event) else {
            return;
        };
        if self.cooldown_until.is_some_and(|t| now < t) {
            return;
        }
        let Some((pad, known)) = &self.pad else {
            // The first press only says which pad is being set up.
            if let Some(identity) = identity {
                self.pad = Some((event.pad, identity.clone()));
                self.step_started = now;
                self.cooldown_until = Some(now + COOLDOWN);
            }
            return;
        };
        if *pad != event.pad {
            return;
        }
        let Some(button) = SnesButton::ALL.get(self.step).copied() else {
            return;
        };
        if !button.is_direction() && !matches!(press, Press::Button(_)) {
            return;
        }
        let Some(bind) = bind_of(known, press) else {
            return;
        };
        self.cooldown_until = Some(now + COOLDOWN);
        if let Some((other, _)) = self.binds.iter().find(|(_, b)| **b == bind) {
            self.message = Some(tr!(
                "That button is already {action}. Press another one, or wait to skip.",
                action = crate::i18n::tr(other.label())
            ));
            return;
        }
        self.binds.insert(button, bind);
        self.step += 1;
        self.step_started = now;
        self.message = None;
    }

    /// Advance the clock; `Some` when the setup is over.
    pub fn tick(&mut self, now: Instant) -> Option<SetupEnd> {
        if self.waiting_for_pad() {
            return (now.duration_since(self.step_started) >= WAIT_FOR_PAD)
                .then_some(SetupEnd::NoPad);
        }
        if self.step < SnesButton::ALL.len() && now.duration_since(self.step_started) >= SKIP_AFTER
        {
            self.step += 1;
            self.step_started = now;
            self.message = None;
        }
        (self.step >= SnesButton::ALL.len()).then_some(SetupEnd::Finished)
    }

    /// What was learned; `None` if every button was skipped.
    pub fn profile(&self) -> Option<PadProfile> {
        let (_, identity) = self.pad.as_ref()?;
        (!self.binds.is_empty()).then(|| PadProfile {
            name: identity.name.clone(),
            vendor: identity.vendor,
            product: identity.product,
            binds: self.binds.clone(),
        })
    }
}

fn bind_of(identity: &PadIdentity, press: Press) -> Option<PadBind> {
    let number = |list: &[u16], code: u16| {
        list.iter()
            .position(|c| *c == code)
            .and_then(|i| u16::try_from(i).ok())
    };
    match press {
        Press::Button(code) => number(&identity.buttons, code).map(PadBind::Button),
        Press::Axis(axis @ (PadAxis::X | PadAxis::Y), direction) => {
            let code = u16::from(axis == PadAxis::Y);
            number(&identity.axes, code).map(|axis| PadBind::Axis {
                axis,
                positive: direction > 0,
            })
        }
        Press::Axis(PadAxis::HatX, direction) => Some(PadBind::Hat(if direction < 0 {
            HatDirection::Left
        } else {
            HatDirection::Right
        })),
        Press::Axis(PadAxis::HatY, direction) => Some(PadBind::Hat(if direction < 0 {
            HatDirection::Up
        } else {
            HatDirection::Down
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BTN_TRIGGER: u16 = 0x120;

    /// The pad on the TV: ten buttons from `BTN_TRIGGER`, axes X and Y.
    fn snes_pad() -> PadIdentity {
        PadIdentity {
            name: "USB Gamepad ".into(),
            vendor: 0x0079,
            product: 0x0011,
            buttons: (0..10).map(|i| BTN_TRIGGER + i).collect(),
            axes: vec![0, 1],
        }
    }

    fn button(pad: u32, code: u16) -> PadEvent {
        PadEvent {
            pad,
            input: PadInput::Button {
                code,
                pressed: true,
            },
        }
    }

    fn axis(pad: u32, axis: PadAxis, value: f32) -> PadEvent {
        PadEvent {
            pad,
            input: PadInput::Axis(axis, value),
        }
    }

    /// A setup whose pad is chosen, on the first question; plus the time after the cooldown.
    fn started(pad: &PadIdentity) -> (PadSetup, Instant) {
        let t0 = Instant::now();
        let mut setup = PadSetup::new(t0);
        setup.event(&button(7, BTN_TRIGGER), Some(pad), t0);
        (setup, t0 + COOLDOWN)
    }

    #[test]
    fn learns_a_whole_pad_in_retroarchs_numbers() {
        let pad = snes_pad();
        let (mut setup, mut now) = started(&pad);
        assert_eq!(setup.current(), Some(SnesButton::Up));
        let events = [
            axis(7, PadAxis::Y, -1.0),
            axis(7, PadAxis::Y, 0.0), // back to the middle
            axis(7, PadAxis::Y, 1.0),
            axis(7, PadAxis::Y, 0.0),
            axis(7, PadAxis::X, -1.0),
            axis(7, PadAxis::X, 0.0),
            axis(7, PadAxis::X, 1.0),
            axis(7, PadAxis::X, 0.0),
            button(7, BTN_TRIGGER + 2),
            button(7, BTN_TRIGGER + 1),
            button(7, BTN_TRIGGER + 3),
            button(7, BTN_TRIGGER),
            button(7, BTN_TRIGGER + 4),
            button(7, BTN_TRIGGER + 5),
            button(7, BTN_TRIGGER + 8),
            button(7, BTN_TRIGGER + 9),
        ];
        for event in &events {
            setup.event(event, Some(&pad), now);
            now += COOLDOWN;
        }
        assert_eq!(setup.tick(now), Some(SetupEnd::Finished));
        let profile = setup.profile().unwrap();
        assert_eq!(
            profile,
            builtin().remove(0),
            "the built-in profile is what the setup learns on this pad"
        );
        let cfg = profile.cfg().unwrap();
        for line in [
            "input_device = \"USB Gamepad \"",
            "input_vendor_id = \"121\"",
            "input_product_id = \"17\"",
            "input_b_btn = \"2\"",
            "input_a_btn = \"1\"",
            "input_y_btn = \"3\"",
            "input_x_btn = \"0\"",
            "input_up_axis = \"-1\"",
            "input_right_axis = \"+0\"",
            "input_start_btn = \"9\"",
        ] {
            assert!(cfg.lines().any(|l| l == line), "missing: {line}\n{cfg}");
        }
    }

    #[test]
    fn hats_map_to_retroarchs_hat_names() {
        let pad = PadIdentity {
            axes: vec![0, 1],
            ..snes_pad()
        };
        let (mut setup, now) = started(&pad);
        setup.event(&axis(7, PadAxis::HatY, -1.0), Some(&pad), now);
        let cfg = setup.profile().unwrap().cfg().unwrap();
        assert!(cfg.contains("input_up_btn = \"h0up\""), "{cfg}");
    }

    #[test]
    fn an_already_used_button_is_refused_and_the_same_question_stays() {
        let pad = snes_pad();
        let (mut setup, now) = started(&pad);
        setup.event(&axis(7, PadAxis::Y, -1.0), Some(&pad), now);
        assert_eq!(setup.current(), Some(SnesButton::Down));
        setup.event(&axis(7, PadAxis::Y, 0.0), Some(&pad), now + COOLDOWN);
        setup.event(&axis(7, PadAxis::Y, -1.0), Some(&pad), now + COOLDOWN);
        assert_eq!(setup.current(), Some(SnesButton::Down));
        assert!(setup.message.is_some());
    }

    #[test]
    fn only_the_chosen_pad_counts_and_moving_the_stick_is_no_face_button() {
        let pad = snes_pad();
        let (mut setup, now) = started(&pad);
        setup.event(&axis(9, PadAxis::Y, -1.0), Some(&pad), now);
        assert_eq!(setup.step(), 0, "another pad");
        setup.step = 4; // B
        setup.step_started = now;
        setup.event(&axis(7, PadAxis::X, 1.0), Some(&pad), now);
        assert_eq!(setup.step(), 4, "a direction can't be a face button");
        assert_eq!(setup.profile(), None);
    }

    #[test]
    fn a_held_axis_or_button_during_the_cooldown_is_not_the_next_answer() {
        let pad = snes_pad();
        let (mut setup, now) = started(&pad);
        setup.event(&axis(7, PadAxis::Y, -1.0), Some(&pad), now);
        setup.event(
            &button(7, BTN_TRIGGER + 2),
            Some(&pad),
            now + Duration::from_millis(100),
        );
        assert_eq!(setup.step(), 1);
        // A noisy stick held at the same side is one push, also after the cooldown.
        setup.event(&axis(7, PadAxis::Y, -0.9), Some(&pad), now + COOLDOWN);
        assert_eq!(setup.step(), 1);
    }

    #[test]
    fn skips_after_a_while_and_gives_up_without_a_pad() {
        let t0 = Instant::now();
        let mut idle = PadSetup::new(t0);
        assert_eq!(idle.tick(t0 + WAIT_FOR_PAD / 2), None);
        assert_eq!(idle.tick(t0 + WAIT_FOR_PAD), Some(SetupEnd::NoPad));
        assert_eq!(idle.profile(), None);

        let pad = snes_pad();
        let (mut setup, mut now) = started(&pad);
        for n in 1..=SnesButton::ALL.len() {
            now += SKIP_AFTER;
            let end = setup.tick(now);
            assert_eq!(end.is_some(), n == SnesButton::ALL.len());
        }
        assert_eq!(setup.profile(), None, "nothing learned, nothing saved");
    }

    #[test]
    fn a_learned_profile_replaces_the_built_in_one() {
        let mut mine = builtin().remove(0);
        mine.binds.insert(SnesButton::B, PadBind::Button(7));
        let all = with_builtin(vec![mine.clone()]);
        assert_eq!(all, [mine.clone()]);
        assert_eq!(with_builtin(Vec::new()), builtin());
        let other = PadProfile {
            name: "Other".into(),
            ..mine.clone()
        };
        assert_eq!(merged(vec![mine.clone()], other.clone()).len(), 2);
        assert_eq!(
            merged(vec![other.clone(), mine.clone()], mine.clone()).len(),
            2
        );
    }

    #[test]
    fn replaces_only_the_exact_pad_and_ignores_disabled_lines() {
        let ours = builtin().remove(0);
        let stock = |device: &str, vendor: &str| {
            format!(
                "input_driver = \"udev\"\n{device}\ninput_vendor_id = {vendor}\n\
                 input_product_id = \"17\"\ninput_b_btn = \"1\"\n"
            )
        };
        assert!(ours.replaces(&stock("input_device = \"USB Gamepad \"", "\"121\"")));
        assert!(ours.replaces(&stock("input_device = \"USB Gamepad \"", "121")));
        assert!(ours.replaces(&stock("input_device = \"USB Gamepad \"", "\"0x79\"")));
        // Hyperkin's has no trailing space, and RetroArch's own duplicates are commented out.
        assert!(!ours.replaces(&stock("input_device = \"USB Gamepad\"", "\"121\"")));
        assert!(!ours.replaces(&stock("#input_device = \"USB Gamepad \"", "\"121\"")));
        assert!(!ours.replaces(&stock("input_device = \"USB Gamepad \"", "\"122\"")));
    }

    #[test]
    fn a_device_name_cannot_add_settings() {
        let mut profile = builtin().remove(0);
        for bad in ["x\"\ninput_b_btn = \"9", "a\"b", "two\nlines", ""] {
            profile.name = bad.into();
            assert_eq!(profile.cfg(), None, "{bad:?}");
        }
    }
}
