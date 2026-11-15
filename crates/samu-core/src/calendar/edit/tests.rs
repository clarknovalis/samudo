#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use jiff::civil::date;
use samu_api::{AppointmentStatus, CalendarKind};

use super::*;
use crate::calendar::read_ics;

const GOOGLE: &str = include_str!("../../../../../testdata/calendar/google.ics");

fn berlin() -> TimeZone {
    TimeZone::get("Europe/Berlin").unwrap()
}

fn now() -> Timestamp {
    "2026-11-11T09:00:00Z".parse().unwrap()
}

fn id(n: u8) -> Id {
    format!("01936f5e-8a00-7c3e-9b1a-3f2d4c5b70{n:02x}")
        .parse()
        .unwrap()
}

fn at(s: &str) -> When {
    When::At(s.parse().unwrap())
}

fn calendar(writable: bool, kind: CalendarKind) -> Calendar {
    Calendar {
        id: id(1),
        created: now(),
        modified: now(),
        title: "Kalender".into(),
        kind,
        color: None,
        enabled: true,
        refresh: 15,
        link_template: None,
        position: 0,
        fetched_at: None,
        error: None,
        host: None,
        writable,
    }
}

fn calendars() -> Vec<Calendar> {
    vec![calendar(true, CalendarKind::Caldav)]
}

/// The Google file in calendar 1, ids from 0x10 on.
fn stored() -> Vec<Appointment> {
    read_ics(GOOGLE, &berlin())
        .unwrap()
        .appointments
        .into_iter()
        .enumerate()
        .map(|(i, a)| a.into_appointment(id(0x10 + i as u8), id(1), now(), now()))
        .collect()
}

fn by_uid<'a>(all: &'a [Appointment], uid: &str, rid: Option<&When>) -> &'a Appointment {
    all.iter()
        .find(|a| a.uid == uid && a.recurrence_id.as_ref().map(key_of) == rid.map(key_of))
        .unwrap()
}

/// Ids for what an edit makes, from 0xa0 on.
fn ids() -> impl FnMut() -> Id {
    let mut n = 0xa0;
    move || {
        n += 1;
        id(n)
    }
}

fn run(all: &[Appointment], target: Id, edit: &AppointmentEdit) -> Result<Vec<Draft>, EditError> {
    edit_appointment(all, &calendars(), target, edit, &berlin(), now(), ids())
}

fn patch(p: AppointmentPatch) -> AppointmentEdit {
    AppointmentEdit {
        patch: Some(p),
        ..AppointmentEdit::default()
    }
}

fn set_of<'a>(drafts: &'a [Draft], object: Id, field: &str) -> Option<&'a Value> {
    drafts.iter().find_map(|d| match d {
        Draft::Set {
            object: o,
            field: f,
            value,
        } if *o == object && f == field => Some(value),
        _ => None,
    })
}

fn created(drafts: &[Draft]) -> Vec<Appointment> {
    drafts
        .iter()
        .filter_map(|d| match d {
            Draft::Create { value, .. } => Some(serde_json::from_value(value.clone()).unwrap()),
            _ => None,
        })
        .collect()
}

fn deleted(drafts: &[Draft]) -> Vec<Id> {
    drafts
        .iter()
        .filter_map(|d| match d {
            Draft::Delete { object } => Some(*object),
            _ => None,
        })
        .collect()
}

#[test]
fn a_calendar_samu_may_not_write_and_an_invitation_stay_read_only() {
    let mut all = stored();
    let dentist = by_uid(&all, "zahnarzt@example.org", None).id;
    let edit = patch(AppointmentPatch {
        title: Some("x".into()),
        ..AppointmentPatch::default()
    });
    for c in [
        calendar(false, CalendarKind::Caldav),
        calendar(true, CalendarKind::Ics),
    ] {
        let got = edit_appointment(&all, &[c], dentist, &edit, &berlin(), now(), ids());
        assert!(matches!(got, Err(EditError::ReadOnly(_))), "{got:?}");
    }
    all.iter_mut().find(|a| a.id == dentist).unwrap().meeting = true;
    assert!(matches!(
        run(&all, dentist, &edit),
        Err(EditError::ReadOnly(_))
    ));
}

#[test]
fn a_single_appointment_takes_one_set_per_changed_field() {
    let all = stored();
    let dentist = by_uid(&all, "zahnarzt@example.org", None);
    let drafts = run(
        &all,
        dentist.id,
        &patch(AppointmentPatch {
            title: Some("Zahnarzt".into()),
            start: Some(at("2026-11-20T10:00:00+01:00[Europe/Berlin]")),
            clear: vec!["note".into()],
            ..AppointmentPatch::default()
        }),
    )
    .unwrap();
    assert_eq!(drafts.len(), 4, "{drafts:#?}");
    assert_eq!(
        set_of(&drafts, dentist.id, "title"),
        Some(&Value::from("Zahnarzt"))
    );
    // The length stays: 45 minutes.
    assert_eq!(
        set_of(&drafts, dentist.id, "end"),
        Some(&serde_json::to_value(at("2026-11-20T10:45:00+01:00[Europe/Berlin]")).unwrap())
    );
    assert_eq!(set_of(&drafts, dentist.id, "note"), Some(&Value::Null));
    let delete = AppointmentEdit {
        delete: true,
        ..AppointmentEdit::default()
    };
    assert_eq!(
        deleted(&run(&all, dentist.id, &delete).unwrap()),
        vec![dentist.id]
    );
}

#[test]
fn this_one_makes_an_instance_or_an_exdate() {
    let all = stored();
    let choir = by_uid(&all, "chor@example.org", None);
    let instance = at("2026-11-10T19:30:00+01:00[Europe/Berlin]");
    let drafts = run(
        &all,
        choir.id,
        &AppointmentEdit {
            instance: Some(instance.clone()),
            range: EditRange::This,
            patch: Some(AppointmentPatch {
                start: Some(at("2026-11-10T20:00:00+01:00[Europe/Berlin]")),
                ..AppointmentPatch::default()
            }),
            delete: false,
        },
    )
    .unwrap();
    let made = created(&drafts);
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].uid, "chor@example.org");
    assert_eq!(made[0].recurrence_id, Some(instance.clone()));
    assert_eq!(
        made[0].end,
        Some(at("2026-11-10T21:30:00+01:00[Europe/Berlin]"))
    );
    assert_eq!(made[0].rrule, None);

    let drafts = run(
        &all,
        choir.id,
        &AppointmentEdit {
            instance: Some(instance.clone()),
            range: EditRange::This,
            patch: None,
            delete: true,
        },
    )
    .unwrap();
    let exdate: Vec<When> =
        serde_json::from_value(set_of(&drafts, choir.id, "exdate").unwrap().clone()).unwrap();
    assert_eq!(exdate.len(), 2);
    assert!(exdate.contains(&instance));

    // A time the series does not have is no instance.
    let wrong = AppointmentEdit {
        instance: Some(at("2026-11-11T19:30:00+01:00[Europe/Berlin]")),
        range: EditRange::This,
        patch: None,
        delete: true,
    };
    assert_eq!(run(&all, choir.id, &wrong), Err(EditError::NoInstance));
}

#[test]
fn a_changed_instance_is_set_or_deleted_with_an_exdate() {
    let all = stored();
    let rid = at("2026-11-17T19:30:00+01:00[Europe/Berlin]");
    let rehearsal = by_uid(&all, "chor@example.org", Some(&rid));
    let choir = by_uid(&all, "chor@example.org", None);
    let drafts = run(
        &all,
        rehearsal.id,
        &patch(AppointmentPatch {
            title: Some("Generalprobe".into()),
            ..AppointmentPatch::default()
        }),
    )
    .unwrap();
    assert_eq!(drafts.len(), 1);
    assert!(set_of(&drafts, rehearsal.id, "title").is_some());

    let drafts = run(
        &all,
        rehearsal.id,
        &AppointmentEdit {
            delete: true,
            ..AppointmentEdit::default()
        },
    )
    .unwrap();
    assert_eq!(deleted(&drafts), vec![rehearsal.id]);
    let exdate: Vec<When> =
        serde_json::from_value(set_of(&drafts, choir.id, "exdate").unwrap().clone()).unwrap();
    assert!(exdate.iter().any(|w| key_of(w) == key_of(&rid)));
}

#[test]
fn all_moves_the_series_with_its_exdates_and_instances() {
    let all = stored();
    let choir = by_uid(&all, "chor@example.org", None);
    let drafts = run(
        &all,
        choir.id,
        &AppointmentEdit {
            range: EditRange::All,
            patch: Some(AppointmentPatch {
                start: Some(at("2026-10-20T20:00:00+02:00[Europe/Berlin]")),
                title: Some("Chorprobe".into()),
                ..AppointmentPatch::default()
            }),
            ..AppointmentEdit::default()
        },
    )
    .unwrap();
    let exdate: Vec<When> =
        serde_json::from_value(set_of(&drafts, choir.id, "exdate").unwrap().clone()).unwrap();
    assert_eq!(exdate, vec![at("2026-11-24T20:00:00+01:00[Europe/Berlin]")]);
    // Both instances get a new RECURRENCE-ID: deleted and made anew.
    assert_eq!(deleted(&drafts).len(), 2);
    let made = created(&drafts);
    let rehearsal = made
        .iter()
        .find(|a| a.title == "Chor (Generalprobe)")
        .unwrap();
    assert_eq!(
        rehearsal.recurrence_id,
        Some(at("2026-11-17T20:00:00+01:00[Europe/Berlin]"))
    );
    assert_eq!(
        rehearsal.start,
        at("2026-11-18T19:00:00+01:00[Europe/Berlin]"),
        "moved elsewhere, it keeps its time"
    );
    let cancelled = made
        .iter()
        .find(|a| a.status == AppointmentStatus::Cancelled)
        .unwrap();
    assert_eq!(cancelled.title, "Chorprobe", "it was like the series");
    assert_eq!(
        cancelled.start,
        at("2026-12-01T20:00:00+01:00[Europe/Berlin]")
    );
}

#[test]
fn following_ends_the_series_and_starts_a_new_one() {
    let all = stored();
    let choir = by_uid(&all, "chor@example.org", None);
    let instance = at("2026-11-24T19:30:00+01:00[Europe/Berlin]");
    let drafts = run(
        &all,
        choir.id,
        &AppointmentEdit {
            instance: Some(at("2026-11-17T19:30:00+01:00[Europe/Berlin]")),
            range: EditRange::Following,
            patch: Some(AppointmentPatch {
                location: Some("Aula".into()),
                ..AppointmentPatch::default()
            }),
            delete: false,
        },
    );
    // 17 November is a changed instance; following works from the series.
    let drafts = drafts.unwrap();
    let rule = set_of(&drafts, choir.id, "rrule")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(rule.ends_with("UNTIL=20261117T182959Z"), "{rule}");
    let made = created(&drafts);
    let rest = made.iter().find(|a| a.recurrence_id.is_none()).unwrap();
    assert_eq!(rest.uid, rest.id.to_string());
    assert_eq!(rest.start, at("2026-11-17T19:30:00+01:00[Europe/Berlin]"));
    assert_eq!(rest.location.as_deref(), Some("Aula"));
    assert_eq!(rest.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=TU"));
    assert_eq!(rest.exdate, vec![instance], "the EXDATE after goes along");
    // Both changed instances lie on or after 17 November and move.
    assert_eq!(deleted(&drafts).len(), 2);
    assert_eq!(made.iter().filter(|a| a.uid == rest.uid).count(), 3);
    assert_eq!(
        set_of(&drafts, choir.id, "exdate"),
        Some(&Value::Null),
        "the old series keeps no EXDATE after the cut"
    );
}

#[test]
fn following_with_a_count_splits_the_count() {
    let mut all = stored();
    let choir = all
        .iter_mut()
        .find(|a| a.uid == "chor@example.org" && a.recurrence_id.is_none())
        .unwrap();
    choir.rrule = Some("FREQ=WEEKLY;BYDAY=TU;COUNT=5".into());
    choir.exdate.clear();
    let choir = choir.id;
    all.retain(|a| a.uid != "chor@example.org" || a.recurrence_id.is_none());
    let drafts = run(
        &all,
        choir,
        &AppointmentEdit {
            instance: Some(at("2026-11-03T19:30:00+01:00[Europe/Berlin]")),
            range: EditRange::Following,
            patch: None,
            delete: true,
        },
    )
    .unwrap();
    // 20 Oct, 27 Oct stay; 3, 10, 17 Nov go.
    assert_eq!(
        set_of(&drafts, choir, "rrule"),
        Some(&Value::from("FREQ=WEEKLY;BYDAY=TU;COUNT=2"))
    );
    assert!(created(&drafts).is_empty());

    let drafts = run(
        &all,
        choir,
        &AppointmentEdit {
            instance: Some(at("2026-11-03T19:30:00+01:00[Europe/Berlin]")),
            range: EditRange::Following,
            patch: Some(AppointmentPatch {
                title: Some("Chor neu".into()),
                ..AppointmentPatch::default()
            }),
            delete: false,
        },
    )
    .unwrap();
    let made = created(&drafts);
    assert_eq!(
        made[0].rrule.as_deref(),
        Some("FREQ=WEEKLY;BYDAY=TU;COUNT=3")
    );
}

#[test]
fn following_from_the_first_is_all() {
    let all = stored();
    let choir = by_uid(&all, "chor@example.org", None);
    let drafts = run(
        &all,
        choir.id,
        &AppointmentEdit {
            instance: Some(choir.start.clone()),
            range: EditRange::Following,
            patch: Some(AppointmentPatch {
                title: Some("Chorprobe".into()),
                ..AppointmentPatch::default()
            }),
            delete: false,
        },
    )
    .unwrap();
    assert!(created(&drafts).is_empty());
    assert!(set_of(&drafts, choir.id, "rrule").is_none());
    assert!(set_of(&drafts, choir.id, "title").is_some());
}

#[test]
fn following_on_a_series_of_days_ends_the_day_before() {
    let mut all = stored();
    let birthday = all
        .iter_mut()
        .find(|a| a.uid == "geburtstag@example.org")
        .unwrap();
    birthday.rrule = Some("FREQ=YEARLY".into());
    let birthday = birthday.id;
    let drafts = run(
        &all,
        birthday,
        &AppointmentEdit {
            instance: Some(When::Date(date(2028, 12, 5))),
            range: EditRange::Following,
            patch: None,
            delete: true,
        },
    )
    .unwrap();
    assert_eq!(
        set_of(&drafts, birthday, "rrule"),
        Some(&Value::from("FREQ=YEARLY;UNTIL=20281204"))
    );
}

#[test]
fn a_rule_samu_cannot_compute_and_an_end_before_the_start_are_refused() {
    let all = stored();
    let dentist = by_uid(&all, "zahnarzt@example.org", None).id;
    let bad_rule = patch(AppointmentPatch {
        rrule: Some("FREQ=SOMETIMES".into()),
        ..AppointmentPatch::default()
    });
    assert!(matches!(
        run(&all, dentist, &bad_rule),
        Err(EditError::Invalid(_))
    ));
    let backwards = patch(AppointmentPatch {
        end: Some(at("2026-11-20T08:00:00+01:00[Europe/Berlin]")),
        ..AppointmentPatch::default()
    });
    assert!(matches!(
        run(&all, dentist, &backwards),
        Err(EditError::Invalid(_))
    ));
}

#[test]
fn the_later_change_wins_a_field_both_changed() {
    let all = stored();
    let base = by_uid(&all, "zahnarzt@example.org", None).clone();
    let mut remote = base.clone();
    remote.title = "von drüben".into();
    remote.location = Some("drüben".into());
    let mut ours = base.clone();
    ours.title = "von samu".into();
    ours.location = Some("hier".into());
    ours.note = None;
    let lm: Timestamp = "2026-11-11T10:00:00Z".parse().unwrap();
    let pending = |field: &str, at: &str| Pending {
        field: field.into(),
        hlc: Hlc {
            wall: at.parse::<Timestamp>().unwrap().as_millisecond(),
            counter: 0,
            device: "web".into(),
        },
        base: field_of(&base, field).unwrap(),
    };
    let pending = vec![
        pending("title", "2026-11-11T10:05:00Z"),
        pending("location", "2026-11-11T09:55:00Z"),
        // Only samu changed the note, before the other's change: it holds.
        pending("note", "2026-11-11T09:50:00Z"),
    ];
    let merged = merge_remote(&ours, &pending, &remote, Some(lm)).unwrap();
    assert_eq!(merged.title, "von samu");
    assert_eq!(merged.location.as_deref(), Some("drüben"));
    assert_eq!(merged.note, None, "the other side left the note alone");
    let merged = merge_remote(&ours, &pending, &remote, None).unwrap();
    assert_eq!(
        merged.location.as_deref(),
        Some("hier"),
        "no LAST-MODIFIED: samu"
    );
}

#[test]
fn a_series_turned_into_whole_days_leaves_no_instance_behind() {
    let all = stored();
    let choir = by_uid(&all, "chor@example.org", None);
    let drafts = run(
        &all,
        choir.id,
        &AppointmentEdit {
            range: EditRange::All,
            patch: Some(AppointmentPatch {
                start: Some(When::Date(date(2026, 10, 20))),
                clear: vec!["end".into()],
                ..AppointmentPatch::default()
            }),
            ..AppointmentEdit::default()
        },
    )
    .unwrap();
    assert_eq!(deleted(&drafts).len(), 2, "both instances go");
    assert!(created(&drafts).is_empty());
    assert_eq!(set_of(&drafts, choir.id, "exdate"), Some(&Value::Null));
}

/// Applies the sets one by one, as the store does, and asks [`check`] of
/// every appointment between.
fn every_step_holds(old: &Appointment, drafts: &[Draft]) {
    let mut now = json_of(old).unwrap();
    for d in drafts {
        if let Draft::Set {
            object,
            field,
            value,
        } = d
            && *object == old.id
        {
            if value.is_null() {
                now.remove(field);
            } else {
                now.insert(field.clone(), value.clone());
            }
            let step: Appointment = serde_json::from_value(Value::Object(now.clone())).unwrap();
            assert!(check(&step).is_ok(), "after {field}: {step:#?}");
        }
    }
}

fn fields(drafts: &[Draft]) -> Vec<&str> {
    drafts
        .iter()
        .filter_map(|d| match d {
            Draft::Set { field, .. } => Some(field.as_str()),
            _ => None,
        })
        .collect()
}

// Regression (33dc4c41): moved past its old end, the start alone ended
// before it began, and the store refused the edit.
#[test]
fn start_and_end_are_set_so_that_every_step_is_an_appointment() {
    let all = stored();
    let dentist = by_uid(&all, "zahnarzt@example.org", None);
    let edit = |start: When, end: When| {
        run(
            &all,
            dentist.id,
            &patch(AppointmentPatch {
                start: Some(start),
                end: Some(end),
                ..AppointmentPatch::default()
            }),
        )
        .unwrap()
    };

    let later = edit(
        at("2026-12-20T14:30:00+01:00[Europe/Berlin]"),
        at("2026-12-20T15:30:00+01:00[Europe/Berlin]"),
    );
    assert_eq!(fields(&later), ["end", "start"]);
    every_step_holds(dentist, &later);

    let earlier = edit(
        at("2026-01-05T08:00:00+01:00[Europe/Berlin]"),
        at("2026-01-05T08:30:00+01:00[Europe/Berlin]"),
    );
    assert_eq!(fields(&earlier), ["start", "end"]);
    every_step_holds(dentist, &earlier);

    let whole_day = edit(
        When::Date(date(2026, 11, 20)),
        When::Date(date(2026, 11, 21)),
    );
    assert_eq!(fields(&whole_day), ["end", "start", "end"]);
    every_step_holds(dentist, &whole_day);
}
