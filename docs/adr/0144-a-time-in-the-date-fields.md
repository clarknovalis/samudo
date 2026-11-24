---
number: 0144
title: A time of day in the date fields – every editor of defer, due and planned writes a day or a day with a time, read as quick capture reads it
status: accepted
date: 2026-11-24
adds_to: ["0007", "0011", "0017", "0040"]
---

# 0144 – A time of day in the date fields

## Context

Defer, due and planned are each a local date or an instant with its
zone (0007 §1). Quick capture (0011) writes both: `p:tomorrow 14:30` is
an instant in the account's zone, `p:tomorrow` a whole day. Editing with
marks (`samu edit`, key `e` in the web and in Emacs) applies the same
words to an existing task. The model, the wire types and the
availability (0017 §2) know a time; a task planned at a time is drawn in
the calendar at that time.

The date fields of the editors write only days:

- the web inspector has `<input type="date">`, and changing the day there
  sends `{date}`, which **drops a time the task had**;
- the app has the system's date picker; 0040 §2 left times to quick
  capture;
- Emacs reads `planned` and `due` with `org-read-date` but passes on the
  day only, and taking over an Org heading drops the time of its
  `SCHEDULED` and `DEADLINE`;
- the CLI's `samu date` takes `today`, `tomorrow`, `YYYY-MM-DD` or `none`.

Someone coming from Org mode sets a time where the date is set, not in a
second syntax. And a field that silently loses part of its value is a
bug, whatever else is decided.

## Decision

1. **Every date field writes both forms**, for defer, due and planned
   alike: a day, or a day with a time. Without a time it stays a whole
   day; no editor adds a time by itself.
2. **A time is read in the account's zone** (0007 §2), as quick capture
   reads it, never in the zone of the device.
3. **The day and the time change apart.** Changing the day keeps the
   time, changing the time keeps the day, and taking the time away makes
   it a whole day again. Clearing the field clears both.
4. **One reading of dates** (0011): where an editor takes text, it takes
   the date words of quick capture, with an optional time after them,
   from the one parser in `samu-core`.
5. **Per editor:**
   - **CLI:** `samu date <field> <value>` takes those words:
     `samu date planned "tomorrow 14:30"`, `"2026-11-25 9:30"`,
     `fri`, `none`.
   - **Web:** a time field next to the date field of each of the three;
     empty means a whole day.
   - **App:** after the date picker an optional time picker, and a way
     back to a whole day. This replaces the last sentence of 0040 §2.
   - **Emacs:** the time typed into `org-read-date` is passed on; taking
     over an Org heading keeps the time of `SCHEDULED` and `DEADLINE`. A
     repeater is still left behind.
6. Nothing else changes: no field in `samu-api`, no migration. A deferred
   instant is available from that instant (0017 §2); a repetition keeps
   the time of day (0007 §5).

## Consequences

- Each editor shows a task's time where it shows its day.
- The web fix of §3 (a day changed without losing the time) is worth
  making on its own, before the rest.
- The interface to other programs is unchanged: both forms of a date
  exist on the wire since 0007.

## Changes

None yet.
