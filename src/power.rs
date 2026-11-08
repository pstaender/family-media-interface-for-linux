//! The power button of the home screen: short press = ask, held 3 s = log out, held 8 s =
//! shut the computer down (`PowerHold`, pure and tested); shutting down via logind.

use std::time::{Duration, Instant};

/// Held this long (and released): log out.
pub const LOG_OUT_AFTER: Duration = Duration::from_secs(3);
/// Held this long: shut down.
pub const SHUT_DOWN_AFTER: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hold {
    Nothing,
    /// Held for this long so far (for the progress shown).
    Holding(Duration),
    /// Released before `LOG_OUT_AFTER`: ask what to do.
    Ask,
    LogOut,
    ShutDown,
}

/// Tracks one press of the power button.
#[derive(Debug, Default)]
pub struct PowerHold {
    since: Option<Instant>,
    /// Shutting down was triggered: nothing more until released.
    done: bool,
}

impl PowerHold {
    /// `down`: the button is held right now (OK while it has the focus, or the mouse on it).
    pub fn update(&mut self, down: bool, now: Instant) -> Hold {
        match (self.since, down) {
            (None, true) => {
                self.since = Some(now);
                self.done = false;
                Hold::Holding(Duration::ZERO)
            }
            (Some(_), true) if self.done => Hold::Nothing,
            (Some(since), true) => {
                let held = now.duration_since(since);
                if held >= SHUT_DOWN_AFTER {
                    self.done = true;
                    Hold::ShutDown
                } else {
                    Hold::Holding(held)
                }
            }
            (Some(since), false) => {
                self.since = None;
                if self.done {
                    Hold::Nothing
                } else if now.duration_since(since) < LOG_OUT_AFTER {
                    Hold::Ask
                } else {
                    Hold::LogOut
                }
            }
            (None, false) => Hold::Nothing,
        }
    }
}

/// Shut the computer down through logind (allowed for the user at the screen without a
/// password, unless other users are logged in; then polkit refuses without asking).
pub fn power_off() -> Result<(), String> {
    let bus = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
    bus.call_method(
        Some("org.freedesktop.login1"),
        "/org/freedesktop/login1",
        Some("org.freedesktop.login1.Manager"),
        "PowerOff",
        &(false,),
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn short_press_asks() {
        let (mut h, t0) = (PowerHold::default(), Instant::now());
        assert_eq!(h.update(true, t0), Hold::Holding(Duration::ZERO));
        assert_eq!(
            h.update(true, at(t0, 500)),
            Hold::Holding(Duration::from_millis(500))
        );
        assert_eq!(h.update(false, at(t0, 600)), Hold::Ask);
        assert_eq!(h.update(false, at(t0, 700)), Hold::Nothing);
    }

    #[test]
    fn released_after_3_seconds_logs_out() {
        let (mut h, t0) = (PowerHold::default(), Instant::now());
        h.update(true, t0);
        h.update(true, at(t0, 4000));
        assert_eq!(h.update(false, at(t0, 5000)), Hold::LogOut);
    }

    #[test]
    fn held_8_seconds_shuts_down_once() {
        let (mut h, t0) = (PowerHold::default(), Instant::now());
        h.update(true, t0);
        assert!(matches!(h.update(true, at(t0, 7900)), Hold::Holding(_)));
        assert_eq!(h.update(true, at(t0, 8000)), Hold::ShutDown);
        assert_eq!(h.update(true, at(t0, 9000)), Hold::Nothing);
        // Releasing afterwards neither logs out nor asks.
        assert_eq!(h.update(false, at(t0, 9500)), Hold::Nothing);
        // A new press starts over.
        assert_eq!(
            h.update(true, at(t0, 10_000)),
            Hold::Holding(Duration::ZERO)
        );
    }
}
