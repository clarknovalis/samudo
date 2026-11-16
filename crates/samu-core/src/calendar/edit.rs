// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! An edit of an appointment turned into changes (ADR 0068 §4), and the
//! rule that settles a write that crossed one from elsewhere (§7).
//!
//! Every client calls the same function: the server for
//! `POST /appointments/{id}/edit`, the replica offline, the app through
//! samu-mobile. What comes out are ordinary changes for `/changes`
//! (§3), still without their stamps.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Span, Timestamp, ToSpan};
use samu_api::{
    APPOINTMENT_FIELDS, Appointment, AppointmentEdit, AppointmentPatch, Calendar, CalendarKind,
    Change, EditRange, Hlc, Id, ObjectKind, When,
};
use serde_json::{Map, Value};

use super::{Window, key_of, shifted_end, starts};

/// A change still without its stamp.
#[derive(Debug, Clone, PartialEq)]
pub enum Draft {
    /// A new appointment, its whole value.
    Create {
        object: Id,
        value: Value,
    },
    Set {
        object: Id,
        field: String,
        value: Value,
    },
    Delete {
        object: Id,
    },
}

impl Draft {
    #[must_use]
    pub const fn object(&self) -> Id {
        match self {
            Self::Create { object, .. } | Self::Set { object, .. } | Self::Delete { object } => {
                *object
            }
        }
    }

    /// The change with its stamp.
    #[must_use]
    pub fn stamped(self, hlc: Hlc) -> Change {
        match self {
            Self::Create { object, value } => Change::Create {
                object,
                kind: ObjectKind::Appointment,
                value,
                hlc,
            },
            Self::Set {
                object,
                field,
                value,
            } => Change::Set {
                object,
                field,
                value,
                note: None,
                hlc,
            },
            Self::Delete { object } => Change::Delete { object, hlc },
        }
    }
}

/// Why an edit makes no changes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("no appointment {0}")]
    NoAppointment(Id),
    #[error("read-only: {0}")]
    ReadOnly(String),
    #[error("no occurrence of the appointment starts then")]
    NoInstance,
    #[error("{0}")]
    Invalid(String),
}

/// Whether samu may change appointments of this calendar (ADR 0068 §1).
#[must_use]
pub fn writable(calendar: &Calendar) -> bool {
    calendar.writable && calendar.kind == CalendarKind::Caldav
}

/// The changes an edit of appointment `target` comes to (ADR 0068 §4).
/// `new_id` gives the ids of appointments the edit makes; a new series
/// takes its id as its UID. `now` is their `created`.
pub fn edit_appointment(
    appointments: &[Appointment],
    calendars: &[Calendar],
    target: Id,
    edit: &AppointmentEdit,
    zone: &TimeZone,
    now: Timestamp,
    mut new_id: impl FnMut() -> Id,
) -> Result<Vec<Draft>, EditError> {
    let a = appointments
        .iter()
        .find(|a| a.id == target)
        .ok_or(EditError::NoAppointment(target))?;
    check_writable(a, calendars)?;
    match (&edit.patch, edit.delete) {
        (None, false) => return Err(EditError::Invalid("an edit needs a patch or delete".into())),
        (Some(_), true) => {
            return Err(EditError::Invalid(
                "an edit is a patch or a delete, not both".into(),
            ));
        }
        _ => {}
    }
    if let Some(patch) = &edit.patch {
        check_patch(patch, calendars)?;
    }
    let main = match &a.recurrence_id {
        None => Some(a),
        Some(_) => appointments
            .iter()
            .find(|m| m.calendar == a.calendar && m.uid == a.uid && m.recurrence_id.is_none()),
    };
    let Some(main) = main.filter(|m| m.rrule.is_some() || !m.rdate.is_empty()) else {
        // Not a repetition, or an instance whose series samu does not have.
        return plain(a, edit);
    };
    let overrides: Vec<&Appointment> = appointments
        .iter()
        .filter(|o| o.calendar == main.calendar && o.uid == main.uid && o.recurrence_id.is_some())
        .collect();
    let instance = a.recurrence_id.clone().or_else(|| edit.instance.clone());
    match (edit.range, instance) {
        (EditRange::Unknown, _) => {
            Err(EditError::Invalid("a range this samu does not know".into()))
        }
        (EditRange::All, _) | (_, None) => all(main, &overrides, edit),
        (EditRange::This, Some(instance)) => {
            this(main, &overrides, &instance, edit, zone, now, new_id())
        }
        (EditRange::Following, Some(instance)) if key_of(&instance) == key_of(&main.start) => {
            all(main, &overrides, edit)
        }
        (EditRange::Following, Some(instance)) => {
            following(main, &overrides, &instance, edit, zone, now, &mut new_id)
        }
    }
}

fn check_writable(a: &Appointment, calendars: &[Calendar]) -> Result<(), EditError> {
    let calendar = calendars
        .iter()
        .find(|c| c.id == a.calendar)
        .ok_or_else(|| EditError::ReadOnly(format!("no calendar {}", a.calendar)))?;
    if !writable(calendar) {
        return Err(EditError::ReadOnly(format!(
            "{} is not writable",
            calendar.title
        )));
    }
    if a.meeting {
        return Err(EditError::ReadOnly(
            "an invitation; change it in its calendar".into(),
        ));
    }
    Ok(())
}

fn check_patch(patch: &AppointmentPatch, calendars: &[Calendar]) -> Result<(), EditError> {
    if let Some(id) = patch.calendar {
        let ok = calendars.iter().any(|c| c.id == id && writable(c));
        if !ok {
            return Err(EditError::ReadOnly(format!(
                "calendar {id} is not writable"
            )));
        }
    }
    if let Some(bad) = patch
        .clear
        .iter()
        .find(|f| !matches!(f.as_str(), "end" | "rrule" | "location" | "note"))
    {
        return Err(EditError::Invalid(format!("{bad} cannot be cleared")));
    }
    Ok(())
}

/// An appointment that does not repeat, or one instance alone.
fn plain(a: &Appointment, edit: &AppointmentEdit) -> Result<Vec<Draft>, EditError> {
    if edit.delete {
        return Ok(vec![Draft::Delete { object: a.id }]);
    }
    let patch = edit.patch.as_ref().ok_or(EditError::NoInstance)?;
    if a.recurrence_id.is_some() && (patch.calendar.is_some() || patch.rrule.is_some()) {
        return Err(EditError::Invalid(
            "an instance keeps its calendar and has no rule".into(),
        ));
    }
    let new = apply(patch, a)?;
    sets(a, &new)
}

/// The whole series: the main one and its changed instances.
fn all(
    main: &Appointment,
    overrides: &[&Appointment],
    edit: &AppointmentEdit,
) -> Result<Vec<Draft>, EditError> {
    if edit.delete {
        let mut out = vec![Draft::Delete { object: main.id }];
        out.extend(overrides.iter().map(|o| Draft::Delete { object: o.id }));
        return Ok(out);
    }
    let patch = edit.patch.as_ref().ok_or(EditError::NoInstance)?;
    let (new_main, new_overrides) = series(main, overrides, patch)?;
    let mut out = sets(main, &new_main)?;
    for (old, new) in overrides.iter().zip(new_overrides) {
        out.extend(replaced(old, new)?);
    }
    Ok(out)
}

/// One occurrence: its changed instance, made when there is none yet.
fn this(
    main: &Appointment,
    overrides: &[&Appointment],
    instance: &When,
    edit: &AppointmentEdit,
    zone: &TimeZone,
    now: Timestamp,
    id: Id,
) -> Result<Vec<Draft>, EditError> {
    let key = key_of(instance);
    let existing = overrides
        .iter()
        .find(|o| o.recurrence_id.as_ref().map(key_of).as_ref() == Some(&key));
    if let Some(patch) = &edit.patch
        && (patch.calendar.is_some() || patch.rrule.is_some())
    {
        return Err(EditError::Invalid(
            "an instance keeps its calendar and has no rule".into(),
        ));
    }
    match (existing, &edit.patch) {
        (Some(o), _) if edit.delete => {
            let mut out = vec![Draft::Delete { object: o.id }];
            out.extend(excluded(
                main,
                o.recurrence_id.as_ref().unwrap_or(instance),
            )?);
            Ok(out)
        }
        (Some(o), Some(patch)) => sets(o, &apply(patch, o)?),
        (None, patch) => {
            if !is_occurrence(main, instance, zone) {
                return Err(EditError::NoInstance);
            }
            match patch {
                None => excluded(main, instance),
                Some(patch) => {
                    let base = Appointment {
                        id,
                        created: now,
                        modified: now,
                        recurrence_id: Some(instance.clone()),
                        start: instance.clone(),
                        end: shifted_end(main, instance),
                        rrule: None,
                        rdate: Vec::new(),
                        exdate: Vec::new(),
                        ..main.clone()
                    };
                    Ok(vec![create(&apply(patch, &base)?)?])
                }
            }
        }
        (Some(_), None) => Err(EditError::Invalid("an edit needs a patch or delete".into())),
    }
}

/// The series up to the occurrence before `instance` stays; from there a
/// new series with a new UID takes the rest, the patch laid over it.
fn following(
    main: &Appointment,
    overrides: &[&Appointment],
    instance: &When,
    edit: &AppointmentEdit,
    zone: &TimeZone,
    now: Timestamp,
    new_id: &mut impl FnMut() -> Id,
) -> Result<Vec<Draft>, EditError> {
    if !is_occurrence(main, instance, zone) {
        return Err(EditError::NoInstance);
    }
    let at = instant(instance, zone);
    let later = |w: &When| instant(w, zone) >= at;
    let (old_rule, new_rule) = match &main.rrule {
        None => (None, None),
        Some(rule) => match count_of(rule) {
            Some(count) => {
                let before = u32::try_from(occurrences_before(main, instance, zone))
                    .map_err(|_| EditError::Invalid("too many occurrences".into()))?;
                (
                    Some(with_part(rule, "COUNT", &before.to_string())),
                    Some(with_part(
                        rule,
                        "COUNT",
                        &count.saturating_sub(before).to_string(),
                    )),
                )
            }
            None => (
                Some(with_part(
                    &without_part(rule, "COUNT"),
                    "UNTIL",
                    &until_before(instance)?,
                )),
                Some(rule.clone()),
            ),
        },
    };
    let mut shortened = main.clone();
    shortened.rrule = old_rule;
    shortened.rdate.retain(|w| !later(w));
    shortened.exdate.retain(|w| !later(w));
    let mut out = sets(main, &shortened)?;
    let moving: Vec<&&Appointment> = overrides
        .iter()
        .filter(|o| o.recurrence_id.as_ref().is_some_and(later))
        .collect();
    out.extend(moving.iter().map(|o| Draft::Delete { object: o.id }));
    if edit.delete {
        return Ok(out);
    }
    let patch = edit.patch.as_ref().ok_or(EditError::NoInstance)?;
    let id = new_id();
    let rest = Appointment {
        id,
        created: now,
        modified: now,
        uid: id.to_string(),
        recurrence_id: None,
        start: instance.clone(),
        end: shifted_end(main, instance),
        rrule: new_rule,
        rdate: main.rdate.iter().filter(|w| later(w)).cloned().collect(),
        exdate: main.exdate.iter().filter(|w| later(w)).cloned().collect(),
        ..main.clone()
    };
    let rest_overrides: Vec<Appointment> = moving
        .iter()
        .map(|o| Appointment {
            id: new_id(),
            created: now,
            modified: now,
            uid: rest.uid.clone(),
            ..(**o).clone()
        })
        .collect();
    let refs: Vec<&Appointment> = rest_overrides.iter().collect();
    let (new_rest, new_overrides) = series(&rest, &refs, patch)?;
    out.push(create(&new_rest)?);
    for o in new_overrides.into_iter().flatten() {
        out.push(create(&o)?);
    }
    Ok(out)
}

/// A series with a patch laid over it: the main one changed, and each
/// changed instance with it. A moved start moves EXDATE, RDATE and the
/// instances by as much; an instance at its own time moves along, one moved
/// elsewhere keeps its time. A field an instance had like the series takes
/// the series' new value. A start that turns from a date into a time or
/// back leaves no instance and no EXDATE behind.
fn series(
    main: &Appointment,
    overrides: &[&Appointment],
    patch: &AppointmentPatch,
) -> Result<(Appointment, Vec<Option<Appointment>>), EditError> {
    let mut new_main = apply(patch, main)?;
    let shift = Shift::between(&main.start, &new_main.start);
    match &shift {
        Some(shift) => {
            new_main.exdate = new_main
                .exdate
                .iter()
                .filter_map(|w| shift.apply(w))
                .collect();
            new_main.rdate = new_main
                .rdate
                .iter()
                .filter_map(|w| shift.apply(w))
                .collect();
        }
        None => {
            new_main.exdate.clear();
            new_main.rdate.clear();
        }
    }
    let mut new_overrides = Vec::new();
    for o in overrides {
        // An instance with no place in the new series goes.
        let Some(shift) = &shift else {
            new_overrides.push(None);
            continue;
        };
        let Some(rid) = o.recurrence_id.as_ref().and_then(|r| shift.apply(r)) else {
            new_overrides.push(None);
            continue;
        };
        let mut n = (*o).clone();
        let at_own_time = o
            .recurrence_id
            .as_ref()
            .is_some_and(|r| same_when(r, &o.start));
        n.recurrence_id = Some(rid);
        if at_own_time {
            if let Some(start) = shift.apply(&o.start) {
                n.start = start;
            }
            n.end = o.end.as_ref().and_then(|e| shift.apply(e));
        }
        n.calendar = new_main.calendar;
        for field in ["title", "location", "note", "busy"] {
            let was = field_of(main, field)?;
            if field_of(o, field)? == was {
                set_field(&mut n, field, field_of(&new_main, field)?)?;
            }
        }
        new_overrides.push(Some(n));
    }
    Ok((new_main, new_overrides))
}

/// The changes from `old` to `new` of one instance: `set`s while its
/// RECURRENCE-ID is the same, else delete and make anew.
fn replaced(old: &Appointment, new: Option<Appointment>) -> Result<Vec<Draft>, EditError> {
    let Some(new) = new else {
        return Ok(vec![Draft::Delete { object: old.id }]);
    };
    let same_rid = match (&old.recurrence_id, &new.recurrence_id) {
        (Some(a), Some(b)) => key_of(a) == key_of(b),
        _ => false,
    };
    if same_rid {
        return sets(old, &new);
    }
    Ok(vec![Draft::Delete { object: old.id }, create(&new)?])
}

/// The patch laid over an appointment. A new start without a new end keeps
/// the length.
fn apply(patch: &AppointmentPatch, base: &Appointment) -> Result<Appointment, EditError> {
    let mut a = base.clone();
    if let Some(title) = &patch.title {
        a.title.clone_from(title);
    }
    if let Some(start) = &patch.start {
        a.start = start.clone();
        if patch.end.is_none() && !patch.clear.iter().any(|f| f == "end") {
            a.end = shifted_end(base, start);
        }
    }
    if let Some(end) = &patch.end {
        a.end = Some(end.clone());
    }
    if let Some(location) = &patch.location {
        a.location = Some(location.clone()).filter(|l| !l.is_empty());
    }
    if let Some(note) = &patch.note {
        a.note = Some(note.clone()).filter(|n| !n.is_empty());
    }
    if let Some(busy) = patch.busy {
        a.busy = busy;
    }
    if let Some(rule) = &patch.rrule {
        a.rrule = Some(rule.clone());
    }
    if let Some(calendar) = patch.calendar {
        a.calendar = calendar;
    }
    for field in &patch.clear {
        match field.as_str() {
            "end" => a.end = None,
            "rrule" => a.rrule = None,
            "location" => a.location = None,
            "note" => a.note = None,
            _ => {}
        }
    }
    check(&a)?;
    Ok(a)
}

/// What an appointment must be to be written.
pub fn check(a: &Appointment) -> Result<(), EditError> {
    match (&a.start, &a.end) {
        (When::Unknown, _) | (_, Some(When::Unknown)) => {
            return Err(EditError::Invalid("a time this samu does not know".into()));
        }
        (When::Date(s), Some(When::Date(e))) if e <= s => {
            return Err(EditError::Invalid(
                "an all-day appointment ends after its first day".into(),
            ));
        }
        (When::At(s), Some(When::At(e))) if e.timestamp() < s.timestamp() => {
            return Err(EditError::Invalid(
                "an appointment cannot end before it starts".into(),
            ));
        }
        (When::Date(_), Some(When::At(_))) | (When::At(_), Some(When::Date(_))) => {
            return Err(EditError::Invalid(
                "start and end are both dates or both times".into(),
            ));
        }
        _ => {}
    }
    if let Some(rule) = &a.rrule
        && !super::rule_reads(rule, &a.start)
    {
        return Err(EditError::Invalid(format!(
            "not a rule samu can compute: {rule}"
        )));
    }
    Ok(())
}

/// The `set`s that make `old` into `new`, one per field that differs.
fn sets(old: &Appointment, new: &Appointment) -> Result<Vec<Draft>, EditError> {
    let (was, will) = (json_of(old)?, json_of(new)?);
    let mut drafts: Vec<Draft> = APPOINTMENT_FIELDS
        .iter()
        .filter(|f| was.get(**f) != will.get(**f))
        .map(|f| Draft::Set {
            object: old.id,
            field: (*f).to_string(),
            value: will.get(*f).cloned().unwrap_or(Value::Null),
        })
        .collect();
    order_start_and_end(old, new, &mut drafts);
    Ok(drafts)
}

/// The store takes a change only when the appointment after it is one
/// [`check`] takes, set by set. With both `start` and `end` changed the
/// step between must be one too. `start` comes before `end` (in the order
/// of [`APPOINTMENT_FIELDS`]) while the new start fits the old end; else
/// `end` goes first. From whole days to times or back neither fits, so
/// `end` is cleared before `start` and set after it.
fn order_start_and_end(old: &Appointment, new: &Appointment, drafts: &mut Vec<Draft>) {
    let at = |field: &str, drafts: &[Draft]| {
        drafts
            .iter()
            .position(|d| matches!(d, Draft::Set { field: f, .. } if f == field))
    };
    let (Some(start), Some(end)) = (at("start", drafts), at("end", drafts)) else {
        return;
    };
    let start_first = Appointment {
        start: new.start.clone(),
        ..old.clone()
    };
    if check(&start_first).is_ok() {
        return;
    }
    let end_first = Appointment {
        end: new.end.clone(),
        ..old.clone()
    };
    if check(&end_first).is_ok() {
        let draft = drafts.remove(end);
        drafts.insert(start, draft);
        return;
    }
    drafts.insert(
        start,
        Draft::Set {
            object: old.id,
            field: "end".into(),
            value: Value::Null,
        },
    );
}

fn create(a: &Appointment) -> Result<Draft, EditError> {
    Ok(Draft::Create {
        object: a.id,
        value: Value::Object(json_of(a)?),
    })
}

/// `instance` added to the series' EXDATE.
fn excluded(main: &Appointment, instance: &When) -> Result<Vec<Draft>, EditError> {
    let mut new = main.clone();
    if !new.exdate.iter().any(|w| key_of(w) == key_of(instance)) {
        new.exdate.push(instance.clone());
    }
    sets(main, &new)
}

fn json_of(a: &Appointment) -> Result<Map<String, Value>, EditError> {
    match serde_json::to_value(a) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(EditError::Invalid("an appointment is no object".into())),
        Err(e) => Err(EditError::Invalid(e.to_string())),
    }
}

fn field_of(a: &Appointment, field: &str) -> Result<Value, EditError> {
    Ok(json_of(a)?.get(field).cloned().unwrap_or(Value::Null))
}

fn set_field(a: &mut Appointment, field: &str, value: Value) -> Result<(), EditError> {
    let mut map = json_of(a)?;
    if value.is_null() {
        map.remove(field);
    } else {
        map.insert(field.to_string(), value);
    }
    *a = serde_json::from_value(Value::Object(map))
        .map_err(|e| EditError::Invalid(e.to_string()))?;
    Ok(())
}

/// How far a start moved: whole days for dates, a duration for times.
enum Shift {
    Days(Span),
    By(SignedDuration),
}

impl Shift {
    fn between(old: &When, new: &When) -> Option<Self> {
        match (old, new) {
            (When::Date(a), When::Date(b)) => a.until(*b).ok().map(Self::Days),
            (When::At(a), When::At(b)) => Some(Self::By(a.duration_until(b))),
            _ => None,
        }
    }

    fn apply(&self, when: &When) -> Option<When> {
        match (self, when) {
            (Self::Days(days), When::Date(d)) => d.checked_add(*days).ok().map(When::Date),
            (Self::By(by), When::At(at)) => at.checked_add(*by).ok().map(When::At),
            _ => None,
        }
    }
}

fn same_when(a: &When, b: &When) -> bool {
    key_of(a) == key_of(b)
}

/// Where a time sorts: a date at its midnight in `zone`.
fn instant(when: &When, zone: &TimeZone) -> Timestamp {
    match when {
        When::Date(date) => date
            .to_zoned(zone.clone())
            .map_or(Timestamp::MIN, |z| z.timestamp()),
        When::At(at) => at.timestamp(),
        When::Unknown => Timestamp::MIN,
    }
}

fn day_of(when: &When) -> Date {
    match when {
        When::Date(date) => *date,
        When::At(at) => at.date(),
        When::Unknown => Date::MIN,
    }
}

/// The starts of the series from its first day to the day of `instance`.
fn starts_up_to(main: &Appointment, instance: &When, zone: &TimeZone) -> Vec<When> {
    let first = day_of(&main.start).min(day_of(instance));
    let last = day_of(instance).saturating_add(1.day());
    starts(main, &Window::new(first, last, zone))
}

fn is_occurrence(main: &Appointment, instance: &When, zone: &TimeZone) -> bool {
    let key = key_of(instance);
    starts_up_to(main, instance, zone)
        .iter()
        .any(|w| key_of(w) == key)
}

/// How many occurrences of the rule (RDATE not counted, as COUNT does not)
/// start before `instance`.
fn occurrences_before(main: &Appointment, instance: &When, zone: &TimeZone) -> usize {
    let rule_only = Appointment {
        rdate: Vec::new(),
        ..main.clone()
    };
    let at = instant(instance, zone);
    starts_up_to(&rule_only, instance, zone)
        .iter()
        .filter(|w| instant(w, zone) < at)
        .count()
}

fn count_of(rule: &str) -> Option<u32> {
    part(rule, "COUNT").and_then(|v| v.parse().ok())
}

fn part<'a>(rule: &'a str, key: &str) -> Option<&'a str> {
    rule.split(';').find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.eq_ignore_ascii_case(key).then_some(v)
    })
}

fn without_part(rule: &str, key: &str) -> String {
    rule.split(';')
        .filter(|p| {
            !p.split_once('=')
                .is_some_and(|(k, _)| k.eq_ignore_ascii_case(key))
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn with_part(rule: &str, key: &str, value: &str) -> String {
    let mut out = without_part(rule, key);
    if !out.is_empty() {
        out.push(';');
    }
    out.push_str(&format!("{key}={value}"));
    out
}

/// UNTIL just before an occurrence, in the form RFC 5545 wants: the day
/// before for dates, a second before in UTC for times.
fn until_before(instance: &When) -> Result<String, EditError> {
    match instance {
        When::Date(d) => Ok(d
            .yesterday()
            .map_err(|e| EditError::Invalid(e.to_string()))?
            .strftime("%Y%m%d")
            .to_string()),
        When::At(at) => Ok(at
            .timestamp()
            .checked_sub(SignedDuration::from_secs(1))
            .map_err(|e| EditError::Invalid(e.to_string()))?
            .strftime("%Y%m%dT%H%M%SZ")
            .to_string()),
        When::Unknown => Err(EditError::Invalid("a time this samu does not know".into())),
    }
}

/// A field samu changed and has not written yet (ADR 0068 §7): its
/// latest stamp, and its value before samu's first change to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub field: String,
    pub hlc: Hlc,
    pub base: Value,
}

/// The appointment a crossed write leaves (ADR 0068 §7): what came from
/// elsewhere, with each field samu changed and has not written yet decided
/// on its own. Where the other side left the field as samu found it, samu's
/// value holds; where both changed it, the later change wins: samu's when
/// its stamp is after the other's LAST-MODIFIED, or when that is missing.
pub fn merge_remote(
    local: &Appointment,
    pending: &[Pending],
    remote: &Appointment,
    remote_modified: Option<Timestamp>,
) -> Result<Appointment, EditError> {
    let mut out = remote.clone();
    out.id = local.id;
    out.created = local.created;
    for p in pending {
        if !APPOINTMENT_FIELDS.contains(&p.field.as_str()) {
            continue;
        }
        let theirs = field_of(remote, &p.field)?;
        let untouched = theirs == p.base;
        let ours_is_later = remote_modified.is_none_or(|lm| p.hlc.wall > lm.as_millisecond());
        if untouched || ours_is_later {
            set_field(&mut out, &p.field, field_of(local, &p.field)?)?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
