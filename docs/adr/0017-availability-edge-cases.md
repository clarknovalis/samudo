---
number: 0017
title: Availability edge cases – a date counts from midnight in the account's zone, overdue from the next day, due soon P2D, unknown counts as not active, tags inherit, single-action lists never stuck, cycles without a crash
status: accepted
date: 2026-11-07
adds_to: ["0006", "0007", "0015"]
---

# 0017 – Availability: edge cases

## Context

ADR 0006 sets the eight rules of availability and computes them as a pure
function in `samu-core`. While building it, edge cases were open: when a
date is reached, what `unknown` (ADR 0015) means, how tags, sequences and
broken data count.

## Decision

1. **Input** is a snapshot of the account (folders, projects, tasks, tags,
   links as `samu-api` types, without tombstones), "now", the account's
   zone (ADR 0007 §2) and the default for "due soon". The function reads
   nothing itself.
2. **Dates:**
   - Defer as a date is reached from 00:00 of that day in the account's
     zone; as an instant, from that instant.
   - Due as a date is overdue from the next day; the whole day still
     counts. As an instant, from that instant.
   - Due soon: `due − due_soon ≤ now`. The account's default is **P2D**,
     as in OmniFocus; the setting lives in `settings` (ADR 0045 §4).
   - Today: `planned` is today or earlier and the task is open.
   - A date form `unknown` makes a task unavailable for defer; for due and
     planned it is skipped.
3. **An `unknown` status** of project, folder or tag counts as not active:
   unavailable, reason "unknown value".
4. **Tags pass their status down:** a parent tag on hold puts its children
   on hold too.
5. **Sequential:** next is the first open action by `position`, even if
   deferred; the following ones wait anyway (as in OmniFocus). This holds
   in a project for the tasks without `parent`, and in every sequential
   group for its children.
6. **Inbox** (no project): available by the other rules. A task whose
   project points nowhere counts as inbox (ADR 0036 §6).
7. **`blocked_by`** blocks while the target task is open. A target that is
   not in the snapshot (unresolved, another account, deleted) does not
   block.
8. **Stuck** is a project that is active, not deferred, and has neither an
   available nor a deferred open action. **Single-action lists are never
   stuck.**
9. **Broken data** (a cycle in parents, folders, tags or `blocked_by`; a
   `parent` in another project) never leads to a crash or an endless
   loop: the affected task is unavailable with the reason "cycle" or
   "unknown value". The store refuses moves that would create a cycle
   (ADR 0037 §2).
10. **Result** per task: available yes/no and **all** reasons, plus `due`
    (`none | soon | overdue`) and `today`; per project available and
    stuck. The result types came into `samu-api` with fixtures once a
    route delivered them (`TaskStatus`, ADR 0020).

## Consequences

- The account's zone and the default "due soon" are account settings.
- On Android `samu-core` needs time zone data as well; it comes through
  `jiff`, as everywhere else.

## Changes

- 2026-11-08 (ADR 0036): §6 did not cover tasks whose project was
  deleted.
- 2026-11-09 (ADR 0045 §4): `due_soon` lived on the account row.
