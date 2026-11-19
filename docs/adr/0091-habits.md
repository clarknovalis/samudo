---
number: 0091
title: Habits – a task with the field habit (kind, target, unit), entries as objects of their own, streak and rate in samu-core
status: accepted
date: 2026-11-13
adds_to: ["0006", "0011", "0072", "0073"]
---

# 0091 – Habits

## Context

The first idea was a habit as a daily repetition (`fixed` with
`catch_up`): every check leaves a completed task (ADR 0007 §4, ADR 0033),
and a gap in the `repeat_of` chain would be a missed day. Templates with
`every+:` (ADR 0073, 0084), `scope_only` (ADR 0069, 0070) and the
inspector layout "Habit" (ADR 0072) were built for that.

The use cases, numbered:

| No. | Case | When |
|---|---|---|
| 1 | done (yes/no on a day) | now |
| 2 | avoided: record only relapses, "free for n days" | now |
| 3 | failed: explicitly not done, unlike "not recorded" | now |
| 4 | amount with a daily target, several +1 a day (water) | now |
| 7 | daily | now |
| 8 | rate per period (strength training 3× a week) | now |
| 11 | pause (holiday, illness) without breaking the streak | now |
| 12 | recording a past day | now |
| 5 | limit: at most n | later |
| 6 | duration in minutes, optionally from the clock | later |
| 9 | fixed weekdays | later |
| 10 | time window with a reminder | later |
| 13 | note on an entry | later |
| 14 | time-boxed (challenge with an end) | later |
| 15 | minimal version | later |

A chain of repetitions carries only 1, 7 and 9. A task is done or not; it
has no amount, no fail, no pause, and two devices that each tap +1
offline give no sum. It would also add 365 completed tasks per habit and
year to logbook, report and search. And a task with `FREQ=DAILY` gets a
`due` on completion (ADR 0033 §3) and is overdue the next day.

## Decision

### 1. A habit is a task with the field `habit`

No new kind for the habit itself. A task is a habit when its field
`habit` is set. Project, tags, note, scope, `scope_only` and `samu:`
links work as on any task. A tag named `habit` has no meaning of its own.

`habit` is a value that changes as a whole, with one stamp, like
`Template.reminders`:

- **`kind`**: `build` or `avoid`. An unknown value is read per ADR 0015,
  never written.
- **`times`** and **`per`**: the target, `times` per `per` (`day`,
  `week`, `month`); default 1 per `day`. The week starts on Monday, the
  month is the calendar month, the day is in the account's zone (ADR 0007
  §2). For `avoid` the target is not evaluated (case 5 will use these
  fields).
- **`unit`**: optionally a word for the amount, such as "glass". Without
  a unit every entry counts 1, and `times` is the number of entries.

A habit has no repetition and no dates. Availability (ADR 0006) treats it
like any open task without dates. Completing or dropping it ends the
habit; its entries stay. A habit cannot be turned into a project (ADR
0126 §7).

### 2. Entries: new kind `habit_entry`

Synced over `/changes` like every object (ADR 0004), so offline in app
and replica too. Fields:

- **`habit`**: the id of the task.
- **`day`**: the local date the entry is for. It may be a past day
  (case 12), not a future one; the store rejects a later `day`.
- **`status`**: `done`, `fail` or `pause`.
- **`amount`**: an integer, default 1, only with `done`.
- **`until`**: only with `pause`, the last day of a pause over several
  days.

**Every tap is an entry of its own.** A day's amount is the sum. Two
devices that each write +1 offline give 2, not one of them by LWW. A
mistyped entry is deleted (tombstone); its fields never change.

Entries are not in the logbook: they are a record themselves, and the
report "Worked on" (ADR 0080) would overflow. `created` and `created_by`
show when and by whom an entry came.

### 3. The computation in `samu-core`

`samu_core::habit` computes a `HabitStatus` from habit, entries and
"today". No client computes it itself, as with availability (ADR 0006).

- **A day for `build`:** `done` when the sum of `done` amounts reaches
  the daily target (with `per = day`); otherwise `fail` if there is a
  `fail`; otherwise `pause` if a pause covers the day; otherwise
  `partial` for an amount below target; otherwise `none` (not recorded).
- **A day for `avoid`:** `fail` with a `fail` entry (relapse), otherwise
  `pause` within a pause, otherwise `done`. Counted from the habit's
  `created`.
- **Period:** achieved when the `done` amounts in it reach `times`. With
  `per = day` it is the day.
- **Streak:** the number of achieved periods in a row up to today.
  - The current period never breaks it; if already achieved, it counts.
  - A missed period with at least one pause day neither breaks it nor
    counts (case 11).
  - "Not recorded" and `fail` both break it; they differ in display (grey
    or red) and in the rate.
  - For `avoid` the streak is "free for n days": days since the last
    relapse, without pause days.
- **Longest streak** over the whole history.
- **Rate** of the last 4 weeks and of the current year: achieved periods
  over completed periods, without paused ones.
- **Today / this period:** amount, target and status; this gives the
  check in the row.
- **History:** the status per day over a requested range, from which the
  interfaces draw the calendar graphic.

Table tests as in ADR 0011, across week and month boundaries, daylight
saving and back-filling.

### 4. API

- `Task.habit: Option<Habit>` with `Habit`, `HabitKind` and `HabitPer`.
- `HabitEntry`, `HabitEntryStatus`, `ObjectKind::HabitEntry` and
  `AnyObject`.
- **`GET /habits?day=&from=`**: all open habits in the scope with
  `HabitStatus`, optionally with the history from `from`.
- **`GET /habits/{id}`**: one habit, also a completed one.

Writing only over `/changes`. The replica (ADR 0066) answers both routes
from mirror and outbox.

### 5. Capture

Quick capture (ADR 0011) knows `habit:`:

- `habit:` or `habit:1/d` daily, `habit:3/w` three times a week,
  `habit:2/m` twice a month;
- `habit:8/d:glass` an amount with a unit;
- `habit:avoid` avoiding.

So a template `/habit` captures a habit in one go, for instance
`#Guardrails @habit habit:1/d`. The "Edit" dialog (ADR 0078) takes the
same syntax, also for existing habits.

### 6. Interfaces

- **Row in every list** (web; the app per ADR 0092): instead of the
  circle, `build` has a ✓ button; with a unit +1 and `5/8` beside it;
  `avoid` shows "free for n days" with a "Relapse" button. Fail and pause
  are in the context menu. The streak stands beside it as a small number.
- **Inspector:** part `series` (foreseen in ADR 0072 §6) with streak,
  longest streak, rate, a calendar graphic of the last 12 weeks, and the
  chosen day's entries for deleting. A click on a past day records there.
- **CLI:** `samu habit list` (habits in the scope with today's status),
  `samu habit show <id>` (status and history), `samu habit done <id>
  [--day D] [--amount N]`, `samu habit fail <id> [--day D]`, `samu habit
  pause <id> --from D [--until D]`, `samu habit entries <id>`, `samu
  habit undo <entry>`, `samu habit set <id> <as habit:>`.

### 7. Not now

- Cases 5, 6, 9, 13, 14 and 15, each with a record or an addition of
  its own. Case 10 (reminder) came with ADR 0093: a reminder at a time of
  day that stays quiet once the day's target is met.
- App, Emacs package and desktop widget followed web and CLI (ADR 0092
  for the app).
- No view "Habits today": a scope "Habit" with the row of §6 covers it.

## Consequences

- `samu-api`: the types of §4 with fixtures, version 0.1.42. catastr
  reads neither `habit` nor `habit_entry` and skips an unknown kind and
  field (ADR 0015).
- `samu-store`: a migration adds the column `habit` to `task` and the
  table `habit_entry` with an index on `(habit, day)`. The store checks
  that `day` is not in the future, that `amount` and `until` appear only
  with their status, and that `habit` points to a task with `habit`.
  Deleting a task takes its entries along, after ADR 0076.
- `samu-core`: `habit` (§3), the `habit:` token in the parser and in
  `capture::apply_to`.
- Server: the routes of §4. Export and restore (ADR 0074) carry
  `habit_entries.json`.
- Web: row, inspector part `series`, context menu, recognising `habit:`.
- The `repeat_of` chain stays for repetitions as it is.

## Changes

- 2026-11-13 (ADR 0093): §7 left habits without a daily reminder,
  because reminders hung on dates (ADR 0030).
