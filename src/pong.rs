//! Ping Pong: a small built-in game for 1 player (against the computer) or 2 players, with
//! gamepads (`remote.rs` reads them), the remote or the keyboard.
//!
//! The field is 16 × 9 units (scaled to the screen). The rules (`Game::step`) are pure and
//! tested without a display; the screen (`PongView::show`) only turns input into paddle
//! movement and paints.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Vec2};

use crate::config::PongLevel;
use crate::tr;

pub const W: f32 = 16.0;
pub const H: f32 = 9.0;
const PADDLE_H: f32 = 1.8;
const PADDLE_W: f32 = 0.28;
/// Distance of the paddles' centers from the side walls.
const PADDLE_X: f32 = 0.7;
const BALL: f32 = 0.28;
const PADDLE_SPEED: f32 = 10.0;
const BALL_START: f32 = 7.0;
const BALL_MAX: f32 = 15.0;
const SPEED_UP: f32 = 1.06;
/// Steepest bounce off a paddle's edge.
const MAX_ANGLE: f32 = 1.0; // radians, about 57°
const SERVE_DELAY: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// How the computer plays the right paddle.
struct Skill {
    /// Its paddle's top speed (the players have `PADDLE_SPEED`).
    speed: f32,
    /// How far off it aims, at most (new error after every hit). More than half the paddle
    /// plus ball (≈ 1.04) can miss.
    error: f32,
    /// It only starts following the ball once the ball is past this x (reaction).
    reacts_from: f32,
}

fn skill(level: PongLevel) -> Skill {
    match level {
        PongLevel::Easy => Skill {
            speed: 3.5,
            error: 1.6,
            reacts_from: W * 0.6,
        },
        PongLevel::Normal => Skill {
            speed: 4.8,
            error: 1.25,
            reacts_from: W * 0.5,
        },
        PongLevel::Hard => Skill {
            speed: 8.5,
            error: 0.3,
            reacts_from: 0.0,
        },
    }
}

#[derive(Debug, Clone)]
pub struct Game {
    /// Paddle centers (y).
    pub left: f32,
    pub right: f32,
    pub ball: [f32; 2],
    pub velocity: [f32; 2],
    pub score: [u32; 2],
    pub to_win: u32,
    /// The computer plays the right paddle.
    pub computer: Option<PongLevel>,
    /// Seconds until the next serve, and towards whom.
    pub serve_in: f32,
    pub serve_to: Side,
    pub winner: Option<Side>,
    ai_error: f32,
    rng: u64,
}

impl Game {
    pub fn new(to_win: u32, computer: Option<PongLevel>, seed: u64) -> Self {
        Self {
            left: H / 2.0,
            right: H / 2.0,
            ball: [W / 2.0, H / 2.0],
            velocity: [0.0, 0.0],
            score: [0, 0],
            to_win: to_win.max(1),
            computer,
            serve_in: SERVE_DELAY,
            serve_to: Side::Left,
            winner: None,
            ai_error: 0.0,
            rng: seed | 1,
        }
    }

    /// Random number in -1..1 (xorshift; deterministic for tests).
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % 20_001) as f32 / 10_000.0 - 1.0
    }

    /// Advance by `dt` seconds. `left` / `right`: paddle input, -1 (up) … 1 (down); `right`
    /// is ignored when the computer plays.
    pub fn step(&mut self, dt: f32, left: f32, right: f32) {
        if self.winner.is_some() {
            return;
        }
        let dt = dt.clamp(0.0, 0.05);
        let right = match self.computer {
            Some(level) => self.computer_input(level),
            None => right.clamp(-1.0, 1.0) * PADDLE_SPEED,
        };
        let limit = |y: f32| y.clamp(PADDLE_H / 2.0, H - PADDLE_H / 2.0);
        self.left = limit(self.left + left.clamp(-1.0, 1.0) * PADDLE_SPEED * dt);
        self.right = limit(self.right + right * dt);

        if self.serve_in > 0.0 {
            self.serve_in -= dt;
            self.ball = [W / 2.0, H / 2.0];
            if self.serve_in <= 0.0 {
                let angle = self.random() * 0.5;
                let direction = if self.serve_to == Side::Left {
                    -1.0
                } else {
                    1.0
                };
                self.velocity = [
                    direction * BALL_START * angle.cos(),
                    BALL_START * angle.sin(),
                ];
                self.new_ai_error();
            }
            return;
        }
        // Small steps, so a fast ball can't jump through a paddle.
        let speed = self.velocity[0].hypot(self.velocity[1]);
        let steps = ((speed * dt) / (BALL / 2.0)).ceil().max(1.0) as u32;
        for _ in 0..steps {
            if self.move_ball(dt / steps as f32) {
                return;
            }
        }
    }

    /// One small step of the ball. True when a point was scored.
    fn move_ball(&mut self, dt: f32) -> bool {
        let r = BALL / 2.0;
        self.ball[0] += self.velocity[0] * dt;
        self.ball[1] += self.velocity[1] * dt;
        if self.ball[1] < r {
            self.ball[1] = r;
            self.velocity[1] = self.velocity[1].abs();
        } else if self.ball[1] > H - r {
            self.ball[1] = H - r;
            self.velocity[1] = -self.velocity[1].abs();
        }
        let reach = PADDLE_H / 2.0 + r;
        let left_face = PADDLE_X + PADDLE_W / 2.0;
        let right_face = W - PADDLE_X - PADDLE_W / 2.0;
        if self.velocity[0] < 0.0
            && self.ball[0] - r <= left_face
            && self.ball[0] + r >= PADDLE_X - PADDLE_W / 2.0
            && (self.ball[1] - self.left).abs() <= reach
        {
            self.bounce(Side::Left, (self.ball[1] - self.left) / reach);
            self.ball[0] = left_face + r;
        } else if self.velocity[0] > 0.0
            && self.ball[0] + r >= right_face
            && self.ball[0] - r <= W - PADDLE_X + PADDLE_W / 2.0
            && (self.ball[1] - self.right).abs() <= reach
        {
            self.bounce(Side::Right, (self.ball[1] - self.right) / reach);
            self.ball[0] = right_face - r;
        }
        if self.ball[0] < -r {
            self.point(Side::Right);
            return true;
        }
        if self.ball[0] > W + r {
            self.point(Side::Left);
            return true;
        }
        false
    }

    /// Off a paddle: the further from its middle, the steeper; a bit faster every hit.
    fn bounce(&mut self, paddle: Side, offset: f32) {
        let speed = (self.velocity[0].hypot(self.velocity[1]) * SPEED_UP).min(BALL_MAX);
        let angle = offset.clamp(-1.0, 1.0) * MAX_ANGLE;
        let direction = if paddle == Side::Left { 1.0 } else { -1.0 };
        self.velocity = [direction * speed * angle.cos(), speed * angle.sin()];
        self.new_ai_error();
    }

    fn point(&mut self, scorer: Side) {
        let i = if scorer == Side::Left { 0 } else { 1 };
        self.score[i] += 1;
        if self.score[i] >= self.to_win {
            self.winner = Some(scorer);
        }
        // The next ball goes to whoever lost the point.
        self.serve_to = if scorer == Side::Left {
            Side::Right
        } else {
            Side::Left
        };
        self.serve_in = SERVE_DELAY;
        self.ball = [W / 2.0, H / 2.0];
        self.velocity = [0.0, 0.0];
    }

    fn new_ai_error(&mut self) {
        if let Some(level) = self.computer {
            self.ai_error = self.random() * skill(level).error;
        }
    }

    /// The computer's paddle speed: towards where the ball will arrive (bounces included),
    /// a little off; back to the middle while the ball moves away.
    fn computer_input(&self, level: PongLevel) -> f32 {
        let skill = skill(level);
        let target = if self.velocity[0] > 0.0 && self.ball[0] >= skill.reacts_from {
            let x = W - PADDLE_X - PADDLE_W / 2.0 - BALL / 2.0;
            arrival_y(self.ball, self.velocity, x) + self.ai_error
        } else if self.velocity[0] > 0.0 {
            return 0.0; // not reacting yet
        } else {
            H / 2.0
        };
        ((target - self.right) / 0.25).clamp(-1.0, 1.0) * skill.speed
    }
}

/// Where a ball at `pos` moving with `velocity` reaches `x`, bouncing off the walls.
pub fn arrival_y(pos: [f32; 2], velocity: [f32; 2], x: f32) -> f32 {
    if velocity[0] == 0.0 {
        return pos[1];
    }
    let r = BALL / 2.0;
    let t = ((x - pos[0]) / velocity[0]).max(0.0);
    let span = H - 2.0 * r;
    // Unfold the bounces: a triangle wave between r and H - r.
    let y = (pos[1] - r + velocity[1] * t).rem_euclid(2.0 * span);
    r + if y <= span { y } else { 2.0 * span - y }
}

// --------------------------------------------------------------------------------- screen

/// Input for one frame.
#[derive(Debug, Default, Clone)]
pub struct PongInput {
    /// Paddle input, -1 (up) … 1 (down).
    pub left: f32,
    pub right: f32,
    /// Menu navigation (edges).
    pub up: bool,
    pub down: bool,
    pub enter: bool,
    pub back: bool,
    /// Start / select on a gamepad, Esc, the remote's back: pause.
    pub pause: bool,
    /// Gamepads connected (for the menu's hint).
    pub gamepads: usize,
    /// Clicks count (not right after the game was opened by a click).
    pub clicks: bool,
}

pub enum PongState {
    Menu { selected: usize },
    Playing(Game),
    Paused { game: Game, selected: usize },
    Over { game: Game, selected: usize },
}

pub struct PongView {
    pub state: PongState,
    pub level: PongLevel,
    pub to_win: u32,
    /// Players of the current / last game (1 or 2), for "Play again".
    players: u8,
    seed: u64,
}

/// What the game wants the media center to do.
pub enum PongAction {
    None,
    /// Back to the home screen.
    Quit,
}

impl PongView {
    pub fn new(level: PongLevel, to_win: u32, seed: u64) -> Self {
        Self {
            state: PongState::Menu { selected: 0 },
            level,
            to_win,
            players: 1,
            seed,
        }
    }

    fn new_game(&mut self, players: u8) -> Game {
        self.players = players;
        self.seed = self
            .seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let computer = (players == 1).then_some(self.level);
        Game::new(self.to_win, computer, self.seed)
    }

    pub fn show(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        screen: Rect,
        s: f32,
        dt: f32,
        input: &PongInput,
    ) -> PongAction {
        painter.rect_filled(screen, 0.0, Color32::from_rgb(0x0b, 0x10, 0x1a));
        let state = std::mem::replace(&mut self.state, PongState::Menu { selected: 0 });
        let (next, action) = match state {
            PongState::Menu { selected } => {
                let items = [
                    tr!("1 player (against the computer)"),
                    tr!("2 players"),
                    tr!("Back"),
                ];
                let hint = match input.gamepads {
                    0 => tr!("No gamepad found: use the remote's arrows, or connect a gamepad.")
                        .to_string(),
                    1 => tr!("1 gamepad connected").to_string(),
                    n => tr!("{n} gamepads connected", n = n),
                };
                let (selected, chosen) = menu(
                    ui,
                    painter,
                    screen,
                    s,
                    tr!("Ping Pong"),
                    &items,
                    selected,
                    input,
                );
                paint_hint(painter, screen, s, &hint);
                match chosen {
                    Some(0) => (PongState::Playing(self.new_game(1)), PongAction::None),
                    Some(1) => (PongState::Playing(self.new_game(2)), PongAction::None),
                    Some(_) => (PongState::Menu { selected }, PongAction::Quit),
                    None if input.back => (PongState::Menu { selected }, PongAction::Quit),
                    None => (PongState::Menu { selected }, PongAction::None),
                }
            }
            PongState::Playing(mut game) => {
                game.step(dt, input.left, input.right);
                paint_game(painter, screen, s, &game);
                ui.ctx().request_repaint();
                if game.winner.is_some() {
                    (PongState::Over { game, selected: 0 }, PongAction::None)
                } else if input.pause || input.back {
                    (PongState::Paused { game, selected: 0 }, PongAction::None)
                } else {
                    (PongState::Playing(game), PongAction::None)
                }
            }
            PongState::Paused { game, selected } => {
                paint_game(painter, screen, s, &game);
                dim(painter, screen);
                let items = [tr!("Continue"), tr!("End the game")];
                let (selected, chosen) = menu(
                    ui,
                    painter,
                    screen,
                    s,
                    tr!("Paused"),
                    &items,
                    selected,
                    input,
                );
                match chosen {
                    Some(0) => (PongState::Playing(game), PongAction::None),
                    Some(_) => (PongState::Menu { selected: 0 }, PongAction::None),
                    None if input.back || input.pause => {
                        (PongState::Playing(game), PongAction::None)
                    }
                    None => (PongState::Paused { game, selected }, PongAction::None),
                }
            }
            PongState::Over { game, selected } => {
                paint_game(painter, screen, s, &game);
                dim(painter, screen);
                let title = match (game.winner, game.computer) {
                    (Some(Side::Left), Some(_)) => tr!("You win!").to_string(),
                    (Some(Side::Right), Some(_)) => tr!("The computer wins").to_string(),
                    (Some(Side::Left), None) => tr!("Left wins!").to_string(),
                    _ => tr!("Right wins!").to_string(),
                };
                let items = [tr!("Play again"), tr!("Back")];
                let (selected, chosen) =
                    menu(ui, painter, screen, s, &title, &items, selected, input);
                match chosen {
                    Some(0) => {
                        let players = self.players;
                        (PongState::Playing(self.new_game(players)), PongAction::None)
                    }
                    Some(_) => (PongState::Menu { selected: 0 }, PongAction::None),
                    None if input.back => (PongState::Menu { selected: 0 }, PongAction::None),
                    None => (PongState::Over { game, selected }, PongAction::None),
                }
            }
        };
        self.state = next;
        action
    }
}

/// A vertical menu; returns the selection and the chosen item (OK or a click).
#[allow(clippy::too_many_arguments)]
pub(crate) fn menu(
    ui: &egui::Ui,
    painter: &egui::Painter,
    screen: Rect,
    s: f32,
    title: &str,
    items: &[&str],
    selected: usize,
    input: &PongInput,
) -> (usize, Option<usize>) {
    let mut selected = selected.min(items.len() - 1);
    if input.up {
        selected = selected.saturating_sub(1);
    }
    if input.down {
        selected = (selected + 1).min(items.len() - 1);
    }
    let center = screen.center();
    painter.text(
        Pos2::new(center.x, center.y - 220.0 * s),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(64.0 * s),
        crate::ui::TEXT,
    );
    let mut chosen = input.enter.then_some(selected);
    for (i, item) in items.iter().enumerate() {
        let rect = Rect::from_center_size(
            Pos2::new(center.x, center.y - 60.0 * s + i as f32 * 100.0 * s),
            Vec2::new(720.0 * s, 80.0 * s),
        );
        let response = ui.interact(
            rect,
            ui.make_persistent_id(("pong-menu", i)),
            egui::Sense::click(),
        );
        if response.clicked() && input.clicks {
            chosen = Some(i);
        }
        let fill = if i == selected {
            crate::ui::ACCENT
        } else {
            Color32::from_rgb(0x22, 0x26, 0x34)
        };
        painter.rect_filled(rect, CornerRadius::same((16.0 * s) as u8), fill);
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            *item,
            FontId::proportional(34.0 * s),
            crate::ui::TEXT,
        );
    }
    (selected, chosen)
}

pub(crate) fn paint_hint(painter: &egui::Painter, screen: Rect, s: f32, text: &str) {
    painter.text(
        Pos2::new(screen.center().x, screen.max.y - 70.0 * s),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(26.0 * s),
        crate::ui::TEXT_DIM,
    );
}

pub(crate) fn dim(painter: &egui::Painter, screen: Rect) {
    painter.rect_filled(screen, 0.0, Color32::from_black_alpha(170));
}

/// The field, fitted into the screen (16:9) with the score above.
fn paint_game(painter: &egui::Painter, screen: Rect, s: f32, game: &Game) {
    let scale = (screen.width() / W).min((screen.height() - 140.0 * s) / H);
    let field = Rect::from_center_size(
        Pos2::new(screen.center().x, screen.center().y + 50.0 * s),
        Vec2::new(W * scale, H * scale),
    );
    let at = |x: f32, y: f32| field.min + Vec2::new(x * scale, y * scale);
    painter.rect_filled(field, 0.0, Color32::from_rgb(0x10, 0x24, 0x3a));
    // Dashed middle line.
    let dash = H / 18.0;
    for i in 0..18 {
        if i % 2 == 0 {
            let top = at(W / 2.0 - 0.04, i as f32 * dash);
            painter.rect_filled(
                Rect::from_min_size(top, Vec2::new(0.08 * scale, dash * scale)),
                0.0,
                Color32::from_white_alpha(60),
            );
        }
    }
    let paddle = |x: f32, y: f32| {
        Rect::from_center_size(at(x, y), Vec2::new(PADDLE_W * scale, PADDLE_H * scale))
    };
    let radius = CornerRadius::same((PADDLE_W * scale / 2.0) as u8);
    painter.rect_filled(
        paddle(PADDLE_X, game.left),
        radius,
        Color32::from_rgb(0x5c, 0xc8, 0xff),
    );
    painter.rect_filled(
        paddle(W - PADDLE_X, game.right),
        radius,
        Color32::from_rgb(0xff, 0xa0, 0x5c),
    );
    if game.winner.is_none() {
        painter.circle_filled(
            at(game.ball[0], game.ball[1]),
            BALL * scale / 2.0,
            crate::ui::TEXT,
        );
    }
    let score_y = field.min.y - 60.0 * s;
    for (i, x) in [
        (0, field.center().x - 120.0 * s),
        (1, field.center().x + 120.0 * s),
    ] {
        painter.text(
            Pos2::new(x, score_y),
            Align2::CENTER_CENTER,
            game.score[i].to_string(),
            FontId::monospace(64.0 * s),
            crate::ui::TEXT,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn served(game: &mut Game) {
        while game.serve_in > 0.0 {
            game.step(0.05, 0.0, 0.0);
        }
    }

    #[test]
    fn ball_bounces_off_the_walls() {
        let mut g = Game::new(7, None, 1);
        g.serve_in = 0.0;
        g.ball = [W / 2.0, 0.5];
        g.velocity = [0.0, -5.0];
        for _ in 0..10 {
            g.step(0.05, 0.0, 0.0);
        }
        assert!(g.velocity[1] > 0.0);
        assert!(g.ball[1] >= BALL / 2.0);
    }

    #[test]
    fn paddle_returns_the_ball_faster_and_steeper_off_its_edge() {
        let mut g = Game::new(7, None, 1);
        g.serve_in = 0.0;
        g.left = 4.5;
        g.ball = [3.0, 4.5];
        g.velocity = [-8.0, 0.0];
        for _ in 0..20 {
            g.step(0.02, 0.0, 0.0);
        }
        assert!(g.velocity[0] > 0.0, "returned");
        assert!((g.velocity[0].hypot(g.velocity[1]) - 8.0 * SPEED_UP).abs() < 0.01);
        assert!(g.velocity[1].abs() < 0.01, "straight off the middle");
        // Off the lower edge: steep, downwards.
        let mut g = Game::new(7, None, 1);
        g.serve_in = 0.0;
        g.left = 4.5;
        g.ball = [3.0, 5.3];
        g.velocity = [-8.0, 0.0];
        for _ in 0..20 {
            g.step(0.02, 0.0, 0.0);
        }
        assert!(g.velocity[0] > 0.0 && g.velocity[1] > 4.0);
    }

    #[test]
    fn a_missed_ball_scores_and_the_game_ends() {
        let mut g = Game::new(2, None, 7);
        for round in 0..2 {
            served(&mut g);
            g.ball = [2.0, 0.5];
            g.left = H - PADDLE_H / 2.0; // far away
            g.velocity = [-12.0, 0.0];
            for _ in 0..40 {
                g.step(0.05, 0.0, 0.0);
            }
            assert_eq!(g.score, [0, round + 1]);
            assert_eq!(g.serve_to, Side::Left);
        }
        assert_eq!(g.winner, Some(Side::Right));
        let before = g.clone();
        g.step(0.05, 1.0, 1.0);
        assert_eq!(g.left, before.left, "nothing moves after the end");
    }

    #[test]
    fn a_fast_ball_does_not_tunnel_through_a_paddle() {
        let mut g = Game::new(7, None, 1);
        g.serve_in = 0.0;
        g.left = 4.5;
        g.ball = [2.0, 4.5];
        g.velocity = [-BALL_MAX, 0.0];
        g.step(0.05, 0.0, 0.0); // 0.75 units in one frame
        g.step(0.05, 0.0, 0.0);
        assert!(g.velocity[0] > 0.0);
        assert_eq!(g.score, [0, 0]);
    }

    #[test]
    fn paddles_stay_on_the_field() {
        let mut g = Game::new(7, None, 1);
        for _ in 0..100 {
            g.step(0.05, -1.0, 1.0);
        }
        assert_eq!(g.left, PADDLE_H / 2.0);
        assert_eq!(g.right, H - PADDLE_H / 2.0);
        // Input beyond -1…1 doesn't make a paddle faster.
        let mut g = Game::new(7, None, 1);
        g.step(0.05, 50.0, 0.0);
        assert!((g.left - (H / 2.0 + PADDLE_SPEED * 0.05)).abs() < 1e-4);
    }

    #[test]
    fn arrival_includes_bounces() {
        let r = BALL / 2.0;
        assert!((arrival_y([0.0, 4.5], [1.0, 0.0], 5.0) - 4.5).abs() < 1e-4);
        // Up 2 units from y = 1: bounces off the top at r, ends at r + (2 - (1 - r)).
        let y = arrival_y([0.0, 1.0], [1.0, -1.0], 2.0);
        assert!((y - (r + 2.0 - (1.0 - r))).abs() < 1e-4, "{y}");
    }

    /// Simulated games: the hard computer returns more balls than the easy one, and a
    /// player who doesn't move loses.
    #[test]
    fn computer_strength_matters() {
        let returned = |level| {
            let mut hits = 0;
            for seed in 1..40 {
                let mut g = Game::new(1, Some(level), seed);
                g.serve_to = Side::Right;
                served(&mut g);
                // Make the ball arrive anywhere on the right side.
                let angle = (seed as f32 * 0.37).sin() * 0.9;
                g.velocity = [11.0 * angle.cos(), 11.0 * angle.sin()];
                for _ in 0..200 {
                    g.step(1.0 / 60.0, 0.0, 0.0);
                    if g.velocity[0] < 0.0 {
                        hits += 1;
                        break;
                    }
                    if g.winner.is_some() {
                        break;
                    }
                }
            }
            hits
        };
        let (easy, normal, hard) = (
            returned(PongLevel::Easy),
            returned(PongLevel::Normal),
            returned(PongLevel::Hard),
        );
        assert!(
            hard > normal && normal > easy,
            "hard {hard}, normal {normal}, easy {easy}"
        );
        assert!(hard >= 30, "hard {hard}");
        assert!(easy <= 30, "easy {easy}: beatable");
    }
}
