//! High score lists of the games (top 10 per game, with the player's name), kept in
//! `$XDG_DATA_HOME/curator/highscores.json`, and the name entry: up / down change the letter,
//! left / right the position, OK confirms; on a keyboard, just type.

use std::collections::BTreeMap;
use std::path::Path;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Vec2};
use serde::{Deserialize, Serialize};

use crate::store;
use crate::tr;

pub const MAX_ENTRIES: usize = 10;
pub const NAME_LEN: usize = 8;
const LETTERS: &str = " ABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÜ0123456789";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Board {
    Bricks,
    Blocks,
}

impl Board {
    fn key(self) -> &'static str {
        match self {
            Board::Bricks => "bricks",
            Board::Blocks => "blocks",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub score: u32,
    /// `YYYY-MM-DD`.
    pub date: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scores {
    boards: BTreeMap<String, Vec<Entry>>,
    /// The name entered last: the next entry starts with it.
    last_name: String,
}

impl Scores {
    pub fn load() -> Self {
        store::highscores_path()
            .and_then(|p| store::load(&p))
            .unwrap_or_default()
    }

    fn save(&self, path: Option<&Path>) {
        if let Some(path) = path
            && let Err(e) = store::save(path, self)
        {
            log::warn!("could not save the high scores: {e}");
        }
    }

    pub fn list(&self, board: Board) -> &[Entry] {
        self.boards.get(board.key()).map_or(&[], Vec::as_slice)
    }

    pub fn best(&self, board: Board) -> u32 {
        self.list(board).first().map_or(0, |e| e.score)
    }

    /// Does this score get onto the list?
    pub fn qualifies(&self, board: Board, score: u32) -> bool {
        let list = self.list(board);
        score > 0 && (list.len() < MAX_ENTRIES || list.last().is_some_and(|e| score > e.score))
    }

    /// Add a score (a later equal score goes below the earlier one) and save.
    pub fn add(&mut self, board: Board, name: &str, score: u32, date: &str) {
        self.insert(board, name, score, date);
        self.save(store::highscores_path().as_deref());
    }

    fn insert(&mut self, board: Board, name: &str, score: u32, date: &str) {
        let name = clean_name(name);
        self.last_name = name.clone();
        let list = self.boards.entry(board.key().to_string()).or_default();
        let at = list
            .iter()
            .position(|e| e.score < score)
            .unwrap_or(list.len());
        list.insert(
            at,
            Entry {
                name,
                score,
                date: date.to_string(),
            },
        );
        list.truncate(MAX_ENTRIES);
    }

    pub fn name_entry(&self, score: u32) -> NameEntry {
        NameEntry::new(&self.last_name, score)
    }
}

/// At most `NAME_LEN` of our letters; `?` if nothing is left.
fn clean_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .flat_map(char::to_uppercase)
        .filter(|c| LETTERS.contains(*c))
        .take(NAME_LEN)
        .collect();
    match clean.trim() {
        "" => "?".to_string(),
        name => name.to_string(),
    }
}

/// Input for the name entry, one frame.
#[derive(Debug, Default, Clone)]
pub struct NameInput {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub enter: bool,
    /// Letters typed on a keyboard.
    pub typed: Vec<char>,
    /// Backspace on a keyboard.
    pub erase: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameEntry {
    letters: Vec<char>,
    at: usize,
    pub score: u32,
}

impl NameEntry {
    fn new(last: &str, score: u32) -> Self {
        let mut letters: Vec<char> = last.chars().filter(|c| LETTERS.contains(*c)).collect();
        letters.resize(NAME_LEN, ' ');
        letters.truncate(NAME_LEN);
        Self {
            letters,
            at: 0,
            score,
        }
    }

    pub fn name(&self) -> String {
        clean_name(&self.letters.iter().collect::<String>())
    }

    /// True when the name is confirmed.
    pub fn update(&mut self, input: &NameInput) -> bool {
        let all: Vec<char> = LETTERS.chars().collect();
        let index = all
            .iter()
            .position(|c| *c == self.letters[self.at])
            .unwrap_or(0);
        if input.up {
            self.letters[self.at] = all[(index + 1) % all.len()];
        }
        if input.down {
            self.letters[self.at] = all[(index + all.len() - 1) % all.len()];
        }
        if input.left {
            self.at = self.at.saturating_sub(1);
        }
        if input.right {
            self.at = (self.at + 1).min(NAME_LEN - 1);
        }
        for c in input.typed.iter().flat_map(|c| c.to_uppercase()) {
            if LETTERS.contains(c) {
                self.letters[self.at] = c;
                self.at = (self.at + 1).min(NAME_LEN - 1);
            }
        }
        if input.erase {
            if self.letters[self.at] == ' ' {
                self.at = self.at.saturating_sub(1);
            }
            self.letters[self.at] = ' ';
        }
        input.enter
    }

    pub fn paint(&self, painter: &egui::Painter, screen: Rect, s: f32) {
        let center = screen.center();
        painter.rect_filled(screen, 0.0, Color32::from_black_alpha(215));
        painter.text(
            Pos2::new(center.x, center.y - 230.0 * s),
            Align2::CENTER_CENTER,
            tr!("New high score: {score}", score = self.score),
            FontId::proportional(56.0 * s),
            crate::ui::TEXT,
        );
        painter.text(
            Pos2::new(center.x, center.y - 140.0 * s),
            Align2::CENTER_CENTER,
            tr!("Enter your name"),
            FontId::proportional(32.0 * s),
            crate::ui::TEXT_DIM,
        );
        let (w, gap) = (78.0 * s, 14.0 * s);
        let left = center.x - (NAME_LEN as f32 * (w + gap) - gap) / 2.0;
        for (i, letter) in self.letters.iter().enumerate() {
            let rect = Rect::from_min_size(
                Pos2::new(left + i as f32 * (w + gap), center.y - 60.0 * s),
                Vec2::new(w, 110.0 * s),
            );
            let fill = if i == self.at {
                crate::ui::ACCENT
            } else {
                Color32::from_rgb(0x22, 0x26, 0x34)
            };
            painter.rect_filled(rect, CornerRadius::same((12.0 * s) as u8), fill);
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                letter.to_string(),
                FontId::proportional(60.0 * s),
                crate::ui::TEXT,
            );
        }
        painter.text(
            Pos2::new(center.x, center.y + 130.0 * s),
            Align2::CENTER_CENTER,
            tr!("Up / down: letter · left / right: position · OK: done"),
            FontId::proportional(26.0 * s),
            crate::ui::TEXT_DIM,
        );
    }
}

/// The list of a game.
pub fn paint_list(painter: &egui::Painter, screen: Rect, s: f32, title: &str, list: &[Entry]) {
    let center = screen.center();
    painter.rect_filled(screen, 0.0, Color32::from_rgb(0x0b, 0x10, 0x1a));
    painter.text(
        Pos2::new(center.x, screen.min.y + 120.0 * s),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(60.0 * s),
        crate::ui::TEXT,
    );
    if list.is_empty() {
        painter.text(
            center,
            Align2::CENTER_CENTER,
            tr!("No high scores yet"),
            FontId::proportional(34.0 * s),
            crate::ui::TEXT_DIM,
        );
    }
    let font = FontId::proportional(38.0 * s);
    for (i, entry) in list.iter().enumerate() {
        let y = screen.min.y + 220.0 * s + i as f32 * 66.0 * s;
        let color = if i == 0 {
            Color32::from_rgb(0xff, 0xe0, 0x6a)
        } else {
            crate::ui::TEXT
        };
        let at = |dx: f32| Pos2::new(center.x + dx * s, y);
        painter.text(
            at(-400.0),
            Align2::RIGHT_CENTER,
            format!("{}.", i + 1),
            font.clone(),
            color,
        );
        painter.text(
            at(-370.0),
            Align2::LEFT_CENTER,
            &entry.name,
            font.clone(),
            color,
        );
        painter.text(
            at(190.0),
            Align2::RIGHT_CENTER,
            entry.score.to_string(),
            font.clone(),
            color,
        );
        painter.text(
            at(400.0),
            Align2::RIGHT_CENTER,
            &entry.date,
            FontId::proportional(26.0 * s),
            crate::ui::TEXT_DIM,
        );
    }
    painter.text(
        Pos2::new(center.x, screen.max.y - 60.0 * s),
        Align2::CENTER_CENTER,
        tr!("OK or back: continue"),
        FontId::proportional(26.0 * s),
        crate::ui::TEXT_DIM,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_best_ten_in_order() {
        let mut s = Scores::default();
        assert!(!s.qualifies(Board::Bricks, 0));
        for score in 1..=12 {
            assert!(s.qualifies(Board::Bricks, score * 10));
            s.insert(Board::Bricks, "ann", score * 10, "2026-11-08");
        }
        let list = s.list(Board::Bricks);
        assert_eq!(list.len(), MAX_ENTRIES);
        assert_eq!(list[0].score, 120);
        assert_eq!(list[9].score, 30);
        assert!(!s.qualifies(Board::Bricks, 30), "must beat the last one");
        assert!(s.qualifies(Board::Bricks, 31));
        assert!(s.list(Board::Blocks).is_empty(), "one list per game");
        assert_eq!(s.best(Board::Bricks), 120);
        // Equal scores: the earlier one stays above.
        s.insert(Board::Bricks, "ben", 120, "2026-11-09");
        assert_eq!(s.list(Board::Bricks)[0].name, "ANN");
        assert_eq!(s.list(Board::Bricks)[1].name, "BEN");
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  max "), "MAX");
        assert_eq!(clean_name("Jürgen-Maximilian"), "JÜRGENMA");
        assert_eq!(clean_name("<>"), "?");
        assert_eq!(clean_name(""), "?");
    }

    #[test]
    fn name_entry_with_a_remote_and_a_keyboard() {
        let mut s = Scores::default();
        s.insert(Board::Blocks, "ANN", 5, "2026-11-08");
        let mut e = s.name_entry(99);
        assert_eq!(e.name(), "ANN", "starts with the last name");
        let press = |up, right| NameInput {
            up,
            right,
            ..Default::default()
        };
        assert!(!e.update(&press(true, false))); // A → B
        assert_eq!(e.name(), "BNN");
        e.update(&press(false, true));
        e.update(&NameInput {
            down: true,
            ..Default::default()
        }); // N → M
        assert_eq!(e.name(), "BMN");
        // Keyboard: typing replaces and moves on, backspace removes.
        let mut k = NameEntry::new("", 1);
        k.update(&NameInput {
            typed: vec!['t', 'i', 'm'],
            ..Default::default()
        });
        assert_eq!(k.name(), "TIM");
        k.update(&NameInput {
            erase: true,
            ..Default::default()
        });
        assert_eq!(k.name(), "TI");
        assert!(k.update(&NameInput {
            enter: true,
            ..Default::default()
        }));
        // Letters wrap around.
        let mut w = NameEntry::new("", 1);
        w.update(&NameInput {
            down: true,
            ..Default::default()
        });
        assert_eq!(w.name(), "9");
    }

    #[test]
    fn file_format_roundtrip() {
        let mut s = Scores::default();
        s.insert(Board::Blocks, "ANN", 5, "2026-11-08");
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Scores>(&json).unwrap(), s);
        assert_eq!(
            serde_json::from_str::<Scores>("{}").unwrap(),
            Scores::default()
        );
    }
}
