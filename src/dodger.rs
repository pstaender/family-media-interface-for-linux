//! Star Dodger: steer the striped ball left and right past the balls coming down. Every 10
//! balls passed the game gets faster. Beside the field: the balls passed on the left, the
//! time and the level on the right. One hit ends the game; the balls passed count for the
//! high score list (3 letters).
//!
//! Drawn like an 80s/90s console game: a 288 × 216 picture of big square pixels at a whole
//! number of screen pixels each. Objects move in screen pixels, so the motion stays smooth
//! while the art keeps its pixels. Everything is drawn here, no image files.
//!
//! The rules (`Game`) are pure and tested; `DodgerView` shows menus and paints.

use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

use crate::highscores::{self, Board, NameEntry, NameInput, Scores};
use crate::pong::PongInput;
use crate::sound::Sound;
use crate::tr;

pub const W: f32 = 288.0;
pub const H: f32 = 216.0;
/// Room beside the field for the numbers (game pixels, each side).
const PANEL: f32 = 44.0;
/// Letters of a name in the high score list.
const NAME_LETTERS: usize = 3;
/// The side walls' width.
const WALL: f32 = 8.0;
const PLAYER_Y: f32 = H - 22.0;
const PLAYER_R: f32 = 10.0;
const BALL_R: f32 = 9.0;
const PLAYER_SPEED: f32 = 190.0;
/// Fall speed at level 1, and what each level adds (pixels / s): a lot up to `STEEP_UNTIL`,
/// then little, so that even level 99 can still be dodged.
const FALL_START: f32 = 90.0;
const FALL_STEP: f32 = 18.0;
const FALL_STEP_LATE: f32 = 3.0;
const STEEP_UNTIL: u32 = 9;
pub const MAX_LEVEL: u32 = 99;
/// The background stops speeding up at this level (it would only flicker).
const SCROLL_MAX_LEVEL: u32 = 20;
pub const BALLS_PER_LEVEL: u32 = 10;
/// A new ball when the last one has fallen this far.
const SPACING: f32 = 80.0;
/// A new ball starts at least this far beside the one before.
const MIN_SIDEWAYS: f32 = 30.0;
/// Every so many balls come straight at the player, so standing still doesn't work.
const AIMED_EVERY: u32 = 3;
/// A hit counts a little inside the outlines (fair near misses).
const HIT_FORGIVENESS: f32 = 2.0;
const CRASH_SECS: f32 = 1.2;

/// The falling balls' colors: bright, middle, dark.
const BALLS: [[Color32; 3]; 7] = [
    shades(0xff, 0x4f, 0xa8),
    shades(0x6c, 0xf0, 0x5a),
    shades(0x35, 0xd8, 0xff),
    shades(0xff, 0x9a, 0x2e),
    shades(0xb0, 0x6c, 0xff),
    shades(0xff, 0xe6, 0x4a),
    shades(0xff, 0x4a, 0x4a),
];

const fn shades(r: u8, g: u8, b: u8) -> [Color32; 3] {
    [
        Color32::from_rgb(
            (r as u16 + (255 - r as u16) / 2) as u8,
            (g as u16 + (255 - g as u16) / 2) as u8,
            (b as u16 + (255 - b as u16) / 2) as u8,
        ),
        Color32::from_rgb(r, g, b),
        Color32::from_rgb(
            (r as u16 * 5 / 8) as u8,
            (g as u16 * 5 / 8) as u8,
            (b as u16 * 5 / 8) as u8,
        ),
    ]
}

/// The player's stripes.
const STRIPES: [Color32; 6] = [
    Color32::from_rgb(0xff, 0xff, 0xff),
    Color32::from_rgb(0xff, 0x4f, 0xa8),
    Color32::from_rgb(0xff, 0xe6, 0x4a),
    Color32::from_rgb(0xff, 0xff, 0xff),
    Color32::from_rgb(0x35, 0xd8, 0xff),
    Color32::from_rgb(0x6c, 0xf0, 0x5a),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ball {
    pub x: f32,
    pub y: f32,
    pub color: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Play,
    /// Hit: the scene stays for this many seconds.
    Crash(f32),
    Over,
}

#[derive(Debug, Clone)]
pub struct Game {
    pub player: f32,
    pub balls: Vec<Ball>,
    pub passed: u32,
    pub level: u32,
    /// Seconds played.
    pub time: f32,
    pub phase: Phase,
    /// How far the background has moved (faster with the level).
    pub scroll: f32,
    fallen: f32,
    spawned: u32,
    last_x: f32,
    last_color: usize,
    rng: u64,
    /// Sounds since the last frame (played by the screen if sound is on).
    pub sounds: Vec<Sound>,
}

pub fn fall_speed(level: u32) -> f32 {
    let level = level.clamp(1, MAX_LEVEL);
    let steep = level.min(STEEP_UNTIL);
    FALL_START + FALL_STEP * (steep - 1) as f32 + FALL_STEP_LATE * (level - steep) as f32
}

impl Game {
    pub fn new(seed: u64) -> Self {
        Self {
            player: W / 2.0,
            balls: Vec::new(),
            passed: 0,
            level: 1,
            time: 0.0,
            phase: Phase::Play,
            scroll: 0.0,
            // The first ball comes right away.
            fallen: SPACING,
            spawned: 0,
            last_x: W / 2.0,
            last_color: 0,
            rng: seed | 1,
            sounds: Vec::new(),
        }
    }

    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    fn spawn(&mut self) {
        let (low, high) = (WALL + BALL_R, W - WALL - BALL_R);
        self.spawned += 1;
        let mut x = if self.spawned.is_multiple_of(AIMED_EVERY) {
            self.player.clamp(low, high)
        } else {
            low + (self.random() % 10_000) as f32 / 10_000.0 * (high - low)
        };
        // Always somewhere else than the ball before.
        if (x - self.last_x).abs() < MIN_SIDEWAYS {
            x = if self.last_x + MIN_SIDEWAYS <= high {
                self.last_x + MIN_SIDEWAYS
            } else {
                self.last_x - MIN_SIDEWAYS
            };
        }
        // Another color than the ball before.
        let step = 1 + (self.random() % (BALLS.len() as u64 - 1)) as usize;
        let color = (self.last_color + step) % BALLS.len();
        self.balls.push(Ball {
            x,
            y: -BALL_R,
            color,
        });
        self.last_x = x;
        self.last_color = color;
    }

    /// Advance by `dt` seconds. `x`: -1 (left) … 1 (right).
    pub fn step(&mut self, dt: f32, x: f32) {
        let dt = dt.clamp(0.0, 0.05);
        self.scroll += dt * (1.0 + 0.15 * (self.level.min(SCROLL_MAX_LEVEL) - 1) as f32);
        match self.phase {
            Phase::Over => return,
            Phase::Crash(left) => {
                self.phase = if left - dt <= 0.0 {
                    Phase::Over
                } else {
                    Phase::Crash(left - dt)
                };
                return;
            }
            Phase::Play => {}
        }
        self.time += dt;
        self.player = (self.player + x.clamp(-1.0, 1.0) * PLAYER_SPEED * dt)
            .clamp(WALL + PLAYER_R, W - WALL - PLAYER_R);

        let fall = fall_speed(self.level) * dt;
        for ball in &mut self.balls {
            ball.y += fall;
        }
        self.fallen += fall;
        if self.fallen >= SPACING {
            self.fallen -= SPACING;
            self.spawn();
        }

        let reach = PLAYER_R + BALL_R - HIT_FORGIVENESS;
        let player = self.player;
        if self
            .balls
            .iter()
            .any(|b| (b.x - player).powi(2) + (b.y - PLAYER_Y).powi(2) < reach * reach)
        {
            self.phase = Phase::Crash(CRASH_SECS);
            self.sounds.push(Sound::GameOver);
            return;
        }
        let before = self.balls.len();
        self.balls.retain(|b| b.y - BALL_R <= H);
        for _ in self.balls.len()..before {
            self.passed += 1;
            if self.passed.is_multiple_of(BALLS_PER_LEVEL) && self.level < MAX_LEVEL {
                self.level += 1;
                self.sounds.push(Sound::Cleared);
            }
        }
    }
}

/// `m:ss.t`
pub fn time_text(secs: f32) -> String {
    let tenths = (secs.max(0.0) * 10.0) as u32;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

/// Stars in three layers: (how many, speed, pixel size, color). Near ones are faster,
/// bigger and brighter, which makes the depth.
const LAYERS: [(u32, f32, f32, Color32); 3] = [
    (70, 14.0, 1.0, Color32::from_rgb(0x5a, 0x4f, 0x8c)),
    (40, 34.0, 1.0, Color32::from_rgb(0xa8, 0x9c, 0xd8)),
    (18, 80.0, 2.0, Color32::from_rgb(0xff, 0xf6, 0xe0)),
];

/// Where star `i` of a layer is (stable positions, moving down, wrapping around).
pub fn star(layer: usize, i: u32, scroll: f32) -> (f32, f32) {
    let mut h = 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(u64::from(i) + 1 + (layer as u64 + 1) * 1000);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    let x = (h % W as u64) as f32;
    let y0 = ((h >> 20) % H as u64) as f32;
    (x, (y0 + scroll * LAYERS[layer].1).rem_euclid(H))
}

/// Half the width of a pixel-art ball of radius `r` in row `dy`.
pub fn row_half(r: i32, dy: i32) -> i32 {
    // A little over the radius: the top and bottom rows are a few pixels wide, not a nub.
    let outer = r as f32 + 0.4;
    (outer * outer - (dy * dy) as f32).max(0.0).sqrt() as i32
}

// --------------------------------------------------------------------------------- screen

#[derive(Debug, Default, Clone)]
pub struct DodgerInput {
    pub x: f32,
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

pub enum DodgerState {
    Menu { selected: usize },
    Playing(Box<Game>),
    Paused { game: Box<Game>, selected: usize },
    EnterName { game: Box<Game>, entry: NameEntry },
    Over { game: Box<Game>, selected: usize },
    Scores,
}

pub struct DodgerView {
    pub state: DodgerState,
    seed: u64,
    /// The moving background behind the menu.
    idle: Game,
    /// Sound and music on / off (settings; switchable in the menus).
    pub sound: bool,
    pub music: bool,
    player: crate::music::Music,
    scores: Scores,
}

impl DodgerView {
    pub fn new(seed: u64, sound: bool, music: bool) -> Self {
        Self {
            state: DodgerState::Menu { selected: 0 },
            seed,
            idle: Game::new(seed),
            sound,
            music,
            player: Default::default(),
            scores: Scores::load(),
        }
    }

    fn music_item(&self) -> &'static str {
        if self.music {
            tr!("Music: on")
        } else {
            tr!("Music: off")
        }
    }

    fn sound_item(&self) -> &'static str {
        if self.sound {
            tr!("Sound: on")
        } else {
            tr!("Sound: off")
        }
    }

    fn new_game(&mut self) -> Box<Game> {
        self.seed = self
            .seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        Box::new(Game::new(self.seed))
    }

    /// True when the player wants back to the home screen.
    pub fn show(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen: Rect,
        s: f32,
        dt: f32,
        input: &DodgerInput,
    ) -> bool {
        painter.rect_filled(screen, 0.0, Color32::from_rgb(0x05, 0x03, 0x12));
        ui.ctx().request_repaint();
        let ppp = ui.ctx().pixels_per_point();
        let menu_input = PongInput {
            up: input.up,
            down: input.down,
            enter: input.enter,
            back: input.back,
            pause: input.pause,
            clicks: input.clicks,
            ..Default::default()
        };
        let menu = |title: &str, items: &[&str], selected: usize| {
            crate::pong::menu(ui, painter, screen, s, title, items, selected, &menu_input)
        };
        let state = std::mem::replace(&mut self.state, DodgerState::Menu { selected: 0 });
        let (next, quit) = match state {
            DodgerState::Menu { selected } => {
                self.idle.scroll += dt.clamp(0.0, 0.05);
                paint_game(painter, screen, ppp, &self.idle);
                crate::pong::dim(painter, screen);
                let items = [
                    tr!("Play"),
                    self.sound_item(),
                    self.music_item(),
                    tr!("High scores"),
                    tr!("Back"),
                ];
                let (selected, chosen) = menu(tr!("Star Dodger"), &items, selected);
                crate::pong::paint_hint(painter, screen, s, tr!("Left / right: dodge the balls"));
                match chosen {
                    Some(0) => (DodgerState::Playing(self.new_game()), false),
                    Some(1) => {
                        self.sound = !self.sound;
                        (DodgerState::Menu { selected }, false)
                    }
                    Some(2) => {
                        self.music = !self.music;
                        (DodgerState::Menu { selected }, false)
                    }
                    Some(3) => (DodgerState::Scores, false),
                    Some(_) => (DodgerState::Menu { selected }, true),
                    None => (DodgerState::Menu { selected }, input.back),
                }
            }
            DodgerState::Playing(mut game) => {
                game.step(dt, input.x);
                for sound in game.sounds.drain(..) {
                    if self.sound {
                        crate::sound::play(sound);
                    }
                }
                paint_game(painter, screen, ppp, &game);
                if game.phase == Phase::Over {
                    if self.scores.qualifies(Board::Dodger, game.passed) {
                        let entry = self.scores.name_entry_of(game.passed, NAME_LETTERS);
                        (DodgerState::EnterName { game, entry }, false)
                    } else {
                        (DodgerState::Over { game, selected: 0 }, false)
                    }
                } else if input.pause || input.back {
                    (DodgerState::Paused { game, selected: 0 }, false)
                } else {
                    (DodgerState::Playing(game), false)
                }
            }
            DodgerState::Paused { game, selected } => {
                paint_game(painter, screen, ppp, &game);
                crate::pong::dim(painter, screen);
                let items = [
                    tr!("Continue"),
                    self.sound_item(),
                    self.music_item(),
                    tr!("End the game"),
                ];
                let (selected, chosen) = menu(tr!("Paused"), &items, selected);
                match chosen {
                    Some(0) => (DodgerState::Playing(game), false),
                    Some(1) => {
                        self.sound = !self.sound;
                        (DodgerState::Paused { game, selected }, false)
                    }
                    Some(2) => {
                        self.music = !self.music;
                        (DodgerState::Paused { game, selected }, false)
                    }
                    Some(_) => (DodgerState::Menu { selected: 0 }, false),
                    None if input.back || input.pause => (DodgerState::Playing(game), false),
                    None => (DodgerState::Paused { game, selected }, false),
                }
            }
            DodgerState::EnterName { game, mut entry } => {
                paint_game(painter, screen, ppp, &game);
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
                        .add(Board::Dodger, &entry.name(), entry.score, &today);
                    (DodgerState::Scores, false)
                } else {
                    (DodgerState::EnterName { game, entry }, false)
                }
            }
            DodgerState::Scores => {
                highscores::paint_list(
                    painter,
                    screen,
                    s,
                    tr!("Star Dodger"),
                    self.scores.list(Board::Dodger),
                );
                if input.enter || input.back {
                    (DodgerState::Menu { selected: 0 }, false)
                } else {
                    (DodgerState::Scores, false)
                }
            }
            DodgerState::Over { game, selected } => {
                paint_game(painter, screen, ppp, &game);
                crate::pong::popup(
                    painter,
                    screen,
                    s,
                    Rect::from_center_size(
                        screen.center() + Vec2::new(0.0, -100.0 * s),
                        Vec2::new(1000.0 * s, 430.0 * s),
                    ),
                );
                let title = tr!(
                    "Balls: {balls} · {time}",
                    balls = game.passed,
                    time = time_text(game.time)
                );
                let items = [tr!("Play again"), tr!("Back")];
                let (selected, chosen) = menu(&title, &items, selected);
                match chosen {
                    Some(0) => (DodgerState::Playing(self.new_game()), false),
                    Some(_) => (DodgerState::Menu { selected: 0 }, false),
                    None if input.back => (DodgerState::Menu { selected: 0 }, false),
                    None => (DodgerState::Over { game, selected }, false),
                }
            }
        };
        self.state = next;
        // Through a pause too; it ends with the game.
        let in_game = matches!(
            self.state,
            DodgerState::Playing(_) | DodgerState::Paused { .. }
        );
        self.player.update(self.music && in_game && !quit);
        quit
    }
}

/// How one pixel of a lit ball looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BallPixel {
    pub tone: Tone,
    /// Where on the ball's own surface: -1 … 1 from pole to pole; turns as the ball rolls.
    pub latitude: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// The round white reflection.
    Reflection,
    Bright,
    Middle,
    Dark,
}

/// The light comes from the top right, in front of the ball (x right, y down, z to the viewer).
const LIGHT: [f32; 3] = [0.48, -0.60, 0.64];
/// The reflection: its center and radius, in ball radii.
const REFLECTION: [f32; 3] = [0.38, -0.44, 0.24];

/// Pixel (`dx`, `dy`) of a ball of radius `r` that has rolled by `roll` radians (`None`:
/// outside the ball). A sphere lit in a few flat tones, as in pixel art.
pub fn ball_pixel(r: i32, dx: i32, dy: i32, roll: f32) -> Option<BallPixel> {
    if dy.abs() > r || dx.abs() > row_half(r, dy) {
        return None;
    }
    let size = r as f32 + 0.5;
    let (nx, ny) = (dx as f32 / size, dy as f32 / size);
    let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
    let reflected = (nx - REFLECTION[0]).powi(2) + (ny - REFLECTION[1]).powi(2);
    let light = nx * LIGHT[0] + ny * LIGHT[1] + nz * LIGHT[2];
    let tone = if reflected < REFLECTION[2] * REFLECTION[2] {
        Tone::Reflection
    } else if light > 0.72 {
        Tone::Bright
    } else if light > 0.18 {
        Tone::Middle
    } else {
        Tone::Dark
    };
    Some(BallPixel {
        tone,
        // The surface turns around the x axis: its front moves downwards as `roll` grows.
        latitude: ny * roll.cos() - nz * roll.sin(),
    })
}

/// A color in a tone, for surfaces with only one color per stripe.
fn toned(color: Color32, tone: Tone) -> Color32 {
    let scale = |c: u8, f: f32| (f32::from(c) * f) as u8;
    let f = match tone {
        Tone::Reflection => return Color32::WHITE,
        Tone::Bright => 1.0,
        Tone::Middle => 0.78,
        Tone::Dark => 0.48,
    };
    Color32::from_rgb(
        scale(color.r(), f),
        scale(color.g(), f),
        scale(color.b(), f),
    )
}

/// Draws in the game's big pixels, exactly on the screen's pixel grid (no seams).
struct Canvas<'a> {
    painter: &'a egui::Painter,
    origin: Pos2,
    unit: f32,
    ppp: f32,
}

impl Canvas<'_> {
    fn snap(&self, v: f32) -> f32 {
        (v * self.ppp).round() / self.ppp
    }

    /// A rectangle in game pixels.
    fn rect(&self, x: f32, y: f32, w: f32, h: f32, color: Color32) {
        let min = Pos2::new(
            self.snap(self.origin.x + x * self.unit),
            self.snap(self.origin.y + y * self.unit),
        );
        let max = Pos2::new(
            self.snap(self.origin.x + (x + w) * self.unit),
            self.snap(self.origin.y + (y + h) * self.unit),
        );
        self.painter
            .rect_filled(Rect::from_min_max(min, max), 0.0, color);
    }

    /// A lit ball of big pixels, rolled by `roll`; `color` picks each pixel's color. The
    /// center moves in screen pixels (smooth), the pixels stay whole.
    fn ball(&self, cx: f32, cy: f32, r: i32, roll: f32, color: impl Fn(BallPixel) -> Color32) {
        let center = Pos2::new(
            self.snap(self.origin.x + cx * self.unit),
            self.snap(self.origin.y + cy * self.unit),
        );
        for dy in -r..=r {
            let half = row_half(r, dy);
            // Pixels of the same color in a row become one rectangle.
            let mut start = -half;
            let mut current = ball_pixel(r, -half, dy, roll).map(&color);
            for dx in -half + 1..=half + 1 {
                let next = ball_pixel(r, dx, dy, roll).map(&color);
                if next != current {
                    if let Some(c) = current {
                        let min =
                            center + Vec2::new(start as f32 - 0.5, dy as f32 - 0.5) * self.unit;
                        let size = Vec2::new((dx - start) as f32, 1.0) * self.unit;
                        self.painter
                            .rect_filled(Rect::from_min_size(min, size), 0.0, c);
                    }
                    (start, current) = (dx, next);
                }
            }
        }
    }

    /// Digits, `:` and `.` in a 3 × 5 pixel font, right-aligned at `right`.
    fn text_right(&self, right: f32, y: f32, size: f32, text: &str, color: Color32) {
        let mut x = right - text_width(text, size);
        for c in text.chars() {
            let rows = match c {
                '0' => [7, 5, 5, 5, 7],
                '1' => [2, 6, 2, 2, 7],
                '2' => [7, 1, 7, 4, 7],
                '3' => [7, 1, 3, 1, 7],
                '4' => [5, 5, 7, 1, 1],
                '5' => [7, 4, 7, 1, 7],
                '6' => [7, 4, 7, 5, 7],
                '7' => [7, 1, 2, 2, 2],
                '8' => [7, 5, 7, 5, 7],
                '9' => [7, 5, 7, 1, 7],
                ':' => [0, 2, 0, 2, 0],
                _ => [0, 0, 0, 0, 2],
            };
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..3 {
                    if bits & (4 >> col) != 0 {
                        let shift = if glyph_w(c) < 4.0 {
                            col as f32 - 1.0
                        } else {
                            col as f32
                        };
                        self.rect(x + shift * size, y + row as f32 * size, size, size, color);
                    }
                }
            }
            x += glyph_w(c) * size;
        }
    }
}

fn glyph_w(c: char) -> f32 {
    if c == ':' || c == '.' { 2.0 } else { 4.0 }
}

/// Width of a text in the pixel font (game pixels).
fn text_width(text: &str, size: f32) -> f32 {
    text.chars().map(glyph_w).sum::<f32>() * size
}

/// The largest size up to `max` at which a text fits `width`.
fn fit_size(text: &str, max: u32, width: f32) -> f32 {
    (1..=max)
        .rev()
        .map(|size| size as f32)
        .find(|size| text_width(text, *size) <= width)
        .unwrap_or(1.0)
}

/// The numbers beside the field: balls passed on the left, time and level on the right.
fn paint_numbers(canvas: &Canvas, game: &Game) {
    let room = PANEL - 4.0;
    let passed = game.passed.to_string();
    canvas.text_right(
        -4.0,
        6.0,
        fit_size(&passed, 5, room),
        &passed,
        Color32::from_rgb(0xff, 0x8a, 0xc8),
    );
    let time = time_text(game.time);
    let size = fit_size(&time, 2, room);
    canvas.text_right(
        W + 4.0 + text_width(&time, size),
        6.0,
        size,
        &time,
        Color32::from_rgb(0xff, 0xc8, 0x4a),
    );
    // The level: big, at the bottom right.
    let level = game.level.to_string();
    let size = fit_size(&level, 8, room);
    canvas.text_right(
        W + PANEL,
        H - 4.0 - 5.0 * size,
        size,
        &level,
        Color32::from_rgb(0x5a, 0xff, 0xd0),
    );
}

fn paint_game(painter: &egui::Painter, screen: Rect, ppp: f32, game: &Game) {
    // A whole number of screen pixels per game pixel, with room for the numbers beside it.
    let unit =
        (((screen.height() * ppp / H).min(screen.width() * ppp / (W + 2.0 * PANEL))).floor() / ppp)
            .max(1.0 / ppp);
    let size = Vec2::new(W, H) * unit;
    let canvas = Canvas {
        painter: &painter.with_clip_rect(Rect::from_center_size(screen.center(), size)),
        origin: screen.center() - size / 2.0,
        unit,
        ppp,
    };
    canvas.rect(0.0, 0.0, W, H, Color32::from_rgb(0x0d, 0x06, 0x24));
    for (layer, (count, _, pixel, color)) in LAYERS.iter().enumerate() {
        for i in 0..*count {
            let (x, y) = star(layer, i, game.scroll);
            canvas.rect(x, y, *pixel, *pixel, *color);
        }
    }
    // The moving grid.
    const CELL: f32 = 24.0;
    let line = Color32::from_rgb(0x4a, 0x24, 0x7e);
    let mut x = WALL + CELL;
    while x < W - WALL {
        canvas.rect(x, 0.0, 1.0, H, line);
        x += CELL;
    }
    let mut y = (game.scroll * 40.0).rem_euclid(CELL) - CELL;
    while y < H {
        canvas.rect(WALL, y, W - 2.0 * WALL, 1.0, line);
        y += CELL;
    }
    // Side walls: blocks moving with the background.
    let mut y = (game.scroll * 40.0).rem_euclid(16.0) - 16.0;
    let mut odd = false;
    while y < H {
        let color = if odd {
            Color32::from_rgb(0xc4, 0x2e, 0xa0)
        } else {
            Color32::from_rgb(0x7a, 0x1e, 0x8c)
        };
        canvas.rect(0.0, y, WALL, 8.0, color);
        canvas.rect(W - WALL, y, WALL, 8.0, color);
        y += 8.0;
        odd = !odd;
    }

    for ball in &game.balls {
        // Rolling as it falls, with a band in another color around it.
        let body = BALLS[ball.color];
        let band = BALLS[(ball.color + 3) % BALLS.len()];
        canvas.ball(ball.x, ball.y, BALL_R as i32, ball.y / BALL_R, |p| {
            let colors = if p.latitude.abs() < 0.3 { band } else { body };
            match p.tone {
                Tone::Reflection => Color32::WHITE,
                Tone::Bright => colors[0],
                Tone::Middle => colors[1],
                Tone::Dark => colors[2],
            }
        });
    }
    // The player: stripes rolling with the game; blinks when hit.
    let hit = matches!(game.phase, Phase::Crash(t) if (t * 10.0) as i32 % 2 == 0);
    let visible = !matches!(game.phase, Phase::Over);
    if visible && !hit {
        canvas.ball(
            game.player,
            PLAYER_Y,
            PLAYER_R as i32,
            game.scroll * 6.0,
            |p| {
                let along = p.latitude.clamp(-1.0, 1.0).asin() / std::f32::consts::PI + 0.5;
                let stripe = ((along * STRIPES.len() as f32) as usize).min(STRIPES.len() - 1);
                toned(STRIPES[stripe], p.tone)
            },
        );
    }

    paint_numbers(&Canvas { painter, ..canvas }, game);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ball just past the bottom edge of the field next frame.
    fn pass_one(g: &mut Game) {
        g.balls.clear();
        g.balls.push(Ball {
            x: if g.player < W / 2.0 { W - 30.0 } else { 30.0 },
            y: H + BALL_R,
            color: 0,
        });
        g.fallen = 0.0;
        g.step(0.001, 0.0);
    }

    #[test]
    fn faster_every_ten_balls_up_to_level_99() {
        let mut g = Game::new(1);
        for n in 1..=9 {
            pass_one(&mut g);
            assert_eq!((g.passed, g.level), (n, 1));
        }
        assert!(g.sounds.is_empty(), "no sound for a ball passed");
        pass_one(&mut g);
        assert_eq!(g.level, 2);
        assert_eq!(g.sounds.last(), Some(&Sound::Cleared), "level up");
        assert!(fall_speed(2) > fall_speed(1));
        for _ in 0..1100 {
            pass_one(&mut g);
        }
        assert_eq!(g.level, 99);
        assert_eq!(fall_speed(500), fall_speed(99));
        // Every level is faster than the one before.
        for level in 1..99 {
            assert!(fall_speed(level + 1) > fall_speed(level), "level {level}");
        }
        assert_eq!(fall_speed(9), 234.0);
        assert_eq!(
            fall_speed(99),
            504.0,
            "still far from crossing the field in a frame"
        );
    }

    #[test]
    fn each_ball_starts_elsewhere_in_another_color_inside_the_walls() {
        let mut g = Game::new(7);
        let mut seen = Vec::new();
        for _ in 0..300 {
            g.spawn();
            seen.push(*g.balls.last().unwrap());
        }
        for pair in seen.windows(2) {
            assert!((pair[0].x - pair[1].x).abs() >= MIN_SIDEWAYS - 0.01);
            assert_ne!(pair[0].color, pair[1].color);
        }
        for b in &seen {
            assert!(
                b.x >= WALL + BALL_R - 0.01 && b.x <= W - WALL - BALL_R + 0.01,
                "{b:?}"
            );
        }
        let colors: std::collections::BTreeSet<_> = seen.iter().map(|b| b.color).collect();
        assert_eq!(colors.len(), BALLS.len());
    }

    #[test]
    fn a_hit_ends_the_game_and_the_time_stops() {
        let mut g = Game::new(1);
        g.fallen = 0.0;
        g.balls.push(Ball {
            x: g.player,
            y: PLAYER_Y - 30.0,
            color: 0,
        });
        for _ in 0..30 {
            g.step(1.0 / 60.0, 0.0);
        }
        assert!(matches!(g.phase, Phase::Crash(_)), "{:?}", g.phase);
        assert_eq!(g.sounds.last(), Some(&Sound::GameOver));
        let time = g.time;
        for _ in 0..100 {
            g.step(1.0 / 60.0, 0.0);
        }
        assert_eq!(g.phase, Phase::Over);
        assert_eq!(g.time, time);
    }

    #[test]
    fn a_ball_beside_the_player_passes() {
        let mut g = Game::new(1);
        g.fallen = 0.0;
        let beside = g.player + PLAYER_R + BALL_R;
        g.balls.push(Ball {
            x: beside,
            y: PLAYER_Y - 30.0,
            color: 0,
        });
        for _ in 0..20 {
            g.step(1.0 / 60.0, 0.0);
            g.balls.retain(|b| b.x == beside);
        }
        assert_eq!(g.phase, Phase::Play);
    }

    #[test]
    fn the_player_stays_between_the_walls() {
        let mut g = Game::new(1);
        g.fallen = -100_000.0;
        for _ in 0..200 {
            g.step(0.05, -1.0);
        }
        assert_eq!(g.player, WALL + PLAYER_R);
        for _ in 0..200 {
            g.step(0.05, 5.0);
        }
        assert_eq!(g.player, W - WALL - PLAYER_R);
    }

    #[test]
    fn someone_who_never_moves_is_hit() {
        let mut g = Game::new(3);
        for _ in 0..60 * 60 {
            g.step(1.0 / 60.0, 0.0);
            if g.phase != Phase::Play {
                return;
            }
        }
        panic!("standing still survived a minute");
    }

    #[test]
    fn time_is_counted_and_shown() {
        assert_eq!(time_text(0.0), "0:00.0");
        assert_eq!(time_text(12.34), "0:12.3");
        assert_eq!(time_text(83.0), "1:23.0");
        let mut g = Game::new(1);
        g.fallen = -100_000.0;
        for _ in 0..60 {
            g.step(1.0 / 60.0, 0.0);
        }
        assert!((g.time - 1.0).abs() < 0.01);
    }

    #[test]
    fn stars_stay_on_screen_and_near_layers_move_faster() {
        for (layer, (count, ..)) in LAYERS.iter().enumerate() {
            for i in 0..*count {
                let (x, y) = star(layer, i, 12.3);
                assert!((0.0..W).contains(&x) && (0.0..H).contains(&y));
            }
        }
        assert!(LAYERS[0].1 < LAYERS[1].1 && LAYERS[1].1 < LAYERS[2].1);
    }

    #[test]
    fn numbers_fit_beside_the_field() {
        let room = PANEL - 4.0;
        assert_eq!(fit_size("7", 8, room), 8.0, "the level, big");
        assert_eq!(fit_size("99", 8, room), 5.0);
        assert_eq!(fit_size("7", 5, room), 5.0);
        assert_eq!(fit_size("123", 5, room), 3.0);
        assert_eq!(fit_size("12345", 5, room), 2.0);
        assert_eq!(fit_size(&time_text(83.0), 2, room), 2.0);
        assert_eq!(
            fit_size(&time_text(700.0), 2, room),
            1.0,
            "from 10 minutes on"
        );
        for text in ["0", "99999", "1:23.4", "59:59.9"] {
            let size = fit_size(text, 5, room);
            assert!(text_width(text, size) <= room, "{text}");
        }
    }

    #[test]
    fn balls_are_lit_from_the_top_right() {
        let r = BALL_R as i32;
        let tone = |dx, dy| ball_pixel(r, dx, dy, 0.0).unwrap().tone;
        // A round white reflection, up and to the right of the middle.
        let at = |v: f32| (v * (r as f32 + 0.5)).round() as i32;
        assert_eq!(tone(at(REFLECTION[0]), at(REFLECTION[1])), Tone::Reflection);
        assert_ne!(
            tone(-at(REFLECTION[0]), at(REFLECTION[1])),
            Tone::Reflection
        );
        assert_eq!(tone(-5, 6), Tone::Dark, "the side away from the light");
        assert_eq!(tone(0, 0), Tone::Middle);
        let mut white = 0;
        for dy in -r..=r {
            for dx in -r..=r {
                white += i32::from(
                    ball_pixel(r, dx, dy, 0.0).is_some_and(|p| p.tone == Tone::Reflection),
                );
            }
        }
        assert!((4..=20).contains(&white), "{white} white pixels");
        assert_eq!(ball_pixel(r, r, r, 0.0), None, "outside");
        assert!(ball_pixel(r, 0, -r, 0.0).is_some());
    }

    #[test]
    fn the_surface_rolls_downwards_and_the_light_stays() {
        let r = BALL_R as i32;
        // The band (latitude 0) is at the middle at first, lower after a little roll.
        let row_of_band = |roll: f32| {
            (-r..=r)
                .min_by(|a, b| {
                    let lat = |y| ball_pixel(r, 0, y, roll).unwrap().latitude.abs();
                    lat(*a).total_cmp(&lat(*b))
                })
                .unwrap()
        };
        assert_eq!(row_of_band(0.0), 0);
        assert!(row_of_band(0.5) > 0);
        assert!(row_of_band(1.0) > row_of_band(0.5));
        // The light doesn't turn with the ball.
        assert_eq!(
            ball_pixel(r, 3, -4, 0.0).unwrap().tone,
            ball_pixel(r, 3, -4, 2.0).unwrap().tone
        );
    }

    #[test]
    fn pixel_balls_are_round() {
        assert_eq!(row_half(10, 0), 10);
        assert_eq!(row_half(10, 10), 2, "no one-pixel nub at the top");
        assert_eq!(row_half(9, 9), 2);
        assert_eq!(row_half(10, -6), row_half(10, 6));
        assert_eq!(row_half(3, 9), 0);
    }
}
