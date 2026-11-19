---
number: 0100
title: Managing and viewing calendars – more calendars from the same server, importing an ICS file, month and list views, a repetition editor for appointments
status: accepted
date: 2026-11-13
adds_to: ["0047", "0066", "0068", "0095", "0096"]
---

# 0100 – Managing and viewing calendars

## Context

ADR 0047 fetches calendars over ICS and CalDAV, ADR 0068 writes
appointments back over CalDAV. Moving calendars from a hosted provider
to a self-hosted CalDAV server showed four gaps:

- **Discovery** (ADR 0047 §8) asked for server, user and password every
  time, even when a calendar of the same server was already set up.
- An **`.ics` file** (export of an old calendar, an invitation, a
  timetable) could not get into samu.
- The calendar view had only **day and week** (ADR 0047 §11); a month
  and a list of coming appointments were wanted.
- **Repetition** of an appointment was a raw RRULE text field in the
  form. The CalDAV server needs nothing for it: RRULE, exceptions and
  `EXDATE` pass back and forth unchanged (ADR 0068 §4, §6).

Chosen: further calendars with the stored credentials, colour on adding;
import into a writable CalDAV calendar with VEVENTs unchanged; month and
list in the web first; an editor instead of the text field. Creating new
collections on the CalDAV server stays with that server's own interface.

## Decision

1. **More calendars from the same server.** `POST
   /calendars/{id}/discover` without a body searches with the stored
   source of CalDAV calendar `{id}`: from the host's root via
   `.well-known/caldav`, otherwise via the collection's own
   `current-user-principal`, then on as ADR 0047 §8. The answer is the
   same list as from `/calendars/discover`; a collection already
   subscribed carries `known: true` (field on `DiscoveredCalendar`), so
   the interface greys it out. An ICS calendar or one without a source
   gives `400`.
2. **Taking over a source.** `CalendarSource` has the optional field
   `credentials_from` (a calendar id). With it, `PUT
   /calendars/{id}/source` sets `url` from the request and user and
   password from that calendar's source. The password thus never leaves
   the server; a client neither knows nor needs it. The origin must be
   CalDAV and have the same host as `url`, otherwise `400`. Credentials
   are copied, not linked: a new password is set per calendar (or in one
   and taken over again).
3. **Colour on adding.** Web and app show the discovered collections
   with a colour picker per row, preset with the server's
   `calendar-color`, otherwise the next free one of the palette. It sets
   `color` on the new `calendar` as before. The colour is not written
   back to the CalDAV server.
4. **Importing an ICS file.** `POST /calendars/{id}/import` takes an
   iCalendar file (`text/calendar`, at most 10 MB) for a **writable**
   CalDAV calendar (ADR 0068 §1); otherwise `400`. The server
   - splits it by `UID`: master and exceptions with the same `UID` become
     one resource, together with the `VTIMEZONE`s they refer to;
   - uploads each resource **unchanged** (VALARM, ATTENDEE, X- properties
     stay) as `PUT <collection>/<uid>.ics`. A `UID` that already exists in
     the calendar replaces that resource (`If-Match` with its `etag`); a
     new one goes with `If-None-Match: *`. Characters that disturb a path
     are percent-encoded in the file name; the `UID` itself stays;
   - skips `VTODO`, `VJOURNAL` and VEVENTs without `UID` or `DTSTART`,
     and counts them;
   - then fetches the calendar at once (ADR 0047 §7), so the
     appointments arrive over `/changes`. They are thus not created
     through the outbox and are not in the logbook; the import itself is
     one entry on the calendar (file, numbers).
   The answer names `created`, `replaced`, `skipped` and `failed` (each
   with `UID` and reason). A failed appointment does not stop the rest.
   The route is online only; the replica relays it (ADR 0096), offline
   `503 offline`. `SAMU_CALENDAR_HOSTS` (ADR 0097) applies.
5. **Month view** (web): six weeks from the Monday before the first,
   Monday to Sunday. Per day the appointments as a line in their
   calendar's colour (all-day first, then by start), below them tasks
   with `planned` on that day. What does not fit becomes "+N". A click on
   the date opens the day view; a click on an appointment or task opens
   it as in the week; a click on free space creates an all-day
   appointment (ADR 0068 §11). Multi-day appointments stand on each of
   their days, as in the forecast (ADR 0047 §10).
6. **List view** (web): from today, day by day, only days with something
   on them, each with appointments and tasks with `planned` as in §5;
   first 30 days, "More" loads the next 30. Cancelled appointments are
   missing as everywhere. The view is for reading and jumping; editing
   goes through the sheet.
7. **Switch** day / week / month / list in the calendar view's header,
   remembered in the browser as before. Month and list ask
   `/appointments` (or the computation in `samu-core`) like day and
   week; no client expands on its own (ADR 0047 §5). The app gets month
   and list as a task of its own.
8. **Repetition editor for appointments.** The appointment form has the
   same editor as tasks (`web/lib/repeat.js`), without OmniFocus's mode
   (an appointment always repeats on schedule): daily, weekly with
   weekdays, monthly on a day or on the n-th weekday, yearly, interval,
   end never / on a date (`UNTIL`) / after N (`COUNT`). An RRULE the
   editor cannot show (such as `BYSETPOS`, `BYHOUR`, several `BYMONTH`)
   is shown as text and left unchanged unless replaced. `RDATE` and
   `EXDATE` stay as they are.
9. **At the local replica** (changes ADR 0066 §5 for these routes): the
   calendar routes `POST /calendars/discover`, `PUT
   /calendars/{id}/source`, `POST /calendars/{id}/refresh` and the new
   ones of §1 and §4 are no longer `Online` (always `503`) but `Relay` as
   in ADR 0096: the replica passes them to the server with its session,
   offline or without a valid session `503 offline`. Before, discovering
   calendars on the computer never worked, not even in the browser
   through the replica. `relay` passes the request body with its
   `Content-Type` (JSON, for the import `text/calendar`) and returns the
   response's `Content-Type` unchanged instead of a fixed
   `application/json`.
10. **Restricted sessions** (ADR 0095): the new routes of §1 and §4 are
    forbidden there like the other calendar routes, each with a test.

## Consequences

- `samu-api`: `DiscoveredCalendar.known`, `CalendarSource.credentials_from`,
  the response of `/calendars/{id}/import`, each with fixtures. catastr
  reads no calendars and needs no change.
- `samu-server`: the two routes, the splitter by `UID` (reused from the
  writer of ADR 0068); the test CalDAV server learns
  `current-user-principal` on a collection.
- `samu-store`: no migration; `credentials_from` is resolved on setting
  and not stored.
- CLI: `samu calendar discover --from <calendar>`, `samu calendar add
  --from <calendar> [--color …] [title …]`, `samu calendar import
  <calendar> <file|->`. Every route with its command.
- Web: "More calendars from this server" in a CalDAV calendar's editor,
  "Import ICS file" on writable ones, month, list and the editor.
- Moving from one CalDAV provider to another thus happens entirely in
  samu: export as `.ics`, create a collection on the new server,
  subscribe to it through §1, import the file through §4.
- Tests run against the test CalDAV server; manual probes against a real
  server only with a calendar made for the probe and deleted afterwards.

## Changes

None yet.
