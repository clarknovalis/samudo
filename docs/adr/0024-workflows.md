---
number: 0024
title: Workflows – exclusive tag groups with order, keys and a logbook
status: accepted
date: 2026-11-07
adds_to: ["0006", "0016", "0017", "0018", "0020"]
---

# 0024 – Workflows: exclusive tag groups with order, keys and logbook

## Context

The wish: set up workflows like org-mode's `TODO NEXT WAITING | DONE
CANCELED` and configure tags. Org packs three independent things into
one line: what is up next (`NEXT`, computed in OmniFocus), what is not
available right now (`WAITING`, in OmniFocus a tag on hold), and what is
done or dropped (in samu `completed_at` and `dropped_at`). A status field
of its own would undo that separation of ADR 0006 and create two truths.
What org does better is handling: cycling by key, a stamp and note on a
change, workflows per file.

Chosen: map OmniFocus completely and add org's handling, no second status
system; cycling as a round trip, completing separate; the full logbook
now; a default workflow in the account.

## Decision

1. **A workflow is an exclusive tag group** (ADR 0006): a tag with
   `exclusive = true`. Its children in `position` order are the states.
   Their meaning is the existing tag status: a state on hold makes the
   task unavailable (ADR 0017 §4). Every exclusive group is a workflow.
   Priority is a field of its own, not a group (ADR 0056).
2. **The server decides exclusivity as an LWW register per object:** an
   `add` of a child sets the state, a `remove` of a child clears it. Both
   take effect only when their HLC is greater than every stamp of the
   siblings on that object, present or removed; then the present siblings
   are removed with the same stamp. Otherwise the change is `superseded`.
   The result does not depend on the order of arrival (a test runs all
   orders). Older data with two present siblings stays until someone
   switches.
3. **Workflow per project:** `Project.workflow` names an exclusive group.
   Without one the group with `default_workflow = true` applies; if
   several carry it, the one with the smallest `position`, then the
   smallest id. Tasks in the inbox take the default.
4. **Keys and cycling:** `Tag.key` is one character for quick selection.
   Cycling is a round trip: no state → first → … → last → no state.
   Completing and dropping stay commands of their own; the state tag stays
   on completion. `NEXT` is no state: availability computes it (ADR 0006).
   A workflow with its states is created in one step (ADR 0132), and its
   fields can be changed afterwards under the rules of
   `samu_core::tagfields`.
5. **Logbook** (ADR 0006, ADR 0018 §8): the server writes, in the same
   transaction, an entry for every applied state change (`state`, with old
   and new state) and for setting `completed_at`/`dropped_at`
   (`completed`, `dropped`; back to empty: `reopened`). Every entry has a
   time, the device from the HLC, the kind of session (ADR 0039 §1) and
   optionally a note. The note comes as the field `note` on `set`, `add`
   and `remove` (ADR 0016). A repeated or superseded change writes
   nothing. Org's `!` is thus always on. Deletions and moves are logged
   too (ADR 0123).
6. **`Tag.ask_note`:** entering this state makes clients ask for a note
   (org's `@`). The server does not enforce it, since an outbox created
   offline must not fail.
7. **Reading:** `GET /objects/{id}/logbook`, type `LogEntry` in
   `samu-api`; the whole account's logbook is `GET /logbook` (ADR 0123
   §4). The logbook does not replicate over `/changes`; it is history, not
   state.
8. **Display:** interfaces show the state tag before the title (adds to
   ADR 0020). The board takes its columns from the workflow (ADR 0061).

## Consequences

- `samu-api` 0.1.4: `Tag.key`, `Tag.ask_note`, `Tag.default_workflow`,
  `Project.workflow`, `note` on `set`/`add`/`remove`, `LogEntry`, the new
  route; each with a fixture. catastr already reads tags (ADR 0003 §7)
  and can derive a task's state from the workflow, like its own `todo`
  field.
- The order of states and the round trip are pure functions in
  `samu-core` for web and Android; the CLI depends only on `samu-api` and
  computes the few lines itself.
- The org import stays general (BACKLOG → `Someday`, WAITING →
  `Waiting`); a workflow is set up with `samu tag …`.

## Changes

- 2026-11-08 (ADR 0039): §5 recorded only the device, not the kind of
  session.
- 2026-11-09 (ADR 0056): §1 named a priority `A/B/C` as an example of an
  exclusive group.
- 2026-11-18 (ADR 0123): §5 and §7 had no deletions, moves or
  account-wide reading.
- 2026-11-19 (ADR 0132): §4 had no way to create a workflow with its
  states in one step.
