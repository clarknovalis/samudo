---
number: 0045
title: Forecast, second round – rule in samu-core, a flagged section, a forecast tag under today, deferred tasks, settings as an object
status: accepted
date: 2026-11-09
adds_to: ["0006", "0020", "0025", "0028"]
---

# 0045 – Forecast, second round

## Context

Forecast showed open tasks with a due date, otherwise a planned date,
from today to seven days ahead, plus "Overdue". A task with both dates
stood only on its due day. The rule existed twice, in the web UI and in
`samu-mobile`. The plan calls for "due, deferred, calendar entries" in
Forecast; deferred tasks were missing.

OmniFocus lets one configure Forecast, among other things with a
forecast tag whose tasks stand under "Today".

Chosen: flagged tasks as a section of their own (switchable), a forecast
tag, deferred tasks, planned and due on both days, a configurable span, a
day strip, today's completed tasks stay visible, the rule in `samu-core`;
the settings per account and synced, offline too.

## Decision

1. **The rule lives in `samu-core`** (`forecast`), like availability (ADR
   0006). It takes the snapshot, the evaluation, the settings and "now"
   and returns days: "Overdue", then today to the end of the span, then
   "Later". Each row says why it stands on that day: `due`, `planned`,
   `defer`, `tag`, `done`, and for appointments `event` (ADR 0047 §10).
   After today comes a section **"Flagged"** with the flagged available
   tasks (switchable). The scope applies as everywhere (ADR 0035, 0043).
2. **Route `GET /forecast`** with `scope=`, and `samu forecast` in the
   CLI. The web UI computes nothing itself any more; the app calls
   `samu-core` offline.
3. **What stands on a day:**
   - due on that day (`due`);
   - planned on that day (`planned`), even if due another day, then on
     both days, each with its reason;
   - becomes available on that day because its defer date reaches it
     (`defer`), when switched on;
   - only under today: with the forecast tag or one below it (`tag`), when
     one is chosen; completed today (`done`), when switched on.

   A task stands once per day; if several apply, the first line above
   wins. Across days and the "Flagged" section it may stand several times,
   each row with its reason: a flagged task due tomorrow stands under
   "Flagged" and under tomorrow as "due".
4. **Settings per account as the object `settings`**, a kind of its own,
   exactly one per account, created by the migration. It travels over
   `/changes` like every object and is changed with `set` per field (LWW,
   ADR 0004), offline too from the app's outbox. The server rejects
   creating and deleting it. Fields from this record: `forecast_days`
   (default 7), `forecast_flagged` (on), `forecast_tag` (none),
   `forecast_defer` (on), `forecast_done` (off). `due_soon` and
   `remind_time` moved here from the `account` table, so there is one
   place for settings; `/session` still returns both, read from the
   object. The zone stays on the account. Later records add fields here:
   `forecast_events` (ADR 0047 §10), `property_links` (ADR 0054 §3),
   `default_calendar` (ADR 0068 §10), `keys` (ADR 0079 §2),
   `report_worked` (ADR 0080 §1). Settings that belong to one device (such
   as the reminder switches of ADR 0101 §6) are not here.
5. **Changing** in web, app and CLI: `samu settings` (show, set) over
   `/changes`; in web and app a section "Forecast" in the settings menu
   (ADR 0035 §8, 0037). No route of its own.
6. **Day strip** at the top of Forecast, in web and app: the days of the
   span with their count, overdue in red; a click jumps to the day.
7. **Later:** calendar entries (since ADR 0047 §10 in the forecast),
   precomputed repetitions.

## Consequences

- `samu-api` has the kind `settings` with `AccountSettings`, and
  `ForecastDay`/`ForecastRow`, with fixtures. An older reader sees the
  kind as `unknown` (ADR 0015).
- `samu-store` migration: rebuild `object` for the CHECK on `kind` (as
  ADR 0030), table `settings` with one row, the values of `due_soon` and
  `remind_time` taken from `account`.
- Web and app lost their own forecast computation; the "Today" widget
  (ADR 0028 §9) uses the same rule.

## Changes

- 2026-11-09 (ADR 0047): §7 left appointments for later.
