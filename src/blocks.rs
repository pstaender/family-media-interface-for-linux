//! Block Stacker: our own falling-block puzzle game (own name and look), with the timing and
//! rules of the classic hand-held version:
//!
//! - Field 10 × 18, one piece preview, no hold, no hard drop, no wall kicks (a rotation that
//!   doesn't fit doesn't happen). The game runs in frames of 1/59.73 s.
//! - Gravity by level (`GRAVITY`), soft drop one row per 3 frames, sideways auto-repeat after
//!   24 frames, then every 9; 2 frames before the next piece, 93 with a line clear.
//! - Points: 40 / 100 / 300 / 1200 × (level + 1), plus 1 per row of soft drop.
//! - Endless: the level rises after (start level × 10 + 10) lines, then every 10, up to 20.
//! - 25 lines: clear 25 lines, on top of height × 2 rows of random blocks.
//! - 2 players, side by side, same pieces: a double sends 1 row, a triple 2, four lines 4 (with
//!   a common hole). A round is won by clearing 30 lines or when the other player tops out;
//!   the first to win 4 rounds wins.
//!
//! `Player` / `Game` are pure and tested; `BlocksView` is the screen.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Vec2};

use crate::highscores::{self, Board, NameEntry, NameInput, Scores};
use crate::sound::Sound;
use crate::tr;

pub const W: i32 = 10;
pub const H: i32 = 18;
const FRAME: f32 = 1.0 / 59.73;
/// Frames per row, by level.
const GRAVITY: [u32; 21] = [
    53, 49, 45, 41, 37, 33, 28, 22, 17, 11, 10, 9, 8, 7, 6, 6, 5, 5, 4, 4, 3,
];
const SOFT_DROP: u32 = 3;
const DAS_FIRST: u32 = 24;
const DAS_REPEAT: u32 = 9;
const ENTRY_DELAY: u32 = 2;
/// With the entry delay: 93 frames.
const CLEAR_DELAY: u32 = 91;
const MAX_SCORE: u32 = 999_999;
const MAX_LEVEL: u32 = 20;
const LINES_GOAL: u32 = 25;
const VERSUS_LINES: u32 = 30;
const VERSUS_ROUNDS: u32 = 4;
const ROUND_PAUSE: u32 = 180;
/// Garbage rows' cell value (pieces are 1–7).
const GARBAGE: u8 = 8;

/// Cells of a piece in its 4 × 4 box, per rotation.
type Shape = [(i32, i32); 4];
const SHAPES: [&[Shape]; 7] = [
    // I
    &[
        [(0, 1), (1, 1), (2, 1), (3, 1)],
        [(1, 0), (1, 1), (1, 2), (1, 3)],
    ],
    // O
    &[[(1, 1), (2, 1), (1, 2), (2, 2)]],
    // T
    &[
        [(0, 1), (1, 1), (2, 1), (1, 2)],
        [(1, 0), (0, 1), (1, 1), (1, 2)],
        [(1, 1), (0, 2), (1, 2), (2, 2)],
        [(1, 0), (1, 1), (2, 1), (1, 2)],
    ],
    // J
    &[
        [(0, 1), (1, 1), (2, 1), (2, 2)],
        [(1, 0), (1, 1), (0, 2), (1, 2)],
        [(0, 1), (0, 2), (1, 2), (2, 2)],
        [(1, 0), (2, 0), (1, 1), (1, 2)],
    ],
    // L
    &[
        [(0, 1), (1, 1), (2, 1), (0, 2)],
        [(0, 0), (1, 0), (1, 1), (1, 2)],
        [(2, 1), (0, 2), (1, 2), (2, 2)],
        [(1, 0), (1, 1), (1, 2), (2, 2)],
    ],
    // S
    &[
        [(1, 1), (2, 1), (0, 2), (1, 2)],
        [(0, 0), (0, 1), (1, 1), (1, 2)],
    ],
    // Z
    &[
        [(0, 1), (1, 1), (1, 2), (2, 2)],
        [(1, 0), (0, 1), (1, 1), (0, 2)],
    ],
];

/// The shared random numbers and piece sequence (2 players get the same pieces).
#[derive(Debug, Clone)]
struct Sequence {
    rng: u64,
    pieces: Vec<u8>,
}

impl Sequence {
    fn new(seed: u64) -> Self {
        Self {
            rng: seed | 1,
            pieces: Vec::new(),
        }
    }

    fn random(&mut self, n: u32) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % u64::from(n)) as u32
    }

    /// Piece number `i`: random, with fewer direct repeats of the last two pieces.
    fn piece(&mut self, i: usize) -> u8 {
        while self.pieces.len() <= i {
            let last = self
                .pieces
                .iter()
                .rev()
                .take(2)
                .copied()
                .collect::<Vec<_>>();
            let mut p = self.random(7) as u8;
            for _ in 0..2 {
                if !last.contains(&p) {
                    break;
                }
                p = self.random(7) as u8;
            }
            self.pieces.push(p);
        }
        self.pieces[i]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Active {
    pub kind: u8,
    pub rot: usize,
    pub x: i32,
    pub y: i32,
}

impl Active {
    pub fn cells(&self) -> [(i32, i32); 4] {
        SHAPES[self.kind as usize][self.rot].map(|(cx, cy)| (self.x + cx, self.y + cy))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Falling,
    /// Full rows are shown for this many frames more.
    Clearing(u32),
    /// The next piece comes in this many frames.
    Entry(u32),
    Over,
    Won,
}

/// One player's input for one frame.
#[derive(Debug, Default, Clone, Copy)]
pub struct Held {
    pub left: bool,
    pub right: bool,
    pub down: bool,
    pub rotate_right: bool,
    pub rotate_left: bool,
}

#[derive(Debug, Clone)]
pub struct Player {
    pub grid: [[u8; W as usize]; H as usize],
    pub active: Option<Active>,
    pub next: u8,
    pub phase: Phase,
    pub level: u32,
    start_level: u32,
    pub lines: u32,
    pub score: u32,
    /// Index of the next piece in the sequence.
    index: usize,
    gravity: u32,
    das: u32,
    das_dir: i32,
    soft_rows: u32,
    /// Down must be released between pieces.
    down_armed: bool,
    pub clearing: Vec<usize>,
    /// Rows to send to the other player / holes of the rows coming in.
    outgoing: u32,
    incoming: Vec<i32>,
    pub sounds: Vec<Sound>,
}

impl Player {
    fn new(start_level: u32, garbage: &[[u8; W as usize]]) -> Self {
        let mut grid = [[0; W as usize]; H as usize];
        let top = H as usize - garbage.len();
        grid[top..].copy_from_slice(garbage);
        Self {
            grid,
            active: None,
            next: 0,
            phase: Phase::Entry(ENTRY_DELAY),
            level: start_level,
            start_level,
            lines: 0,
            score: 0,
            index: 0,
            gravity: 0,
            das: 0,
            das_dir: 0,
            soft_rows: 0,
            down_armed: true,
            clearing: Vec::new(),
            outgoing: 0,
            incoming: Vec::new(),
            sounds: Vec::new(),
        }
    }

    fn fits(&self, piece: &Active) -> bool {
        piece.cells().iter().all(|&(x, y)| {
            (0..W).contains(&x) && y < H && (y < 0 || self.grid[y as usize][x as usize] == 0)
        })
    }

    fn spawn(&mut self, seq: &mut Sequence) {
        let piece = Active {
            kind: seq.piece(self.index),
            rot: 0,
            x: 3,
            y: -1,
        };
        self.index += 1;
        self.next = seq.piece(self.index);
        self.gravity = 0;
        self.soft_rows = 0;
        self.down_armed = false;
        if self.fits(&piece) {
            self.active = Some(piece);
            self.phase = Phase::Falling;
        } else {
            self.active = None;
            self.phase = Phase::Over;
            self.sounds.push(Sound::GameOver);
        }
    }

    fn frame(&mut self, held: Held, seq: &mut Sequence) {
        match self.phase {
            Phase::Over | Phase::Won => {}
            Phase::Clearing(0) => {
                for &row in &self.clearing {
                    self.grid.copy_within(0..row, 1);
                    self.grid[0] = [0; W as usize];
                }
                self.clearing.clear();
                self.phase = Phase::Entry(ENTRY_DELAY);
            }
            Phase::Clearing(n) => self.phase = Phase::Clearing(n - 1),
            Phase::Entry(0) => {
                self.take_incoming();
                self.spawn(seq);
            }
            Phase::Entry(n) => self.phase = Phase::Entry(n - 1),
            Phase::Falling => self.fall(held),
        }
    }

    fn take_incoming(&mut self) {
        for hole in std::mem::take(&mut self.incoming) {
            self.grid.copy_within(1.., 0);
            let mut row = [GARBAGE; W as usize];
            row[hole as usize] = 0;
            self.grid[H as usize - 1] = row;
        }
    }

    fn fall(&mut self, held: Held) {
        let Some(mut piece) = self.active else { return };
        // Rotate: only if it fits, no wall kicks.
        let states = SHAPES[piece.kind as usize].len();
        for (pressed, step) in [(held.rotate_right, 1), (held.rotate_left, states - 1)] {
            if pressed && states > 1 {
                let turned = Active {
                    rot: (piece.rot + step) % states,
                    ..piece
                };
                if self.fits(&turned) {
                    piece = turned;
                    self.sounds.push(Sound::Wall);
                }
            }
        }
        // Sideways: at once, then after 24 frames every 9.
        let dir = i32::from(held.right) - i32::from(held.left);
        let moves = if dir != self.das_dir {
            self.das_dir = dir;
            self.das = 0;
            dir != 0
        } else if dir != 0 {
            self.das += 1;
            self.das >= DAS_FIRST && (self.das - DAS_FIRST).is_multiple_of(DAS_REPEAT)
        } else {
            false
        };
        if moves {
            let moved = Active {
                x: piece.x + dir,
                ..piece
            };
            if self.fits(&moved) {
                piece = moved;
            }
        }
        // Down: gravity, faster with a soft drop.
        if !held.down {
            self.down_armed = true;
            self.soft_rows = 0;
        }
        let soft = held.down && self.down_armed;
        self.gravity += 1;
        let speed = GRAVITY[self.level.min(MAX_LEVEL) as usize];
        let due = if soft { SOFT_DROP.min(speed) } else { speed };
        self.active = Some(piece);
        if self.gravity < due {
            return;
        }
        self.gravity = 0;
        let lower = Active {
            y: piece.y + 1,
            ..piece
        };
        if self.fits(&lower) {
            self.active = Some(lower);
            if soft {
                self.soft_rows += 1;
            }
        } else {
            self.lock(piece);
        }
    }

    fn lock(&mut self, piece: Active) {
        for (x, y) in piece.cells() {
            if y >= 0 {
                self.grid[y as usize][x as usize] = piece.kind + 1;
            }
        }
        self.active = None;
        self.score = (self.score + self.soft_rows).min(MAX_SCORE);
        self.soft_rows = 0;
        self.sounds.push(Sound::Paddle);
        // Top to bottom, so removing a row doesn't move the ones still to remove.
        self.clearing = (0..H as usize)
            .filter(|&y| self.grid[y].iter().all(|c| *c != 0))
            .collect();
        let n = self.clearing.len() as u32;
        if n == 0 {
            self.phase = Phase::Entry(ENTRY_DELAY);
            return;
        }
        self.score =
            (self.score + [40, 100, 300, 1200][n as usize - 1] * (self.level + 1)).min(MAX_SCORE);
        self.lines += n;
        self.outgoing = [0, 1, 2, 4][n as usize - 1];
        let first_rise = self.start_level * 10 + 10;
        if self.lines >= first_rise {
            self.level = (self.start_level + 1 + (self.lines - first_rise) / 10).min(MAX_LEVEL);
        }
        self.sounds.push(if n == 4 {
            Sound::ExtraLife
        } else {
            Sound::Cleared
        });
        self.phase = Phase::Clearing(CLEAR_DELAY);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Endless,
    Lines,
    Versus,
}

#[derive(Debug, Clone)]
pub struct Game {
    pub mode: Mode,
    pub players: Vec<Player>,
    pub wins: [u32; 2],
    /// Round finished: who won it (`None`: both topped out), frames until the next.
    pub round_end: Option<(Option<usize>, u32)>,
    pub match_winner: Option<usize>,
    start_level: u32,
    height: u32,
    seq: Sequence,
    carry: f32,
}

impl Game {
    pub fn new(mode: Mode, start_level: u32, height: u32, seed: u64) -> Self {
        let mut game = Self {
            mode,
            players: Vec::new(),
            wins: [0, 0],
            round_end: None,
            match_winner: None,
            start_level: start_level.min(9),
            height: height.min(5),
            seq: Sequence::new(seed),
            carry: 0.0,
        };
        game.start_round();
        game
    }

    /// Rows of random blocks at the bottom, the same for both players.
    fn start_round(&mut self) {
        let rows = if self.mode == Mode::Endless {
            0
        } else {
            self.height * 2
        };
        let mut garbage = Vec::new();
        for _ in 0..rows {
            let mut row = [0u8; W as usize];
            for cell in &mut row {
                if self.seq.random(100) < 60 {
                    *cell = GARBAGE;
                }
            }
            row[self.seq.random(W as u32) as usize] = 0; // never a full row
            garbage.push(row);
        }
        let count = if self.mode == Mode::Versus { 2 } else { 1 };
        self.players = (0..count)
            .map(|_| Player::new(self.start_level, &garbage))
            .collect();
        // A new round: new pieces, the same for both.
        self.seq.pieces.clear();
        self.round_end = None;
    }

    pub fn finished(&self) -> bool {
        match self.mode {
            Mode::Versus => self.match_winner.is_some(),
            _ => matches!(self.players[0].phase, Phase::Over | Phase::Won),
        }
    }

    /// Advance by `dt` seconds. Rotations in `held` count once, on the first frame.
    pub fn step(&mut self, dt: f32, held: [Held; 2]) {
        self.carry += dt.clamp(0.0, 0.1);
        let mut held = held;
        while self.carry >= FRAME {
            self.carry -= FRAME;
            self.frame(held);
            for h in &mut held {
                h.rotate_left = false;
                h.rotate_right = false;
            }
        }
    }

    pub fn frame(&mut self, held: [Held; 2]) {
        if self.finished() {
            return;
        }
        if let Some((winner, frames)) = self.round_end {
            if frames > 0 {
                self.round_end = Some((winner, frames - 1));
            } else if winner.is_some_and(|w| self.wins[w] >= VERSUS_ROUNDS) {
                self.match_winner = winner;
            } else {
                self.start_round();
            }
            return;
        }
        for (i, player) in self.players.iter_mut().enumerate() {
            player.frame(held[i], &mut self.seq);
        }
        match self.mode {
            Mode::Endless => {}
            Mode::Lines => {
                let p = &mut self.players[0];
                if p.lines >= LINES_GOAL && p.phase != Phase::Over {
                    p.phase = Phase::Won;
                    p.sounds.push(Sound::Cleared);
                }
            }
            Mode::Versus => self.versus(),
        }
    }

    fn versus(&mut self) {
        // Rows sent: the same hole for all of them in one batch.
        for from in 0..2 {
            let rows = std::mem::take(&mut self.players[from].outgoing);
            if rows > 0 {
                let hole = self.seq.random(W as u32) as i32;
                self.players[1 - from]
                    .incoming
                    .extend(std::iter::repeat_n(hole, rows as usize));
            }
        }
        let over = [0, 1].map(|i| self.players[i].phase == Phase::Over);
        let reached = [0, 1].map(|i| self.players[i].lines >= VERSUS_LINES);
        let winner = match (over, reached) {
            ([true, true], _) => None,
            ([true, false], _) | (_, [false, true]) => Some(1),
            ([false, true], _) | (_, [true, false]) => Some(0),
            (_, [true, true]) => None,
            _ => return,
        };
        if let Some(w) = winner {
            self.wins[w] += 1;
        }
        self.round_end = Some((winner, ROUND_PAUSE));
    }
}

// --------------------------------------------------------------------------------- screen

#[derive(Debug, Default, Clone)]
pub struct BlocksInput {
    pub held: [Held; 2],
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub enter: bool,
    pub back: bool,
    pub pause: bool,
    pub clicks: bool,
    pub typed: Vec<char>,
    pub erase: bool,
}

pub enum BlocksState {
    Menu { selected: usize },
    Playing(Game),
    Paused { game: Game, selected: usize },
    EnterName { game: Game, entry: NameEntry },
    Over { game: Game, selected: usize },
    Scores,
}

pub struct BlocksView {
    pub state: BlocksState,
    level: u32,
    lines_mode: bool,
    height: u32,
    pub sound: bool,
    players: usize,
    seed: u64,
    scores: Scores,
}

impl BlocksView {
    pub fn new(level: u32, sound: bool, seed: u64) -> Self {
        Self {
            state: BlocksState::Menu { selected: 0 },
            level: level.min(9),
            lines_mode: false,
            height: 0,
            sound,
            players: 1,
            seed,
            scores: Scores::load(),
        }
    }

    fn new_game(&mut self, players: usize) -> Game {
        self.players = players;
        self.seed = self
            .seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let mode = match (players, self.lines_mode) {
            (2, _) => Mode::Versus,
            (_, true) => Mode::Lines,
            _ => Mode::Endless,
        };
        Game::new(mode, self.level, self.height, self.seed)
    }

    fn sound_item(&self) -> &'static str {
        if self.sound {
            tr!("Sound: on")
        } else {
            tr!("Sound: off")
        }
    }

    /// True when the player wants back to the home screen.
    pub fn show(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen: Rect,
        s: f32,
        dt: f32,
        input: &BlocksInput,
    ) -> bool {
        painter.rect_filled(screen, 0.0, Color32::from_rgb(0x12, 0x10, 0x20));
        let menu_input = crate::pong::PongInput {
            up: input.up,
            down: input.down,
            enter: input.enter,
            back: input.back,
            pause: input.pause,
            clicks: input.clicks,
            ..Default::default()
        };
        let menu = |title: &str, items: &[&str], selected| {
            crate::pong::menu(ui, painter, screen, s, title, items, selected, &menu_input)
        };
        let state = std::mem::replace(&mut self.state, BlocksState::Menu { selected: 0 });
        let (next, quit) = match state {
            BlocksState::Menu { selected } => {
                let level = tr!("Start level: {n}", n = self.level);
                let goal = if self.lines_mode {
                    tr!("Goal: clear 25 lines")
                } else {
                    tr!("Goal: endless")
                };
                let height = tr!("Height: {n}", n = self.height);
                let items = [
                    tr!("1 player"),
                    tr!("2 players"),
                    &level,
                    goal,
                    &height,
                    self.sound_item(),
                    tr!("High scores"),
                    tr!("Back"),
                ];
                let (selected, chosen) = menu(tr!("Block Stacker"), &items, selected);
                let stay = BlocksState::Menu { selected };
                match chosen {
                    Some(0) => (BlocksState::Playing(self.new_game(1)), false),
                    Some(1) => (BlocksState::Playing(self.new_game(2)), false),
                    Some(2) => {
                        self.level = (self.level + 1) % 10;
                        (stay, false)
                    }
                    Some(3) => {
                        self.lines_mode = !self.lines_mode;
                        (stay, false)
                    }
                    Some(4) => {
                        self.height = (self.height + 1) % 6;
                        (stay, false)
                    }
                    Some(5) => {
                        self.sound = !self.sound;
                        (stay, false)
                    }
                    Some(6) => (BlocksState::Scores, false),
                    Some(_) => (stay, true),
                    None => (stay, input.back),
                }
            }
            BlocksState::Playing(mut game) => {
                game.step(dt, input.held);
                for player in &mut game.players {
                    for sound in player.sounds.drain(..) {
                        if self.sound {
                            crate::sound::play(sound);
                        }
                    }
                }
                paint_game(painter, screen, s, &game, self.scores.best(Board::Blocks));
                ui.ctx().request_repaint();
                if game.finished() {
                    let score = game.players[0].score;
                    if game.mode == Mode::Endless && self.scores.qualifies(Board::Blocks, score) {
                        let entry = self.scores.name_entry(score);
                        (BlocksState::EnterName { game, entry }, false)
                    } else {
                        (BlocksState::Over { game, selected: 0 }, false)
                    }
                } else if input.pause || input.back {
                    (BlocksState::Paused { game, selected: 0 }, false)
                } else {
                    (BlocksState::Playing(game), false)
                }
            }
            BlocksState::Paused { game, selected } => {
                // The field is hidden while paused, as a pause must not help.
                let items = [tr!("Continue"), self.sound_item(), tr!("End the game")];
                let (selected, chosen) = menu(tr!("Paused"), &items, selected);
                match chosen {
                    Some(0) => (BlocksState::Playing(game), false),
                    Some(1) => {
                        self.sound = !self.sound;
                        (BlocksState::Paused { game, selected }, false)
                    }
                    Some(_) => (BlocksState::Menu { selected: 0 }, false),
                    None if input.back || input.pause => (BlocksState::Playing(game), false),
                    None => (BlocksState::Paused { game, selected }, false),
                }
            }
            BlocksState::EnterName { game, mut entry } => {
                paint_game(painter, screen, s, &game, self.scores.best(Board::Blocks));
                entry.paint(painter, screen, s);
                let name_input = NameInput {
                    up: input.up,
                    down: input.down,
                    left: input.left,
                    right: input.right,
                    enter: input.enter,
                    typed: input.typed.clone(),
                    erase: input.erase,
                };
                if entry.update(&name_input) {
                    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                    self.scores
                        .add(Board::Blocks, &entry.name(), entry.score, &today);
                    (BlocksState::Scores, false)
                } else {
                    (BlocksState::EnterName { game, entry }, false)
                }
            }
            BlocksState::Over { game, selected } => {
                paint_game(painter, screen, s, &game, self.scores.best(Board::Blocks));
                crate::pong::dim(painter, screen);
                let title = match (game.mode, game.match_winner) {
                    (Mode::Versus, Some(0)) => tr!("Player 1 wins!").to_string(),
                    (Mode::Versus, _) => tr!("Player 2 wins!").to_string(),
                    (Mode::Lines, _) if game.players[0].phase == Phase::Won => {
                        tr!("25 lines cleared!").to_string()
                    }
                    _ => tr!("Game over · {score} points", score = game.players[0].score),
                };
                let items = [tr!("Play again"), tr!("Back")];
                let (selected, chosen) = menu(&title, &items, selected);
                match chosen {
                    Some(0) => {
                        let players = self.players;
                        (BlocksState::Playing(self.new_game(players)), false)
                    }
                    Some(_) => (BlocksState::Menu { selected: 0 }, false),
                    None if input.back => (BlocksState::Menu { selected: 0 }, false),
                    None => (BlocksState::Over { game, selected }, false),
                }
            }
            BlocksState::Scores => {
                highscores::paint_list(
                    painter,
                    screen,
                    s,
                    tr!("Block Stacker"),
                    self.scores.list(Board::Blocks),
                );
                if input.enter || input.back {
                    (BlocksState::Menu { selected: 0 }, false)
                } else {
                    (BlocksState::Scores, false)
                }
            }
        };
        self.state = next;
        quit
    }
}

/// Our palette: soft round tiles.
fn color(cell: u8) -> Color32 {
    match cell {
        1 => Color32::from_rgb(0x4f, 0xd1, 0xc5),
        2 => Color32::from_rgb(0xf6, 0xc4, 0x5a),
        3 => Color32::from_rgb(0xb3, 0x88, 0xff),
        4 => Color32::from_rgb(0x5b, 0x8d, 0xef),
        5 => Color32::from_rgb(0xf4, 0x8f, 0x5c),
        6 => Color32::from_rgb(0x7b, 0xd8, 0x6e),
        7 => Color32::from_rgb(0xef, 0x6f, 0x8f),
        _ => Color32::from_rgb(0x6a, 0x6f, 0x82),
    }
}

fn tile(painter: &egui::Painter, field: Rect, unit: f32, x: i32, y: i32, c: Color32) {
    let rect = Rect::from_min_size(
        field.min + Vec2::new(x as f32 * unit, y as f32 * unit),
        Vec2::splat(unit),
    )
    .shrink(unit * 0.06);
    painter.rect_filled(rect, CornerRadius::same((unit * 0.28) as u8), c);
}

fn paint_game(painter: &egui::Painter, screen: Rect, s: f32, game: &Game, best: u32) {
    let unit = ((screen.height() - 80.0 * s) / H as f32).min(screen.width() / 40.0);
    let size = Vec2::new(W as f32 * unit, H as f32 * unit);
    let versus = game.mode == Mode::Versus;
    let centers: Vec<f32> = if versus {
        vec![
            screen.center().x - size.x * 0.85,
            screen.center().x + size.x * 0.85,
        ]
    } else {
        vec![screen.center().x]
    };
    let text = |pos: Pos2, align, t: String, px: f32, c| {
        painter.text(pos, align, t, FontId::proportional(px * s), c);
    };
    for (i, player) in game.players.iter().enumerate() {
        let field = Rect::from_center_size(Pos2::new(centers[i], screen.center().y), size);
        painter.rect_filled(
            field,
            CornerRadius::same(6),
            Color32::from_rgb(0x1d, 0x1a, 0x30),
        );
        for (y, row) in player.grid.iter().enumerate() {
            let full = player.clearing.contains(&y);
            for (x, cell) in row.iter().enumerate() {
                if *cell != 0 {
                    let c = if full { Color32::WHITE } else { color(*cell) };
                    tile(painter, field, unit, x as i32, y as i32, c);
                }
            }
        }
        if let Some(piece) = player.active {
            for (x, y) in piece.cells() {
                if y >= 0 {
                    tile(painter, field, unit, x, y, color(piece.kind + 1));
                }
            }
        }
        // Next piece and numbers: outside of each field (2 players: towards the middle
        // for player 1's right side, outer side for the numbers).
        let right = Pos2::new(field.max.x + 30.0 * s, field.min.y + 10.0 * s);
        let (dim, bright) = (crate::ui::TEXT_DIM, crate::ui::TEXT);
        text(right, Align2::LEFT_TOP, tr!("Next piece").into(), 24.0, dim);
        let preview =
            Rect::from_min_size(right + Vec2::new(0.0, 36.0 * s), Vec2::splat(unit * 3.2));
        if player.phase != Phase::Over {
            for (x, y) in SHAPES[player.next as usize][0] {
                let small = unit * 0.75;
                tile(painter, preview, small, x, y, color(player.next + 1));
            }
        }
        let numbers = [
            (tr!("Points").to_string(), player.score.to_string()),
            (tr!("Level").to_string(), player.level.to_string()),
            (tr!("Lines").to_string(), player.lines.to_string()),
        ];
        for (n, (label, value)) in numbers.iter().enumerate() {
            let y = 200.0 * s + n as f32 * 96.0 * s;
            text(
                right + Vec2::new(0.0, y),
                Align2::LEFT_TOP,
                label.clone(),
                24.0,
                dim,
            );
            text(
                right + Vec2::new(0.0, y + 28.0 * s),
                Align2::LEFT_TOP,
                value.clone(),
                38.0,
                bright,
            );
        }
        if game.mode == Mode::Endless {
            let y = 200.0 * s + 3.0 * 96.0 * s;
            text(
                right + Vec2::new(0.0, y),
                Align2::LEFT_TOP,
                tr!("Best").into(),
                24.0,
                dim,
            );
            text(
                right + Vec2::new(0.0, y + 28.0 * s),
                Align2::LEFT_TOP,
                best.max(player.score).to_string(),
                32.0,
                bright,
            );
        }
        if versus {
            text(
                Pos2::new(field.center().x, field.min.y - 4.0 * s),
                Align2::CENTER_BOTTOM,
                tr!("Player {n}: {wins} of 4", n = i + 1, wins = game.wins[i]),
                24.0,
                bright,
            );
        }
    }
    if let Some((winner, _)) = game.round_end {
        let t = match winner {
            Some(0) => tr!("Player 1 wins the round").to_string(),
            Some(_) => tr!("Player 2 wins the round").to_string(),
            None => tr!("Nobody wins this round").to_string(),
        };
        text(
            screen.center(),
            Align2::CENTER_CENTER,
            t,
            48.0,
            crate::ui::TEXT,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Held = Held {
        left: false,
        right: false,
        down: false,
        rotate_right: false,
        rotate_left: false,
    };

    fn run(p: &mut Player, seq: &mut Sequence, frames: u32, held: Held) {
        for _ in 0..frames {
            p.frame(held, seq);
        }
    }

    /// A player with a piece in play.
    fn falling(kind: u8, level: u32) -> (Player, Sequence) {
        let mut seq = Sequence::new(7);
        seq.pieces.push(kind);
        let mut p = Player::new(level, &[]);
        run(&mut p, &mut seq, ENTRY_DELAY + 1, NONE);
        assert_eq!(p.phase, Phase::Falling);
        (p, seq)
    }

    #[test]
    fn shapes_have_the_right_rotation_counts() {
        let counts: Vec<_> = SHAPES.iter().map(|s| s.len()).collect();
        assert_eq!(counts, [2, 1, 4, 4, 4, 2, 2]);
        for shape in SHAPES.iter().flat_map(|s| s.iter()) {
            for (x, y) in shape {
                assert!((0..4).contains(x) && (0..4).contains(y));
            }
        }
    }

    #[test]
    fn gravity_follows_the_level() {
        for (level, frames) in [(0, 53), (9, 11), (20, 3)] {
            let (mut p, mut seq) = falling(1, level);
            let y = p.active.unwrap().y;
            run(&mut p, &mut seq, frames - 1, NONE);
            assert_eq!(p.active.unwrap().y, y, "level {level}");
            run(&mut p, &mut seq, 1, NONE);
            assert_eq!(p.active.unwrap().y, y + 1, "level {level}");
        }
    }

    #[test]
    fn sideways_auto_repeat_is_24_then_9_frames() {
        let (mut p, mut seq) = falling(1, 0);
        let x = p.active.unwrap().x;
        let right = Held {
            right: true,
            ..NONE
        };
        run(&mut p, &mut seq, 1, right);
        assert_eq!(p.active.unwrap().x, x + 1, "at once");
        run(&mut p, &mut seq, 23, right);
        assert_eq!(p.active.unwrap().x, x + 1);
        run(&mut p, &mut seq, 1, right);
        assert_eq!(p.active.unwrap().x, x + 2, "after 24 frames");
        run(&mut p, &mut seq, 8, right);
        assert_eq!(p.active.unwrap().x, x + 2);
        run(&mut p, &mut seq, 1, right);
        assert_eq!(p.active.unwrap().x, x + 3, "then every 9");
    }

    #[test]
    fn rotation_without_wall_kicks() {
        let (mut p, mut seq) = falling(0, 0); // I, horizontal
        // Vertical, against the left wall; turning back would stick out: nothing happens.
        let turn = Held {
            rotate_right: true,
            ..NONE
        };
        run(&mut p, &mut seq, 1, turn);
        assert_eq!(p.active.unwrap().rot, 1);
        let mut piece = p.active.unwrap();
        piece.x = -1; // column 0
        piece.y = 3;
        p.active = Some(piece);
        assert!(p.fits(&piece));
        run(&mut p, &mut seq, 1, turn);
        assert_eq!(p.active.unwrap().rot, 1, "no wall kick");
        // O doesn't rotate.
        let (mut o, mut seq) = falling(1, 0);
        run(&mut o, &mut seq, 1, turn);
        assert_eq!(o.active.unwrap().rot, 0);
    }

    fn fill_rows(p: &mut Player, rows: std::ops::Range<usize>, hole: usize) {
        for y in rows {
            p.grid[y] = [1; W as usize];
            p.grid[y][hole] = 0;
        }
    }

    /// Drop a vertical I into column `x` by placing it just above the floor.
    fn drop_i(p: &mut Player, seq: &mut Sequence, x: i32, y: i32) {
        p.active = Some(Active {
            kind: 0,
            rot: 1,
            x: x - 1,
            y,
        });
        p.gravity = 0;
        let speed = GRAVITY[p.level as usize];
        run(p, seq, speed, NONE);
    }

    #[test]
    fn lines_score_by_level_and_clear() {
        for (rows, points) in [(1usize, 40u32), (2, 100), (3, 300), (4, 1200)] {
            let (mut p, mut seq) = falling(1, 3);
            fill_rows(&mut p, H as usize - rows..H as usize, 0);
            // Floor at row 17: a vertical I occupies rows y..y+3; only `rows` of them count.
            drop_i(&mut p, &mut seq, 0, H - 4);
            assert_eq!(p.score, points * 4, "{rows} rows at level 3");
            assert_eq!(p.lines, rows as u32);
            assert_eq!(p.phase, Phase::Clearing(CLEAR_DELAY));
            run(&mut p, &mut seq, CLEAR_DELAY + 1, NONE);
            assert!(p.clearing.is_empty());
            // The full rows are gone; the rest of the I moved down.
            assert_eq!(p.grid[H as usize - 1][1], 0, "cleared");
        }
    }

    #[test]
    fn line_clear_takes_93_frames_with_the_entry_delay() {
        let (mut p, mut seq) = falling(1, 0);
        fill_rows(&mut p, H as usize - 1..H as usize, 0);
        drop_i(&mut p, &mut seq, 0, H - 4);
        let mut frames = 0;
        while p.phase != Phase::Falling {
            run(&mut p, &mut seq, 1, NONE);
            frames += 1;
        }
        assert_eq!(frames, CLEAR_DELAY + ENTRY_DELAY + 2); // counting down from n reaches 0 too
    }

    #[test]
    fn levels_rise_like_the_original() {
        let (mut p, mut seq) = falling(1, 0);
        p.lines = 9;
        fill_rows(&mut p, H as usize - 1..H as usize, 0);
        drop_i(&mut p, &mut seq, 0, H - 4);
        assert_eq!((p.lines, p.level), (10, 1));
        // Starting at 5: the first rise after 60 lines, not 10.
        let (mut p, mut seq) = falling(1, 5);
        p.lines = 10;
        fill_rows(&mut p, H as usize - 1..H as usize, 0);
        drop_i(&mut p, &mut seq, 0, H - 4);
        assert_eq!(p.level, 5);
        let (mut p, mut seq) = falling(1, 5);
        p.lines = 69;
        fill_rows(&mut p, H as usize - 1..H as usize, 0);
        drop_i(&mut p, &mut seq, 0, H - 4);
        assert_eq!(p.level, 7);
    }

    #[test]
    fn soft_drop_counts_rows_and_needs_a_new_press_per_piece() {
        let (mut p, mut seq) = falling(1, 0);
        let down = Held { down: true, ..NONE };
        // Down held since before this piece: doesn't count until released.
        let y = p.active.unwrap().y;
        run(&mut p, &mut seq, 3, down);
        assert_eq!(p.active.unwrap().y, y);
        run(&mut p, &mut seq, 1, NONE);
        run(&mut p, &mut seq, 3, down);
        assert_eq!(p.active.unwrap().y, y + 1);
        // All the way down: one point per row.
        run(&mut p, &mut seq, 200, down);
        assert_eq!(
            p.score,
            17 - 1,
            "O from row 1 to the floor, soft all the way"
        );
    }

    #[test]
    fn topping_out_ends_the_game() {
        let mut seq = Sequence::new(3);
        let mut p = Player::new(0, &[]);
        for x in 0..W as usize {
            p.grid[0][x] = 1;
            p.grid[1][x] = 1;
        }
        run(&mut p, &mut seq, ENTRY_DELAY + 1, NONE);
        assert_eq!(p.phase, Phase::Over);
    }

    #[test]
    fn twenty_five_lines_mode_starts_with_height_and_wins() {
        let mut g = Game::new(Mode::Lines, 0, 3, 11);
        let garbage = g.players[0].grid[12..]
            .iter()
            .filter(|r| r.iter().any(|c| *c != 0))
            .count();
        assert_eq!(garbage, 6);
        assert!(
            g.players[0].grid.iter().all(|r| r.contains(&0)),
            "no full row"
        );
        g.players[0].lines = 25;
        g.frame([NONE, NONE]);
        assert_eq!(g.players[0].phase, Phase::Won);
        assert!(g.finished());
    }

    #[test]
    fn two_players_get_the_same_pieces_and_send_rows() {
        let mut g = Game::new(Mode::Versus, 0, 2, 5);
        for _ in 0..5 {
            g.frame([NONE, NONE]);
        }
        let kinds = |g: &Game| {
            g.players
                .iter()
                .map(|p| p.active.unwrap().kind)
                .collect::<Vec<_>>()
        };
        let k = kinds(&g);
        assert_eq!(k[0], k[1]);
        assert_eq!(g.players[0].next, g.players[1].next);
        assert_eq!(g.players[0].grid, g.players[1].grid, "same starting blocks");
        // Four lines send 4 rows, with one hole.
        g.players[0].outgoing = 4;
        g.versus();
        assert_eq!(g.players[1].incoming.len(), 4);
        assert!(g.players[1].incoming.windows(2).all(|w| w[0] == w[1]));
        g.players[1].take_incoming();
        let bottom = g.players[1].grid[H as usize - 1];
        assert_eq!(bottom.iter().filter(|c| **c == 0).count(), 1);
        assert_eq!(bottom.iter().filter(|c| **c == GARBAGE).count(), 9);
    }

    #[test]
    fn rounds_are_won_by_lines_or_topping_out_first_to_four() {
        let mut g = Game::new(Mode::Versus, 0, 0, 5);
        for round in 1..=4 {
            g.frame([NONE, NONE]);
            g.players[0].lines = VERSUS_LINES;
            g.versus();
            assert_eq!(g.wins, [round, 0]);
            assert_eq!(g.round_end.map(|r| r.0), Some(Some(0)));
            for _ in 0..=ROUND_PAUSE + 1 {
                g.frame([NONE, NONE]);
            }
        }
        assert_eq!(g.match_winner, Some(0));
        assert!(g.finished());
        let mut g = Game::new(Mode::Versus, 0, 0, 5);
        g.players[0].phase = Phase::Over;
        g.versus();
        assert_eq!(g.wins, [0, 1]);
        // Both at once: nobody.
        let mut g = Game::new(Mode::Versus, 0, 0, 5);
        g.players[0].phase = Phase::Over;
        g.players[1].phase = Phase::Over;
        g.versus();
        assert_eq!(g.wins, [0, 0]);
        assert_eq!(g.round_end.map(|r| r.0), Some(None));
    }

    #[test]
    fn a_whole_game_runs_without_going_off_the_field() {
        let mut g = Game::new(Mode::Endless, 9, 0, 99);
        let mut held = NONE;
        for f in 0..30_000u32 {
            held.left = f % 97 < 20;
            held.right = f % 131 < 25;
            held.down = f % 11 < 8;
            held.rotate_right = f % 17 == 0;
            g.frame([held, NONE]);
            if let Some(p) = g.players[0].active {
                for (x, y) in p.cells() {
                    assert!((0..W).contains(&x) && y < H);
                }
            }
            if g.finished() {
                return;
            }
        }
    }
}
