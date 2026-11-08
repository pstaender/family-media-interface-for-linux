//! Usage statistics (only with `statistics.enabled`): what was used when, on which home
//! screen. One JSON line per activity in `$XDG_DATA_HOME/curator/statistics/<YYYY-MM-DD>.jsonl`
//! (the day it started), written when the activity ends. Files older than `keep_days` are
//! deleted when the media center starts and when it is unlocked (`prune`). Stays on this
//! computer; the admin panel reads the files (Parental control → Usage).

use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};

/// Shorter activities aren't written (opening something by mistake).
const MIN_SECS: i64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Disc,
    Video,
    Music,
    Pictures,
    Website,
    Game,
    Program,
    Emulator,
}

/// What is being used: e.g. Music / "Stories" (folder) / "Janosch/01 Tiger.mp3".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub category: Category,
    pub item: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

impl Activity {
    pub fn new(category: Category, item: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            category,
            item: item.into(),
            detail: detail.into(),
        }
    }
}

/// One line of a statistics file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Unix times.
    pub from: i64,
    pub until: i64,
    pub home_screen: String,
    #[serde(flatten)]
    pub activity: Activity,
}

fn dir() -> Option<PathBuf> {
    crate::store::statistics_dir()
}

fn file_for(dir: &Path, day: NaiveDate) -> PathBuf {
    dir.join(format!("{}.jsonl", day.format("%Y-%m-%d")))
}

fn local_day(unix: i64) -> NaiveDate {
    DateTime::from_timestamp(unix, 0)
        .map(|t| t.with_timezone(&Local).date_naive())
        .unwrap_or_else(|| Local::now().date_naive())
}

/// Follows what is used (`update` every frame) and writes each activity when it ends.
#[derive(Debug, Default)]
pub struct Recorder {
    current: Option<(Activity, String, i64)>,
    /// Where to write (`None`: the XDG data dir); tests use their own.
    dir: Option<PathBuf>,
}

impl Recorder {
    #[cfg(test)]
    fn in_dir(dir: PathBuf) -> Self {
        Self {
            current: None,
            dir: Some(dir),
        }
    }

    /// `activity`: what is used right now (`None`: nothing, e.g. the home screen or the lock
    /// screen). With `enabled` false nothing is recorded (a running activity still ends).
    pub fn update(&mut self, now: i64, activity: Option<Activity>, screen: &str, enabled: bool) {
        let activity = activity.filter(|_| enabled);
        let same = match (&self.current, &activity) {
            (Some((a, s, _)), Some(b)) => a == b && s == screen,
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.finish(now);
        self.current = activity.map(|a| (a, screen.to_string(), now));
    }

    /// End the current activity (also when the media center exits).
    pub fn finish(&mut self, now: i64) {
        let Some((activity, home_screen, from)) = self.current.take() else {
            return;
        };
        if now - from < MIN_SECS {
            return;
        }
        let entry = Entry {
            from,
            until: now,
            home_screen,
            activity,
        };
        let Some(dir) = self.dir.clone().or_else(dir) else {
            return;
        };
        if let Err(e) = append(&dir, &entry) {
            log::warn!("statistics: could not write: {e}");
        }
    }
}

fn append(dir: &Path, entry: &Entry) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_for(dir, local_day(entry.from)))?;
    let line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
    writeln!(file, "{line}")
}

/// Delete statistics files older than `keep_days` (only our own `YYYY-MM-DD.jsonl` files).
pub fn prune(keep_days: u32) {
    if let Some(dir) = dir() {
        prune_in(&dir, keep_days, Local::now().date_naive());
    }
}

fn prune_in(dir: &Path, keep_days: u32, today: NaiveDate) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(day) = name
            .to_str()
            .and_then(|n| n.strip_suffix(".jsonl"))
            .and_then(crate::config::parse_date)
        else {
            continue;
        };
        if (today - day).num_days() >= i64::from(keep_days) {
            if let Err(e) = std::fs::remove_file(entry.path()) {
                log::warn!(
                    "statistics: could not delete {}: {e}",
                    entry.path().display()
                );
            } else {
                log::info!("statistics: deleted {} (older than {keep_days} days)", day);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("curator-stats-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn lines(dir: &Path) -> Vec<Entry> {
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
        files.sort_by_key(|e| e.file_name());
        files
            .iter()
            .flat_map(|f| {
                std::fs::read_to_string(f.path())
                    .unwrap()
                    .lines()
                    .map(|l| serde_json::from_str(l).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn records_each_activity_when_it_ends() {
        let dir = temp_dir("record");
        let mut r = Recorder::in_dir(dir.clone());
        let music = Activity::new(Category::Music, "Stories", "Tiger/01.mp3");
        let t0 = 1_800_000_000;
        r.update(t0, Some(music.clone()), "kids", true);
        r.update(t0 + 60, Some(music.clone()), "kids", true); // still the same
        let next = Activity::new(Category::Music, "Stories", "Tiger/02.mp3");
        r.update(t0 + 120, Some(next.clone()), "kids", true);
        r.update(t0 + 122, None, "kids", true); // too short: not written
        let game = Activity::new(Category::Game, "Ping Pong", "");
        r.update(t0 + 200, Some(game.clone()), "kids", true);
        r.finish(t0 + 500);
        let entries = lines(&dir);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].activity, music);
        assert_eq!((entries[0].from, entries[0].until), (t0, t0 + 120));
        assert_eq!(entries[0].home_screen, "kids");
        assert_eq!(entries[1].activity, game);
        assert_eq!(entries[1].until - entries[1].from, 300);
        // Switched off: nothing new is recorded.
        r.update(t0 + 600, Some(game), "kids", false);
        r.finish(t0 + 900);
        assert_eq!(lines(&dir).len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prunes_only_old_statistics_files() {
        let dir = temp_dir("prune");
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "2026-01-01.jsonl",
            "2026-11-01.jsonl",
            "2026-11-08.jsonl",
            "notes.txt",
            "2026-01-01.json",
        ] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        prune_in(&dir, 7, NaiveDate::from_ymd_opt(2026, 11, 8).unwrap());
        let mut left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, ["2026-01-01.json", "2026-11-08.jsonl", "notes.txt"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn entry_format() {
        let e = Entry {
            from: 1,
            until: 2,
            home_screen: "kids".into(),
            activity: Activity::new(Category::Website, "KiKA", "kika.de"),
        };
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"from":1,"until":2,"home_screen":"kids","category":"website","item":"KiKA","detail":"kika.de"}"#
        );
    }
}
