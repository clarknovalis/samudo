---
number: 0046
title: Clocking – one running clock, stopped on completion, evaluated in samu-core
status: accepted
date: 2026-11-09
adds_to: ["0006", "0020", "0028", "0043"]
---

# 0046 – Clocking: one running clock, stop on completion, evaluation in `samu-core`

## Context

ADR 0006 has the **clock** in the data model: task or project, start, end
(empty = running), optional note; at most one running clock per account;
start and stop in every interface; evaluation per project, folder, tag and
period; sum against `estimate`. The object had been in `samu-api` and the
schema from the start, but nothing used it. ADR 0043 §5 reserved the chip
`is:clocked` until clocking existed.

Open was how "at most one running clock" holds when two devices start one
offline, and whether a clock keeps running on completion.

Chosen: the newer clock stops the older; completing stops it; evaluation
splits at midnight in the account's zone; the first round covers core,
CLI, evaluation, web UI and Android app, without importing org's `CLOCK:`
lines.

## Decision

1. **Start and stop are ordinary changes** over `POST /changes`: start
   creates a `clock` without `end`, stop sets `end`. No route of its own;
   offline both go through the outbox (ADR 0016). Booking afterwards is a
   `clock` with `start` and `end` (in text form `z:`, ADR 0081 §2).
2. **One running clock per account, the newer wins.** A pure function in
   `samu-core` decides which clocks end how: when a running clock arrives,
   every other running one ends at the new one's start. If the arriving one
   is older than a running one, it ends itself at the newer one's start.
   "Newer" means later `start`; with equal `start` the larger id wins. The
   store applies the function in the same transaction and writes the ends
   as ordinary stamped field changes, which reach all devices over
   `/changes` and the event stream. The app applies the same function
   offline.
3. **Completing or dropping stops it.** When `completed_at` or
   `dropped_at` of a task or project is set, a running clock on it ends at
   that instant, in the same transaction (like the follow-up, ADR 0033).
   Reopening starts nothing.
4. **Checks in the store:** `end` is not before `start`; the owner is a
   task or a project.
5. **Evaluation in `samu-core`**, over `GET /clocks/report` with `from`,
   `to` (local dates, `to` inclusive), `by` (`task`, `project`, `folder`,
   `tag`, `day`) and `scope` as for `/forecast`. A clock counts only with
   its share inside the period; day boundaries are the midnights in the
   account's zone. A running clock counts up to now. A clock on a project
   counts for the project, not for a task. Per row: key, title, seconds;
   for `task` also `estimate`. A task with several tags counts under each
   for `tag`; a total counts it once. No client computes this itself; the
   app uses the same function offline. The report (ADR 0080) builds on it.
6. **`is:clocked`** (`ist:geclockt`) matches a task with at least one
   clock that is not deleted, in a scope's filter (evaluated in
   `samu-core`) and in the search. This lifts the reservation of ADR 0043
   §5.
7. **Web UI:** a status line with account, running clock (title, running
   time, a click stops) and sync (ADR 0020 §4); a key clocks the selected
   task in or out; in the inspector a section "Time" with the clocks, the
   sum against the estimate, adding and deleting entries. The view "Time"
   with period and grouping of §5 was replaced by "Report" (ADR 0081 §1).
8. **Android app:** start and stop on the task page and in the
   long-press menu (ADR 0040), the task's clocks with their sum; the
   running clock as a persistent notification with running time and
   "Stop". Booking afterwards on the task page (ADR 0081 §4).

## Consequences

- `samu-api` has `ClockReport`, `ClockReportRow` and `ClockReportBy`
  (unknown value as `unknown`, ADR 0015), with fixtures. `Clock` itself is
  unchanged.
- No migration: the `clock` table existed.
- Route `GET /clocks/report` with the CLI `samu clock report`, and `samu
  clock in|out|status|list|add` over `/changes`.
- The ends of §2 and §3 are written by the server; they do not appear in
  the task's logbook.
- catastr needs no change.
- Later: importing org's `CLOCK:` lines, idle detection, Pomodoro.

## Changes

- 2026-11-12 (ADR 0081): §7 had a view "Time" of its own; booking
  afterwards needed start and end.
