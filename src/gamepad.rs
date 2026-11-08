//! Gamepads (read in `remote.rs`): their current state, and navigation for the media
//! center's menus (d-pad / stick = arrows, buttons = OK / back), so a child can also start
//! the game with the gamepad.
//!
//! Buttons differ between pads (Xbox / PlayStation pads report `BTN_SOUTH` …, simple SNES
//! style pads `BTN_TRIGGER` …), so only a few have a fixed meaning: B / circle / select =
//! back, start / select / mode = pause (in the game); every other button = OK.

use std::collections::BTreeMap;

use crate::remote::{PadAxis, PadEvent, PadInput};

const BTN_EAST: u16 = 0x131;
const BTN_SELECT: u16 = 0x13a;
const BTN_START: u16 = 0x13b;
const BTN_MODE: u16 = 0x13c;
/// "Select" / "start" of simple pads (`BTN_BASE3` / `BTN_BASE4`).
const BTN_BASE3: u16 = 0x128;
const BTN_BASE4: u16 = 0x129;

/// Stick positions below this count as the middle.
const DEAD_ZONE: f32 = 0.25;
/// A direction for menus.
const PRESSED: f32 = 0.5;

fn is_back(code: u16) -> bool {
    matches!(code, BTN_EAST | BTN_SELECT | BTN_BASE3)
}

fn is_pause(code: u16) -> bool {
    matches!(
        code,
        BTN_START | BTN_SELECT | BTN_MODE | BTN_BASE3 | BTN_BASE4
    )
}

#[derive(Debug, Default, Clone)]
struct Pad {
    x: f32,
    y: f32,
    hat_x: f32,
    hat_y: f32,
    /// Buttons pressed since the last frame, and those held down now.
    pressed: Vec<u16>,
    held: Vec<u16>,
    /// The buttons of the last `Pads::frame`.
    last_pressed: Vec<u16>,
    /// The menu direction last frame (for edges).
    direction: (i8, i8),
}

impl Pad {
    /// -1 … 1 per axis: the d-pad if pressed, else the stick (outside the dead zone).
    fn position(&self) -> (f32, f32) {
        let pick = |hat: f32, stick: f32| {
            if hat != 0.0 {
                hat
            } else if stick.abs() > DEAD_ZONE {
                stick
            } else {
                0.0
            }
        };
        (pick(self.hat_x, self.x), pick(self.hat_y, self.y))
    }
}

/// One gamepad as a player's controls (Block Stacker).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct PadPlayer {
    pub x: f32,
    pub y: f32,
    /// B / circle.
    pub rotate_left: bool,
    /// Any other button (except start / select).
    pub rotate_right: bool,
}

/// Navigation from all gamepads for one frame.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PadNav {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub enter: bool,
    pub back: bool,
    /// Start / select: pause the game (also OK in menus).
    pub pause: bool,
}

#[derive(Debug, Default)]
pub struct Pads {
    pads: BTreeMap<u32, Pad>,
}

impl Pads {
    pub fn apply(&mut self, event: PadEvent) {
        match event.input {
            PadInput::Gone => {
                self.pads.remove(&event.pad);
            }
            PadInput::Connected(_) => {
                self.pads.entry(event.pad).or_default();
            }
            PadInput::Button { code, pressed } => {
                let pad = self.pads.entry(event.pad).or_default();
                if pressed {
                    pad.pressed.push(code);
                    if !pad.held.contains(&code) {
                        pad.held.push(code);
                    }
                } else {
                    pad.held.retain(|c| *c != code);
                }
            }
            PadInput::Axis(axis, value) => {
                let pad = self.pads.entry(event.pad).or_default();
                match axis {
                    PadAxis::X => pad.x = value,
                    PadAxis::Y => pad.y = value,
                    PadAxis::HatX => pad.hat_x = value,
                    PadAxis::HatY => pad.hat_y = value,
                }
            }
        }
    }

    pub fn count(&self) -> usize {
        self.pads.len()
    }

    /// Each pad as a player, in a stable order; buttons as of the last `frame`.
    pub fn players(&self) -> Vec<PadPlayer> {
        self.pads
            .values()
            .map(|p| {
                let (x, y) = p.position();
                PadPlayer {
                    x,
                    y,
                    rotate_left: p.last_pressed.contains(&BTN_EAST),
                    rotate_right: p.last_pressed.iter().any(|c| !is_back(*c) && !is_pause(*c)),
                }
            })
            .collect()
    }

    /// Left / right of all pads together (-1 … 1).
    pub fn horizontal(&self) -> f32 {
        self.pads
            .values()
            .map(|p| p.position().0)
            .sum::<f32>()
            .clamp(-1.0, 1.0)
    }

    /// A button (not start / select / back) is held on any pad.
    pub fn button_held(&self) -> bool {
        self.pads
            .values()
            .any(|p| p.held.iter().any(|c| !is_pause(*c) && !is_back(*c)))
    }

    /// Up / down of each pad (-1 … 1), in a stable order (player 1 first).
    pub fn vertical(&self) -> Vec<f32> {
        self.pads.values().map(|p| p.position().1).collect()
    }

    /// Navigation since the last call (direction changes and button presses).
    pub fn frame(&mut self) -> PadNav {
        let mut nav = PadNav::default();
        for pad in self.pads.values_mut() {
            let (x, y) = pad.position();
            let dir = |v: f32| {
                if v <= -PRESSED {
                    -1
                } else if v >= PRESSED {
                    1
                } else {
                    0
                }
            };
            let now = (dir(x), dir(y));
            if now.0 != pad.direction.0 {
                nav.left |= now.0 == -1;
                nav.right |= now.0 == 1;
            }
            if now.1 != pad.direction.1 {
                nav.up |= now.1 == -1;
                nav.down |= now.1 == 1;
            }
            pad.direction = now;
            pad.last_pressed = pad.pressed.clone();
            for code in pad.pressed.drain(..) {
                if is_pause(code) {
                    nav.pause = true;
                }
                if is_back(code) {
                    nav.back = true;
                } else if !is_pause(code) {
                    nav.enter = true;
                }
            }
        }
        nav
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(pad: u32, input: PadInput) -> PadEvent {
        PadEvent { pad, input }
    }

    #[test]
    fn directions_are_edges_and_dpad_wins_over_the_stick() {
        let mut pads = Pads::default();
        pads.apply(event(5, PadInput::Connected("Pad".into())));
        pads.apply(event(5, PadInput::Axis(PadAxis::Y, -1.0)));
        assert!(pads.frame().up);
        assert!(!pads.frame().up, "held: only once");
        pads.apply(event(5, PadInput::Axis(PadAxis::Y, 0.0)));
        assert_eq!(pads.frame(), PadNav::default());
        // A slightly off-center stick doesn't move anything.
        pads.apply(event(5, PadInput::Axis(PadAxis::X, 0.2)));
        assert_eq!(pads.vertical(), [0.0]);
        assert_eq!(pads.frame(), PadNav::default());
        // D-pad down while the stick points up: the d-pad counts.
        pads.apply(event(5, PadInput::Axis(PadAxis::Y, -0.9)));
        pads.apply(event(5, PadInput::Axis(PadAxis::HatY, 1.0)));
        assert_eq!(pads.vertical(), [1.0]);
        assert!(pads.frame().down);
    }

    #[test]
    fn buttons() {
        let mut pads = Pads::default();
        let press = |pads: &mut Pads, code| {
            pads.apply(event(
                3,
                PadInput::Button {
                    code,
                    pressed: true,
                },
            ));
            pads.frame()
        };
        let a = press(&mut pads, 0x130); // BTN_SOUTH
        assert!(a.enter && !a.back && !a.pause);
        assert!(pads.button_held(), "still held");
        let snes = press(&mut pads, 0x121); // a simple pad's face button
        assert!(snes.enter);
        let b = press(&mut pads, BTN_EAST);
        assert!(b.back && !b.enter);
        let start = press(&mut pads, BTN_START);
        assert!(start.pause && !start.enter && !start.back);
        let select = press(&mut pads, BTN_SELECT);
        assert!(select.pause && select.back);
        // Releases do nothing (but end holding).
        for code in [0x130, 0x121] {
            pads.apply(event(
                3,
                PadInput::Button {
                    code,
                    pressed: false,
                },
            ));
        }
        assert_eq!(pads.frame(), PadNav::default());
        assert!(
            !pads.button_held(),
            "start / select / back don't count as held"
        );
    }

    #[test]
    fn pads_as_players() {
        let mut pads = Pads::default();
        let button = |pad, code| {
            event(
                pad,
                PadInput::Button {
                    code,
                    pressed: true,
                },
            )
        };
        pads.apply(event(9, PadInput::Axis(PadAxis::HatX, 1.0)));
        pads.apply(button(9, 0x130));
        pads.apply(button(4, BTN_EAST));
        pads.frame();
        let players = pads.players();
        assert!(
            players[0].rotate_left && !players[0].rotate_right,
            "pad 4 first"
        );
        assert!(players[1].rotate_right && !players[1].rotate_left);
        assert_eq!(players[1].x, 1.0);
        pads.frame();
        assert!(
            !pads.players()[1].rotate_right,
            "a press counts for one frame"
        );
    }

    #[test]
    fn players_in_a_stable_order_and_unplugged_pads_go() {
        let mut pads = Pads::default();
        pads.apply(event(21, PadInput::Axis(PadAxis::Y, 1.0)));
        pads.apply(event(7, PadInput::Axis(PadAxis::Y, -1.0)));
        assert_eq!(pads.vertical(), [-1.0, 1.0]);
        pads.apply(event(7, PadInput::Gone));
        assert_eq!(pads.count(), 1);
        assert_eq!(pads.vertical(), [1.0]);
    }
}
