// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reminders on the phone (ADR 0030 §5, §6; appointments ADR 0101 §6, §7).
//!
//! The app keeps one alarm: the next one [`Samu::next_alarm`] names. When
//! it goes off, [`Samu::fire`] says what to show — everything due since the
//! last time, at most a day back — and the app asks for the next alarm.
//! "Later" and "tomorrow" stay on this phone. When a reminder goes off is
//! computed by `samu_core::reminders`, nowhere here. Appointments remind
//! only where this phone is told to: a phone whose calendar app already
//! syncs them (DAVx5) would remind twice.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Span, SpanRelativeTo, Timestamp, ToSpan};
use samu_api::{
    AlarmRelated, Alert, AppointmentAlarm, AppointmentInstance, Change, Id, ObjectKind, Period,
    Reminder, ReminderAnchor, Schedule, When,
};
use samu_core::availability::Settings;
use samu_core::overlay::Objects;
use samu_core::reminders::{self, Alarm};

use crate::{AppointmentRow, MobileError, Samu, appointment_row, input, parse_id};
use samu_replica::Cache;

/// How far back a missed reminder is still shown.
const CATCH_UP_HOURS: i64 = 24;
/// How far ahead the next alarm is looked for; the app asks again after
/// every sync anyway.
const AHEAD_DAYS: i64 = 60;
/// Up to where reminders have been shown.
const SHOWN: &str = "reminded_until";
/// Whether this phone reminds of appointments (ADR 0101 §6); off unless set.
const APPOINTMENTS: &str = "appointment_reminders";

/// A reminder to show.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Notice {
    /// What it is about, to tag the notification and to snooze it: a
    /// task's or project's id, or `<appointment id>@<start in ms>` for one
    /// occurrence of an appointment.
    pub key: String,
    /// The task, project or appointment.
    pub owner_id: String,
    pub title: String,
    /// When it went off, milliseconds since the epoch.
    pub at_ms: i64,
    /// A task can be completed from the notice; a project is opened.
    pub is_task: bool,
    /// The occurrence, for an appointment (ADR 0101 §7): its time and
    /// place to show; tapping opens its sheet with [`Samu::appointment`].
    pub appointment: Option<AppointmentRow>,
}

/// One reminder of an appointment, for its sheet (ADR 0101 §9).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AlarmRow {
    /// Minutes before the start (or the end); negative after it.
    pub before_minutes: Option<i64>,
    /// Counted from the end, not the start.
    pub end: bool,
    /// At a fixed instant, milliseconds since the epoch.
    pub at_ms: Option<i64>,
}

/// What a reminder is relative to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Anchor {
    At,
    Due,
    Planned,
    Defer,
    /// Every day at its time; a habit only while there is something to do
    /// (ADR 0093).
    Daily,
    /// From a newer samu; it reminds of nothing here.
    Unknown,
}

/// A reminder set on a task, for its page.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReminderRow {
    pub id: String,
    pub anchor: Anchor,
    /// With `At`: the instant, milliseconds since the epoch.
    pub at_ms: Option<i64>,
    /// How long before the anchor, in minutes; none = at it.
    pub before_minutes: Option<i64>,
    /// When it goes off as things stand; none when the date is not set.
    /// With `Daily`: the next time, within a week.
    pub goes_off_ms: Option<i64>,
    /// With `Daily`: the time of day, in minutes after midnight.
    pub time_minutes: Option<i64>,
}

fn timestamp(ms: i64) -> Result<Timestamp, MobileError> {
    Timestamp::from_millisecond(ms).map_err(|e| input(e.to_string()))
}

fn shown_until(cache: &Cache) -> Result<Option<Timestamp>, MobileError> {
    Ok(cache.meta(SHOWN)?.and_then(|s| s.parse().ok()))
}

/// Minutes as a span a person would write: days, hours or minutes.
fn span_of(minutes: i64) -> Span {
    if minutes % (24 * 60) == 0 {
        Span::new().days(minutes / (24 * 60))
    } else if minutes % 60 == 0 {
        Span::new().hours(minutes / 60)
    } else {
        Span::new().minutes(minutes)
    }
}

fn minutes_of(span: Span) -> i64 {
    span.get_days() as i64 * 24 * 60 + span.get_hours() as i64 * 60 + span.get_minutes()
}

#[uniffi::export]
impl Samu {
    /// When to set the alarm, milliseconds since the epoch; none when no
    /// reminder is ahead. In the past when one was missed: set it anyway,
    /// it goes off at once.
    pub fn next_alarm(&self, now_ms: i64) -> Result<Option<i64>, MobileError> {
        let now = timestamp(now_ms)?;
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, _) = Self::view(&cache)?;
        // Watching starts with the first alarm set, not with the first one
        // that goes off: else that one would be "from before" and shown
        // never.
        let shown = match shown_until(&cache)? {
            Some(shown) => shown,
            None => {
                cache.set_meta(SHOWN, Some(&now.to_string()))?;
                now
            }
        };
        let after = window_start(Some(shown), now);
        let next = reminders::next(
            &objects.snapshot(),
            &objects.reminders,
            &objects.habit_entries,
            &settings,
            after,
            AHEAD_DAYS.days(),
        )
        .map(|a| a.at);
        let appointment = if appointments_on(&cache)? {
            let until = after
                .checked_add((AHEAD_DAYS * 24).hours())
                .unwrap_or(Timestamp::MAX);
            reminders::appointments::due(
                &objects.appointments,
                &objects.calendars,
                &settings,
                after,
                until,
            )
            .first()
            .map(|a| a.at)
        } else {
            None
        };
        let snoozed = cache
            .snoozes()?
            .into_iter()
            .filter_map(|(_, until)| until.parse::<Timestamp>().ok())
            .min();
        Ok([next, appointment, snoozed]
            .into_iter()
            .flatten()
            .min()
            .map(|t| t.as_millisecond()))
    }

    /// What to show now: every reminder since the last call (at most a day
    /// back) and every snooze that is over. Shown is shown: the next call
    /// starts here.
    pub fn fire(&self, now_ms: i64) -> Result<Vec<Notice>, MobileError> {
        let now = timestamp(now_ms)?;
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, _) = Self::view(&cache)?;
        let after = window_start(shown_until(&cache)?, now);
        let mut alarms: Vec<Alarm> = reminders::due(
            &objects.snapshot(),
            &objects.reminders,
            &objects.habit_entries,
            &settings,
            after,
            now,
        );
        let mut later: Vec<Notice> = Vec::new();
        if appointments_on(&cache)? {
            for alert in reminders::appointments::due(
                &objects.appointments,
                &objects.calendars,
                &settings,
                after,
                now,
            ) {
                if let Some(notice) = appointment_notice(&objects, &settings, &alert) {
                    later.push(notice);
                }
            }
        }
        // A snooze shows its owner again, if it is still open; that is
        // what `due` decides over all time. An appointment's shows again
        // while the occurrence is still there.
        for (owner, until) in cache.snoozes()? {
            let Ok(until) = until.parse::<Timestamp>() else {
                cache.unsnooze(&owner)?;
                continue;
            };
            if until > now {
                continue;
            }
            cache.unsnooze(&owner)?;
            if let Some((id, start)) = parse_key(&owner) {
                if let Some(instance) = occurrence(&objects, &settings, id, start) {
                    later.push(Notice {
                        key: owner.clone(),
                        owner_id: id.to_string(),
                        title: instance.title.clone(),
                        at_ms: until.as_millisecond(),
                        is_task: false,
                        appointment: Some(appointment_row(&instance, &objects, &settings.zone)),
                    });
                }
                continue;
            }
            let Ok(id) = owner.parse::<Id>() else {
                continue;
            };
            if let Some(title) = open_title(&objects, &settings, id) {
                alarms.push(Alarm {
                    owner: id,
                    at: until,
                    title,
                });
            }
        }
        cache.set_meta(SHOWN, Some(&now.to_string()))?;
        // One notice per owner: the latest reminder stands for the rest.
        alarms.sort_by_key(|a| (a.owner, a.at));
        alarms.dedup_by(|later, earlier| {
            if later.owner == earlier.owner {
                earlier.at = later.at;
                true
            } else {
                false
            }
        });
        let mut notices: Vec<Notice> = alarms
            .into_iter()
            .map(|a| Notice {
                key: a.owner.to_string(),
                owner_id: a.owner.to_string(),
                title: a.title,
                at_ms: a.at.as_millisecond(),
                is_task: objects.tasks.iter().any(|t| t.id == a.owner),
                appointment: None,
            })
            .collect();
        // One notice per occurrence, too: the latest alarm stands.
        later.sort_by(|x, y| x.key.cmp(&y.key).then(x.at_ms.cmp(&y.at_ms)));
        later.dedup_by(|later, earlier| {
            if later.key == earlier.key {
                earlier.at_ms = later.at_ms;
                true
            } else {
                false
            }
        });
        notices.extend(later);
        notices.sort_by(|x, y| x.at_ms.cmp(&y.at_ms).then_with(|| x.key.cmp(&y.key)));
        Ok(notices)
    }

    /// Whether this phone reminds of appointments (ADR 0101 §6).
    pub fn appointment_reminders(&self) -> Result<bool, MobileError> {
        appointments_on(&*self.cache()?)
    }

    /// Switches reminders of appointments on or off, on this phone only.
    /// Switched on, what went off before is not shown afterwards.
    pub fn set_appointment_reminders(&self, on: bool, now_ms: i64) -> Result<(), MobileError> {
        let now = timestamp(now_ms)?;
        let cache = self.cache()?;
        if on && !appointments_on(&cache)? {
            let shown = shown_until(&cache)?.map_or(now, |s| s.max(now));
            cache.set_meta(SHOWN, Some(&shown.to_string()))?;
        }
        cache.set_meta(APPOINTMENTS, on.then_some("1"))?;
        Ok(())
    }

    /// One occurrence of an appointment by the key of its notice, for
    /// its sheet; none when it is gone, cancelled or its calendar off.
    pub fn appointment(&self, key: String) -> Result<Option<AppointmentRow>, MobileError> {
        let (id, start) =
            parse_key(&key).ok_or_else(|| input(format!("not an occurrence: {key}")))?;
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, _) = Self::view(&cache)?;
        Ok(occurrence(&objects, &settings, id, start)
            .map(|instance| appointment_row(&instance, &objects, &settings.zone)))
    }

    /// "Later": the reminder again in `minutes`; `owner` is the key of
    /// the notice.
    pub fn snooze(&self, owner: String, minutes: i64, now_ms: i64) -> Result<(), MobileError> {
        if parse_key(&owner).is_none() {
            parse_id(&owner)?;
        }
        let until = timestamp(now_ms)?
            .checked_add(minutes.minutes())
            .map_err(|e| input(e.to_string()))?;
        self.cache()?.snooze(&owner, &until.to_string())?;
        Ok(())
    }

    /// "Tomorrow": the reminder again tomorrow at the account's reminder
    /// time.
    pub fn snooze_tomorrow(&self, owner: String, now_ms: i64) -> Result<(), MobileError> {
        parse_id(&owner)?;
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let tomorrow = timestamp(now_ms)?
            .to_zoned(settings.zone.clone())
            .date()
            .tomorrow()
            .map_err(|e| input(e.to_string()))?;
        let until = tomorrow
            .to_datetime(settings.remind_time)
            .to_zoned(settings.zone.clone())
            .map_err(|e| input(e.to_string()))?
            .timestamp();
        cache.snooze(&owner, &until.to_string())?;
        Ok(())
    }

    /// The reminders set on a task or project, earliest first.
    pub fn reminders(&self, owner: String) -> Result<Vec<ReminderRow>, MobileError> {
        let owner = parse_id(&owner)?;
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, _) = Self::view(&cache)?;
        let schedule = schedule_of(&objects, owner);
        let now = Timestamp::now();
        let week = now
            .to_zoned(settings.zone.clone())
            .checked_add(7.days())
            .map_or(Timestamp::MAX, |z| z.timestamp());
        let snapshot = objects.snapshot();
        let mut rows: Vec<ReminderRow> = objects
            .reminders
            .iter()
            .filter(|r| r.owner == owner)
            .map(|r| ReminderRow {
                id: r.id.to_string(),
                anchor: match r.anchor {
                    ReminderAnchor::At => Anchor::At,
                    ReminderAnchor::Due => Anchor::Due,
                    ReminderAnchor::Planned => Anchor::Planned,
                    ReminderAnchor::Defer => Anchor::Defer,
                    ReminderAnchor::Daily => Anchor::Daily,
                    ReminderAnchor::Unknown => Anchor::Unknown,
                },
                at_ms: r.at.map(|t| t.as_millisecond()),
                before_minutes: r.before.map(|p| minutes_of(p.0)),
                goes_off_ms: if r.anchor == ReminderAnchor::Daily {
                    reminders::due(
                        &snapshot,
                        std::slice::from_ref(r),
                        &objects.habit_entries,
                        &settings,
                        now,
                        week,
                    )
                    .first()
                    .map(|a| a.at)
                } else {
                    schedule.and_then(|s| reminders::goes_off(r, s, &settings))
                }
                .map(|t| t.as_millisecond()),
                time_minutes: r
                    .time
                    .map(|t| i64::from(t.hour()) * 60 + i64::from(t.minute())),
            })
            .collect();
        rows.sort_by_key(|r| (r.goes_off_ms.is_none(), r.goes_off_ms, r.id.clone()));
        Ok(rows)
    }

    /// A new reminder, through the outbox: at `at_ms` for `Anchor::At`,
    /// else `before_minutes` before the date (none = at it).
    pub fn add_reminder(
        &self,
        owner: String,
        anchor: Anchor,
        at_ms: Option<i64>,
        before_minutes: Option<i64>,
    ) -> Result<String, MobileError> {
        let owner = parse_id(&owner)?;
        let (anchor, at) = match anchor {
            Anchor::At => (
                ReminderAnchor::At,
                Some(timestamp(at_ms.ok_or_else(|| {
                    input("a reminder at an instant needs it")
                })?)?),
            ),
            Anchor::Due => (ReminderAnchor::Due, None),
            Anchor::Planned => (ReminderAnchor::Planned, None),
            Anchor::Defer => (ReminderAnchor::Defer, None),
            Anchor::Daily => {
                return Err(input("a daily reminder is made with add_daily_reminder"));
            }
            Anchor::Unknown => return Err(input("not a kind of reminder")),
        };
        if before_minutes.is_some_and(|m| m < 0) {
            return Err(input("a reminder comes before, not after"));
        }
        let now = Timestamp::now();
        let reminder = Reminder {
            id: Id::new(),
            created: now,
            modified: now,
            owner,
            anchor,
            at,
            before: before_minutes
                .filter(|m| *m > 0)
                .map(|m| Period(span_of(m))),
            time: None,
        };
        self.queue_reminder(owner, reminder)
    }

    /// A reminder every day at `minutes` after midnight, in the account's
    /// zone (ADR 0093), through the outbox.
    pub fn add_daily_reminder(&self, owner: String, minutes: i64) -> Result<String, MobileError> {
        let owner = parse_id(&owner)?;
        if !(0..24 * 60).contains(&minutes) {
            return Err(input("a time of day is between 00:00 and 23:59"));
        }
        let time = jiff::civil::Time::new((minutes / 60) as i8, (minutes % 60) as i8, 0, 0)
            .map_err(|e| input(e.to_string()))?;
        let now = Timestamp::now();
        let reminder = Reminder {
            id: Id::new(),
            created: now,
            modified: now,
            owner,
            anchor: ReminderAnchor::Daily,
            at: None,
            before: None,
            time: Some(time),
        };
        self.queue_reminder(owner, reminder)
    }
}

impl Samu {
    fn queue_reminder(&self, owner: Id, reminder: Reminder) -> Result<String, MobileError> {
        let id = reminder.id;
        let value = serde_json::to_value(&reminder).map_err(|e| input(e.to_string()))?;
        self.queue(|objects, stamp| {
            if schedule_of(objects, owner).is_none() {
                return Err(input(format!("no task or project {owner}")));
            }
            Ok(vec![Change::Create {
                object: id,
                kind: ObjectKind::Reminder,
                value,
                hlc: stamp(),
            }])
        })?;
        Ok(id.to_string())
    }
}

#[uniffi::export]
impl Samu {
    /// A reminder gone, through the outbox.
    pub fn delete_reminder(&self, id: String) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        self.queue(|objects, stamp| {
            if !objects.reminders.iter().any(|r| r.id == id) {
                return Err(input(format!("no reminder {id}")));
            }
            Ok(vec![Change::Delete {
                object: id,
                hlc: stamp(),
            }])
        })
    }
}

fn appointments_on(cache: &Cache) -> Result<bool, MobileError> {
    Ok(cache.meta(APPOINTMENTS)?.as_deref() == Some("1"))
}

/// An appointment's alarm in the sheet's terms; none for one this samu
/// cannot place (it reminds of nothing either).
pub(crate) fn alarm_row(alarm: &AppointmentAlarm) -> Option<AlarmRow> {
    if alarm.related == AlarmRelated::Unknown {
        return None;
    }
    let before_minutes = match alarm.before {
        Some(Period(span)) => Some(
            span.to_duration(SpanRelativeTo::days_are_24_hours())
                .ok()?
                .as_secs()
                / 60,
        ),
        None => None,
    };
    let at_ms = alarm.at.map(|t| t.as_millisecond());
    if before_minutes.is_none() && at_ms.is_none() {
        return None;
    }
    Some(AlarmRow {
        before_minutes,
        end: alarm.related == AlarmRelated::End,
        at_ms,
    })
}

/// Where an occurrence starts: its instant, or midnight of its day in the
/// account's zone.
fn start_of(when: &When, zone: &TimeZone) -> Option<Timestamp> {
    match when {
        When::At(at) => Some(at.timestamp()),
        When::Date(date) => date.to_zoned(zone.clone()).ok().map(|z| z.timestamp()),
        When::Unknown => None,
    }
}

/// The key of one occurrence: `<appointment id>@<start in ms>`.
fn key_of(appointment: Id, start: Timestamp) -> String {
    format!("{appointment}@{}", start.as_millisecond())
}

fn parse_key(key: &str) -> Option<(Id, Timestamp)> {
    let (id, ms) = key.split_once('@')?;
    Some((
        id.parse().ok()?,
        Timestamp::from_millisecond(ms.parse().ok()?).ok()?,
    ))
}

/// The occurrence of an appointment that starts at `start`, as
/// `samu_core::calendar` unfolds it; none when it is gone, cancelled or
/// its calendar is off.
fn occurrence(
    objects: &Objects,
    settings: &Settings,
    appointment: Id,
    start: Timestamp,
) -> Option<AppointmentInstance> {
    let zone = &settings.zone;
    let day = start.to_zoned(zone.clone()).date();
    let from = day.yesterday().unwrap_or(Date::MIN);
    let to = day.tomorrow().unwrap_or(Date::MAX);
    samu_core::calendar::occurrences(
        &objects.appointments,
        &objects.calendars,
        &[],
        from,
        to,
        zone,
    )
    .into_iter()
    .find(|o| o.appointment == appointment && start_of(&o.start, zone) == Some(start))
}

/// The notice of an appointment's alarm.
fn appointment_notice(objects: &Objects, settings: &Settings, alert: &Alert) -> Option<Notice> {
    let id = alert.appointment?;
    let start = start_of(alert.start.as_ref()?, &settings.zone)?;
    let instance = occurrence(objects, settings, id, start)?;
    Some(Notice {
        key: key_of(id, start),
        owner_id: id.to_string(),
        title: alert.title.clone(),
        at_ms: alert.at.as_millisecond(),
        is_task: false,
        appointment: Some(appointment_row(&instance, objects, &settings.zone)),
    })
}

/// From where to look: where the last showing stopped, but at most a day
/// back; before any alarm was set, from now.
fn window_start(shown: Option<Timestamp>, now: Timestamp) -> Timestamp {
    let earliest = now.checked_sub(CATCH_UP_HOURS.hours()).unwrap_or(now);
    match shown {
        Some(shown) => shown.max(earliest),
        None => now,
    }
}

fn schedule_of(objects: &Objects, owner: Id) -> Option<&Schedule> {
    objects
        .tasks
        .iter()
        .find(|t| t.id == owner)
        .map(|t| &t.schedule)
        .or_else(|| {
            objects
                .projects
                .iter()
                .find(|p| p.id == owner)
                .map(|p| &p.schedule)
        })
}

/// The title of an open task or project; none when it is closed or gone.
/// Open is what `samu_core::reminders` says: a reminder at the owner's own
/// instant, over all time, would go off.
fn open_title(objects: &Objects, settings: &Settings, owner: Id) -> Option<String> {
    let probe = Reminder {
        id: Id::new(),
        created: Timestamp::UNIX_EPOCH,
        modified: Timestamp::UNIX_EPOCH,
        owner,
        anchor: ReminderAnchor::At,
        at: Some(Timestamp::UNIX_EPOCH),
        before: None,
        time: None,
    };
    reminders::due(
        &objects.snapshot(),
        &[probe],
        &[],
        settings,
        Timestamp::MIN,
        Timestamp::MAX,
    )
    .into_iter()
    .find(|a| a.owner == owner)
    .map(|a| a.title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_occurrence_key_reads_back() {
        let id = Id::new();
        let start: Timestamp = "2026-11-20T08:00:00Z"
            .parse()
            .unwrap_or(Timestamp::UNIX_EPOCH);
        assert_eq!(parse_key(&key_of(id, start)), Some((id, start)));
        assert_eq!(parse_key(&id.to_string()), None);
        assert_eq!(parse_key("x@1"), None);
    }

    #[test]
    fn alarms_read_as_minutes_from_start_or_end() {
        let before = |s: &str, related| AppointmentAlarm {
            before: Some(Period(s.parse().unwrap_or_default())),
            related,
            at: None,
        };
        let row = |s, related| alarm_row(&before(s, related));
        assert_eq!(
            row("PT15M", AlarmRelated::Start),
            Some(AlarmRow {
                before_minutes: Some(15),
                end: false,
                at_ms: None
            })
        );
        assert_eq!(
            row("-PT5M", AlarmRelated::End),
            Some(AlarmRow {
                before_minutes: Some(-5),
                end: true,
                at_ms: None
            })
        );
        assert_eq!(
            row("P1D", AlarmRelated::Start).and_then(|r| r.before_minutes),
            Some(1440)
        );
        assert_eq!(row("PT15M", AlarmRelated::Unknown), None);
    }

    #[test]
    fn minutes_read_back_as_written() {
        for minutes in [5, 30, 60, 120, 90, 24 * 60, 3 * 24 * 60] {
            assert_eq!(minutes_of(span_of(minutes)), minutes);
        }
        assert_eq!(span_of(120).get_hours(), 2);
        assert_eq!(span_of(2 * 24 * 60).get_days(), 2);
    }

    #[test]
    fn the_window_starts_at_most_a_day_back_and_first_at_now() {
        let now: Timestamp = "2026-11-20T12:00:00Z"
            .parse()
            .unwrap_or(Timestamp::UNIX_EPOCH);
        assert_eq!(window_start(None, now), now);
        let hour_ago = now.checked_sub(1.hour()).unwrap_or(now);
        assert_eq!(window_start(Some(hour_ago), now), hour_ago);
        let week_ago = now.checked_sub(168.hours()).unwrap_or(now);
        assert_eq!(
            window_start(Some(week_ago), now),
            now.checked_sub(24.hours()).unwrap_or(now)
        );
    }
}
