// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a habit is today: day, period, streak, rate and history, from its
//! entries (ADR 0091 §3). Computed here and nowhere else.

use std::collections::BTreeMap;

use jiff::ToSpan;
use jiff::civil::Date;

use crate::api::{
    Habit, HabitDay, HabitDayState, HabitEntry, HabitEntryStatus, HabitKind, HabitPer, HabitPeriod,
    HabitRatio, HabitStatus, Task,
};

/// How far back the rate of the last weeks reaches.
const RECENT_DAYS: i64 = 28;

/// The status of `task` as a habit on `today`, from its entries (others in
/// `entries` are passed over). `begin` is the first day it counts from: its
/// creation day in the account's zone. With `from`, the history and the
/// entries from that day on come along. `None` when the task is no habit.
#[must_use]
pub fn status(
    task: &Task,
    begin: Date,
    entries: &[HabitEntry],
    today: Date,
    from: Option<Date>,
) -> Option<HabitStatus> {
    let habit = task.habit.as_ref()?;
    let own: Vec<&HabitEntry> = entries.iter().filter(|e| e.habit == task.id).collect();
    // An entry for a day before the creation moves the beginning back
    // (ADR 0091 §2, entering afterwards).
    let begin = own.iter().map(|e| e.day).fold(begin, Date::min).min(today);
    let days = Days::new(habit, &own, begin);

    let (streak, longest, recent, year, period) = match habit.kind {
        HabitKind::Avoid => avoid(&days, begin, today),
        _ => build(habit, &days, begin, today),
    };
    let last_fail = (habit.kind == HabitKind::Avoid)
        .then(|| {
            own.iter()
                .filter(|e| e.status == HabitEntryStatus::Fail && e.day <= today)
                .map(|e| e.day)
                .max()
        })
        .flatten();

    let mut history = Vec::new();
    let mut listed = Vec::new();
    if let Some(from) = from {
        let mut day = from;
        while day <= today {
            history.push(HabitDayState {
                day,
                state: days.state(day),
                amount: days.amount(day),
            });
            day = next(day);
        }
        listed = own
            .iter()
            .filter(|e| e.day >= from)
            .map(|e| (*e).clone())
            .collect();
        listed.sort_by_key(|e| (e.day, e.created, e.id));
    }

    Some(HabitStatus {
        task: task.clone(),
        today,
        day: days.state(today),
        period,
        streak,
        longest,
        recent,
        year,
        last_fail,
        history,
        entries: listed,
    })
}

/// The entries folded per day.
struct Days {
    kind: HabitKind,
    /// The goal of one day: `times` for a daily habit, else 1.
    day_goal: u32,
    begin: Date,
    done: BTreeMap<Date, u32>,
    fail: BTreeMap<Date, ()>,
    /// Pauses as (first, last) day.
    pauses: Vec<(Date, Date)>,
}

impl Days {
    fn new(habit: &Habit, entries: &[&HabitEntry], begin: Date) -> Self {
        let mut days = Self {
            kind: habit.kind,
            day_goal: if habit.per == HabitPer::Day {
                habit.times.max(1)
            } else {
                1
            },
            begin,
            done: BTreeMap::new(),
            fail: BTreeMap::new(),
            pauses: Vec::new(),
        };
        for e in entries {
            match e.status {
                HabitEntryStatus::Done => {
                    *days.done.entry(e.day).or_default() += e.amount.unwrap_or(1);
                }
                HabitEntryStatus::Fail => {
                    days.fail.insert(e.day, ());
                }
                HabitEntryStatus::Pause => {
                    days.pauses
                        .push((e.day, e.until.unwrap_or(e.day).max(e.day)));
                }
                HabitEntryStatus::Unknown => {}
            }
        }
        days
    }

    fn amount(&self, day: Date) -> u32 {
        self.done.get(&day).copied().unwrap_or(0)
    }

    fn paused(&self, day: Date) -> bool {
        self.pauses.iter().any(|(a, b)| *a <= day && day <= *b)
    }

    /// ADR 0091 §3: for `build` done, fail, pause, partial, none in this
    /// order; for `avoid` fail, pause, done.
    fn state(&self, day: Date) -> HabitDay {
        if day < self.begin {
            return HabitDay::None;
        }
        let failed = self.fail.contains_key(&day);
        if self.kind == HabitKind::Avoid {
            return if failed {
                HabitDay::Fail
            } else if self.paused(day) {
                HabitDay::Pause
            } else {
                HabitDay::Done
            };
        }
        let amount = self.amount(day);
        if amount >= self.day_goal {
            HabitDay::Done
        } else if failed {
            HabitDay::Fail
        } else if self.paused(day) {
            HabitDay::Pause
        } else if amount > 0 {
            HabitDay::Partial
        } else {
            HabitDay::None
        }
    }
}

/// What a whole period came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Met,
    /// Missed, but with a pause in it: neither counts nor breaks.
    Paused,
    /// Missed, but begun before the habit: neither counts nor breaks.
    Before,
    Missed,
}

/// Streak, longest, recent, year and the running period of a habit to
/// build, counted in its periods.
fn build(
    habit: &Habit,
    days: &Days,
    begin: Date,
    today: Date,
) -> (u32, u32, HabitRatio, HabitRatio, HabitPeriod) {
    let per = habit.per;
    let target = habit.times.max(1);
    let outcome = |from: Date, to: Date| -> (Outcome, u32) {
        let mut amount = 0;
        let mut paused = false;
        let mut day = from;
        while day <= to {
            amount += days.amount(day);
            paused |= days.paused(day);
            day = next(day);
        }
        let outcome = if amount >= target {
            Outcome::Met
        } else if paused {
            Outcome::Paused
        } else if from < begin {
            Outcome::Before
        } else {
            Outcome::Missed
        };
        (outcome, amount)
    };

    let (now_from, now_to) = period_of(per, today);
    let (running, amount) = outcome(now_from, today.min(now_to));
    let period = HabitPeriod {
        from: now_from,
        to: now_to,
        amount,
        target,
        met: running == Outcome::Met,
    };

    // Every closed period from the one the habit began in, oldest first.
    let mut closed = Vec::new();
    let mut from = period_of(per, begin).0;
    while from < now_from {
        let to = period_of(per, from).1;
        closed.push((from, to, outcome(from, to).0));
        from = next(to);
    }

    let mut streak = u32::from(running == Outcome::Met);
    for (_, _, o) in closed.iter().rev() {
        match o {
            Outcome::Met => streak += 1,
            Outcome::Paused => {}
            Outcome::Before | Outcome::Missed => break,
        }
    }
    let mut longest = 0;
    let mut run = 0;
    for o in closed.iter().map(|c| c.2).chain([running]) {
        match o {
            Outcome::Met => run += 1,
            Outcome::Paused | Outcome::Before => {}
            Outcome::Missed => run = 0,
        }
        longest = longest.max(run);
    }
    // The running period has not missed yet; only a met one counts above.
    longest = longest.max(streak);

    let recent_from = today - RECENT_DAYS.days();
    let ratio = |keep: &dyn Fn(Date) -> bool| {
        let mut r = HabitRatio::default();
        for (_, to, o) in &closed {
            if !keep(*to) {
                continue;
            }
            match o {
                Outcome::Met => {
                    r.met += 1;
                    r.of += 1;
                }
                Outcome::Missed => r.of += 1,
                Outcome::Paused | Outcome::Before => {}
            }
        }
        r
    };
    let recent = ratio(&|to| to >= recent_from);
    let year = ratio(&|to| to.year() == today.year());
    (streak, longest, recent, year, period)
}

/// Streak, longest, recent, year and today of a habit to avoid, counted in
/// whole days: today is not over yet and does not count.
fn avoid(days: &Days, begin: Date, today: Date) -> (u32, u32, HabitRatio, HabitRatio, HabitPeriod) {
    let mut longest = 0;
    let mut run = 0;
    let mut recent = HabitRatio::default();
    let mut year = HabitRatio::default();
    let recent_from = today - RECENT_DAYS.days();
    let mut day = begin;
    while day < today {
        let state = days.state(day);
        match state {
            HabitDay::Fail => run = 0,
            HabitDay::Done => run += 1,
            _ => {}
        }
        longest = longest.max(run);
        if matches!(state, HabitDay::Done | HabitDay::Fail) {
            let met = u32::from(state == HabitDay::Done);
            if day >= recent_from {
                recent.met += met;
                recent.of += 1;
            }
            if day.year() == today.year() {
                year.met += met;
                year.of += 1;
            }
        }
        day = next(day);
    }
    let failed_today = days.state(today) == HabitDay::Fail;
    let streak = if failed_today { 0 } else { run };
    let period = HabitPeriod {
        from: today,
        to: today,
        amount: 0,
        target: 1,
        met: !failed_today,
    };
    (streak, longest, recent, year, period)
}

/// Whether `task` needs something on `day` (ADR 0093 §3): always when it
/// is no habit; never for `avoid`; for a habit to build only while the day
/// has no `done` that reaches its goal, no `fail` and no `pause`, and the
/// period's goal is not reached.
#[must_use]
pub fn to_do(task: &Task, begin: Date, entries: &[HabitEntry], day: Date) -> bool {
    let Some(habit) = task.habit.as_ref() else {
        return true;
    };
    if habit.kind == HabitKind::Avoid {
        return false;
    }
    let Some(status) = status(task, begin, entries, day, None) else {
        return true;
    };
    matches!(status.day, HabitDay::None | HabitDay::Partial) && !status.period.met
}

/// The first and last day of the period `day` is in: the day, the week from
/// Monday, the calendar month.
fn period_of(per: HabitPer, day: Date) -> (Date, Date) {
    match per {
        HabitPer::Week => {
            let back = i64::from(day.weekday().to_monday_zero_offset());
            let monday = day - back.days();
            (monday, monday + 6.days())
        }
        HabitPer::Month => (day.first_of_month(), day.last_of_month()),
        HabitPer::Day | HabitPer::Unknown => (day, day),
    }
}

fn next(day: Date) -> Date {
    day.tomorrow().unwrap_or(day)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use jiff::Timestamp;
    use jiff::civil::date;

    use super::*;
    use crate::api::Id;

    fn habit(kind: HabitKind, times: u32, per: HabitPer) -> Task {
        Task {
            id: Id::new(),
            created: Timestamp::UNIX_EPOCH,
            modified: Timestamp::UNIX_EPOCH,
            created_by: None,
            habit: Some(Habit {
                kind,
                times,
                per,
                unit: None,
            }),
            title: "h".into(),
            note: None,
            project: None,
            parent: None,
            schedule: crate::api::Schedule::default(),
            flagged: false,
            estimate: None,
            priority: None,
            completed_at: None,
            dropped_at: None,
            repeat_of: None,
            sequential: false,
            complete_with_last_child: false,
            tags: Vec::new(),
            properties: BTreeMap::new(),
            position: 0,
        }
    }

    fn entry(task: &Task, day: Date, status: HabitEntryStatus) -> HabitEntry {
        HabitEntry {
            id: Id::new(),
            created: Timestamp::UNIX_EPOCH,
            modified: Timestamp::UNIX_EPOCH,
            created_by: None,
            habit: task.id,
            day,
            status,
            amount: None,
            until: None,
        }
    }

    fn done(task: &Task, day: Date) -> HabitEntry {
        entry(task, day, HabitEntryStatus::Done)
    }

    fn done_n(task: &Task, day: Date, n: u32) -> HabitEntry {
        HabitEntry {
            amount: Some(n),
            ..done(task, day)
        }
    }

    fn pause(task: &Task, day: Date, until: Date) -> HabitEntry {
        HabitEntry {
            until: Some(until),
            ..entry(task, day, HabitEntryStatus::Pause)
        }
    }

    /// Friday, 13 November 2026.
    const TODAY: Date = date(2026, 11, 13);

    #[test]
    fn no_habit_no_status() {
        let mut t = habit(HabitKind::Build, 1, HabitPer::Day);
        t.habit = None;
        assert!(status(&t, TODAY, &[], TODAY, None).is_none());
    }

    #[test]
    fn daily_streak_counts_back_and_today_does_not_break_it() {
        let t = habit(HabitKind::Build, 1, HabitPer::Day);
        let e: Vec<_> = (10..=12).map(|d| done(&t, date(2026, 11, d))).collect();
        let s = status(&t, date(2026, 11, 1), &e, TODAY, None).unwrap();
        assert_eq!(s.day, HabitDay::None);
        assert_eq!((s.streak, s.longest), (3, 3));
        assert!(!s.period.met);

        let mut e = e;
        e.push(done(&t, TODAY));
        let s = status(&t, date(2026, 11, 1), &e, TODAY, None).unwrap();
        assert_eq!(s.day, HabitDay::Done);
        assert_eq!((s.streak, s.longest), (4, 4));
    }

    #[test]
    fn a_missed_day_breaks_a_fail_too_and_the_longest_stays() {
        let t = habit(HabitKind::Build, 1, HabitPer::Day);
        let mut e: Vec<_> = (1..=5).map(|d| done(&t, date(2026, 11, d))).collect();
        // 6 not entered, 7 failed on purpose.
        e.push(entry(&t, date(2026, 11, 7), HabitEntryStatus::Fail));
        e.extend(
            (8..=12)
                .filter(|d| *d != 10)
                .map(|d| done(&t, date(2026, 11, d))),
        );
        let s = status(&t, date(2026, 11, 1), &e, TODAY, Some(date(2026, 11, 5))).unwrap();
        assert_eq!((s.streak, s.longest), (2, 5));
        let states: Vec<_> = s.history.iter().map(|d| d.state).collect();
        assert_eq!(
            states,
            [
                HabitDay::Done,
                HabitDay::None,
                HabitDay::Fail,
                HabitDay::Done,
                HabitDay::Done,
                HabitDay::None,
                HabitDay::Done,
                HabitDay::Done,
                HabitDay::None,
            ]
        );
        // 12 closed days, 9 done; all within the last 28 days and this year.
        assert_eq!(s.recent, HabitRatio { met: 9, of: 12 });
        assert_eq!(s.year, HabitRatio { met: 9, of: 12 });
    }

    #[test]
    fn a_pause_neither_breaks_nor_counts() {
        let t = habit(HabitKind::Build, 1, HabitPer::Day);
        let mut e: Vec<_> = (1..=4).map(|d| done(&t, date(2026, 11, d))).collect();
        e.push(pause(&t, date(2026, 11, 5), date(2026, 11, 9)));
        e.extend((10..=12).map(|d| done(&t, date(2026, 11, d))));
        let s = status(&t, date(2026, 11, 1), &e, TODAY, None).unwrap();
        assert_eq!((s.streak, s.longest), (7, 7));
        assert_eq!(s.recent, HabitRatio { met: 7, of: 7 });
    }

    #[test]
    fn amounts_add_up_to_the_daily_goal() {
        let t = habit(HabitKind::Build, 8, HabitPer::Day);
        // Two devices, offline, each a few: 3 + 4 + 1.
        let e = vec![
            done_n(&t, TODAY, 3),
            done_n(&t, TODAY, 4),
            done_n(&t, date(2026, 11, 12), 5),
        ];
        let s = status(&t, date(2026, 11, 12), &e, TODAY, Some(date(2026, 11, 12))).unwrap();
        assert_eq!(s.day, HabitDay::Partial);
        assert_eq!((s.period.amount, s.period.target), (7, 8));
        assert_eq!(s.history[0].state, HabitDay::Partial);
        let mut e = e;
        e.push(done(&t, TODAY));
        let s = status(&t, date(2026, 11, 12), &e, TODAY, None).unwrap();
        assert_eq!(s.day, HabitDay::Done);
        assert!(s.period.met);
        assert_eq!(s.streak, 1);
    }

    #[test]
    fn weekly_quota_counts_weeks_from_monday() {
        let t = habit(HabitKind::Build, 3, HabitPer::Week);
        let days = [
            // Week of 19 Oct: 3 – met.
            date(2026, 10, 19),
            date(2026, 10, 21),
            date(2026, 10, 25),
            // Week of 26 Oct: 2 – missed.
            date(2026, 10, 26),
            date(2026, 10, 28),
            // Weeks of 2 and 9 Nov: 3 each.
            date(2026, 11, 2),
            date(2026, 11, 3),
            date(2026, 11, 8),
            date(2026, 11, 9),
            date(2026, 11, 10),
        ];
        let e: Vec<_> = days.iter().map(|d| done(&t, *d)).collect();
        // Begun on a Wednesday: the first week is missed, but began before.
        let s = status(&t, date(2026, 10, 14), &e, TODAY, None).unwrap();
        assert_eq!(s.period.from, date(2026, 11, 9));
        assert_eq!(s.period.to, date(2026, 11, 15));
        assert_eq!((s.period.amount, s.period.met), (2, false));
        // The running week does not break it: 2 Nov met, 26 Oct missed.
        assert_eq!((s.streak, s.longest), (1, 1));
        // Closed weeks ending in the last 28 days: 19 Oct, 26 Oct, 2 Nov.
        assert_eq!(s.recent, HabitRatio { met: 2, of: 3 });

        let mut e = e;
        e.push(done(&t, TODAY));
        let s = status(&t, date(2026, 10, 14), &e, TODAY, None).unwrap();
        assert!(s.period.met);
        assert_eq!(s.streak, 2);
    }

    #[test]
    fn monthly_goal_over_the_turn_of_the_year() {
        let t = habit(HabitKind::Build, 2, HabitPer::Month);
        let e = vec![
            done(&t, date(2026, 12, 3)),
            done(&t, date(2026, 12, 30)),
            done(&t, date(2027, 1, 2)),
        ];
        let s = status(&t, date(2026, 12, 1), &e, date(2027, 1, 10), None).unwrap();
        assert_eq!(s.period.from, date(2027, 1, 1));
        assert_eq!(s.period.to, date(2027, 1, 31));
        assert_eq!(s.streak, 1);
        // December closed in 2026: not this year's.
        assert_eq!(s.year, HabitRatio::default());
        assert_eq!(s.recent, HabitRatio { met: 1, of: 1 });
    }

    #[test]
    fn avoid_counts_whole_days_since_the_last_relapse() {
        let t = habit(HabitKind::Avoid, 1, HabitPer::Day);
        let e = vec![
            entry(&t, date(2026, 11, 3), HabitEntryStatus::Fail),
            pause(&t, date(2026, 11, 8), date(2026, 11, 9)),
        ];
        let s = status(&t, date(2026, 10, 30), &e, TODAY, None).unwrap();
        assert_eq!(s.day, HabitDay::Done);
        assert_eq!(s.last_fail, Some(date(2026, 11, 3)));
        // 4–12 Nov without 8 and 9.
        assert_eq!(s.streak, 7);
        // 30 Oct – 2 Nov before it.
        assert_eq!(s.longest, 7);
        assert_eq!(s.recent, HabitRatio { met: 11, of: 12 });

        let mut e = e;
        e.push(entry(&t, TODAY, HabitEntryStatus::Fail));
        let s = status(&t, date(2026, 10, 30), &e, TODAY, None).unwrap();
        assert_eq!((s.day, s.streak), (HabitDay::Fail, 0));
        assert!(!s.period.met);
    }

    #[test]
    fn entered_afterwards_before_the_creation_moves_the_beginning() {
        let t = habit(HabitKind::Build, 1, HabitPer::Day);
        let e: Vec<_> = (10..=12).map(|d| done(&t, date(2026, 11, d))).collect();
        let s = status(&t, TODAY, &e, TODAY, None).unwrap();
        assert_eq!(s.streak, 3);
    }

    #[test]
    fn other_habits_entries_are_passed_over() {
        let t = habit(HabitKind::Build, 1, HabitPer::Day);
        let other = habit(HabitKind::Build, 1, HabitPer::Day);
        let e = vec![done(&other, TODAY)];
        let s = status(&t, TODAY, &e, TODAY, None).unwrap();
        assert_eq!(s.day, HabitDay::None);
    }
}
