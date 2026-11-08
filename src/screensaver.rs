//! The screensaver of the media center's own screens (not over a running player or website).
//!
//! - **Title:** the title moves slowly across the screen (bouncing off the edges, so no pixel
//!   shows the same thing for long) over a very dark background whose color changes a little
//!   every minute (black to dark gray, with a faint tint). Painted by egui.
//! - **Aerial:** the aerial videos the admin interface downloaded
//!   (`$XDG_CACHE_HOME/familynovice/aerial/<id>.<quality>.mov`), played by mpv without sound in
//!   random order (`player::screensaver_command`). Without any downloaded video: title.
//!
//! Any input ends it, and that input does nothing else. The pure parts (timing, motion,
//! colors, choosing the files) are here and tested; `ui.rs` draws and starts mpv.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{AerialQuality, ScreensaverMode, ScreensaverSettings};

/// Should the screensaver start now?
pub fn due(settings: &ScreensaverSettings, idle: Duration) -> bool {
    settings.mode != ScreensaverMode::Off
        && idle >= Duration::from_secs(u64::from(settings.idle_minutes) * 60)
}

/// Speed of the title in screen heights per second (slow: about 30 px/s at 1080p).
const SPEED: f32 = 0.028;

/// Top-left corner of the title (size `text`) at `t` seconds, inside `screen` (both in the
/// same units). Moves diagonally and bounces off the edges; x and y have different periods,
/// so the path doesn't repeat quickly.
pub fn title_position(t: f32, screen: [f32; 2], text: [f32; 2]) -> [f32; 2] {
    let speed = SPEED * screen[1];
    let axis = |i: usize, factor: f32| {
        let room = (screen[i] - text[i]).max(0.0);
        if room <= 0.0 {
            return 0.0;
        }
        // Triangle wave between 0 and room.
        let d = (t * speed * factor) % (2.0 * room);
        if d <= room { d } else { 2.0 * room - d }
    };
    [axis(0, 1.0), axis(1, 0.73)]
}

/// Background color for minute `minute` (RGB): black to dark gray with a faint tint, never
/// brighter than 0x30 per channel.
pub fn minute_color(minute: u64) -> [u8; 3] {
    // Small deterministic hash (splitmix64), so every minute has its own color.
    let mut x = minute.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    let gray = (x % 33) as i32; // 0..=32
    let tint = |shift: u32| ((x >> shift) % 13) as i32 - 6; // -6..=6
    let channel = |shift| (gray + tint(shift)).clamp(0, 0x30) as u8;
    [channel(8), channel(16), channel(24)]
}

/// The background color at `t` seconds: each minute's color, blended over 8 s into the next.
pub fn background(t: f64) -> [u8; 3] {
    const BLEND: f64 = 8.0;
    let minute = (t / 60.0).floor().max(0.0);
    let into = t - minute * 60.0;
    let current = minute_color(minute as u64);
    if into >= BLEND || minute == 0.0 {
        return current;
    }
    let before = minute_color(minute as u64 - 1);
    let f = into / BLEND;
    std::array::from_fn(|i| {
        (f64::from(before[i]) + (f64::from(current[i]) - f64::from(before[i])) * f).round() as u8
    })
}

/// The downloaded aerial videos to play: for each chosen id the file in the chosen quality,
/// else in another quality (left over while the new one downloads). Only complete files
/// (the admin interface renames `.part` files when done).
pub fn aerial_files(dir: &Path, settings: &ScreensaverSettings) -> Vec<PathBuf> {
    settings
        .aerial_videos
        .iter()
        .filter(|id| crate::config::is_aerial_id(id))
        .filter_map(|id| {
            std::iter::once(settings.aerial_quality)
                .chain(AerialQuality::ALL)
                .map(|q| dir.join(format!("{id}.{}.mov", q.name())))
                .find(|p| p.metadata().is_ok_and(|m| m.is_file() && m.len() > 0))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(mode: ScreensaverMode, minutes: u32) -> ScreensaverSettings {
        ScreensaverSettings {
            mode,
            idle_minutes: minutes,
            ..ScreensaverSettings::default()
        }
    }

    #[test]
    fn starts_after_the_idle_time() {
        let s = settings(ScreensaverMode::Title, 10);
        assert!(!due(&s, Duration::from_secs(599)));
        assert!(due(&s, Duration::from_secs(600)));
        assert!(!due(
            &settings(ScreensaverMode::Off, 1),
            Duration::from_secs(99_999)
        ));
        assert!(due(
            &settings(ScreensaverMode::Aerial, 1),
            Duration::from_secs(60)
        ));
    }

    #[test]
    fn title_stays_on_screen_and_moves() {
        let (screen, text) = ([1920.0, 1080.0], [500.0, 90.0]);
        let mut last = title_position(0.0, screen, text);
        assert_eq!(last, [0.0, 0.0]);
        let mut moved = 0.0;
        for i in 1..20_000 {
            let p = title_position(i as f32 * 0.5, screen, text);
            assert!(
                (0.0..=1420.0).contains(&p[0]) && (0.0..=990.0).contains(&p[1]),
                "{p:?}"
            );
            // Smooth: at most ~SPEED * 1080 px/s * 0.5 s per step.
            assert!((p[0] - last[0]).abs() <= 16.0 && (p[1] - last[1]).abs() <= 16.0);
            moved += (p[0] - last[0]).abs();
            last = p;
        }
        assert!(moved > 10_000.0);
        // Title wider than the screen: pinned to the edge instead of jumping around.
        assert_eq!(title_position(50.0, [400.0, 300.0], [500.0, 90.0])[0], 0.0);
    }

    #[test]
    fn colors_stay_dark_and_change_every_minute() {
        let colors: Vec<_> = (0..500).map(minute_color).collect();
        assert!(colors.iter().flatten().all(|c| *c <= 0x30));
        let changed = colors.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(changed > 450, "{changed}");
        // Smooth blend at the start of a minute, then the minute's color.
        assert_eq!(background(60.0), minute_color(0));
        assert_eq!(background(68.0), minute_color(1));
        assert_eq!(background(90.0), minute_color(1));
        let mid = background(64.0);
        for ((m, a), b) in mid.iter().zip(minute_color(0)).zip(minute_color(1)) {
            assert!((a.min(b)..=a.max(b)).contains(m));
        }
    }

    #[test]
    fn picks_downloaded_aerial_files() {
        let dir = std::env::temp_dir().join(format!("familynovice-aerial-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "A.1080p_hevc.mov",
            "B.1080p_h264.mov", // old quality, new one still downloading
            "B.1080p_hevc.mov.part",
            "D.1080p_hevc.mov", // not chosen
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        std::fs::write(dir.join("E.1080p_hevc.mov"), b"").unwrap(); // empty: broken
        let s = ScreensaverSettings {
            mode: ScreensaverMode::Aerial,
            aerial_quality: AerialQuality::HdHevc,
            aerial_videos: ["A", "B", "C", "E", "../D"].map(String::from).to_vec(),
            ..ScreensaverSettings::default()
        };
        assert_eq!(
            aerial_files(&dir, &s),
            [dir.join("A.1080p_hevc.mov"), dir.join("B.1080p_h264.mov")]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
