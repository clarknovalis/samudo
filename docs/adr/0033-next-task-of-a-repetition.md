---
number: 0033
title: The next task of a repetition – crate rrule, anchor, end, what comes along; a repetition without a date gets its first one
status: accepted
date: 2026-11-08
adds_to: ["0007"]
---

# 0033 – The next task of a repetition

## Context

ADR 0007 decides *that* completing a repetition makes a new task with
`repeat_of`, and which three modes there are. Left open were: which
crate, from which date the next one is counted, when a chain ends, what
the new task takes along, and what happens on reopening or on a change
sent twice. `samu-api` already has every field (`Schedule.repeat`,
`Task.repeat_of`); the Org import translates Org repeaters.

## Decision

1. **Crate `rrule` 0.14** (MIT/Apache-2.0, maintained, full RFC 5545
   with `BYDAY` and ordinals, `BYMONTHDAY`, `BYSETPOS`, `UNTIL`,
   `COUNT`, zones), only in `samu-core`, behind `samu_core::repeat`. It
   computes in chrono and chrono-tz; dates go to it and come back to jiff
   as ISO text, and chrono is used nowhere else. The price: a second time
   library and zone database in the server and in the app.
2. **When:** when `completed_at` of a task with `repeat` goes from empty
   to set, in the same transaction in the store (ADR 0004). No next task
   when there already is one with `repeat_of` = this one, even a deleted
   one: the same change twice, reopening and completing again never make
   a second. Dropping makes none.
3. **Anchor and new date:**
   - `fixed`: the anchor is `due`, else `defer`, else `planned`; the new
     date is the first occurrence of the rule after the anchor. With
     `catch_up` the first one after today (in the account's zone).
   - `due_after_completion` / `defer_after_completion`: the anchor is the
     moment of completion — as a local date in the account's zone when
     the date is a date; otherwise with the time of day and zone of the
     old date, on the day of completion. The new date is the first
     occurrence after it.
   - The other dates move by the same distance (ADR 0007 §5); a date
     stays a date, an instant keeps its time of day and zone across a
     change of daylight saving. `due_warning` and `planned_duration` stay.
   - **A repetition without any date gets its first one when it is
     set**: when a task is created with `repeat`, or `repeat` is set on
     it, and it is open and has neither `due` nor `defer` nor `planned`,
     the store writes the first occurrence of the rule from today on (in
     the account's zone) as a date into `due` (into `defer` for
     `defer_after_completion`), in the same transaction and under the
     same stamp. A date written later than that stamp wins: a `due`
     cleared on one device after `repeat` was set offline on another
     stays empty. A rule that makes no date from today on fills nothing.
   - A task that still has no date when completed (one from before this
     rule, or whose date was cleared) gets the new date as `due` (as
     `defer` for `defer_after_completion`), as a date.
4. **End:** when the next occurrence lies past `UNTIL`, or the chain
   (over `repeat_of`, the first one counted) has `COUNT` links, there is
   no next task. A rule `rrule` cannot read is refused by the store
   already when `repeat` is set.
5. **What comes along:** title, note, project, parent, tags with their
   position, the flag, the estimate, `sequential`,
   `complete_with_last_child`, `repeat`, a position right after the old
   one; properties except `org_id` (ADR 0023, unique). Reminders relative
   to `due`, `planned` or `defer` (ADR 0030) are copied as new
   `reminder` objects, absolute ones are not. The completed task keeps
   everything, `repeat` too.
6. **Stamp:** the new objects carry the HLC of the completing change and
   new UUIDv7s. No new field, no new route: no new version of
   `samu-api`.
7. **Not now:** a task with children (an action group) makes no next
   task; the logbook says why on completion. Repeating projects neither.
   Each is a task of its own. The app shows the next task only after a
   sync; the offline overlay (ADR 0028 §4) does not compute it, nor the
   first date of §3.

## Consequences

- `samu_core::repeat` with `check`, `first` and `next`; the store calls
  them.
- When `next` computes an error, the task stays completed; the logbook
  names the reason.
- New dependencies `rrule`, `chrono`, `chrono-tz` in `samu-core` and so
  in `samu-store`, `samu-server` and `samu-mobile`.
- A repetition without a date shows its due date in every list, in
  Forecast and in "Overdue" like any other, instead of only the preview
  of the next one in the inspector.

## Changes

- 2026-11-23: §3 left a repetition without a date without one until it
  was completed; a list row showed no due date, only the inspector the
  preview of the next one. Since then the store fills the first date when
  `repeat` is set. Tasks that already repeated without a date keep
  theirs empty until they are completed or `repeat` is set again.
