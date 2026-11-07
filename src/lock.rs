//! The lock screen: shown when the time is up, outside the allowed time or when a parent
//! locked the media center. With a PIN set, a number pad (remote: arrows + OK; keyboard:
//! digits, Backspace, Enter) unlocks it for 30 minutes, an hour or the rest of the day.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, Sense, Vec2};

use crate::limits::{self, LockReason};

/// Number pad: 1–9, then ⌫, 0, OK.
const KEYS: [&str; 12] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "⌫", "0", "OK"];
const BACKSPACE: usize = 9;
const OK: usize = 11;
const MAX_DIGITS: usize = 8;

/// Unlock choices after a correct PIN: minutes (`None` = rest of today).
pub const UNLOCK_CHOICES: [(&str, Option<u32>); 3] = [
    ("30 more minutes", Some(30)),
    ("1 more hour", Some(60)),
    ("Rest of today", None),
];

pub struct LockView {
    pub reason: LockReason,
    entered: String,
    focus: usize,
    pub checking: bool,
    pub message: Option<String>,
    /// After a correct PIN: the selected unlock choice (`UNLOCK_CHOICES`, then "Cancel").
    pub unlock_menu: Option<usize>,
}

/// What the lock screen wants the app to do.
pub enum LockAction {
    None,
    /// Check this PIN (in the background).
    CheckPin(String),
    /// Unlock for minutes / rest of today.
    Unlock(Option<u32>),
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
        }
    }

    pub fn pin_checked(&mut self, ok: bool) {
        self.checking = false;
        self.entered.clear();
        if ok {
            self.message = None;
            self.unlock_menu = Some(0);
        } else {
            self.message = Some("Wrong PIN".into());
        }
    }

    pub fn show(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen: Rect,
        s: f32,
        info: &LockInfo,
    ) -> LockAction {
        painter.rect_filled(screen, 0.0, Color32::from_rgb(0x0c, 0x0e, 0x14));
        let center = screen.center();
        let (title, color) = match self.reason {
            LockReason::Parent => ("Locked", crate::ui::TEXT),
            LockReason::BudgetUsed => ("Time is up", crate::ui::TEXT),
            LockReason::OutsideWindow { .. } => ("Not now", crate::ui::TEXT),
        };
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
            limits::reason_text(&self.reason),
            FontId::proportional(30.0 * s),
            crate::ui::TEXT_DIM,
        );

        if let Some(selected) = self.unlock_menu {
            return self.unlock_menu(ui, painter, screen, s, selected);
        }
        if !info.has_pin {
            painter.text(
                center,
                Align2::CENTER_CENTER,
                "A parent can unlock it in the admin panel.",
                FontId::proportional(30.0 * s),
                crate::ui::TEXT,
            );
            return LockAction::None;
        }
        self.pin_pad(ui, painter, screen, s, info)
    }

    fn unlock_menu(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen: Rect,
        s: f32,
        mut selected: usize,
    ) -> LockAction {
        let (up, down, enter, back) = ui.ctx().input(|i| {
            (
                i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::ArrowRight),
                i.key_pressed(Key::Enter) || i.key_pressed(Key::Space),
                i.key_pressed(Key::Escape) || i.key_pressed(Key::Backspace),
            )
        });
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
            .map(|(label, _)| crate::ui::MenuItem::new(*label, ""))
            .collect();
        items.push(crate::ui::MenuItem::new("Cancel", ""));
        let clicked =
            crate::ui::paint_menu(ui, painter, screen, s, "Unlock for…", &items, selected);
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
            Some("Checking…".to_string())
        } else if info.wait_secs > 0 {
            Some(format!("Wrong PIN. Try again in {} s", info.wait_secs))
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
        let (back, clear, enter, dx, dy) = ui.ctx().input(|i| {
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
            (
                i.key_pressed(Key::Backspace),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Enter) || i.key_pressed(Key::Space),
                i32::from(i.key_pressed(Key::ArrowRight))
                    - i32::from(i.key_pressed(Key::ArrowLeft)),
                i32::from(i.key_pressed(Key::ArrowDown)) - i32::from(i.key_pressed(Key::ArrowUp)),
            )
        });
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
                self.message = Some("The PIN has 4 to 8 digits".into());
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
