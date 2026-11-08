//! Brick Breaker: a paddle, a ball and walls of bricks (our own game; its rules follow the
//! classic hand-held brick games):
//!
//! - The paddle moves left / right; holding a button makes it faster. Before each ball the
//!   paddle can be placed; the ball then comes at 45° towards the paddle's middle.
//! - The ball flies only at 15°, 30° or 45°: the middle of the paddle sends it up steeply,
//!   the ends flat. A brick sends it back at the same angle. The paddle's corner lets it fall.
//!   Changing the paddle's direction right when it hits ("snap") makes the ball faster.
//! - Bricks: light 1, medium 2, dark 3 points; medium and dark ones make the ball faster.
//!   Unbreakable bricks from stage 13 on. From stage 5 on, touching the ceiling halves the
//!   paddle until the stage is cleared or a ball is lost.
//! - 8 patterns, each played three times: still, moving sideways, moving down a row with every
//!   paddle hit (bricks that come too low disappear without points). After every three stages
//!   a bonus stage: the ball goes through the bricks, a lost ball costs nothing, a timer runs,
//!   and clearing it gives bonus points. 32 stages, then it starts over.
//! - 5 paddles; one more every 1000 points up to 10 000 (at most 99). The high scores are kept
//!   on this computer, with the players' names (`highscores`).
//! - A ball that keeps bouncing without hitting a brick gets another angle.
//!
//! The rules (`Game`) are pure and tested; `BricksView` turns input into paddle movement,
//! shows menus and paints.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Vec2};

use crate::highscores::{self, Board, NameEntry, NameInput, Scores};
use crate::sound::Sound;
use crate::tr;

pub const W: f32 = 13.0;
pub const H: f32 = 15.0;
const BRICK_H: f32 = 0.5;
/// Top of the first brick row.
const TOP: f32 = 1.5;
const BALL_R: f32 = 0.15;
const PADDLE_Y: f32 = H - 1.0;
const PADDLE_H: f32 = 0.3;
const PADDLE_W: f32 = 2.0;
const PADDLE_SPEED: f32 = 9.0;
const PADDLE_FAST: f32 = 16.0;
/// Ball speeds (units/s): normal, after medium bricks, after dark bricks, fastest.
const SPEEDS: [f32; 4] = [6.0, 7.5, 9.0, 10.5];
/// 15°, 30°, 45°.
const ANGLES: [f32; 3] = [
    std::f32::consts::FRAC_PI_6 / 2.0,
    std::f32::consts::FRAC_PI_6,
    std::f32::consts::FRAC_PI_4,
];
const SERVE_DELAY: f32 = 1.5;
const STAGE_PAUSE: f32 = 2.0;
/// Changing the paddle's direction this shortly before it hits the ball: a "snap".
const SNAP: f32 = 0.15;
/// Paddle hits without hitting a brick before the ball gets another angle.
const LOOP_HITS: u32 = 6;
/// Paddles at the start (setting `games.bricks.lives`, 1–9).
pub const START_LIVES: u32 = 5;
pub const MAX_LIVES: u32 = 99;
/// Paddle icons drawn at most (two rows); the label shows the real number.
const MAX_LIFE_ICONS: u32 = 10;
/// Bricks that move down this close above the paddle disappear.
const LOW_LIMIT: f32 = PADDLE_Y - 2.5;
/// Moving stages: a step sideways every … seconds.
const SCROLL_STEP: f32 = 0.8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Brick {
    Light,
    Medium,
    Dark,
    Solid,
}

impl Brick {
    fn points(self) -> u32 {
        match self {
            Brick::Light => 1,
            Brick::Medium => 2,
            Brick::Dark => 3,
            Brick::Solid => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub col: i32,
    pub row: i32,
    pub kind: Brick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageKind {
    Still,
    Scrolling,
    Advancing,
    Bonus,
}

/// Our 8 patterns, 13 bricks wide: `1`/`2`/`3` light / medium / dark, `#` unbreakable.
const PATTERNS: [&[&str]; 8] = [
    &[
        "3333333333333",
        "2222222222222",
        "2222222222222",
        "1111111111111",
        "1111111111111",
        "1111111111111",
    ],
    &[
        "......3......",
        ".....323.....",
        "....32123....",
        "...3211123...",
        "..321111123..",
        ".32111111123.",
    ],
    &[
        "3.3.3.3.3.3.3",
        ".2.2.2.2.2.2.",
        "1.1.1.1.1.1.1",
        ".1.1.1.1.1.1.",
        "2.2.2.2.2.2.2",
        ".3.3.3.3.3.3.",
    ],
    &[
        "33.33.33.33.3",
        "22.22.22.22.2",
        "11.11.11.11.1",
        "11.11.11.11.1",
        "11.11.11.11.1",
        "22.22.22.22.2",
    ],
    &[
        "3333333333333",
        "#...........#",
        "#.222222222.#",
        "#.211111112.#",
        "#.222222222.#",
        "#...........#",
    ],
    &[
        "..3.......3..",
        ".323.....323.",
        "32123...32123",
        ".323.....323.",
        "..3...#...3..",
        "......#......",
    ],
    &[
        "1111111111111",
        "2222222222222",
        "###.......###",
        "3333333333333",
        ".............",
        "#...#####...#",
    ],
    &[
        "3333333333333",
        "1...........3",
        "1.222222222.3",
        "1.2.......2.3",
        "1.2.#####.2.3",
        "1...........3",
    ],
];

/// Bonus stages: pictures made of bricks.
const BONUS: [&[&str]; 4] = [
    &[
        "..111...111..",
        ".1121...1211.",
        "1122211122211",
        "1123333333211",
        ".11233333211.",
        "..112333211..",
        "...1123211...",
        "....11211....",
        ".....111.....",
    ],
    &[
        "......3......",
        ".....333.....",
        "....33333....",
        "...3333333...",
        "..222222222..",
        "..2.11111.2..",
        "..2.1...1.2..",
        "..2.1...1.2..",
        "..222222222..",
    ],
    &[
        "....11111....",
        "..111111111..",
        ".11133133111.",
        ".11133133111.",
        ".11111111111.",
        ".11311111311.",
        "..113333311..",
        "....11111....",
    ],
    &[
        "......2......",
        ".....222.....",
        ".....131.....",
        ".....131.....",
        ".....111.....",
        "....21112....",
        "...2211122...",
        "......3......",
        ".....3.3.....",
    ],
];

/// Stage `index` (0-based, of 32; then it starts over): its kind, pattern and number.
pub fn stage(index: u32) -> (StageKind, &'static [&'static str], u32) {
    let slot = index % 32;
    let group = slot / 4;
    match slot % 4 {
        3 => (StageKind::Bonus, BONUS[(group % 4) as usize], group),
        n => {
            let kind = [StageKind::Still, StageKind::Scrolling, StageKind::Advancing][n as usize];
            (kind, PATTERNS[group as usize], group * 3 + n + 1)
        }
    }
}

/// The bricks of a pattern (unbreakable ones only from stage 13 on; before, they're dark).
fn bricks(pattern: &[&str], number: u32) -> Vec<Cell> {
    let mut cells = Vec::new();
    for (row, line) in pattern.iter().enumerate() {
        for (col, c) in line.chars().enumerate() {
            let kind = match c {
                '1' => Brick::Light,
                '2' => Brick::Medium,
                '3' => Brick::Dark,
                '#' if number >= 13 => Brick::Solid,
                '#' => Brick::Dark,
                _ => continue,
            };
            cells.push(Cell {
                col: col as i32,
                row: row as i32,
                kind,
            });
        }
    }
    cells
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// The ball comes in this many seconds.
    Serve(f32),
    Play,
    /// Stage cleared (or bonus stage over): the next one in this many seconds.
    Cleared(f32),
    Over,
}

#[derive(Debug, Clone)]
pub struct Game {
    pub stage_index: u32,
    pub kind: StageKind,
    /// Stage number (1–24) or, for bonus stages, which bonus stage (0–7).
    pub number: u32,
    pub cells: Vec<Cell>,
    /// Moving stages: columns moved (per row direction) and rows moved down.
    scroll: i32,
    scroll_clock: f32,
    pub drop: f32,
    pub paddle: f32,
    pub small: bool,
    pub ball: [f32; 2],
    /// Direction (-1 / 1 per axis), angle (index into `ANGLES`) and speed (index into
    /// `SPEEDS`): the ball only ever flies at these angles.
    dir: [f32; 2],
    angle: usize,
    speed: usize,
    pub phase: Phase,
    pub score: u32,
    pub lives: u32,
    next_extra: u32,
    /// Bonus stage: seconds left; points for clearing it.
    pub timer: f32,
    pub bonus_points: u32,
    paddle_dir: f32,
    since_turn: f32,
    hits_without_brick: u32,
    /// Sounds since the last frame (played by the screen if sound is on).
    pub sounds: Vec<Sound>,
}

impl Game {
    /// A new game with this many paddles (1–9).
    pub fn new(lives: u32) -> Self {
        let mut game = Self {
            stage_index: 0,
            kind: StageKind::Still,
            number: 1,
            cells: Vec::new(),
            scroll: 0,
            scroll_clock: 0.0,
            drop: 0.0,
            paddle: W / 2.0,
            small: false,
            ball: [W / 2.0, H / 2.0],
            dir: [1.0, 1.0],
            angle: 2,
            speed: 0,
            phase: Phase::Serve(SERVE_DELAY),
            score: 0,
            lives: lives.clamp(1, MAX_LIVES),
            next_extra: 1000,
            timer: 0.0,
            bonus_points: 0,
            paddle_dir: 0.0,
            since_turn: 1.0,
            hits_without_brick: 0,
            sounds: Vec::new(),
        };
        game.start_stage(0);
        game
    }

    fn start_stage(&mut self, index: u32) {
        let (kind, pattern, number) = stage(index);
        self.stage_index = index;
        self.kind = kind;
        self.number = number;
        self.cells = bricks(pattern, if kind == StageKind::Bonus { 0 } else { number });
        self.scroll = 0;
        self.scroll_clock = 0.0;
        self.drop = 0.0;
        self.small = false;
        self.timer = (60.0 - 5.0 * number as f32).max(30.0);
        self.bonus_points = (500 + 250 * number).min(1500);
        self.serve();
    }

    fn serve(&mut self) {
        self.phase = Phase::Serve(SERVE_DELAY);
        self.speed = 0;
        self.angle = 2;
        self.hits_without_brick = 0;
    }

    pub fn paddle_width(&self) -> f32 {
        if self.small { PADDLE_W / 2.0 } else { PADDLE_W }
    }

    /// Where a brick is now (moving stages move it).
    pub fn brick_rect(&self, cell: &Cell) -> [f32; 4] {
        let shift = if self.kind == StageKind::Scrolling {
            if cell.row % 2 == 0 {
                self.scroll
            } else {
                -self.scroll
            }
        } else {
            0
        };
        let x = (cell.col + shift).rem_euclid(W as i32) as f32;
        let y = TOP + cell.row as f32 * BRICK_H + self.drop;
        [x, y, x + 1.0, y + BRICK_H]
    }

    pub fn velocity(&self) -> [f32; 2] {
        let (a, v) = (ANGLES[self.angle], SPEEDS[self.speed]);
        [self.dir[0] * a.cos() * v, self.dir[1] * a.sin() * v]
    }

    fn breakable_left(&self) -> bool {
        self.cells.iter().any(|c| c.kind != Brick::Solid)
    }

    fn add_points(&mut self, points: u32) {
        self.score += points;
        while self.score >= self.next_extra && self.next_extra <= 10_000 {
            if self.lives < MAX_LIVES {
                self.sounds.push(Sound::ExtraLife);
            }
            self.lives = (self.lives + 1).min(MAX_LIVES);
            self.next_extra += 1000;
        }
    }

    /// Advance by `dt` seconds. `x`: paddle input -1 (left) … 1 (right); `fast`: a button is
    /// held.
    pub fn step(&mut self, dt: f32, x: f32, fast: bool) {
        let dt = dt.clamp(0.0, 0.05);
        if self.phase == Phase::Over {
            return;
        }
        // Paddle.
        let x = x.clamp(-1.0, 1.0);
        let moving = if x.abs() > 0.1 { x.signum() } else { 0.0 };
        if moving != 0.0 && moving != self.paddle_dir {
            self.since_turn = 0.0;
            self.paddle_dir = moving;
        } else {
            self.since_turn += dt;
        }
        let half = self.paddle_width() / 2.0;
        let speed = if fast { PADDLE_FAST } else { PADDLE_SPEED };
        self.paddle = (self.paddle + x * speed * dt).clamp(half, W - half);

        match self.phase {
            Phase::Serve(left) => {
                // The ball waits up and to the side, then comes at 45° towards the paddle.
                let distance = 5.0_f32
                    .min(PADDLE_Y - (TOP + 6.0 * BRICK_H + self.drop) - 0.5)
                    .max(1.0);
                let from_left = self.paddle - distance >= BALL_R;
                let side = if from_left { -1.0 } else { 1.0 };
                self.ball = [
                    (self.paddle + side * distance).clamp(BALL_R, W - BALL_R),
                    PADDLE_Y - distance,
                ];
                self.dir = [-side, 1.0];
                self.phase = if left - dt <= 0.0 {
                    Phase::Play
                } else {
                    Phase::Serve(left - dt)
                };
                return;
            }
            Phase::Cleared(left) => {
                if left - dt <= 0.0 {
                    self.start_stage(self.stage_index + 1);
                } else {
                    self.phase = Phase::Cleared(left - dt);
                }
                return;
            }
            Phase::Play | Phase::Over => {}
        }

        if self.kind == StageKind::Bonus {
            self.timer -= dt;
            if self.timer <= 0.0 {
                self.timer = 0.0;
                self.phase = Phase::Cleared(STAGE_PAUSE);
                return;
            }
        }
        if self.kind == StageKind::Scrolling {
            self.scroll_clock += dt;
            while self.scroll_clock >= SCROLL_STEP {
                self.scroll_clock -= SCROLL_STEP;
                self.scroll += 1;
            }
        }

        let v = self.velocity();
        let distance = v[0].hypot(v[1]) * dt;
        let steps = (distance / (BALL_R * 0.5)).ceil().max(1.0) as u32;
        for _ in 0..steps {
            if !self.move_ball(dt / steps as f32) {
                break;
            }
        }
    }

    /// One small step of the ball. False when something ended the step (lost ball, cleared).
    fn move_ball(&mut self, dt: f32) -> bool {
        let before = self.ball;
        let v = self.velocity();
        self.ball = [before[0] + v[0] * dt, before[1] + v[1] * dt];
        let [x, y] = self.ball;

        // Walls and ceiling.
        if x < BALL_R {
            self.ball[0] = BALL_R;
            self.dir[0] = 1.0;
            self.sounds.push(Sound::Wall);
        } else if x > W - BALL_R {
            self.ball[0] = W - BALL_R;
            self.dir[0] = -1.0;
            self.sounds.push(Sound::Wall);
        }
        if y < BALL_R {
            self.ball[1] = BALL_R;
            self.dir[1] = 1.0;
            self.sounds.push(Sound::Wall);
            // From stage 5 on, the ceiling halves the paddle (not in bonus stages).
            if self.kind != StageKind::Bonus && self.number >= 5 {
                self.small = true;
            }
        }

        // Bricks.
        let mut hit = None;
        for (i, cell) in self.cells.iter().enumerate() {
            let [l, t, r, b] = self.brick_rect(cell);
            let nx = self.ball[0].clamp(l, r);
            let ny = self.ball[1].clamp(t, b);
            if (self.ball[0] - nx).powi(2) + (self.ball[1] - ny).powi(2) < BALL_R * BALL_R {
                hit = Some((i, [l, t, r, b]));
                break;
            }
        }
        if let Some((i, [l, t, r, b])) = hit {
            let kind = self.cells[i].kind;
            let passes = self.kind == StageKind::Bonus && kind != Brick::Solid;
            if !passes {
                // Back the way it came, on the side it came from; same angle.
                let outside_x = before[0] < l || before[0] > r;
                let outside_y = before[1] < t || before[1] > b;
                if outside_y || !outside_x {
                    self.dir[1] = -self.dir[1];
                }
                if outside_x {
                    self.dir[0] = -self.dir[0];
                }
                self.ball = before;
            }
            self.sounds.push(match kind {
                Brick::Light => Sound::Brick,
                _ => Sound::HardBrick,
            });
            if kind != Brick::Solid {
                self.cells.remove(i);
                self.add_points(kind.points());
                self.hits_without_brick = 0;
                let at_least = match kind {
                    Brick::Medium => 1,
                    Brick::Dark => 2,
                    _ => 0,
                };
                self.speed = self.speed.max(at_least);
                if !self.breakable_left() {
                    if self.kind == StageKind::Bonus {
                        self.add_points(self.bonus_points);
                    }
                    self.sounds.push(Sound::Cleared);
                    self.phase = Phase::Cleared(STAGE_PAUSE);
                    return false;
                }
            }
        }

        // Paddle.
        let half = self.paddle_width() / 2.0;
        let (pl, pr) = (self.paddle - half, self.paddle + half);
        let falling = self.dir[1] > 0.0;
        let reaches = before[1] + BALL_R <= PADDLE_Y && self.ball[1] + BALL_R >= PADDLE_Y;
        if falling && reaches && self.ball[0] >= pl - BALL_R && self.ball[0] <= pr + BALL_R {
            if self.ball[0] < pl || self.ball[0] > pr {
                // The corner: off to the side, and down.
                self.dir[0] = if self.ball[0] < pl { -1.0 } else { 1.0 };
            } else {
                self.bounce_off_paddle();
            }
        }

        // Lost.
        if self.ball[1] - BALL_R > H {
            if self.kind == StageKind::Bonus {
                self.phase = Phase::Cleared(STAGE_PAUSE);
            } else {
                self.lives = self.lives.saturating_sub(1);
                self.small = false;
                if self.lives == 0 {
                    self.sounds.push(Sound::GameOver);
                    self.phase = Phase::Over;
                } else {
                    self.sounds.push(Sound::Lost);
                    self.serve();
                }
            }
            return false;
        }
        true
    }

    fn bounce_off_paddle(&mut self) {
        self.sounds.push(Sound::Paddle);
        let half = self.paddle_width() / 2.0;
        let offset = (self.ball[0] - self.paddle) / half;
        // The middle sends the ball up steeply, the ends flat.
        let mut angle = if offset.abs() < 0.34 {
            2
        } else if offset.abs() < 0.67 {
            1
        } else {
            0
        };
        if offset.abs() > 0.05 {
            self.dir[0] = offset.signum();
        }
        self.dir[1] = -1.0;
        self.ball[1] = PADDLE_Y - BALL_R;
        // Endless bouncing without a brick: another angle.
        self.hits_without_brick += 1;
        if self.hits_without_brick >= LOOP_HITS {
            self.hits_without_brick = 0;
            angle = (angle + 1) % ANGLES.len();
        }
        self.angle = angle;
        // Snap: the paddle just turned around.
        if self.since_turn < SNAP {
            self.speed = (self.speed + 1).min(SPEEDS.len() - 1);
        }
        // Moving down stage: one row per paddle hit; too low bricks disappear.
        if self.kind == StageKind::Advancing {
            self.drop += BRICK_H;
            let drop = self.drop;
            self.cells
                .retain(|c| TOP + c.row as f32 * BRICK_H + drop + BRICK_H < LOW_LIMIT);
            if !self.breakable_left() {
                self.phase = Phase::Cleared(STAGE_PAUSE);
            }
        }
    }
}

impl Default for Game {
    fn default() -> Self {
        Self::new(START_LIVES)
    }
}

// --------------------------------------------------------------------------------- screen

#[derive(Debug, Default, Clone)]
pub struct BricksInput {
    /// -1 (left) … 1 (right).
    pub x: f32,
    /// A button held: the paddle moves faster.
    pub fast: bool,
    pub up: bool,
    pub down: bool,
    pub enter: bool,
    pub back: bool,
    pub pause: bool,
    pub clicks: bool,
    pub left: bool,
    pub right: bool,
    pub typed: Vec<char>,
    pub erase: bool,
}

pub enum BricksState {
    Menu { selected: usize },
    Playing(Game),
    Paused { game: Game, selected: usize },
    EnterName { game: Game, entry: NameEntry },
    Over { game: Game, selected: usize },
    Scores,
}

pub struct BricksView {
    pub state: BricksState,
    /// Paddles per game (setting), and sound on / off (setting; switchable in the menus).
    lives: u32,
    pub sound: bool,
    scores: Scores,
}

impl BricksView {
    pub fn new(lives: u32, sound: bool) -> Self {
        Self {
            state: BricksState::Menu { selected: 0 },
            lives,
            sound,
            scores: Scores::load(),
        }
    }

    fn sound_item(&self) -> &'static str {
        if self.sound {
            tr!("Sound: on")
        } else {
            tr!("Sound: off")
        }
    }
}

impl BricksView {
    /// True when the player wants back to the home screen.
    pub fn show(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen: Rect,
        s: f32,
        dt: f32,
        input: &BricksInput,
    ) -> bool {
        painter.rect_filled(screen, 0.0, Color32::from_rgb(0x0b, 0x10, 0x1a));
        let menu_input = crate::pong::PongInput {
            up: input.up,
            down: input.down,
            enter: input.enter,
            back: input.back,
            pause: input.pause,
            clicks: input.clicks,
            ..Default::default()
        };
        let state = std::mem::replace(&mut self.state, BricksState::Menu { selected: 0 });
        let (next, quit) = match state {
            BricksState::Menu { selected } => {
                let items = [
                    tr!("Play"),
                    self.sound_item(),
                    tr!("High scores"),
                    tr!("Back"),
                ];
                let (selected, chosen) = crate::pong::menu(
                    ui,
                    painter,
                    screen,
                    s,
                    tr!("Brick Breaker"),
                    &items,
                    selected,
                    &menu_input,
                );
                let hint = tr!(
                    "Left / right: move · hold a button: faster · best: {score}",
                    score = self.scores.best(Board::Bricks)
                );
                crate::pong::paint_hint(painter, screen, s, &hint);
                match chosen {
                    Some(0) => (BricksState::Playing(Game::new(self.lives)), false),
                    Some(1) => {
                        self.sound = !self.sound;
                        (BricksState::Menu { selected }, false)
                    }
                    Some(2) => (BricksState::Scores, false),
                    Some(_) => (BricksState::Menu { selected }, true),
                    None => (BricksState::Menu { selected }, input.back),
                }
            }
            BricksState::Playing(mut game) => {
                game.step(dt, input.x, input.fast);
                for sound in game.sounds.drain(..) {
                    if self.sound {
                        crate::sound::play(sound);
                    }
                }
                paint_game(painter, screen, s, &game, self.scores.best(Board::Bricks));
                ui.ctx().request_repaint();
                if game.phase == Phase::Over {
                    if self.scores.qualifies(Board::Bricks, game.score) {
                        let entry = self.scores.name_entry(game.score);
                        (BricksState::EnterName { game, entry }, false)
                    } else {
                        (BricksState::Over { game, selected: 0 }, false)
                    }
                } else if input.pause || input.back {
                    (BricksState::Paused { game, selected: 0 }, false)
                } else {
                    (BricksState::Playing(game), false)
                }
            }
            BricksState::Paused { game, selected } => {
                paint_game(painter, screen, s, &game, self.scores.best(Board::Bricks));
                crate::pong::dim(painter, screen);
                let items = [tr!("Continue"), self.sound_item(), tr!("End the game")];
                let (selected, chosen) = crate::pong::menu(
                    ui,
                    painter,
                    screen,
                    s,
                    tr!("Paused"),
                    &items,
                    selected,
                    &menu_input,
                );
                match chosen {
                    Some(0) => (BricksState::Playing(game), false),
                    Some(1) => {
                        self.sound = !self.sound;
                        (BricksState::Paused { game, selected }, false)
                    }
                    Some(_) => (BricksState::Menu { selected: 0 }, false),
                    None if input.back || input.pause => (BricksState::Playing(game), false),
                    None => (BricksState::Paused { game, selected }, false),
                }
            }
            BricksState::EnterName { game, mut entry } => {
                paint_game(painter, screen, s, &game, self.scores.best(Board::Bricks));
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
                        .add(Board::Bricks, &entry.name(), entry.score, &today);
                    (BricksState::Scores, false)
                } else {
                    (BricksState::EnterName { game, entry }, false)
                }
            }
            BricksState::Scores => {
                highscores::paint_list(
                    painter,
                    screen,
                    s,
                    tr!("Brick Breaker"),
                    self.scores.list(Board::Bricks),
                );
                if input.enter || input.back {
                    (BricksState::Menu { selected: 0 }, false)
                } else {
                    (BricksState::Scores, false)
                }
            }
            BricksState::Over { game, selected } => {
                paint_game(painter, screen, s, &game, self.scores.best(Board::Bricks));
                crate::pong::dim(painter, screen);
                let title = tr!("Game over · {score} points", score = game.score);
                let items = [tr!("Play again"), tr!("Back")];
                let (selected, chosen) = crate::pong::menu(
                    ui,
                    painter,
                    screen,
                    s,
                    &title,
                    &items,
                    selected,
                    &menu_input,
                );
                match chosen {
                    Some(0) => (BricksState::Playing(Game::new(self.lives)), false),
                    Some(_) => (BricksState::Menu { selected: 0 }, false),
                    None if input.back => (BricksState::Menu { selected: 0 }, false),
                    None => (BricksState::Over { game, selected }, false),
                }
            }
        };
        self.state = next;
        quit
    }
}

/// Colors per stage group: light, medium and dark shade.
fn shades(stage_index: u32) -> [Color32; 3] {
    const HUES: [[Color32; 3]; 4] = [
        [
            Color32::from_rgb(0x9a, 0xd8, 0xff),
            Color32::from_rgb(0x3d, 0x8b, 0xff),
            Color32::from_rgb(0x1d, 0x4f, 0xb8),
        ],
        [
            Color32::from_rgb(0xff, 0xd0, 0x8a),
            Color32::from_rgb(0xff, 0x9a, 0x3d),
            Color32::from_rgb(0xc2, 0x5a, 0x12),
        ],
        [
            Color32::from_rgb(0xa8, 0xf0, 0xb0),
            Color32::from_rgb(0x3c, 0xc0, 0x6a),
            Color32::from_rgb(0x1a, 0x7a, 0x40),
        ],
        [
            Color32::from_rgb(0xf2, 0xb0, 0xf0),
            Color32::from_rgb(0xd0, 0x5c, 0xd0),
            Color32::from_rgb(0x84, 0x2c, 0x96),
        ],
    ];
    HUES[((stage_index / 4) % 4) as usize]
}

fn paint_game(painter: &egui::Painter, screen: Rect, s: f32, game: &Game, best: u32) {
    let scale = ((screen.height() - 80.0 * s) / H).min((screen.width() - 600.0 * s) / W);
    let field = Rect::from_center_size(screen.center(), Vec2::new(W * scale, H * scale));
    let at = |x: f32, y: f32| field.min + Vec2::new(x * scale, y * scale);
    painter.rect_filled(field, 0.0, Color32::from_rgb(0x12, 0x18, 0x28));
    painter.rect_stroke(
        field.expand(3.0 * s),
        0.0,
        egui::Stroke::new(3.0 * s, Color32::from_rgb(0x3a, 0x44, 0x5c)),
        egui::StrokeKind::Outside,
    );
    let colors = shades(game.stage_index);
    for cell in &game.cells {
        let [l, t, r, b] = game.brick_rect(cell);
        let rect = Rect::from_min_max(at(l, t), at(r, b)).shrink(1.5 * s);
        let color = match cell.kind {
            Brick::Light => colors[0],
            Brick::Medium => colors[1],
            Brick::Dark => colors[2],
            Brick::Solid => Color32::from_rgb(0x8a, 0x90, 0x9e),
        };
        painter.rect_filled(rect, CornerRadius::same((3.0 * s) as u8), color);
        if cell.kind == Brick::Solid {
            painter.rect_stroke(
                rect.shrink(3.0 * s),
                CornerRadius::same(2),
                egui::Stroke::new(1.5 * s, Color32::from_rgb(0x5a, 0x60, 0x6e)),
                egui::StrokeKind::Inside,
            );
        }
    }
    let half = game.paddle_width() / 2.0;
    painter.rect_filled(
        Rect::from_min_max(
            at(game.paddle - half, PADDLE_Y),
            at(game.paddle + half, PADDLE_Y + PADDLE_H),
        ),
        CornerRadius::same((PADDLE_H * scale / 2.0) as u8),
        Color32::from_rgb(0xf2, 0xf4, 0xf8),
    );
    let blink = matches!(game.phase, Phase::Serve(t) if (t * 6.0) as i32 % 2 == 0);
    if matches!(game.phase, Phase::Play) || blink {
        painter.circle_filled(
            at(game.ball[0], game.ball[1]),
            BALL_R * scale,
            Color32::from_rgb(0xff, 0xe0, 0x6a),
        );
    }

    // Side panels: stage, score, best, paddles left, bonus timer.
    let left = Pos2::new(field.min.x - 40.0 * s, field.min.y + 20.0 * s);
    let label = |pos: Pos2, align: Align2, text: String, size: f32, color: Color32| {
        painter.text(pos, align, text, FontId::proportional(size * s), color);
    };
    let stage_text = if game.kind == StageKind::Bonus {
        tr!("Bonus stage").to_string()
    } else {
        tr!("Stage {n}", n = game.number)
    };
    label(left, Align2::RIGHT_TOP, stage_text, 34.0, crate::ui::TEXT);
    if game.kind == StageKind::Bonus {
        label(
            left + Vec2::new(0.0, 50.0 * s),
            Align2::RIGHT_TOP,
            tr!("Time: {secs}", secs = game.timer.ceil() as u32),
            30.0,
            Color32::from_rgb(0xff, 0xb4, 0x5c),
        );
    }
    let right = Pos2::new(field.max.x + 40.0 * s, field.min.y + 20.0 * s);
    label(
        right,
        Align2::LEFT_TOP,
        tr!("Points").into(),
        24.0,
        crate::ui::TEXT_DIM,
    );
    label(
        right + Vec2::new(0.0, 32.0 * s),
        Align2::LEFT_TOP,
        game.score.to_string(),
        44.0,
        crate::ui::TEXT,
    );
    label(
        right + Vec2::new(0.0, 100.0 * s),
        Align2::LEFT_TOP,
        tr!("Best").into(),
        24.0,
        crate::ui::TEXT_DIM,
    );
    label(
        right + Vec2::new(0.0, 132.0 * s),
        Align2::LEFT_TOP,
        best.max(game.score).to_string(),
        32.0,
        crate::ui::TEXT,
    );
    label(
        right + Vec2::new(0.0, 190.0 * s),
        Align2::LEFT_TOP,
        tr!("Paddles: {n}", n = game.lives),
        24.0,
        crate::ui::TEXT_DIM,
    );
    for i in 0..game.lives.min(MAX_LIFE_ICONS) {
        let p = right
            + Vec2::new(
                (i % 5) as f32 * 46.0 * s,
                228.0 * s + (i / 5) as f32 * 26.0 * s,
            );
        painter.rect_filled(
            Rect::from_min_size(p, Vec2::new(38.0 * s, 10.0 * s)),
            CornerRadius::same((5.0 * s) as u8),
            Color32::from_rgb(0xf2, 0xf4, 0xf8),
        );
    }
    if let Phase::Cleared(_) = game.phase {
        let text = if game.kind == StageKind::Bonus {
            tr!("Bonus stage over")
        } else {
            tr!("Stage cleared!")
        };
        painter.text(
            field.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(48.0 * s),
            crate::ui::TEXT,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A game in play with the ball at `ball` going in `dir`, and only the given bricks.
    fn playing(stage_index: u32, cells: Vec<Cell>, ball: [f32; 2], dir: [f32; 2]) -> Game {
        let mut g = Game::new(START_LIVES);
        g.start_stage(stage_index);
        g.cells = cells;
        g.phase = Phase::Play;
        g.ball = ball;
        g.dir = dir;
        g
    }

    fn cell(col: i32, row: i32, kind: Brick) -> Cell {
        Cell { col, row, kind }
    }

    #[test]
    fn patterns_are_13_wide_and_stages_go_in_threes_plus_bonus() {
        for p in PATTERNS.iter().chain(BONUS.iter()) {
            assert!(p.iter().all(|l| l.chars().count() == 13), "{p:?}");
        }
        let kinds: Vec<_> = (0..8).map(|i| stage(i).0).collect();
        use StageKind::*;
        assert_eq!(
            kinds,
            [
                Still, Scrolling, Advancing, Bonus, Still, Scrolling, Advancing, Bonus
            ]
        );
        assert_eq!(stage(4).2, 4);
        assert_eq!(stage(30).2, 24);
        assert_eq!(stage(32).0, Still, "starts over after 32");
        // Unbreakable bricks only from stage 13 on.
        let solid = |i| {
            bricks(stage(i).1, stage(i).2)
                .iter()
                .any(|c| c.kind == Brick::Solid)
        };
        assert!(!(0..16).any(solid));
        assert!(solid(16));
    }

    #[test]
    fn the_ball_only_flies_at_15_30_or_45_degrees() {
        let mut g = Game::new(START_LIVES);
        let mut x = 1.0;
        for i in 0..20_000 {
            if i % 300 == 0 {
                x = -x;
            }
            g.step(1.0 / 60.0, x * 0.7, i % 500 < 100);
            if g.phase == Phase::Play {
                let v = g.velocity();
                let a = (v[1].abs() / v[0].hypot(v[1])).asin().to_degrees();
                assert!(
                    [15.0, 30.0, 45.0].iter().any(|d| (a - d).abs() < 0.01),
                    "{a}"
                );
            }
            if g.phase == Phase::Over {
                break;
            }
        }
    }

    #[test]
    fn bricks_give_points_and_dark_ones_speed_up() {
        let mut g = playing(
            0,
            vec![cell(5, 0, Brick::Dark), cell(0, 9, Brick::Light)],
            [4.6, 3.0], // at 45° it reaches the brick (x 5–6, y 1.5–2)
            [1.0, -1.0],
        );
        g.angle = 2;
        for _ in 0..60 {
            g.step(1.0 / 60.0, 0.0, false);
        }
        assert_eq!(g.score, 3);
        assert_eq!(g.speed, 2);
        assert_eq!(g.dir[1], 1.0, "back down");
        assert_eq!(g.cells.len(), 1);
    }

    #[test]
    fn unbreakable_bricks_stay() {
        let mut g = playing(
            16,
            vec![cell(5, 0, Brick::Solid), cell(0, 9, Brick::Light)],
            [4.6, 3.0],
            [1.0, -1.0],
        );
        for _ in 0..60 {
            g.step(1.0 / 60.0, 0.0, false);
        }
        assert_eq!(g.score, 0);
        assert_eq!(g.cells.len(), 2);
        assert_eq!(g.dir[1], 1.0);
    }

    #[test]
    fn ceiling_halves_the_paddle_from_stage_5_on() {
        for (index, number, small) in [(0, 1, false), (5, 5, true), (3, 0, false)] {
            let mut g = playing(
                index,
                vec![cell(0, 9, Brick::Light)],
                [6.0, 1.0],
                [1.0, -1.0],
            );
            assert_eq!(g.number, number);
            for _ in 0..30 {
                g.step(1.0 / 60.0, 0.0, false);
            }
            assert_eq!(g.small, small, "stage index {index}");
        }
    }

    #[test]
    fn a_lost_ball_costs_a_paddle_and_the_last_one_ends_the_game() {
        let mut g = playing(
            5,
            vec![cell(0, 0, Brick::Light)],
            [1.0, H - 0.5],
            [-1.0, 1.0],
        );
        g.paddle = W - 1.0;
        g.small = true;
        for _ in 0..60 {
            g.step(1.0 / 60.0, 0.0, false);
        }
        assert_eq!(g.lives, START_LIVES - 1);
        assert!(!g.small, "full paddle again");
        assert!(matches!(g.phase, Phase::Serve(_)));
        g.lives = 1;
        g.phase = Phase::Play;
        g.ball = [1.0, H - 0.5];
        g.dir = [-1.0, 1.0];
        for _ in 0..60 {
            g.step(1.0 / 60.0, 0.0, false);
        }
        assert_eq!(g.phase, Phase::Over);
    }

    #[test]
    fn starting_paddles_and_sounds() {
        assert_eq!(Game::new(3).lives, 3);
        assert_eq!(Game::new(0).lives, 1);
        assert_eq!(Game::new(99).lives, 99);
        assert_eq!(Game::new(200).lives, MAX_LIVES);
        let mut g = playing(
            0,
            vec![cell(5, 0, Brick::Light), cell(0, 9, Brick::Light)],
            [4.6, 3.0],
            [1.0, -1.0],
        );
        for _ in 0..60 {
            g.step(1.0 / 60.0, 0.0, false);
        }
        assert!(g.sounds.contains(&Sound::Brick));
    }

    #[test]
    fn extra_paddles_every_1000_points_up_to_10000_and_99_at_most() {
        let mut g = Game::new(START_LIVES);
        g.add_points(999);
        assert_eq!(g.lives, 5);
        g.add_points(1);
        assert_eq!(g.lives, 6);
        g.add_points(2500);
        assert_eq!(g.lives, 8);
        g.add_points(20_000);
        assert_eq!(g.lives, 15, "ten more at most, by 10 000 points");
        let mut g = Game::new(98);
        g.add_points(5000);
        assert_eq!(g.lives, MAX_LIVES, "never more than 99");
    }

    #[test]
    fn paddle_middle_is_steep_ends_flat_corner_loses() {
        for (offset, angle) in [(0.0, 2), (0.5, 1), (0.9, 0)] {
            let mut g = playing(
                0,
                vec![cell(0, 0, Brick::Light)],
                [0.0, PADDLE_Y - 0.4],
                [1.0, 1.0],
            );
            g.paddle = 6.0;
            // From 0.4 above, the ball moves 0.25 sideways until it touches the paddle.
            g.ball[0] = 6.0 + offset * PADDLE_W / 2.0 - 0.25;
            for _ in 0..10 {
                g.step(1.0 / 120.0, 0.0, false);
            }
            assert_eq!(g.dir[1], -1.0, "offset {offset}");
            assert_eq!(g.angle, angle, "offset {offset}");
        }
        let mut g = playing(
            0,
            vec![cell(0, 0, Brick::Light)],
            [6.85, PADDLE_Y - 0.4],
            [1.0, 1.0],
        );
        g.paddle = 6.0;
        for _ in 0..10 {
            g.step(1.0 / 120.0, 0.0, false);
        }
        assert_eq!(g.dir[1], 1.0, "the corner lets it fall");
    }

    #[test]
    fn snap_makes_the_ball_faster() {
        let mut g = playing(
            0,
            vec![cell(0, 0, Brick::Light)],
            [6.0, PADDLE_Y - 0.6],
            [1.0, 1.0],
        );
        g.paddle = 6.0;
        g.step(1.0 / 60.0, -1.0, false); // moving left …
        g.step(1.0 / 60.0, 1.0, false); // … turns right just before the hit
        for _ in 0..10 {
            g.step(1.0 / 60.0, 0.0, false);
        }
        assert_eq!(g.dir[1], -1.0);
        assert_eq!(g.speed, 1);
    }

    #[test]
    fn bonus_stage_ball_goes_through_and_costs_nothing() {
        let mut g = playing(
            3,
            vec![
                cell(5, 0, Brick::Light),
                cell(5, 1, Brick::Light),
                cell(0, 9, Brick::Light),
            ],
            [4.5, 3.0],
            [1.0, -1.0],
        );
        assert_eq!(g.kind, StageKind::Bonus);
        for _ in 0..30 {
            g.step(1.0 / 60.0, 0.0, false);
        }
        assert_eq!(g.score, 2);
        assert_eq!(g.dir[1], -1.0, "no bounce");
        // Lost ball: no paddle lost, the stage ends.
        g.ball = [1.0, H - 0.3];
        g.dir = [-1.0, 1.0];
        g.paddle = W - 1.0;
        for _ in 0..30 {
            g.step(1.0 / 60.0, 0.0, false);
        }
        assert_eq!(g.lives, START_LIVES);
        assert!(matches!(g.phase, Phase::Cleared(_)));
        // Timer.
        let mut g = playing(3, vec![cell(0, 9, Brick::Light)], [6.0, 8.0], [1.0, -1.0]);
        g.timer = 0.01;
        g.step(0.02, 0.0, false);
        assert!(matches!(g.phase, Phase::Cleared(_)));
    }

    #[test]
    fn clearing_a_bonus_stage_gives_bonus_points_and_the_next_stage_follows() {
        let mut g = playing(3, vec![cell(5, 0, Brick::Light)], [4.6, 3.0], [1.0, -1.0]);
        for _ in 0..60 {
            g.step(1.0 / 60.0, 0.0, false);
        }
        assert_eq!(g.score, 1 + 500);
        for _ in 0..200 {
            g.step(1.0 / 60.0, 0.0, false);
        }
        assert_eq!(g.stage_index, 4);
        assert_eq!(g.number, 4);
        assert!(matches!(g.phase, Phase::Serve(_) | Phase::Play));
    }

    #[test]
    fn moving_stages() {
        // Sideways: whole bricks, wrapping around.
        let mut g = playing(
            1,
            vec![cell(12, 0, Brick::Light), cell(0, 1, Brick::Light)],
            [6.0, 12.0],
            [1.0, 1.0],
        );
        g.scroll = 1;
        assert_eq!(g.brick_rect(&g.cells[0])[0], 0.0);
        assert_eq!(g.brick_rect(&g.cells[1])[0], 12.0);
        // Down: one row per paddle hit; bricks too low go without points.
        let mut g = playing(
            2,
            vec![cell(0, 0, Brick::Light), cell(0, 18, Brick::Light)],
            [6.0, PADDLE_Y - 0.6],
            [1.0, 1.0],
        );
        g.paddle = 6.0;
        for _ in 0..10 {
            g.step(1.0 / 60.0, 0.0, false);
        }
        assert_eq!(g.drop, BRICK_H);
        assert_eq!(g.cells.len(), 1);
        assert_eq!(g.score, 0);
    }

    #[test]
    fn endless_bouncing_gets_another_angle() {
        let mut g = playing(
            0,
            vec![cell(0, 0, Brick::Light)],
            [5.55, PADDLE_Y - 0.6],
            [1.0, 1.0],
        );
        g.paddle = 6.0;
        g.hits_without_brick = LOOP_HITS - 1;
        for _ in 0..10 {
            g.step(1.0 / 60.0, 0.0, false);
        }
        assert_eq!(g.angle, 0, "45° would have been next; it changed");
    }
}
