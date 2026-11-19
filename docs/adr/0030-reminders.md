---
number: 0030
title: Reminders – the object reminder, due tasks remind by themselves, an alarm on the phone
status: accepted
date: 2026-11-08
adds_to: ["0006", "0007", "0028"]
---

# 0030 – Reminders: object `reminder`, due tasks by themselves, alarm on the phone

## Context

ADR 0006 has the reminder in the data model: on a task, absolute or
relative to `due`, `planned` or `defer`, several per task. The plan had
reminders as server push over ntfy; ADR 0028 §9 lists reminders as a later
step of the Android app.

A push from the server needs a network, arrives as late as ntfy delivers
it, and nothing can be checked off from the notification. The phone, in
contrast, already has the mirror with all data (ADR 0028 §3) and can set
alarms itself.

Chosen: an alarm on the phone; due tasks remind by themselves, plus
reminders of one's own; in the notification Done, Later (1 h) and
Tomorrow.

## Decision

1. **Object `reminder`**, a new kind beside `link` and `clock`, with
   `owner` (task or project) and:
   - `anchor`: `at`, `due`, `planned`, `defer`, or `daily` (ADR 0093);
   - `at`: an instant (ISO 8601 with `Z`), only with `anchor = at`;
   - `time`: a time of day `HH:MM`, only with `anchor = daily` (ADR 0093);
   - `before`: a duration before the anchor; absent means the anchor
     itself.
   An unknown anchor is read as `unknown` and never written (ADR 0015); it
   triggers nothing. Every field has its own stamp (ADR 0004). If the date
   the anchor points to is missing, the reminder triggers nothing.
2. **Due tasks remind by themselves:** every open task and project with
   `due` notifies at its due time, without a `reminder` of its own. No
   object exists for that. A way to switch it off per task comes when it
   is missed.
3. **All-day:** when the anchor is a date (ADR 0007), it applies at the
   account's reminder time, in its zone: the setting `remind_time` (a time
   of day, default `09:00`), since ADR 0045 §4 in the object `settings`. It
   is also an optional field of `GET /session` and of the answer to `POST
   /login`, next to `zone` and `due_soon` (ADR 0028 §6).
4. **One computation, in `samu-core`:** a pure function determines, from
   a snapshot, the settings and a window, the notifications, each with
   owner, instant and title. It returns only what is open: not completed,
   not dropped, and the project neither completed nor dropped. Several
   reminders of the same owner at the same instant make one notification.
   No client computes this itself (as ADR 0006 for availability). It also
   covers daily reminders and habits (ADR 0093 §3) and appointments (ADR
   0101 §4).
5. **On the phone:** the app sets exactly one alarm (AlarmManager), the
   next. When it rings, the app shows every notification due by then that
   was not shown yet, and sets the next alarm. It is set again after every
   sync, every change on the phone, at app start and after the phone
   restarts. What fell due while no alarm stood is shown once at the next
   setting, at most 24 hours back. Which notifications were shown is kept
   in the cache of `samu-mobile`, not on the server. On ringing the app
   computes again, so a task checked off before the time does not notify
   (ADR 0093 §4).
6. **Actions in the notification:** *Done* checks off through the outbox,
   as in the widget. *Later* notifies again in an hour, *Tomorrow* the
   next day at `remind_time`; both stay on the phone and are not synced.
   Tapping opens the task.
7. **Departs from the plan:** ntfy is not the first path. Notifications
   from the server for computers and the web UI need a record of their
   own, which must settle how phone and server avoid notifying twice. The
   browser shows reminders while the web UI is open (ADR 0101 §8).

## Consequences

- `samu-api` has `Reminder`, `ReminderAnchor`, the kind `reminder` and
  `SessionInfo.remind_time`, with fixtures (ADR 0014). An older reader
  sees the kind as `unknown` (ADR 0015).
- `samu-store` migration with the table `reminder` and the column
  `account.remind_time` (later moved into `settings`, ADR 0045 §4).
- No new routes: reminders are created over `POST /changes`; the CLI has
  `samu remind` and `samu reminders`. The notifications of a window come
  from `GET /alarms` (ADR 0101 §5).
- The app needs the permissions for notifications and exact alarms;
  without the latter it notifies less exactly, but it notifies.
- A repetition's follow-up copies relative reminders, not absolute ones
  (ADR 0033).

## Changes

- 2026-11-09 (ADR 0045 §4): §3 kept `remind_time` on the account row.
- 2026-11-13 (ADR 0093): §1 had no anchor `daily` and no `time`.
- 2026-11-13 (ADR 0101): §7 said nothing reminds without the app.
