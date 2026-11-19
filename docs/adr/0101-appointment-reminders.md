---
number: 0101
title: Reminders for appointments – read VALARM into alarms, one computation in samu_core::reminders, GET /alarms, notifications in app and browser, a switch per device (off)
status: accepted
date: 2026-11-13
adds_to: ["0030", "0047", "0066", "0068"]
replaces: ["0047 §15"]
---

# 0101 – Reminders for appointments

## Context

ADR 0047 §15 excluded reminders for appointments: VALARM was not read,
the calendar itself would remind. Since ADR 0068 samu is the front end
for appointments, and with a plain CalDAV server there is no web calendar
left that reminds. Task reminders exist in the app (ADR 0030, 0093);
desktop and browser were a wish.

Chosen: the reminders stored in the appointment itself (VALARM), not
defaults per calendar; switchable in the settings; notifications in the
browser while the web UI is open; and the switch **per device**: a phone
that syncs calendars itself (for example with DAVx5) already reminds of
the same VALARMs, the browser on the computer does not.

## Decision

1. **Lifts ADR 0047 §15.** The fetch reads VALARM with `ACTION:DISPLAY`
   or `ACTION:AUDIO`; `EMAIL` and unknown actions stay out.
2. **Field `alarms` on the appointment**, written by the server, a list
   of
   - `{ "before": "PT15M" }`: a duration before the start (relative
     `TRIGGER`, `RELATED=START`, the default);
   - `{ "before": "PT0S", "related": "end" }`: relative to the end;
   - `{ "at": "2026-11-20T08:00:00Z" }`: a fixed instant
     (`TRIGGER;VALUE=DATE-TIME`).
   A `TRIGGER` after its reference (positive duration) is stored as a
   negative `before` (`"-PT5M"`). `REPEAT`, `DURATION` and `ACKNOWLEDGED`
   are not read. An exception of a series has its own `alarms`. `alarms`
   is **not** changeable over `/changes`; VALARMs stay untouched when
   writing back anyway (ADR 0068 §6). Setting reminders from samu comes
   when it is missed.
3. **All-day:** a relative `TRIGGER` on an all-day appointment counts from
   midnight of the day in the account's zone (`-PT15H` = the day before
   at 9:00), as calendar apps do.
4. **One computation** (ADR 0030 §4): `samu_core::reminders` also takes
   appointments, expands series over the requested window with the same
   function as `/appointments` (ADR 0047 §5), and returns one
   notification per instance and alarm, with title, start and location. A
   notification refers either to a task or to an appointment instance
   (appointment id and start of the instance). Cancelled appointments and
   those of a disabled calendar (`enabled = false`) notify nothing; the
   scope does not apply (as for tasks). The caller says which kinds it
   wants (tasks, appointments).
5. **`GET /alarms?from&to&appointments=true|false&tasks=true|false`**
   returns the notifications of §4 in the window (at most 7 days), by
   time. At the local replica the route is `Local` (mirror and
   `samu-core`, ADR 0066); in restricted sessions it is forbidden (ADR
   0095). CLI: `samu alarms [--from] [--to] [--no-tasks]
   [--no-appointments]`.
6. **Switch per device, default off:** "Reminders for appointments", and
   in the browser also "Reminders for tasks". They live on the device,
   not in `settings`, and are not synced: in the app in its local
   settings (with a hint about calendar sync on the phone), in the
   browser in that browser's `localStorage`. There is no account field.
7. **App:** the same alarm clock as ADR 0030 §5; with the switch on,
   `samu-mobile` includes appointments in §4. An appointment's
   notification shows title, time and location; tapping opens the
   appointment sheet. Action *Later* (one hour) as for tasks; there is no
   *Done*.
8. **Browser:** with a switch on, the web UI asks once for permission
   (Notification API) and fetches `/alarms` for the next 24 hours on
   load, after every change from the event stream and hourly. A timer
   stands for each notification; when due it shows a browser
   notification, and a click opens the task or the appointment sheet.
   What lies at most 10 minutes back on load is shown once. The browser
   remembers which notifications it showed (`localStorage`), so a second
   tab does not notify twice. Without an open web UI nothing comes; push
   without an open tab and the desktop are a separate wish.
9. **Display:** the appointment sheet in web and app names the reminders
   ("15 min before"), also with the switch off, so one sees what the
   phone will do.

## Consequences

- `samu-api`: `Appointment.alarms`, `AppointmentAlarm`, the response of
  `/alarms`, each with fixtures. An older client ignores the fields.
- `samu-store`: migration with the column `alarms` (JSON) on
  `appointment`. After it the next fetch of every calendar is a full one
  (ctag and ETag forgotten), so `alarms` gets filled.
- `samu-core`: the parser reads VALARM; `reminders` with appointments;
  tests relative to start and end, absolute, all-day, a series with an
  exception and `EXDATE`, cancelled, disabled calendar.
- Server and replica: `/alarms` with a test for both kinds.
- App: switch, notification and sheet for appointments. Web: switch in
  the settings, timers, notifications.

## Changes

None yet.
