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
    /// Buttons pressed since the last frame.
    pressed: Vec<u16>,
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
        let snes = press(&mut pads, 0x121); // a simple pad's face button
        assert!(snes.enter);
        let b = press(&mut pads, BTN_EAST);
        assert!(b.back && !b.enter);
        let start = press(&mut pads, BTN_START);
        assert!(start.pause && !start.enter && !start.back);
        let select = press(&mut pads, BTN_SELECT);
        assert!(select.pause && select.back);
        // Releases do nothing.
        pads.apply(event(
            3,
            PadInput::Button {
                code: 0x130,
                pressed: false,
            },
        ));
        assert_eq!(pads.frame(), PadNav::default());
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
