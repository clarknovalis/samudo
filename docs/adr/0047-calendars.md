---
number: 0047
title: Calendars over ICS and CalDAV, fetched by the server; appointments in the forecast and a calendar view
status: accepted
date: 2026-11-09
adds_to: ["0004", "0006", "0007", "0035", "0045"]
---

# 0047 – Calendars: ICS and CalDAV, appointments in forecast and calendar view

## Context

The plan calls for reading appointments over CalDAV and ICS and for a
calendar view with time blocks. ADR 0007 already says that calendar
entries use the same two forms of time as tasks; ADR 0045 §7 leaves room
for them in the forecast. The wish was several calendars, read-only, each
appointment with a link that jumps into the original calendar.

Sources to cover: ICS subscription addresses, CalDAV with a password
(Nextcloud, Radicale, iCloud with an app password), and a work calendar
from Exchange/Microsoft 365, usually published as an ICS address. No
Google OAuth. The server fetches, clients cache, so the app shows
appointments offline.

This record started read-only. Writing back over CalDAV came with ADR
0068, reminders with ADR 0101; both are worked in below.

## Decision

1. **Two kinds in `samu-api`:** `calendar` (one source) and
   `appointment` (one entry; `Appointment`, `AppointmentStatus`,
   `AppointmentInstance`). Both travel over `/changes` like every object
   (ADR 0004) and so sit in the app's cache.
2. **A `calendar` belongs to the user**, changed per field with `set`
   over `/changes`: `title`, `kind` (`ics`, `caldav`), `color`,
   `enabled`, `refresh` (minutes, default 15, at least 5),
   `link_template`, `position`. The server writes `fetched_at`, `error`
   (text of the last failure, empty when fine), `host` (only the host
   name of the source, to recognise it) and `writable` (ADR 0068 §1).
3. **The source is a secret.** An ICS address carries its key in the URL;
   CalDAV needs user and password. Address, user and password live in a
   table of their own in the account file (`calendar_source`), never in
   `/changes`, never in a response, never in an export (ADR 0074). They
   are set with `PUT /calendars/{id}/source` (CLI `samu calendar source
   <id>`, password from stdin or prompted). They are **encrypted** with
   XChaCha20-Poly1305, a fresh nonce per entry and the calendar's id as
   associated data. The key (32 bytes, Base64) comes from the server's
   environment, `SAMU_SECRET_KEY`, never from the repository. Without a
   key the server starts but rejects `source` and fetches nothing;
   `error` says so. A lost key means setting the sources again; the
   appointments stay until then.
4. **Who writes an `appointment`.** Originally only the server: a client
   change was `rejected`, like creating or deleting `settings` (ADR 0045
   §4). Since ADR 0068 §3 a client may write appointments of a writable
   calendar; the rest stays server-only. Fields: `calendar`, `uid`,
   `recurrence_id` (empty on the master, otherwise the instance it
   replaces), `title`, `start`, `end` (each one of the two forms of ADR
   0007; for all-day the `end` is exclusive, as in iCalendar),
   `location`, `note` (DESCRIPTION), `url`, `status` (`confirmed`,
   `tentative`, `cancelled`), `busy` (TRANSP), `rrule`, `rdate`,
   `exdate`, and from the server `meeting` (ADR 0068 §2) and `alarms`
   (ADR 0101). What the fetch brings is not in the logbook; changes made
   from samu are (ADR 0068 §3).
5. **Repetition is expanded, not stored.** `samu-core` expands it
   (`occurrences(appointments, from, to, zone)`) with the RRULE crate of
   ADR 0007. An instance with `recurrence_id` replaces the computed one;
   `exdate` and `status = cancelled` remove one. Forecast, calendar view,
   `/appointments`, reminders and the app call this one function; no
   client computes it (as ADR 0006).
6. **Time zones:** TZID maps to IANA; Windows names from Exchange ("W.
   Europe Standard Time") through CLDR's `windowsZones` table, embedded
   in `samu-core`. An unknown zone counts as the account's zone and the
   calendar's `error` names it. Floating times count in the account's
   zone.
7. **Fetching in the server**, one task per account in the tokio runtime,
   each calendar every `refresh` minutes:
   - ICS: `GET` with `If-None-Match`/`If-Modified-Since`; `304` means
     nothing to do. `webcal://` is read as `https://`.
   - CalDAV: `PROPFIND` for `getctag`/`sync-token`; on a change
     `REPORT sync-collection` (RFC 6578) where supported, otherwise
     `calendar-query` over the window and `calendar-multiget` for changed
     `etag`s.
   - Window: 30 days back to 400 days ahead; masters of a series still
     running stay, however early they began.
   - Per (`uid`, `recurrence_id`) the server compares field by field and
     writes only what changed; what vanished is deleted. An unchanged
     calendar produces no change.
   - A failure (network, 401, broken ICS) leaves the appointments and
     sets `error`; the next good fetch clears it.
   - A resource with a pending write from samu is skipped (ADR 0068 §8).
   - `POST /calendars/{id}/refresh` fetches at once (CLI `samu calendar
     refresh`); web and app have a button.
   - With `SAMU_CALENDAR_HOSTS` set, only those hosts are fetched, also
     after a redirect (ADR 0097 §3).
8. **Discovering CalDAV:** `POST /calendars/discover` takes an address
   (principal or root, `.well-known/caldav`) with user and password and
   returns the calendar collections with name and colour; it creates
   nothing. CLI (`samu calendar discover`) and the settings in web and
   app create one `calendar` per chosen collection and set its source.
   More calendars from a server already set up: `POST
   /calendars/{id}/discover` with the stored source (ADR 0100 §1).
9. **Link to the original** (one function in `samu-core`): the
   appointment's URL field if it has one; otherwise the calendar's
   `link_template` with `{uid}`, `{date}` (local date `YYYY-MM-DD`),
   `{y}`, `{m}`, `{d}`; otherwise no link. The settings offer ready
   templates for Google, Nextcloud and Outlook on the web (day view at
   that date), each checked before it is added.
10. **Forecast** (adds to ADR 0045 §3): per day the day's appointments
    first, all-day ones first, then by start; reason `event`. A multi-day
    appointment stands on each of its days. Cancelled ones do not. The
    day strip shows, next to the task count, a dot on days with
    appointments (not their number, so the strip still reads by tasks).
    Setting `forecast_events` (on) in `settings`. "Later" and "Overdue"
    show no appointments.
11. **Calendar view** in the web UI: day and week as an hour grid with a
    band of all-day entries on top (month and list since ADR 0100 §5–7).
    Appointments in their calendar's colour, and tasks with `planned` as
    an instant as a block of their estimate (otherwise 30 minutes). A
    click on an appointment opens its sheet with title, time, location,
    note and link; a click on a task opens it as everywhere. Editing in
    the sheet and creating by clicking on free space came with ADR 0068
    §11; dragging tasks into time blocks is a later round. The app shows
    appointments in its forecast.
12. **Scope** (adds to ADR 0035): field `calendars`, a set of calendar
    ids; empty means all. So a scope "Work" shows only the work calendar.
13. **CLI:** `samu calendar list|add|set|source|discover|refresh|delete`,
    and `samu appointments --from --to` over `GET
    /appointments?from&to&scope`, which returns the instances from
    `samu-core` (local dates, `to` inclusive, like `/clocks/report`).
    The Emacs package shows appointments in its forecast.
14. **Search in the web UI** (adds to ADR 0042): chip `ist:termin`
    (English `is:appointment`). Without it the search finds tasks as
    before; with it only appointments, by title, location and note. A
    repeating appointment appears once, with its next occurrence. The
    date chips apply to the start; `-ist:termin` is the search as before.
    In a scope's filter the chip does not apply; `calendars` (12) chooses
    there.
15. **Reminders:** originally none (VALARM was not read). Since ADR 0101
    §1 the fetch reads VALARM with `ACTION:DISPLAY` or `AUDIO` into the
    field `alarms`; the rules are in ADR 0101.

## Consequences

- `samu-api` has `Calendar`, `Appointment`, `AppointmentStatus`,
  `AppointmentInstance`, the responses of `/calendars/discover` and
  `/appointments`, `calendars` on the scope and `forecast_events` on
  `settings`, each with a fixture. catastr reads unknown kinds as
  `unknown` (ADR 0015) and needs no change.
- `samu-store` has the tables `calendar`, `calendar_source` (secret, not
  in the event stream) and `appointment`.
- Dependencies: `reqwest` with `rustls-tls` in the server (redirects,
  gzip, methods such as `PROPFIND` and `REPORT`), `quick-xml` for CalDAV
  multistatus, `chacha20poly1305` for the sources, and an iCalendar
  parser chosen after testing synthetic exports of Google, Nextcloud and
  Exchange (`testdata/`).
- Tests use a small ICS and CalDAV server inside the test (axum) that
  serves fixtures, answers `304` and plays failures; no outside service.
- Load on the server: one request per calendar every 15 minutes, mostly
  `304` or an unchanged ctag.

## Changes

- 2026-11-09 (ADR 0048): the kind was called `event`, the route `GET
  /events?from&to&scope` and the command `samu events`, and the chip
  `is:event`. Route and command clashed with the event stream (ADR
  0019) and were renamed to `appointment`, `/appointments`, `samu
  appointments` and `is:appointment`. The reason `event` in the forecast
  and the setting `forecast_events` kept their names.
- 2026-11-11 (ADR 0068): §4 said that only the server writes
  appointments and that they never enter the logbook; §2 had no
  `writable`; §11 was read-only.
- 2026-11-13 (ADR 0097): §7 fetched from every host.
- 2026-11-13 (ADR 0100): §8 always needed address, user and password;
  §11 had only day and week.
- 2026-11-13 (ADR 0101): §15 said that VALARM is not read and the
  calendar itself reminds.
