//! Time limits: is the media center allowed right now, and for how long?
//!
//! Pure functions (no clock, no files) so every rule is testable with fixed dates.
//!
//! - **Budgets:** for a day, the most specific matching budget wins
//!   (today > weekdays > every_day); its minutes are compared with the usage of that day.
//!   A `this_week` budget is compared with the usage of the whole week (Monday to today) and
//!   applies in addition.
//! - **Windows:** the most specific matching window wins (today > weekdays > this_week >
//!   every_day). Outside of it, the media center is locked.
//! - A parent's lock (admin panel) always locks; an unlock (PIN or admin panel) lifts all
//!   limits until it expires.

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};

use crate::config::{LimitRule, Scope, WEEKDAYS, parse_date, parse_time};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockReason {
    /// Locked from the admin panel.
    Parent,
    /// Today's (or this week's) time is used up.
    BudgetUsed,
    /// Outside the allowed time; `opens_at` = start of today's window, if still ahead.
    OutsideWindow { opens_at: Option<NaiveTime> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Allowed; `remaining_secs` until a limit is reached (`None`: no limit today).
    Allowed {
        remaining_secs: Option<i64>,
    },
    Locked(LockReason),
}

/// What the limits need to know besides the rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Situation {
    /// Seconds of use per day.
    pub usage: BTreeMap<NaiveDate, u64>,
    pub locked_by_parent: bool,
    /// Unlocked (PIN / admin) until this moment: limits don't apply before it.
    pub unlocked_until: Option<NaiveDateTime>,
}

/// How specifically a scope matches a date (0 = not at all).
fn specificity(scope: &Scope, date: NaiveDate) -> u8 {
    match scope {
        Scope::Today { date: d } => u8::from(parse_date(d) == Some(date)) * 4,
        Scope::Weekdays { days } => {
            let day = WEEKDAYS[date.weekday().num_days_from_monday() as usize];
            u8::from(days.iter().any(|d| d == day)) * 3
        }
        Scope::ThisWeek { week_of } => u8::from(parse_date(week_of) == Some(monday_of(date))) * 2,
        Scope::EveryDay {} => 1,
    }
}

pub fn monday_of(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

/// Pick the most specific matching rule (ties: the stricter one, i.e. the first by `stricter`).
fn most_specific<'a, T>(
    candidates: impl Iterator<Item = (u8, &'a T)>,
    stricter: impl Fn(&T, &T) -> std::cmp::Ordering,
) -> Option<&'a T> {
    candidates
        .filter(|(spec, _)| *spec > 0)
        .max_by(|(sa, a), (sb, b)| sa.cmp(sb).then_with(|| stricter(b, a)))
        .map(|(_, rule)| rule)
}

pub fn evaluate(rules: &[LimitRule], situation: &Situation, now: NaiveDateTime) -> Status {
    if situation.locked_by_parent {
        return Status::Locked(LockReason::Parent);
    }
    if let Some(until) = situation.unlocked_until
        && until > now
    {
        return Status::Allowed {
            remaining_secs: Some((until - now).num_seconds()),
        };
    }
    let today = now.date();
    let mut remaining: Option<i64> = None;
    let mut limit_to = |secs: i64| remaining = Some(remaining.map_or(secs, |r| r.min(secs)));

    // Window
    let windows = rules.iter().filter_map(|r| match r {
        LimitRule::Window { from, until, scope } => {
            Some((specificity(scope, today), (from, until)))
        }
        _ => None,
    });
    let windows: Vec<(u8, (&Option<String>, &String))> = windows.collect();
    let window = most_specific(windows.iter().map(|(s, w)| (*s, w)), |a, b| {
        // Stricter = shorter (later start, earlier end).
        let len = |w: &(&Option<String>, &String)| {
            let from =
                w.0.as_deref()
                    .and_then(parse_time)
                    .unwrap_or(NaiveTime::MIN);
            parse_time(w.1).map(|u| u - from).unwrap_or_default()
        };
        len(a).cmp(&len(b))
    });
    if let Some((from, until)) = window {
        let from = from.as_deref().and_then(parse_time);
        let until = parse_time(until).unwrap_or(NaiveTime::MIN);
        let time = now.time();
        if from.is_some_and(|f| time < f) {
            return Status::Locked(LockReason::OutsideWindow { opens_at: from });
        }
        if time >= until {
            return Status::Locked(LockReason::OutsideWindow { opens_at: None });
        }
        limit_to((until - time).num_seconds());
    }

    // Daily budget
    let daily = rules.iter().filter_map(|r| match r {
        LimitRule::Budget { minutes, scope } if !matches!(scope, Scope::ThisWeek { .. }) => {
            Some((specificity(scope, today), minutes))
        }
        _ => None,
    });
    if let Some(minutes) = most_specific(daily, |a, b| a.cmp(b)) {
        let used = situation.usage.get(&today).copied().unwrap_or(0) as i64;
        limit_to(i64::from(*minutes) * 60 - used);
    }

    // Weekly budget
    let weekly = rules.iter().filter_map(|r| match r {
        LimitRule::Budget {
            minutes,
            scope: scope @ Scope::ThisWeek { .. },
        } => Some((specificity(scope, today), minutes)),
        _ => None,
    });
    if let Some(minutes) = most_specific(weekly, |a, b| a.cmp(b)) {
        let monday = monday_of(today);
        let used: u64 = situation.usage.range(monday..=today).map(|(_, s)| s).sum();
        limit_to(i64::from(*minutes) * 60 - used as i64);
    }

    match remaining {
        Some(r) if r <= 0 => Status::Locked(LockReason::BudgetUsed),
        remaining_secs => Status::Allowed { remaining_secs },
    }
}

/// Short text for the lock screen and the admin panel.
pub fn reason_text(reason: &LockReason) -> String {
    match reason {
        LockReason::Parent => "Locked by a parent".into(),
        LockReason::BudgetUsed => "The time for today is used up".into(),
        LockReason::OutsideWindow {
            opens_at: Some(time),
        } => format!("Available from {}", time.format("%H:%M")),
        LockReason::OutsideWindow { opens_at: None } => "Not available any more today".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(date: &str, time: &str) -> NaiveDateTime {
        NaiveDateTime::new(parse_date(date).unwrap(), parse_time(time).unwrap())
    }
    fn budget(minutes: u32, scope: Scope) -> LimitRule {
        LimitRule::Budget { minutes, scope }
    }
    fn window(from: Option<&str>, until: &str, scope: Scope) -> LimitRule {
        LimitRule::Window {
            from: from.map(String::from),
            until: until.into(),
            scope,
        }
    }
    fn days(d: &[&str]) -> Scope {
        Scope::Weekdays {
            days: d.iter().map(|s| s.to_string()).collect(),
        }
    }
    fn used(entries: &[(&str, u64)]) -> Situation {
        Situation {
            usage: entries
                .iter()
                .map(|(d, s)| (parse_date(d).unwrap(), *s))
                .collect(),
            ..Default::default()
        }
    }
    // 2026-11-07 is a Saturday, 2026-11-02 the Monday of that week.
    const SAT: &str = "2026-11-07";

    #[test]
    fn no_rules_no_limit() {
        assert_eq!(
            evaluate(&[], &Situation::default(), at(SAT, "12:00")),
            Status::Allowed {
                remaining_secs: None
            }
        );
    }

    #[test]
    fn daily_budget() {
        let rules = [budget(60, Scope::EveryDay {})];
        assert_eq!(
            evaluate(&rules, &used(&[(SAT, 45 * 60)]), at(SAT, "12:00")),
            Status::Allowed {
                remaining_secs: Some(15 * 60)
            }
        );
        assert_eq!(
            evaluate(&rules, &used(&[(SAT, 60 * 60)]), at(SAT, "12:00")),
            Status::Locked(LockReason::BudgetUsed)
        );
        // Yesterday's usage doesn't count today.
        assert!(matches!(
            evaluate(&rules, &used(&[("2026-11-06", 3600)]), at(SAT, "12:00")),
            Status::Allowed { .. }
        ));
    }

    #[test]
    fn most_specific_budget_wins() {
        let rules = [
            budget(60, Scope::EveryDay {}),
            budget(120, days(&["sat", "sun"])),
            budget(
                30,
                Scope::Today {
                    date: "2026-11-08".into(),
                },
            ),
        ];
        // Saturday: weekend budget (120) beats every day (60).
        assert_eq!(
            evaluate(&rules, &used(&[(SAT, 90 * 60)]), at(SAT, "12:00")),
            Status::Allowed {
                remaining_secs: Some(30 * 60)
            }
        );
        // Sunday 2026-11-08: "today" (30) beats the weekend budget.
        assert_eq!(
            evaluate(
                &rules,
                &used(&[("2026-11-08", 31 * 60)]),
                at("2026-11-08", "12:00")
            ),
            Status::Locked(LockReason::BudgetUsed)
        );
        // Friday: every day (60).
        assert_eq!(
            evaluate(
                &rules,
                &used(&[("2026-11-06", 50 * 60)]),
                at("2026-11-06", "12:00")
            ),
            Status::Allowed {
                remaining_secs: Some(10 * 60)
            }
        );
    }

    #[test]
    fn weekly_budget_counts_the_whole_week_and_expires() {
        let rules = [budget(
            300,
            Scope::ThisWeek {
                week_of: "2026-11-02".into(),
            },
        )];
        let usage = used(&[
            ("2026-11-01", 1000 * 60), // Sunday before: other week
            ("2026-11-02", 100 * 60),
            ("2026-11-05", 150 * 60),
        ]);
        assert_eq!(
            evaluate(&rules, &usage, at(SAT, "12:00")),
            Status::Allowed {
                remaining_secs: Some(50 * 60)
            }
        );
        // Next week the rule doesn't apply any more.
        assert_eq!(
            evaluate(&rules, &usage, at("2026-11-09", "12:00")),
            Status::Allowed {
                remaining_secs: None
            }
        );
    }

    #[test]
    fn weekly_and_daily_budget_both_apply() {
        let rules = [
            budget(60, Scope::EveryDay {}),
            budget(
                100,
                Scope::ThisWeek {
                    week_of: "2026-11-02".into(),
                },
            ),
        ];
        let usage = used(&[("2026-11-03", 90 * 60), (SAT, 5 * 60)]);
        // Daily: 55 min left; weekly: 5 min left → 5 min.
        assert_eq!(
            evaluate(&rules, &usage, at(SAT, "12:00")),
            Status::Allowed {
                remaining_secs: Some(5 * 60)
            }
        );
    }

    #[test]
    fn windows() {
        let rules = [window(Some("14:00"), "19:00", Scope::EveryDay {})];
        assert_eq!(
            evaluate(&rules, &Situation::default(), at(SAT, "13:59")),
            Status::Locked(LockReason::OutsideWindow {
                opens_at: parse_time("14:00")
            })
        );
        assert_eq!(
            evaluate(&rules, &Situation::default(), at(SAT, "18:30")),
            Status::Allowed {
                remaining_secs: Some(30 * 60)
            }
        );
        assert_eq!(
            evaluate(&rules, &Situation::default(), at(SAT, "19:00")),
            Status::Locked(LockReason::OutsideWindow { opens_at: None })
        );
        // "from" is optional: from midnight.
        let until_only = [window(None, "08:00", Scope::EveryDay {})];
        assert!(matches!(
            evaluate(&until_only, &Situation::default(), at(SAT, "00:01")),
            Status::Allowed { .. }
        ));
    }

    #[test]
    fn specific_window_beats_every_day_and_budget_combines() {
        let rules = [
            window(Some("14:00"), "18:00", Scope::EveryDay {}),
            window(Some("09:00"), "20:00", days(&["sat"])),
            budget(60, Scope::EveryDay {}),
        ];
        // Saturday 10:00: weekend window allows; budget 60 min left, window 10 h → 60 min.
        assert_eq!(
            evaluate(&rules, &Situation::default(), at(SAT, "10:00")),
            Status::Allowed {
                remaining_secs: Some(60 * 60)
            }
        );
        // Saturday 19:30: window has 30 min left, budget has 60 → 30 min.
        assert_eq!(
            evaluate(&rules, &Situation::default(), at(SAT, "19:30")),
            Status::Allowed {
                remaining_secs: Some(30 * 60)
            }
        );
        // Friday 10:00: every-day window, not open yet.
        assert!(matches!(
            evaluate(&rules, &Situation::default(), at("2026-11-06", "10:00")),
            Status::Locked(LockReason::OutsideWindow { .. })
        ));
    }

    #[test]
    fn parent_lock_and_unlock() {
        let rules = [budget(10, Scope::EveryDay {})];
        let mut s = used(&[(SAT, 3600)]);
        s.unlocked_until = Some(at(SAT, "12:30"));
        // Unlocked: limits don't apply until 12:30.
        assert_eq!(
            evaluate(&rules, &s, at(SAT, "12:00")),
            Status::Allowed {
                remaining_secs: Some(30 * 60)
            }
        );
        // Expired unlock: locked again.
        assert_eq!(
            evaluate(&rules, &s, at(SAT, "12:31")),
            Status::Locked(LockReason::BudgetUsed)
        );
        // A parent's lock beats everything.
        s.locked_by_parent = true;
        assert_eq!(
            evaluate(&rules, &s, at(SAT, "12:00")),
            Status::Locked(LockReason::Parent)
        );
    }

    #[test]
    fn mondays() {
        assert_eq!(
            monday_of(parse_date(SAT).unwrap()),
            parse_date("2026-11-02").unwrap()
        );
        assert_eq!(
            monday_of(parse_date("2026-11-02").unwrap()),
            parse_date("2026-11-02").unwrap()
        );
    }
}
