---
number: 0081
title: "Report" replaces "Time"; booking time afterwards with z: in capture and edit, a button in the report, the app
status: accepted
date: 2026-11-12
adds_to: ["0011", "0046", "0078", "0080"]
replaces: ["0080 §5 (Report beside Time)"]
---

# 0081 – "Report" replaces "Time"; booking time afterwards

## Context

Two additions to ADR 0080, decided the same day before it was built:

1. The view "Time" is replaced by "Report"; ADR 0080 §5 had said the
   opposite, "Report" beside "Time".
2. Work should be trackable afterwards: create a new task and book time
   on it backwards, or book time backwards on an existing one.

What existed for booking afterwards (ADR 0046 §1): a finished clock is an
ordinary `clock` with `start` and `end` over `/changes`. The web's
inspector had "Add entry" under "Time" with two fields for date and
time; the CLI had `samu clock add <id> --start --end`; the app had
nothing. Creating a task and booking time on it in one step was possible
nowhere, and a bare duration ("yesterday 2 h") not at all.

## Decision

### 1. "Report" replaces "Time" (changes ADR 0080 §5)

The view "Time" is gone. "Report" takes its place with the tabs Done,
Worked on and Time, as described in ADR 0080 §5. The browser's choice for
the time view (`samu.time`) is taken over into `samu.report` on first
opening. The key `g z` of ADR 0079 §6 opens "Report" on the tab "Time".
The "Time" section in the inspector stays, and so do `/clocks/report`
and `samu clock report`.

### 2. The token `z:` in quick capture

Quick capture (ADR 0011) gains **`z:`** for booked time, in three forms:

- `z:1h30` or `z:45m`: a **duration** that ends now.
- `z:yesterday 2h` or `z:mon 1h`: a **duration on a day**. It begins at
  the end of that day's last clock, or at 09:00 if the day has none.
- `z:yesterday 14:00-15:30` or `z:14-15:30`: **from–to**; without a day,
  today.

`z:` may appear several times; each makes a clock of its own. A clock
ending in the future is refused (warning `clock_in_future`). The
computation lives in `samu-core` (`capture::parse`, account zone), so it
is the same offline in app and replica.

Both cases are thus covered:

- **A new task with time:** `n`, then `Review offer #client
  z:yesterday 14-16`. `POST /capture` creates the task and its clocks in
  one go; the answer lists them under `clocks`.
- **An existing task:** `e` (the "Edit" dialog, ADR 0078), then `z:mon
  1h30`. The clocks are added; existing ones stay.

A task to be created as already completed adds **`x:`**. Without a value
it means "completed now"; `x:yesterday` means "completed at the end of
the last clock".

### 3. Button "Book time" in the report

The report has a button **"+ Book time"** at the top. It opens the
capture window prefilled with `z:`. In the tab "Time" every task row has
a "+" that opens that task's "Edit" dialog with `z:`. The preview shows
the clocks as chips ("⏱ yesterday 14:00–16:00 · 2:00").

In the inspector "Add entry" becomes one field in the syntax of `z:`
(`yesterday 2h`, `14-15:30`); the two fields for date and time stay
available folded as "exact".

### 4. App

The same token in the app's capture. On the task page, under "Time", a
"Book" field in the syntax of `z:`, with preview. Both work offline
through the outbox.

### 5. CLI

`samu add "… z:yesterday 2h"` works as everywhere. `samu clock add <id>
<z-text>` takes the text as an alternative to `--start/--end`.

## Consequences

- `samu-api`: `CaptureResponse.clocks`, the warning `clock_in_future`,
  the marks `clock` and `completed` in `CaptureMark`, with fixtures.
  catastr is untouched.
- `samu-core`: `z:` and `x:` in `capture::parse`, placing a duration on a
  day, tests with zones and midnight.
- Server and replica: `POST /capture` writes clocks and `completed_at` in
  the same transaction; with `task` (ADR 0078) the clocks go to the
  existing task.
- Web: the view "Time" is gone (`clock.js` keeps status line and
  inspector); "Report" has the button and the "+" per row; "Add entry"
  in the inspector takes the syntax.
- App: the token in capture and "Book" on the task page.

## Changes

None yet.
