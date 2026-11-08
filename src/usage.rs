//! Usage time, unlocks and the parent's lock (each per home screen), the active home screen,
//! and wrong PIN tries.
//!
//! Persisted in `$XDG_DATA_HOME/curator/usage.json` (our own folder), so restarting the
//! media center neither resets the used time nor the PIN waiting time.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

use crate::limits::{Situation, Status};
use crate::store;

/// Usage of the last days is kept for statistics; older days are dropped.
const KEEP_DAYS: i64 = 60;
const SAVE_INTERVAL: Duration = Duration::from_secs(30);
/// Wrong PIN: first wait …
const PIN_WAIT_SECS: i64 = 5;
/// … and the longest wait when doubling.
const PIN_WAIT_MAX_SECS: i64 = 3600;

/// Usage of one home screen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenUsage {
    /// Seconds of use per day (`YYYY-MM-DD`).
    pub days: BTreeMap<String, u64>,
    /// Unix time until which an unlock lifts the limits.
    pub unlocked_until: Option<i64>,
    pub locked_by_parent: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stored {
    /// Per home screen id.
    pub screens: BTreeMap<String, ScreenUsage>,
    /// The home screen shown (kept over restarts, so a restart can't switch to a less
    /// restricted one). `None` / unknown: the default one.
    pub active: Option<String>,
    /// Wrong PIN tries in a row (for all home screens: one PIN).
    pub pin_failures: u32,
    /// Unix time before which no PIN may be entered.
    pub pin_blocked_until: Option<i64>,
    /// Files from before home screens: one usage for everything; moved to the version-1
    /// home screen (`config::V1_SCREEN_ID`) when loaded, never written.
    #[serde(skip_serializing)]
    days: BTreeMap<String, u64>,
    #[serde(skip_serializing)]
    unlocked_until: Option<i64>,
    #[serde(skip_serializing)]
    locked_by_parent: bool,
}

impl Stored {
    /// Move the usage of a file from before home screens into the version-1 home screen.
    fn take_over_old_format(&mut self) {
        if self.days.is_empty() && self.unlocked_until.is_none() && !self.locked_by_parent {
            return;
        }
        let screen = self
            .screens
            .entry(crate::config::V1_SCREEN_ID.to_string())
            .or_default();
        for (day, secs) in std::mem::take(&mut self.days) {
            *screen.days.entry(day).or_default() += secs;
        }
        screen.unlocked_until = screen.unlocked_until.or(self.unlocked_until.take());
        screen.locked_by_parent |= std::mem::take(&mut self.locked_by_parent);
    }
}

pub struct Tracker {
    pub stored: Stored,
    path: Option<PathBuf>,
    dirty: bool,
    last_save: Instant,
    /// Latest evaluation (by the UI), reported to the admin panel.
    pub status: Status,
}

pub type SharedTracker = Arc<Mutex<Tracker>>;

/// Waiting time after the `failures`-th wrong PIN in a row.
pub fn pin_wait_secs(failures: u32, doubling: bool) -> i64 {
    if !doubling || failures <= 1 {
        return PIN_WAIT_SECS;
    }
    let factor = 1i64.checked_shl(failures - 1).unwrap_or(i64::MAX);
    PIN_WAIT_SECS.saturating_mul(factor).min(PIN_WAIT_MAX_SECS)
}

fn local(unix: i64) -> Option<NaiveDateTime> {
    DateTime::from_timestamp(unix, 0).map(|t| t.with_timezone(&Local).naive_local())
}

/// Start of the next day (local time) as unix time.
fn next_midnight(now: DateTime<Local>) -> i64 {
    let tomorrow = now.date_naive().succ_opt().unwrap_or(now.date_naive());
    tomorrow
        .and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(Local).earliest())
        .map_or(now.timestamp() + 24 * 3600, |t| t.timestamp())
}

impl Tracker {
    pub fn load() -> Self {
        let path = store::usage_path();
        let mut stored: Stored = path.as_deref().and_then(store::load).unwrap_or_default();
        stored.take_over_old_format();
        Self::with(stored, path)
    }

    /// Not saved anywhere (tests).
    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::with(Stored::default(), None)
    }

    fn with(stored: Stored, path: Option<PathBuf>) -> Self {
        Self {
            stored,
            path,
            dirty: false,
            last_save: Instant::now(),
            status: Status::Allowed {
                remaining_secs: None,
            },
        }
    }

    pub fn shared(self) -> SharedTracker {
        Arc::new(Mutex::new(self))
    }

    fn screen(&self, id: &str) -> Option<&ScreenUsage> {
        self.stored.screens.get(id)
    }

    fn screen_mut(&mut self, id: &str) -> &mut ScreenUsage {
        self.stored.screens.entry(id.to_string()).or_default()
    }

    /// The home screen shown (as stored; check it against the config).
    pub fn active(&self) -> Option<&str> {
        self.stored.active.as_deref()
    }

    pub fn set_active(&mut self, id: &str) {
        self.stored.active = Some(id.to_string());
        self.save_now();
    }

    pub fn add_usage(&mut self, screen: &str, date: NaiveDate, secs: u64) {
        if secs == 0 {
            return;
        }
        *self
            .screen_mut(screen)
            .days
            .entry(date.format("%Y-%m-%d").to_string())
            .or_default() += secs;
        self.dirty = true;
    }

    pub fn used_on(&self, screen: &str, date: NaiveDate) -> u64 {
        self.screen(screen)
            .and_then(|s| s.days.get(&date.format("%Y-%m-%d").to_string()))
            .copied()
            .unwrap_or(0)
    }

    /// Seconds of use from this week's Monday up to and including `date`.
    pub fn used_this_week(&self, screen: &str, date: NaiveDate) -> u64 {
        let monday = crate::limits::monday_of(date);
        monday
            .iter_days()
            .take_while(|d| *d <= date)
            .map(|d| self.used_on(screen, d))
            .sum()
    }

    /// Input for `limits::evaluate`.
    pub fn situation(&self, screen: &str) -> Situation {
        let usage = self.screen(screen).cloned().unwrap_or_default();
        Situation {
            usage: usage
                .days
                .iter()
                .filter_map(|(d, s)| Some((crate::config::parse_date(d)?, *s)))
                .collect(),
            locked_by_parent: usage.locked_by_parent,
            unlocked_until: usage.unlocked_until.and_then(local),
        }
    }

    pub fn unlocked_until(&self, screen: &str) -> Option<i64> {
        self.screen(screen).and_then(|s| s.unlocked_until)
    }

    #[cfg(test)]
    pub fn locked_by_parent(&self, screen: &str) -> bool {
        self.screen(screen).is_some_and(|s| s.locked_by_parent)
    }

    /// Unlock for `minutes`, or for the rest of today (`None`). Also lifts a parent's lock.
    pub fn unlock(&mut self, screen: &str, now: DateTime<Local>, minutes: Option<u32>) {
        let until = match minutes {
            Some(m) => now.timestamp() + i64::from(m) * 60,
            None => next_midnight(now),
        };
        let s = self.screen_mut(screen);
        s.unlocked_until = Some(until);
        s.locked_by_parent = false;
        self.save_now();
    }

    /// End an unlock early: the normal limits apply again (doesn't lock by itself).
    pub fn end_unlock(&mut self, screen: &str) {
        self.screen_mut(screen).unlocked_until = None;
        self.save_now();
    }

    /// Lock now (admin panel). Ends any unlock.
    pub fn lock(&mut self, screen: &str) {
        let s = self.screen_mut(screen);
        s.locked_by_parent = true;
        s.unlocked_until = None;
        self.save_now();
    }

    /// Seconds to wait before the next PIN try (0: may try now).
    pub fn pin_wait(&self, now: i64) -> i64 {
        self.stored
            .pin_blocked_until
            .map_or(0, |until| (until - now).max(0))
    }

    /// Record a wrong PIN; returns the waiting time.
    pub fn pin_wrong(&mut self, now: i64, doubling: bool) -> i64 {
        self.stored.pin_failures = self.stored.pin_failures.saturating_add(1);
        let wait = pin_wait_secs(self.stored.pin_failures, doubling);
        self.stored.pin_blocked_until = Some(now + wait);
        self.save_now();
        wait
    }

    pub fn pin_ok(&mut self) {
        self.stored.pin_failures = 0;
        self.stored.pin_blocked_until = None;
        self.save_now();
    }

    /// Save usage regularly (not on every frame).
    pub fn save_if_due(&mut self) {
        if self.dirty && self.last_save.elapsed() >= SAVE_INTERVAL {
            self.save_now();
        }
    }

    pub fn save_now(&mut self) {
        let today = Local::now().date_naive();
        for screen in self.stored.screens.values_mut() {
            screen.days.retain(|d, _| {
                crate::config::parse_date(d).is_some_and(|d| (today - d).num_days() <= KEEP_DAYS)
            });
        }
        if let Some(path) = &self.path
            && let Err(e) = store::save(path, &self.stored)
        {
            log::warn!("could not save usage: {e}");
        }
        self.dirty = false;
        self.last_save = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_waits_double_and_are_capped() {
        let waits: Vec<i64> = (1..=6).map(|n| pin_wait_secs(n, true)).collect();
        assert_eq!(waits, [5, 10, 20, 40, 80, 160]);
        assert_eq!(pin_wait_secs(20, true), 3600);
        assert_eq!(pin_wait_secs(200, true), 3600); // no overflow
        assert_eq!(pin_wait_secs(7, false), 5);
    }

    #[test]
    fn wrong_pins_block_and_a_correct_pin_resets() {
        let mut t = Tracker::with(Stored::default(), None);
        assert_eq!(t.pin_wait(1000), 0);
        assert_eq!(t.pin_wrong(1000, true), 5);
        assert_eq!(t.pin_wait(1002), 3);
        assert_eq!(t.pin_wrong(1005, true), 10);
        assert_eq!(t.pin_wait(1005), 10);
        t.pin_ok();
        assert_eq!(t.pin_wait(1005), 0);
        assert_eq!(t.pin_wrong(1005, true), 5);
    }

    #[test]
    fn usage_unlock_and_lock_per_home_screen() {
        let mut t = Tracker::with(Stored::default(), None);
        let day = crate::config::parse_date("2026-11-07").unwrap();
        t.add_usage("kids", day, 30);
        t.add_usage("kids", day, 45);
        t.add_usage("teen", day, 1000);
        assert_eq!(t.used_on("kids", day), 75);
        assert_eq!(t.used_on("teen", day), 1000);
        assert_eq!(t.used_on("other", day), 0);
        // 2026-11-07 is a Saturday; Monday 2026-11-02 counts, Sunday 2026-11-01 doesn't.
        t.add_usage(
            "kids",
            crate::config::parse_date("2026-11-02").unwrap(),
            100,
        );
        t.add_usage(
            "kids",
            crate::config::parse_date("2026-11-01").unwrap(),
            1000,
        );
        assert_eq!(t.used_this_week("kids", day), 175);
        assert_eq!(t.situation("kids").usage.get(&day), Some(&75));

        let now = Local::now();
        t.lock("kids");
        assert!(t.situation("kids").locked_by_parent);
        assert!(
            !t.situation("teen").locked_by_parent,
            "only that home screen"
        );
        t.unlock("kids", now, Some(30));
        let s = t.situation("kids");
        assert!(!s.locked_by_parent);
        let minutes = (s.unlocked_until.unwrap() - now.naive_local()).num_minutes();
        assert!((29..=30).contains(&minutes), "{minutes}");
        assert_eq!(t.situation("teen").unlocked_until, None);
        // Rest of today: until midnight.
        t.unlock("kids", now, None);
        let until = t.situation("kids").unlocked_until.unwrap();
        assert_eq!(until.date(), now.date_naive().succ_opt().unwrap());
        assert_eq!(until.time(), chrono::NaiveTime::MIN);
        t.set_active("teen");
        assert_eq!(t.active(), Some("teen"));
    }

    #[test]
    fn old_usage_files_move_to_the_first_home_screen() {
        let mut stored: Stored = serde_json::from_str(
            r#"{"days":{"2026-11-07":120},"unlocked_until":1800000000,"locked_by_parent":true,"pin_failures":2}"#,
        )
        .unwrap();
        stored.take_over_old_format();
        let home = &stored.screens[crate::config::V1_SCREEN_ID];
        assert_eq!(home.days["2026-11-07"], 120);
        assert_eq!(home.unlocked_until, Some(1_800_000_000));
        assert!(home.locked_by_parent);
        assert_eq!(stored.pin_failures, 2);
        // Written in the new format only.
        let value = serde_json::to_value(&stored).unwrap();
        assert!(value.get("days").is_none() && value.get("locked_by_parent").is_none());
        assert_eq!(value["screens"]["home"]["days"]["2026-11-07"], 120);
    }

    #[test]
    fn stored_format_roundtrip() {
        let mut stored = Stored::default();
        stored.screens.insert(
            "kids".into(),
            ScreenUsage {
                days: [("2026-11-07".to_string(), 120)].into(),
                unlocked_until: Some(1_800_000_000),
                locked_by_parent: true,
            },
        );
        stored.active = Some("kids".into());
        stored.pin_failures = 2;
        stored.pin_blocked_until = Some(1_800_000_010);
        let json = serde_json::to_string(&stored).unwrap();
        assert_eq!(serde_json::from_str::<Stored>(&json).unwrap(), stored);
        // Missing fields: defaults (older or hand-edited files).
        assert_eq!(
            serde_json::from_str::<Stored>("{}").unwrap(),
            Stored::default()
        );
    }
}
