//! The lock screen: shown when the time is up, outside the allowed time or when a parent
//! locked the media center. With a PIN set, a number pad (remote: arrows + OK; keyboard:
//! digits, Backspace, Enter) unlocks it for 30 minutes, an hour or the rest of the day.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, Sense, Vec2};

use crate::limits::{self, LockReason};
use crate::tr;

/// Number pad: 1–9, then ⌫, 0, OK.
const KEYS: [&str; 12] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "⌫", "0", "OK"];
const BACKSPACE: usize = 9;
const OK: usize = 11;
const MAX_DIGITS: usize = 8;

/// Unlock choices after a correct PIN: minutes (`None` = rest of today).
pub const UNLOCK_CHOICES: [(&str, Option<u32>); 3] = [
    (crate::tr_noop!("30 more minutes"), Some(30)),
    (crate::tr_noop!("1 more hour"), Some(60)),
    (crate::tr_noop!("Rest of today"), None),
];

pub struct LockView {
    pub reason: LockReason,
    entered: String,
    focus: usize,
    pub checking: bool,
    pub message: Option<String>,
    /// After a correct PIN: the selected unlock choice (`UNLOCK_CHOICES`, then "Cancel").
    pub unlock_menu: Option<usize>,
    /// Asking for the PIN for something (not a lock): back cancels, a correct PIN
    /// confirms it.
    purpose: Option<Purpose>,
    confirmed: bool,
}

/// Navigation keys this frame, from the window, calibrated remote buttons and buttons only
/// the remote's input device reports (e.g. OK = `KEY_SELECT`).
#[derive(Default, Clone, Copy)]
pub struct Nav {
    pub enter: bool,
    /// Esc, Backspace, browser back, right click, the remote's back button.
    pub back: bool,
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

/// What the lock screen wants the app to do.
pub enum LockAction {
    None,
    /// Check this PIN (in the background).
    CheckPin(String),
    /// Unlock for minutes / rest of today.
    Unlock(Option<u32>),
    /// The PIN was right for this.
    Confirmed(Purpose),
    /// Asking for the PIN for something: cancelled.
    Cancel,
}

/// What only a parent may do on the TV (asks for the PIN).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    /// Exit the media center (log out).
    Exit,
    ShutDown,
    /// Show another home screen.
    Switch {
        id: String,
        name: String,
    },
}

/// What the lock screen needs to show.
pub struct LockInfo {
    pub has_pin: bool,
    /// Number of PIN digits, if known: the PIN is checked as soon as it is complete.
    pub pin_length: Option<usize>,
    /// Seconds to wait before the next try (after wrong PINs).
    pub wait_secs: i64,
}

impl LockView {
    pub fn new(reason: LockReason) -> Self {
        Self {
            reason,
            entered: String::new(),
            focus: 0,
            checking: false,
            message: None,
            unlock_menu: None,
            purpose: None,
            confirmed: false,
        }
    }

    /// The PIN pad to confirm something only a parent may do.
    pub fn for_purpose(purpose: Purpose) -> Self {
        Self {
            purpose: Some(purpose),
            ..Self::new(LockReason::Parent)
        }
    }

    fn purpose_texts(&self) -> Option<(String, String)> {
        Some(match self.purpose.as_ref()? {
            Purpose::Exit => (
                tr!("Exit the media center").into(),
                tr!("Enter the PIN to exit. Back: cancel.").into(),
            ),
            Purpose::ShutDown => (
                tr!("Shut down the computer").into(),
                tr!("Enter the PIN to shut down. Back: cancel.").into(),
            ),
            Purpose::Switch { name, .. } => (
                tr!("Switch to “{name}”", name = name),
                tr!("Enter the PIN to switch the home screen. Back: cancel.").into(),
            ),
        })
    }

    pub fn pin_checked(&mut self, ok: bool) {
        self.checking = false;
        self.entered.clear();
        if ok && self.purpose.is_some() {
            self.confirmed = true;
        } else if ok {
            self.message = None;
            self.unlock_menu = Some(0);
        } else {
            self.message = Some(tr!("Wrong PIN").into());
        }
    }

    pub fn show(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen: Rect,
        s: f32,
        info: &LockInfo,
        nav: &Nav,
    ) -> LockAction {
        if self.confirmed
            && let Some(purpose) = self.purpose.clone()
        {
            return LockAction::Confirmed(purpose);
        }
        painter.rect_filled(screen, 0.0, Color32::from_rgb(0x0c, 0x0e, 0x14));
        let center = screen.center();
        let (title, subtitle) = self.purpose_texts().unwrap_or_else(|| {
            let title = match self.reason {
                LockReason::Parent => tr!("Locked"),
                LockReason::BudgetUsed => tr!("Time is up"),
                LockReason::OutsideWindow { .. } => tr!("Not now"),
            };
            (
                title.to_string(),
                limits::reason_text_with(&self.reason, crate::i18n::tr),
            )
        });
        let color = crate::ui::TEXT;
        painter.text(
            Pos2::new(center.x, screen.min.y + 120.0 * s),
            Align2::CENTER_TOP,
            title,
            FontId::proportional(64.0 * s),
            color,
        );
        painter.text(
            Pos2::new(center.x, screen.min.y + 210.0 * s),
            Align2::CENTER_TOP,
            subtitle,
            FontId::proportional(30.0 * s),
            crate::ui::TEXT_DIM,
        );

        if let Some(selected) = self.unlock_menu {
            return self.unlock_menu(ui, painter, screen, s, selected, nav);
        }
        if !info.has_pin {
            painter.text(
                center,
                Align2::CENTER_CENTER,
                tr!("A parent can unlock it in the admin panel."),
                FontId::proportional(30.0 * s),
                crate::ui::TEXT,
            );
            return LockAction::None;
        }
        self.pin_pad(ui, painter, screen, s, info, nav)
    }

    fn unlock_menu(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen: Rect,
        s: f32,
        mut selected: usize,
        nav: &Nav,
    ) -> LockAction {
        let (up, down, enter, back) = (
            nav.up || nav.left,
            nav.down || nav.right,
            nav.enter,
            nav.back,
        );
        let count = UNLOCK_CHOICES.len() + 1;
        if up {
            selected = selected.saturating_sub(1);
        }
        if down {
            selected = (selected + 1).min(count - 1);
        }
        self.unlock_menu = Some(selected);
        let mut items: Vec<crate::ui::MenuItem> = UNLOCK_CHOICES
            .iter()
            .map(|(label, _)| crate::ui::MenuItem::new(crate::i18n::tr(label), ""))
            .collect();
        items.push(crate::ui::MenuItem::new(tr!("Cancel"), ""));
        let clicked =
            crate::ui::paint_menu(ui, painter, screen, s, tr!("Unlock for…"), &items, selected);
        match clicked.or(enter.then_some(selected)) {
            _ if back => {
                self.unlock_menu = None;
                LockAction::None
            }
            Some(i) if i < UNLOCK_CHOICES.len() => LockAction::Unlock(UNLOCK_CHOICES[i].1),
            Some(_) => {
                self.unlock_menu = None;
                LockAction::None
            }
            None => LockAction::None,
        }
    }

    fn pin_pad(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen: Rect,
        s: f32,
        info: &LockInfo,
        nav: &Nav,
    ) -> LockAction {
        let disabled = self.checking || info.wait_secs > 0;
        let center = screen.center();

        // Entered digits as dots.
        let dots_y = screen.min.y + 300.0 * s;
        let n = info.pin_length.unwrap_or(self.entered.len().max(4));
        for i in 0..n {
            let x = center.x + (i as f32 - (n as f32 - 1.0) / 2.0) * 44.0 * s;
            let c = Pos2::new(x, dots_y);
            if i < self.entered.len() {
                painter.circle_filled(c, 12.0 * s, crate::ui::TEXT);
            } else {
                painter.circle_stroke(c, 12.0 * s, (2.0 * s, crate::ui::TEXT_DIM));
            }
        }
        let status = if self.checking {
            Some(tr!("Checking…").to_string())
        } else if info.wait_secs > 0 {
            Some(tr!(
                "Wrong PIN. Try again in {secs} s",
                secs = info.wait_secs
            ))
        } else {
            self.message.clone()
        };
        if let Some(text) = status {
            painter.text(
                Pos2::new(center.x, dots_y + 40.0 * s),
                Align2::CENTER_TOP,
                text,
                FontId::proportional(26.0 * s),
                Color32::from_rgb(0xff, 0xb4, 0x6a),
            );
        }

        // Keyboard and remote input.
        let mut press: Option<usize> = None;
        let mut typed: Vec<char> = Vec::new();
        let clear = ui.ctx().input(|i| {
            for (k, c) in [
                (Key::Num0, '0'),
                (Key::Num1, '1'),
                (Key::Num2, '2'),
                (Key::Num3, '3'),
                (Key::Num4, '4'),
                (Key::Num5, '5'),
                (Key::Num6, '6'),
                (Key::Num7, '7'),
                (Key::Num8, '8'),
                (Key::Num9, '9'),
            ] {
                if i.key_pressed(k) {
                    typed.push(c);
                }
            }
            i.key_pressed(Key::Escape)
        });
        // Esc clears everything; the other back keys (Backspace, remote) delete one digit.
        let back = nav.back && !clear;
        let enter = nav.enter;
        let dx = i32::from(nav.right) - i32::from(nav.left);
        let dy = i32::from(nav.down) - i32::from(nav.up);
        let (col, row) = ((self.focus % 3) as i32, (self.focus / 3) as i32);
        self.focus = (((row + dy).clamp(0, 3)) * 3 + (col + dx).clamp(0, 2)) as usize;

        // The pad.
        let key = 104.0 * s;
        let gap = 18.0 * s;
        let top = dots_y + 100.0 * s;
        let left = center.x - (3.0 * key + 2.0 * gap) / 2.0;
        for (i, label) in KEYS.iter().enumerate() {
            let rect = Rect::from_min_size(
                Pos2::new(
                    left + (i % 3) as f32 * (key + gap),
                    top + (i / 3) as f32 * (key + gap),
                ),
                Vec2::splat(key),
            );
            let focused = i == self.focus;
            let base = if i == OK {
                crate::ui::ACCENT.gamma_multiply(0.55)
            } else {
                Color32::from_rgb(0x2e, 0x33, 0x45)
            };
            let fill = if focused { crate::ui::ACCENT } else { base };
            let alpha = if disabled { 0.35 } else { 1.0 };
            painter.rect_filled(
                rect,
                CornerRadius::same((key / 2.0) as u8),
                fill.gamma_multiply(alpha),
            );
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(if i == OK { 30.0 } else { 40.0 } * s),
                crate::ui::TEXT.gamma_multiply(alpha),
            );
            if ui
                .interact(rect, ui.make_persistent_id(("pin-key", i)), Sense::click())
                .clicked()
            {
                self.focus = i;
                press = Some(i);
            }
        }
        if enter {
            press = Some(self.focus);
        }
        // Exiting: back with nothing entered cancels (also while waiting after wrong PINs).
        if self.purpose.is_some() && (back || clear) && self.entered.is_empty() {
            return LockAction::Cancel;
        }
        if disabled {
            return LockAction::None;
        }

        for c in typed {
            if self.entered.len() < info.pin_length.unwrap_or(MAX_DIGITS) {
                self.entered.push(c);
            }
            self.focus = OK; // Enter on a keyboard then submits.
            self.message = None;
        }
        if back {
            self.entered.pop();
        }
        if clear {
            self.entered.clear();
        }
        // Complete (known length): check right away, no OK needed.
        if press.is_none() && info.pin_length == Some(self.entered.len()) {
            self.checking = true;
            return LockAction::CheckPin(self.entered.clone());
        }
        match press {
            Some(BACKSPACE) => {
                self.entered.pop();
            }
            Some(OK) => {
                if self.entered.len() >= 4 {
                    self.checking = true;
                    return LockAction::CheckPin(self.entered.clone());
                }
                self.message = Some(tr!("The PIN has 4 to 8 digits").into());
            }
            Some(i) => {
                if self.entered.len() < info.pin_length.unwrap_or(MAX_DIGITS) {
                    self.entered.push_str(KEYS[i]);
                }
                self.message = None;
                if info.pin_length == Some(self.entered.len()) {
                    self.checking = true;
                    return LockAction::CheckPin(self.entered.clone());
                }
            }
            None => {}
        }
        LockAction::None
    }
}
