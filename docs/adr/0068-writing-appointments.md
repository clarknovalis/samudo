---
number: 0068
title: Writing appointments from samu – create, change and delete over CalDAV, offline through the outbox, series with "this", "this and following", "all"
status: accepted
date: 2026-11-11
adds_to: ["0004", "0047", "0048", "0049", "0066"]
---

# 0068 – Writing appointments from samu

## Context

ADR 0047 read calendars only; only the server wrote appointments. ADR
0049 handed editing to the phone's calendar app and left the web with a
link to the original, and excluded writing over CalDAV until a record of
its own settled it. This is that record.

On the web the link does not go far: some hosted calendars open at best
the day, not the appointment, and a plain CalDAV server such as Radicale
has no front end for appointments at all. The front ends that exist are
old, too heavy for a small server, or also cannot open a single
appointment.

So samu becomes the front end. It holds the appointments anyway and can
write them back, to any CalDAV server that allows writing. Chosen: every
CalDAV calendar with write permission, ICS stays read-only; offline
through the outbox like tasks (ADR 0004, 0028, 0066); series complete in
the first round.

## Decision

1. **What is writable:** a `calendar` has the server-written field
   `writable`. It is true when `kind = caldav` and the collection's
   `current-user-privilege-set` contains `write-content`, `write` or
   `all`. The server reads it on discovery (ADR 0047 §8) and on every
   fetch with a changed ctag. ICS is never writable. Clients offer editing
   only when `writable`. If a server does not report the privilege, the
   calendar counts as writable until a `PUT` fails with `403`.
2. **Invitations stay read-only.** An appointment with `ATTENDEE` gets the
   server-written field `meeting` (true). samu speaks no iTIP, so no
   replies and no invitations to attendees; for these ADR 0049 still
   holds (calendar app, link). Clients offer no editing there, and the
   server rejects changes.
3. **`appointment` is writable over `/changes`** (lifts the server-only
   rule of ADR 0047 §4 for writable calendars), with the existing
   operations and last-writer-wins per field (ADR 0004):
   - `create`: a new appointment. The client chooses the `id` (UUIDv7);
     `uid` is the same UUID as text. With `recurrence_id`, `create` makes
     an exception of a series (same `uid`).
   - `set` on `title`, `start`, `end`, `location`, `note`, `busy`,
     `rrule`, `rdate`, `exdate` and `calendar` (move to another writable
     calendar). `uid`, `recurrence_id`, `url`, `status`, `meeting` and
     `alarms` cannot be changed.
   - `delete`: the whole appointment; on an exception only the exception.
   A change to an appointment of a calendar that is not writable, or to a
   `meeting`, is `rejected`.
   **Logbook:** what a client or the route of §5 changes on an
   appointment, deletion included, is in the appointment's logbook with
   author and device as for tasks (ADR 0024, 0039). What the fetch (ADR
   0047 §7) brings is not.
4. **Series** are computed by `samu-core` in one function,
   `edit_appointment(series, instance, range, patch)`. It returns the
   changes of §3, and every client sends them through its outbox:
   - **this one:** `create` or `set` of an exception with `recurrence_id`
     = start of the instance; delete = extend `exdate` by the instance
     (`set` of the whole list);
   - **this and following:** on the series `rrule` with `UNTIL` just
     before the instance (or a shortened `COUNT`), plus a new series (new
     `uid`) from the instance with the rest of the rule and the patch;
     exceptions and `exdate` from the instance on move to the new series.
     Delete = only the shortening;
   - **all:** `set` on the series. If the patch moves the start,
     exceptions and `exdate` move by the same span. An exception takes
     over title, location, note and busy only where it still had the
     series' value; what was changed on the instance itself stays.
   If the instance is the first, "this and following" is the same as
   "all".
5. **Route for clients without `samu-core`:** `POST
   /appointments/{id}/edit` takes `instance` (optional), `range`
   (`this`, `following`, `all`) and either `patch` (fields of §3) or
   `delete: true`. It calls §4 and applies the result like `/changes`;
   the response lists the changes with their `ChangeOutcome`. The local
   replica answers it offline from mirror and outbox (ADR 0066); the app
   calls §4 through `samu-mobile` directly. Creating needs no route of its
   own; `create` over `/changes` suffices.
6. **Writing back in the server:** an accepted change is written into the
   server's table at once (clients see it over `/changes` as any other)
   and the resource (calendar, `uid`) is queued in `appointment_write` in
   the account file. One writer per account works the queue:
   - new resource: `PUT <collection>/<uid>.ics` with `If-None-Match: *`;
   - changed: `GET` the resource, replace the known properties (§3) in
     each VEVENT, add or drop exception VEVENTs, leave everything else
     (VALARM, ATTENDEE, X- properties, VTIMEZONE) as it is, renew
     `SEQUENCE` and `LAST-MODIFIED`/`DTSTAMP`, then `PUT` with `If-Match:
     <etag>`;
   - deleted: `DELETE` with `If-Match`;
   - other calendar: first `PUT` into the new collection, then `DELETE`
     in the old one.
   For this the server keeps `href` and `etag` per appointment (internal
   columns, not in `samu-api`). With `SAMU_CALENDAR_HOSTS` set, a redirect
   to another host is not followed (ADR 0097 §3).
7. **Collision**, decided per field against the value before samu's
   change: the queue keeps, per field, the value before samu first changed
   it. When the server answers `412`, samu fetches the resource again and
   per field:
   - if the other version still has the value from before samu's change,
     nobody changed it there, and samu's value holds;
   - if both changed it, the later change holds: samu's when the time of
     its HLC is after the other version's `LAST-MODIFIED`, or when
     `LAST-MODIFIED` is missing; otherwise the other version's.
   Then it tries again. If the resource was deleted there (`404`), the
   deletion wins and the change lapses. The rule is
   `samu_core::calendar::edit::merge_remote`.
8. **Fetch and queue:** while a write for a `uid` is pending, the fetch
   (ADR 0047 §7) skips that resource, so it does not overwrite samu's own
   change with the old state.
9. **Failures:** network, `5xx` and `423` are retried with growing
   intervals (1, 5, 15 minutes, then the calendar's `refresh`), without
   end. `403`, `507` or an answer that does not fit end the attempt: the
   entry leaves the queue, the calendar's `error` names appointment and
   reason, and the next fetch restores the server's state.
10. **Which calendar for a new appointment:** setting `default_calendar`
    in `settings` (ADR 0045 §4), a writable calendar. Without it the
    interface asks.
11. **Interfaces:**
    - **Web:** the appointment sheet (ADR 0047 §11) has "Edit" and
      "Delete", for series with the choice "Only this / This and
      following / All". In the calendar view a click on free space creates
      an appointment (one hour), a click on a day's header an all-day one;
      in the forecast a day's header creates an all-day one. Dragging
      appointments comes with the second round of the view.
    - **App:** the same sheet with "Edit". "Open in calendar app" and
      `ACTION_INSERT` from ADR 0049 stay, for invitations and as an
      alternative.
    - **CLI:** `samu appointment add|edit|delete` with `--instance`,
      `--range this|following|all`, time as for tasks. Every route has its
      command.
12. **Left out:** writing to ICS subscriptions, iTIP and invitations,
    setting VALARM from samu (reading it is ADR 0101), VTODO.

## Consequences

- `samu-api`: `Calendar.writable`, `Appointment.meeting`,
  `Settings.default_calendar`, request and response of
  `/appointments/{id}/edit`, each with fixtures. catastr reads no
  appointments and needs no change.
- `samu-store`: migration with `href` and `etag` on `appointment`,
  `writable` on `calendar`, the table `appointment_write` (calendar,
  `uid`, since, attempts, next attempt, last failure, and per field
  `[object, field, hlc, base]`), `meeting`, `default_calendar`.
- `samu-core`: §4 with tests for all three ranges, with `COUNT`, `UNTIL`,
  exceptions and `exdate`, all-day and zoned; and the rule of §7.
- `samu-server` writes iCalendar, not only reads it, and must be able to
  write a resource back unchanged. The test CalDAV server learns `PUT`,
  `DELETE`, `If-Match`, `412` and privileges.
- `samu-replica` and `samu-mobile` run §4 before the outbox; the mirror
  shows the own change at once.
- A change from the phone's calendar sync and one from samu can cross;
  §7 decides per field. Whoever edits offline for long can override a
  later change made elsewhere when that one lacks `LAST-MODIFIED`.
- ADR 0049 §1 ("samu stays read-only") and §6 no longer hold for
  writable calendars; the rest of 0049 stays.

## Changes

- 2026-11-11 (ADR 0071): §7 decided every field by time alone, so a title
  changed on the phone also discarded a location samu had changed and not
  yet written; §4 did not say what "all" does with fields of an
  exception.
- 2026-11-13 (ADR 0097 §3): §6 followed redirects to any host.
- 2026-11-13 (ADR 0101): §3 did not name `alarms`; §12 excluded VALARM
  altogether.
