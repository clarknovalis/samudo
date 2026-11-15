//! The next task of a repetition (ADR 0007 §3–§5, ADR 0033).
//!
//! The rule is RFC 5545 and computed by the `rrule` crate, which works in
//! chrono; dates go to it as text and come back into jiff. Nothing outside
//! this module sees chrono.

use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};
use rrule::RRuleSet;
use samu_api::{Id, NextRepeat, RepeatMode, Schedule, Task, When};

/// Why no next date could be computed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RepeatError {
    #[error("repeat rule {rule:?}: {reason}")]
    Rule { rule: String, reason: String },
    #[error("a repetition mode this server does not know")]
    UnknownMode,
    #[error("a date form this server does not know")]
    UnknownDate,
    #[error("the zone of {at} has no name; a repetition needs one")]
    UnnamedZone { at: String },
}

/// More occurrences than this between the anchor and the next date is a
/// rule gone wrong, not a calendar: a daily rule caught up over 270 years.
const SEARCH_LIMIT: usize = 100_000;

/// Checks a rule before it is stored: RFC 5545, at least daily, without
/// `DTSTART` or an `RRULE:` prefix (ADR 0033 §4).
pub fn check(rule: &str) -> Result<(), RepeatError> {
    let (rest, _) = split_count(rule)?;
    // Any start will do for the check; the rule alone decides.
    parse(rule, &rest, "DTSTART:20260101T000000Z").map(|_| ())
}

/// The schedule of the next task, or `None` when the repetition has ended.
///
/// `completed` is the old task's `completed_at`, `now` decides "today" for
/// `catch_up`, `chain` is how many tasks the chain has with this one, and
/// `zone` is the account's (ADR 0007 §2).
pub fn next(
    schedule: &Schedule,
    completed: Timestamp,
    now: Timestamp,
    chain: u32,
    zone: &TimeZone,
) -> Result<Option<Schedule>, RepeatError> {
    let Some(repeat) = &schedule.repeat else {
        return Ok(None);
    };
    let (rest, count) = split_count(&repeat.rule)?;
    if count.is_some_and(|count| chain >= count) {
        return Ok(None);
    }
    let order = match repeat.mode {
        RepeatMode::Fixed | RepeatMode::DueAfterCompletion => {
            [Lead::Due, Lead::Defer, Lead::Planned]
        }
        RepeatMode::DeferAfterCompletion => [Lead::Defer, Lead::Due, Lead::Planned],
        RepeatMode::Unknown => return Err(RepeatError::UnknownMode),
    };
    let fixed = repeat.mode == RepeatMode::Fixed;
    let lead = order.into_iter().find(|lead| lead.of(schedule).is_some());
    let completed_day = completed.to_zoned(zone.clone()).date();

    let Some(lead) = lead else {
        // No date at all: the new one is counted from completion (ADR 0033
        // §3).
        let found = next_date(&repeat.rule, &rest, completed_day, completed_day)?;
        let mut next = schedule.clone();
        let field = match repeat.mode {
            RepeatMode::DeferAfterCompletion => Lead::Defer,
            _ => Lead::Due,
        };
        return Ok(found.map(|date| {
            *field.of_mut(&mut next) = Some(When::Date(date));
            next
        }));
    };

    let old = lead.of(schedule).cloned().ok_or(RepeatError::UnknownDate)?;
    let new = match &old {
        When::Date(date) => {
            let (start, mut bound) = if fixed {
                (*date, *date)
            } else {
                (completed_day, completed_day)
            };
            if fixed && repeat.catch_up {
                bound = bound.max(now.to_zoned(zone.clone()).date());
            }
            next_date(&repeat.rule, &rest, start, bound)?.map(When::Date)
        }
        When::At(at) => {
            let start = if fixed {
                at.clone()
            } else {
                // The old time of day, on the day of completion, in the
                // old zone.
                let day = completed.to_zoned(at.time_zone().clone()).date();
                day.to_datetime(at.time())
                    .to_zoned(at.time_zone().clone())
                    .map_err(|e| rule_error(&repeat.rule, e))?
            };
            let mut bound = start.timestamp();
            if fixed && repeat.catch_up {
                bound = bound.max(now);
            }
            next_at(&repeat.rule, &rest, &start, bound)?.map(When::At)
        }
        When::Unknown => return Err(RepeatError::UnknownDate),
    };
    let Some(new) = new else {
        return Ok(None);
    };

    // The other dates move by the same number of days (ADR 0007 §5).
    let days = day_of(&old)
        .until(day_of(&new))
        .map_err(|e| rule_error(&repeat.rule, e))?
        .get_days();
    let mut next = schedule.clone();
    for other in [Lead::Defer, Lead::Due, Lead::Planned] {
        let slot = other.of_mut(&mut next);
        *slot = if other == lead {
            Some(new.clone())
        } else {
            match slot.take() {
                None => None,
                Some(when) => Some(shift(when, days, &repeat.rule)?),
            }
        };
    }
    Ok(Some(next))
}

/// How many tasks the chain of `task` has up to and with it, over
/// `repeat_of` (ADR 0007 §3), as far as `tasks` reaches.
pub fn chain(tasks: &[Task], task: &Task) -> u32 {
    let mut length = 1;
    let mut before = task.repeat_of;
    while let Some(id) = before {
        // `repeat_of` can be set, so a chain could loop; stop counting.
        if length >= 100_000 {
            break;
        }
        length += 1;
        before = find(tasks, id).and_then(|t| t.repeat_of);
    }
    length
}

fn find(tasks: &[Task], id: Id) -> Option<&Task> {
    tasks.iter().find(|t| t.id == id)
}

/// What completing `task` now would make: the dates of the next task, or
/// `ended`. `None` for a task that does not repeat, is closed, or whose
/// rule cannot be computed here.
pub fn preview(tasks: &[Task], task: &Task, now: Timestamp, zone: &TimeZone) -> Option<NextRepeat> {
    task.schedule.repeat.as_ref()?;
    if task.completed_at.is_some() || task.dropped_at.is_some() {
        return None;
    }
    match next(&task.schedule, now, now, chain(tasks, task), zone) {
        Ok(Some(s)) => Some(NextRepeat {
            defer: s.defer,
            due: s.due,
            planned: s.planned,
            ended: false,
        }),
        Ok(None) => Some(NextRepeat {
            ended: true,
            ..NextRepeat::default()
        }),
        Err(_) => None,
    }
}

/// The frequencies a form offers, the weekdays in order (ADR 0007 §3).
pub const FREQS: [&str; 4] = ["DAILY", "WEEKLY", "MONTHLY", "YEARLY"];
pub const WEEKDAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

/// A rule taken apart for a form (85b3799a): what a form edits, and every
/// other part as it is, so putting it together keeps what the form does
/// not know. The web has the same in `web/lib/repeat.js`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parts {
    pub freq: Option<String>,
    pub interval: u32,
    /// Plain weekdays of a weekly rule.
    pub byday: Vec<String>,
    pub until: Option<Date>,
    pub count: Option<u32>,
    pub rest: Vec<String>,
}

impl Parts {
    pub fn parse(rule: &str) -> Self {
        let mut parts = Self {
            freq: None,
            interval: 1,
            byday: Vec::new(),
            until: None,
            count: None,
            rest: Vec::new(),
        };
        for part in rule.split(';').filter(|p| !p.is_empty()) {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            let key = key.to_ascii_uppercase();
            let days: Vec<String> = value.split(',').map(str::to_ascii_uppercase).collect();
            match key.as_str() {
                "FREQ" if parts.freq.is_none() => parts.freq = Some(value.to_ascii_uppercase()),
                "INTERVAL" if value.parse::<u32>().is_ok_and(|n| n >= 1) => {
                    parts.interval = value.parse().unwrap_or(1);
                }
                "BYDAY" if days.iter().all(|d| WEEKDAYS.contains(&d.as_str())) => {
                    parts.byday = days;
                }
                "UNTIL" if until_date(value).is_some() => parts.until = until_date(value),
                "COUNT" if value.parse::<u32>().is_ok_and(|n| n >= 1) => {
                    parts.count = value.parse().ok();
                }
                _ => parts.rest.push(part.to_string()),
            }
        }
        // Days on a rule that is not weekly mean something else; they stay.
        if !parts.byday.is_empty() && parts.freq.as_deref() != Some("WEEKLY") {
            parts.rest.push(format!("BYDAY={}", parts.byday.join(",")));
            parts.byday.clear();
        }
        parts
    }

    /// Whether a form can show it: a frequency it offers.
    pub fn fits(&self) -> bool {
        self.freq.as_deref().is_some_and(|f| FREQS.contains(&f))
    }

    /// The rule again, FREQ first, the parts a form does not know after
    /// the ones it does; UNTIL and COUNT exclude each other, COUNT wins.
    pub fn rule(&self) -> String {
        let mut out = vec![format!("FREQ={}", self.freq.as_deref().unwrap_or(""))];
        if self.interval > 1 {
            out.push(format!("INTERVAL={}", self.interval));
        }
        if self.freq.as_deref() == Some("WEEKLY") && !self.byday.is_empty() {
            let days: Vec<&str> = WEEKDAYS
                .into_iter()
                .filter(|d| self.byday.iter().any(|b| b == d))
                .collect();
            out.push(format!("BYDAY={}", days.join(",")));
        }
        out.extend(self.rest.iter().cloned());
        match (self.count, self.until) {
            (Some(n), _) => out.push(format!("COUNT={n}")),
            // The end of that day in UTC: a date alone is refused next to a
            // start in UTC, and the whole day still counts.
            (None, Some(until)) => out.push(format!("UNTIL={}T235959Z", until.strftime("%Y%m%d"))),
            (None, None) => {}
        }
        out.join(";")
    }
}

fn until_date(value: &str) -> Option<Date> {
    let digits = value.get(..8)?;
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    format!("{}-{}-{}", &digits[..4], &digits[4..6], &digits[6..8])
        .parse()
        .ok()
}

/// Which date leads the repetition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lead {
    Defer,
    Due,
    Planned,
}

impl Lead {
    fn of(self, schedule: &Schedule) -> Option<&When> {
        match self {
            Self::Defer => schedule.defer.as_ref(),
            Self::Due => schedule.due.as_ref(),
            Self::Planned => schedule.planned.as_ref(),
        }
    }

    fn of_mut(self, schedule: &mut Schedule) -> &mut Option<When> {
        match self {
            Self::Defer => &mut schedule.defer,
            Self::Due => &mut schedule.due,
            Self::Planned => &mut schedule.planned,
        }
    }
}

fn rule_error(rule: &str, reason: impl std::fmt::Display) -> RepeatError {
    RepeatError::Rule {
        rule: rule.to_string(),
        reason: reason.to_string(),
    }
}

/// The rule without `COUNT`, and the count. A chain counts its own members
/// over `repeat_of` (ADR 0007 §3), while the rule starts anew at each task;
/// so `COUNT` is never handed to the crate.
fn split_count(rule: &str) -> Result<(String, Option<u32>), RepeatError> {
    if rule.contains(['\n', '\r', ':']) {
        return Err(rule_error(
            rule,
            "only the rule's parts, without DTSTART or RRULE:",
        ));
    }
    let mut count = None;
    let mut rest = Vec::new();
    for part in rule.split(';') {
        match part.split_once('=') {
            Some((key, value)) if key.eq_ignore_ascii_case("COUNT") => {
                let n: u32 = value
                    .parse()
                    .map_err(|_| rule_error(rule, format!("COUNT={value}")))?;
                if n == 0 {
                    return Err(rule_error(rule, "COUNT=0"));
                }
                count = Some(n);
            }
            _ => rest.push(part),
        }
    }
    Ok((rest.join(";"), count))
}

fn parse(rule: &str, rest: &str, start: &str) -> Result<RRuleSet, RepeatError> {
    let freq = rest.split(';').find_map(|part| {
        part.split_once('=')
            .filter(|(key, _)| key.eq_ignore_ascii_case("FREQ"))
            .map(|(_, value)| value.to_ascii_uppercase())
    });
    match freq.as_deref() {
        Some("DAILY" | "WEEKLY" | "MONTHLY" | "YEARLY") => {}
        Some(other) => return Err(rule_error(rule, format!("FREQ={other}: at least daily"))),
        None => return Err(rule_error(rule, "no FREQ")),
    }
    format!("{start}\nRRULE:{rest}")
        .parse::<RRuleSet>()
        .map_err(|e| rule_error(rule, e))
}

/// The first date of the rule after `bound`, counted from `start`; dates
/// are computed as midnight UTC, which has no gaps.
fn next_date(
    rule: &str,
    rest: &str,
    start: Date,
    bound: Date,
) -> Result<Option<Date>, RepeatError> {
    let set = parse(
        rule,
        rest,
        &format!("DTSTART:{}T000000Z", start.strftime("%Y%m%d")),
    )?;
    let mut found = None;
    for occurrence in set.into_iter().take(SEARCH_LIMIT) {
        let date: Date = occurrence
            .format("%Y-%m-%d")
            .to_string()
            .parse()
            .map_err(|e| rule_error(rule, e))?;
        if date > bound {
            found = Some(date);
            break;
        }
    }
    Ok(found)
}

/// The first instant of the rule after `bound`, counted from `start` in
/// its zone, keeping the wall time over a change of daylight saving.
fn next_at(
    rule: &str,
    rest: &str,
    start: &Zoned,
    bound: Timestamp,
) -> Result<Option<Zoned>, RepeatError> {
    let tz = start.time_zone().clone();
    let Some(name) = tz.iana_name() else {
        return Err(RepeatError::UnnamedZone {
            at: start.to_string(),
        });
    };
    let dtstart = format!(
        "DTSTART;TZID={name}:{}",
        start.datetime().strftime("%Y%m%dT%H%M%S")
    );
    let set = parse(rule, rest, &dtstart)?;
    let mut found = None;
    for occurrence in set.into_iter().take(SEARCH_LIMIT) {
        let wall: DateTime = occurrence
            .naive_local()
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string()
            .parse()
            .map_err(|e| rule_error(rule, e))?;
        let at = wall.to_zoned(tz.clone()).map_err(|e| rule_error(rule, e))?;
        if at.timestamp() > bound {
            found = Some(at);
            break;
        }
    }
    Ok(found)
}

fn day_of(when: &When) -> Date {
    match when {
        When::Date(date) => *date,
        When::At(at) => at.date(),
        // `next` refuses an unknown form before it gets here.
        When::Unknown => Date::MIN,
    }
}

fn shift(when: When, days: i32, rule: &str) -> Result<When, RepeatError> {
    Ok(match when {
        When::Date(date) => When::Date(
            date.checked_add(days.days())
                .map_err(|e| rule_error(rule, e))?,
        ),
        When::At(at) => When::At(
            at.checked_add(days.days())
                .map_err(|e| rule_error(rule, e))?,
        ),
        When::Unknown => return Err(RepeatError::UnknownDate),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;
    use samu_api::{Period, Repeat};

    fn berlin() -> TimeZone {
        TimeZone::get("Europe/Berlin").unwrap()
    }

    fn day(s: &str) -> When {
        When::Date(s.parse().unwrap())
    }

    fn at(s: &str) -> When {
        When::At(s.parse().unwrap())
    }

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn repeating(rule: &str, mode: RepeatMode, catch_up: bool) -> Schedule {
        Schedule {
            repeat: Some(Repeat {
                rule: rule.into(),
                mode,
                catch_up,
            }),
            ..Schedule::default()
        }
    }

    fn next_of(s: &Schedule, completed: &str) -> Option<Schedule> {
        next(s, ts(completed), ts(completed), 1, &berlin()).unwrap()
    }

    #[test]
    fn fixed_counts_from_the_due_date_not_from_completion() {
        let mut s = repeating("FREQ=WEEKLY", RepeatMode::Fixed, false);
        s.due = Some(day("2026-11-10"));
        // Completed two weeks late: the next one is still a week after due.
        let n = next_of(&s, "2026-11-24T10:00:00Z").unwrap();
        assert_eq!(n.due, Some(day("2026-11-17")));
        assert_eq!(n.repeat, s.repeat);
    }

    #[test]
    fn catch_up_skips_to_the_first_date_after_today() {
        let mut s = repeating("FREQ=WEEKLY", RepeatMode::Fixed, true);
        s.due = Some(day("2026-11-03"));
        // Today is Tuesday 24.11.: three Tuesdays missed, 24.11. is today.
        let n = next_of(&s, "2026-11-24T10:00:00Z").unwrap();
        assert_eq!(n.due, Some(day("2026-12-01")));
    }

    #[test]
    fn due_after_completion_counts_from_the_day_of_completion() {
        let mut s = repeating("FREQ=WEEKLY", RepeatMode::DueAfterCompletion, false);
        s.due = Some(day("2026-11-10"));
        // 23:30 UTC on the 12th is already the 13th in Berlin.
        let n = next_of(&s, "2026-11-12T23:30:00Z").unwrap();
        assert_eq!(n.due, Some(day("2026-11-20")));
    }

    #[test]
    fn defer_after_completion_moves_defer_and_due_with_it() {
        let mut s = repeating("FREQ=MONTHLY", RepeatMode::DeferAfterCompletion, false);
        s.defer = Some(day("2026-11-01"));
        s.due = Some(day("2026-11-05"));
        s.due_warning = Some(Period(1.day()));
        let n = next_of(&s, "2026-11-03T08:00:00Z").unwrap();
        assert_eq!(n.defer, Some(day("2026-12-03")));
        // Four days apart before, four days apart after (ADR 0007 §5).
        assert_eq!(n.due, Some(day("2026-12-07")));
        assert_eq!(n.due_warning, s.due_warning);
    }

    #[test]
    fn planned_moves_along() {
        let mut s = repeating("FREQ=DAILY;INTERVAL=2", RepeatMode::Fixed, false);
        s.due = Some(day("2026-11-10"));
        s.planned = Some(at("2026-11-09T14:00:00+01:00[Europe/Berlin]"));
        let n = next_of(&s, "2026-11-10T08:00:00Z").unwrap();
        assert_eq!(n.due, Some(day("2026-11-12")));
        assert_eq!(
            n.planned,
            Some(at("2026-11-11T14:00:00+01:00[Europe/Berlin]"))
        );
    }

    #[test]
    fn a_time_keeps_its_wall_clock_over_daylight_saving() {
        let mut s = repeating("FREQ=WEEKLY", RepeatMode::Fixed, false);
        s.due = Some(at("2026-10-20T09:00:00+02:00[Europe/Berlin]"));
        let n = next_of(&s, "2026-10-20T08:00:00Z").unwrap();
        // Summer time ends on 25.10.2026; 09:00 stays 09:00.
        assert_eq!(n.due, Some(at("2026-10-27T09:00:00+01:00[Europe/Berlin]")));
    }

    #[test]
    fn a_time_after_completion_keeps_its_time_of_day() {
        let mut s = repeating("FREQ=DAILY", RepeatMode::DueAfterCompletion, false);
        s.due = Some(at("2026-11-01T18:00:00+01:00[Europe/Berlin]"));
        let n = next_of(&s, "2026-11-10T07:00:00Z").unwrap();
        assert_eq!(n.due, Some(at("2026-11-11T18:00:00+01:00[Europe/Berlin]")));
    }

    #[test]
    fn by_day_and_by_month_day_and_ordinals() {
        let mut s = repeating("FREQ=WEEKLY;BYDAY=MO", RepeatMode::Fixed, false);
        s.due = Some(day("2026-11-09")); // a Monday
        assert_eq!(
            next_of(&s, "2026-11-09T10:00:00Z").unwrap().due,
            Some(day("2026-11-16"))
        );

        let mut s = repeating("FREQ=MONTHLY;BYMONTHDAY=-1", RepeatMode::Fixed, false);
        s.due = Some(day("2026-11-30"));
        assert_eq!(
            next_of(&s, "2026-11-30T10:00:00Z").unwrap().due,
            Some(day("2026-12-31"))
        );

        let mut s = repeating("FREQ=MONTHLY;BYDAY=1MO", RepeatMode::Fixed, false);
        s.due = Some(day("2026-11-02"));
        assert_eq!(
            next_of(&s, "2026-11-02T10:00:00Z").unwrap().due,
            Some(day("2026-12-07"))
        );
    }

    #[test]
    fn a_due_off_the_rule_goes_to_the_next_date_on_it() {
        let mut s = repeating("FREQ=WEEKLY;BYDAY=FR", RepeatMode::Fixed, false);
        s.due = Some(day("2026-11-11")); // a Wednesday
        assert_eq!(
            next_of(&s, "2026-11-11T10:00:00Z").unwrap().due,
            Some(day("2026-11-13"))
        );
    }

    #[test]
    fn count_ends_the_chain() {
        let mut s = repeating("FREQ=YEARLY;COUNT=3", RepeatMode::Fixed, false);
        s.due = Some(day("2026-11-10"));
        let completed = ts("2026-11-10T10:00:00Z");
        let zone = berlin();
        assert!(next(&s, completed, completed, 2, &zone).unwrap().is_some());
        assert_eq!(next(&s, completed, completed, 3, &zone).unwrap(), None);
    }

    #[test]
    fn until_ends_the_chain() {
        let mut s = repeating(
            "FREQ=WEEKLY;UNTIL=20261120T000000Z",
            RepeatMode::Fixed,
            false,
        );
        s.due = Some(day("2026-11-10"));
        assert_eq!(
            next_of(&s, "2026-11-10T10:00:00Z").unwrap().due,
            Some(day("2026-11-17"))
        );
        s.due = Some(day("2026-11-17"));
        assert_eq!(next_of(&s, "2026-11-17T10:00:00Z"), None);
    }

    #[test]
    fn until_as_a_date_with_a_zoned_due() {
        let mut s = repeating(
            "FREQ=DAILY;UNTIL=20261112T235959Z",
            RepeatMode::Fixed,
            false,
        );
        s.due = Some(at("2026-11-11T09:00:00+01:00[Europe/Berlin]"));
        assert!(next_of(&s, "2026-11-11T10:00:00Z").is_some());
        s.due = Some(at("2026-11-12T09:00:00+01:00[Europe/Berlin]"));
        assert_eq!(next_of(&s, "2026-11-12T10:00:00Z"), None);
    }

    #[test]
    fn without_any_date_the_new_due_counts_from_completion() {
        let s = repeating("FREQ=WEEKLY", RepeatMode::Fixed, false);
        let n = next_of(&s, "2026-11-10T10:00:00Z").unwrap();
        assert_eq!(n.due, Some(day("2026-11-17")));
        assert_eq!(n.defer, None);

        let s = repeating("FREQ=DAILY", RepeatMode::DeferAfterCompletion, false);
        let n = next_of(&s, "2026-11-10T10:00:00Z").unwrap();
        assert_eq!(n.defer, Some(day("2026-11-11")));
        assert_eq!(n.due, None);
    }

    #[test]
    fn no_repeat_no_next() {
        let s = Schedule {
            due: Some(day("2026-11-10")),
            ..Schedule::default()
        };
        assert_eq!(next_of(&s, "2026-11-10T10:00:00Z"), None);
    }

    #[test]
    fn rules_are_checked() {
        for good in [
            "FREQ=DAILY",
            "FREQ=weekly;BYDAY=MO,TH",
            "FREQ=MONTHLY;BYDAY=-1FR",
            "FREQ=YEARLY;COUNT=5",
        ] {
            assert_eq!(check(good), Ok(()), "{good}");
        }
        for bad in [
            "",
            "BYDAY=MO",
            "FREQ=HOURLY",
            "FREQ=FORTNIGHTLY",
            "RRULE:FREQ=DAILY",
            "FREQ=DAILY\nRRULE:FREQ=WEEKLY",
            "FREQ=DAILY;COUNT=0",
            "FREQ=DAILY;COUNT=x",
            "FREQ=WEEKLY;BYDAY=XY",
        ] {
            assert!(check(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn an_unknown_mode_is_an_error() {
        let mut s = repeating("FREQ=DAILY", RepeatMode::Unknown, false);
        s.due = Some(day("2026-11-10"));
        let t = ts("2026-11-10T10:00:00Z");
        assert_eq!(next(&s, t, t, 1, &berlin()), Err(RepeatError::UnknownMode));
    }

    fn id(n: u128) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-{n:012x}").parse().unwrap()
    }

    fn task(n: u128, repeat_of: Option<u128>, schedule: Schedule) -> Task {
        let mut t: Task = serde_json::from_value(serde_json::json!({
            "id": id(n),
            "created": "2026-11-01T00:00:00Z",
            "modified": "2026-11-01T00:00:00Z",
            "title": "t",
        }))
        .unwrap();
        t.repeat_of = repeat_of.map(id);
        t.schedule = schedule;
        t
    }

    #[test]
    fn the_chain_counts_back_over_repeat_of() {
        let s = Schedule::default();
        let tasks = vec![
            task(1, None, s.clone()),
            task(2, Some(1), s.clone()),
            task(3, Some(2), s),
        ];
        assert_eq!(chain(&tasks, &tasks[0]), 1);
        assert_eq!(chain(&tasks, &tasks[2]), 3);
    }

    #[test]
    fn preview_is_completing_now() {
        let mut s = repeating("FREQ=WEEKLY", RepeatMode::DueAfterCompletion, false);
        s.due = Some(day("2026-11-03"));
        s.defer = Some(day("2026-11-02"));
        let tasks = vec![task(1, None, s)];
        let n = preview(&tasks, &tasks[0], ts("2026-11-10T10:00:00Z"), &berlin()).unwrap();
        assert_eq!(n.due, Some(day("2026-11-17")));
        assert_eq!(n.defer, Some(day("2026-11-16")));
        assert!(!n.ended);
    }

    #[test]
    fn preview_says_when_count_ends_the_chain() {
        let mut s = repeating("FREQ=DAILY;COUNT=2", RepeatMode::Fixed, false);
        s.due = Some(day("2026-11-10"));
        let tasks = vec![task(1, None, s.clone()), task(2, Some(1), s)];
        let now = ts("2026-11-10T10:00:00Z");
        assert!(!preview(&tasks, &tasks[0], now, &berlin()).unwrap().ended);
        assert!(preview(&tasks, &tasks[1], now, &berlin()).unwrap().ended);
    }

    #[test]
    fn no_preview_without_repeat_or_when_closed() {
        let tasks = vec![task(1, None, Schedule::default())];
        let now = ts("2026-11-10T10:00:00Z");
        assert_eq!(preview(&tasks, &tasks[0], now, &berlin()), None);
        let mut s = repeating("FREQ=DAILY", RepeatMode::Fixed, false);
        s.due = Some(day("2026-11-10"));
        let mut done = task(2, None, s);
        done.completed_at = Some(now);
        assert_eq!(preview(&[], &done, now, &berlin()), None);
    }

    #[test]
    fn parts_keep_what_a_form_does_not_know() {
        let p = Parts::parse("FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=4");
        assert_eq!(p.freq.as_deref(), Some("MONTHLY"));
        assert_eq!(p.count, Some(4));
        assert_eq!(p.rest, vec!["BYMONTHDAY=-1".to_string()]);
        assert!(p.fits());
        let changed = Parts { interval: 3, ..p };
        assert_eq!(
            changed.rule(),
            "FREQ=MONTHLY;INTERVAL=3;BYMONTHDAY=-1;COUNT=4"
        );
    }

    #[test]
    fn parts_of_a_weekly_rule_with_days_and_until() {
        let p = Parts::parse("freq=weekly;interval=2;byday=th,mo;until=20261231");
        assert_eq!(p.interval, 2);
        assert_eq!(p.byday, vec!["TH".to_string(), "MO".to_string()]);
        assert_eq!(p.until, Some(jiff::civil::date(2026, 12, 31)));
        assert_eq!(
            p.rule(),
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH;UNTIL=20261231T235959Z"
        );
        assert_eq!(check(&p.rule()), Ok(()));
    }

    #[test]
    fn days_of_a_monthly_rule_are_not_weekdays() {
        let p = Parts::parse("FREQ=MONTHLY;BYDAY=-1FR");
        assert!(p.byday.is_empty());
        assert_eq!(p.rule(), "FREQ=MONTHLY;BYDAY=-1FR");
        let p = Parts::parse("FREQ=MONTHLY;BYDAY=MO");
        assert!(p.byday.is_empty());
        assert_eq!(p.rest, vec!["BYDAY=MO".to_string()]);
    }

    #[test]
    fn a_rule_a_form_cannot_show() {
        assert!(!Parts::parse("BYDAY=MO").fits());
        assert!(!Parts::parse("FREQ=HOURLY").fits());
    }
}
