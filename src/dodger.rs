//! Star Dodger: steer the striped ball left and right past the balls coming down. Every 6
//! balls passed the game gets faster (the level digit at the bottom right); the time runs at
//! the top right. One hit ends the game.
//!
//! Drawn like an 80s/90s console game: a 288 × 216 picture of big square pixels at a whole
//! number of screen pixels each. Objects move in screen pixels, so the motion stays smooth
//! while the art keeps its pixels. Everything is drawn here, no image files.
//!
//! The rules (`Game`) are pure and tested; `DodgerView` shows menus and paints.

use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

use crate::pong::PongInput;
use crate::tr;

pub const W: f32 = 288.0;
pub const H: f32 = 216.0;
/// The side walls' width.
const WALL: f32 = 8.0;
const PLAYER_Y: f32 = H - 22.0;
const PLAYER_R: f32 = 10.0;
const BALL_R: f32 = 9.0;
const PLAYER_SPEED: f32 = 190.0;
/// Fall speed at level 1, and what each level adds (pixels / s).
const FALL_START: f32 = 90.0;
const FALL_STEP: f32 = 18.0;
pub const MAX_LEVEL: u32 = 9;
pub const BALLS_PER_LEVEL: u32 = 6;
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
}

pub fn fall_speed(level: u32) -> f32 {
    FALL_START + FALL_STEP * (level.clamp(1, MAX_LEVEL) - 1) as f32
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
        self.scroll += dt * (1.0 + 0.15 * (self.level - 1) as f32);
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
            return;
        }
        let before = self.balls.len();
        self.balls.retain(|b| b.y - BALL_R <= H);
        for _ in self.balls.len()..before {
            self.passed += 1;
            if self.passed.is_multiple_of(BALLS_PER_LEVEL) && self.level < MAX_LEVEL {
                self.level += 1;
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
    (((r * r - dy * dy).max(0) as f32).sqrt() + 0.5) as i32
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
}

pub enum DodgerState {
    Menu { selected: usize },
    Playing(Box<Game>),
    Paused { game: Box<Game>, selected: usize },
    Over { game: Box<Game>, selected: usize },
}

pub struct DodgerView {
    pub state: DodgerState,
    seed: u64,
    /// The moving background behind the menu.
    idle: Game,
}

impl DodgerView {
    pub fn new(seed: u64) -> Self {
        Self {
            state: DodgerState::Menu { selected: 0 },
            seed,
            idle: Game::new(seed),
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
                let items = [tr!("Play"), tr!("Back")];
                let (selected, chosen) = menu(tr!("Star Dodger"), &items, selected);
                crate::pong::paint_hint(painter, screen, s, tr!("Left / right: dodge the balls"));
                match chosen {
                    Some(0) => (DodgerState::Playing(self.new_game()), false),
                    Some(_) => (DodgerState::Menu { selected }, true),
                    None => (DodgerState::Menu { selected }, input.back),
                }
            }
            DodgerState::Playing(mut game) => {
                game.step(dt, input.x);
                paint_game(painter, screen, ppp, &game);
                if game.phase == Phase::Over {
                    (DodgerState::Over { game, selected: 0 }, false)
                } else if input.pause || input.back {
                    (DodgerState::Paused { game, selected: 0 }, false)
                } else {
                    (DodgerState::Playing(game), false)
                }
            }
            DodgerState::Paused { game, selected } => {
                paint_game(painter, screen, ppp, &game);
                crate::pong::dim(painter, screen);
                let items = [tr!("Continue"), tr!("End the game")];
                let (selected, chosen) = menu(tr!("Paused"), &items, selected);
                match chosen {
                    Some(0) => (DodgerState::Playing(game), false),
                    Some(_) => (DodgerState::Menu { selected: 0 }, false),
                    None if input.back || input.pause => (DodgerState::Playing(game), false),
                    None => (DodgerState::Paused { game, selected }, false),
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
                    "{time} · level {level}",
                    time = time_text(game.time),
                    level = game.level
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
        quit
    }
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

    /// A round ball of pixel rows; `color` picks each row's color. The center moves in
    /// screen pixels (smooth), the rows stay whole big pixels.
    fn ball(&self, cx: f32, cy: f32, r: i32, color: impl Fn(i32) -> Color32) {
        let center = Pos2::new(
            self.snap(self.origin.x + cx * self.unit),
            self.snap(self.origin.y + cy * self.unit),
        );
        for dy in -r..=r {
            let half = row_half(r, dy);
            let min = center + Vec2::new(-(half as f32 + 0.5), dy as f32 - 0.5) * self.unit;
            let size = Vec2::new((2 * half + 1) as f32, 1.0) * self.unit;
            self.painter
                .rect_filled(Rect::from_min_size(min, size), 0.0, color(dy));
        }
    }

    /// Digits, `:` and `.` in a 3 × 5 pixel font, right-aligned at `right`.
    fn text_right(&self, right: f32, y: f32, size: f32, text: &str, color: Color32) {
        let glyph_w = |c: char| if c == ':' || c == '.' { 2.0 } else { 4.0 };
        let width: f32 = text.chars().map(glyph_w).sum::<f32>() * size;
        let mut x = right - width;
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

fn paint_game(painter: &egui::Painter, screen: Rect, ppp: f32, game: &Game) {
    // A whole number of screen pixels per game pixel.
    let unit =
        (((screen.height() * ppp / H).min(screen.width() * ppp / W)).floor() / ppp).max(1.0 / ppp);
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
        let [bright, middle, dark] = BALLS[ball.color];
        let r = BALL_R as i32;
        canvas.ball(ball.x, ball.y, r, |dy| {
            if dy < -r / 3 {
                bright
            } else if dy < r / 2 {
                middle
            } else {
                dark
            }
        });
    }
    // The player: stripes rolling with the game; blinks when hit.
    let hit = matches!(game.phase, Phase::Crash(t) if (t * 10.0) as i32 % 2 == 0);
    let visible = !matches!(game.phase, Phase::Over);
    if visible && !hit {
        let roll = (game.scroll * 12.0) as i32;
        canvas.ball(game.player, PLAYER_Y, PLAYER_R as i32, |dy| {
            STRIPES[((dy + 20 + roll) / 3).rem_euclid(STRIPES.len() as i32) as usize]
        });
    }

    let clock = Color32::from_rgb(0xff, 0xc8, 0x4a);
    canvas.text_right(W - WALL - 4.0, 5.0, 2.0, &time_text(game.time), clock);
    let digit = Color32::from_rgb(0x5a, 0xff, 0xd0);
    canvas.text_right(
        W - WALL - 4.0,
        H - 44.0,
        8.0,
        &game.level.to_string(),
        digit,
    );
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
    fn faster_every_six_balls_up_to_level_9() {
        let mut g = Game::new(1);
        for n in 1..=5 {
            pass_one(&mut g);
            assert_eq!((g.passed, g.level), (n, 1));
        }
        pass_one(&mut g);
        assert_eq!(g.level, 2);
        assert!(fall_speed(2) > fall_speed(1));
        for _ in 0..200 {
            pass_one(&mut g);
        }
        assert_eq!(g.level, MAX_LEVEL);
        assert_eq!(fall_speed(99), fall_speed(MAX_LEVEL));
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
    fn pixel_balls_are_round() {
        assert_eq!(row_half(10, 0), 10);
        assert_eq!(row_half(10, 10), 0, "the top row is one pixel wide");
        assert_eq!(row_half(10, -6), row_half(10, 6));
        assert_eq!(row_half(3, 9), 0);
    }
}
