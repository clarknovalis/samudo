---
number: 0141
title: Checklists – items of a task as objects of their own, typed fast, turned into subtasks and back
status: draft
date: 2026-11-21
adds_to: ["0006", "0016", "0058", "0091"]
---

# 0141 – Checklists

## Context

A shopping list or a packing list is a run of short lines to tick off.
Today there are two ways to write one, and both are awkward:

- Checkboxes in the note (0058) are text. They tick, but each one is typed
  as `- [ ] ` by hand, they cannot be reordered without editing text, and
  two devices editing the same note merge as one field.
- Subtasks are tasks: each has availability, a place in every list and
  the perspectives, and a project tree to clutter. A list of twelve
  groceries should not be twelve rows of the inbox.

Other tools keep the two apart: a checklist inside a task, with a quick
way to turn a line into a task. Entering is the hard part: one line per
item, Enter for the next one, and a pasted list that falls apart into
items.

## Decision

1. **A new kind of object, `checklist_item`.** `owner` (a task), `text`
   (one line), `checked` (default false), `position` (order among the
   items of the owner). Each field has its own stamp, so two devices
   ticking different items, or one ticking while the other renames,
   both win (0004, 0016). `owner` never changes; the others may.
   `samu-api` 0.1.63, additive. Projects and subtasks of a checklist
   item do not exist; an item is neither available nor unavailable.
2. **It goes with its task.** Deleting a task makes tombstones of its
   items in the same change, like the entries of a habit (0091 §2).
3. **Typing and pasting.** The text of several lines is read as several
   items. What a list in Org, Markdown or a note app puts in front of a
   line is dropped: `- `, `* `, `+ `, `1. `, `1) `, and a checkbox, where
   `[x]` ticks the item. Blank lines are skipped, indentation is
   ignored; there is one level. Clients offer the fast path: Enter
   makes the next item, Backspace on an empty one removes it, paste
   splits. `samu checklist add <task> -` reads stdin.
4. **Progress** is `done/all` of the items, shown beside the task.
   It is computed where it is shown, from the items; nothing stores it.
5. **Converting.** An item becomes a subtask: a task under the owner with
   the item's text as title, at the end of its siblings, completed when
   the item was ticked; the item goes. A subtask becomes an item the
   other way, but only a plain one: no children, no habit, no repetition,
   not dropped, and nothing an item cannot keep (note, dates, tags,
   estimate, flag, priority, properties). Anything else is refused and
   says why, instead of losing it silently. Both are one batch of
   ordinary changes, so there is no route of their own; `samu-core`
   plans them (`checklist::to_subtask`, `from_subtask`) and the web UI
   repeats the few lines, held to the same cases.
6. **Not changed:** checkboxes in notes stay as they are (0058); no
   migration turns them into items. `samu import org` leaves them in
   the note.

## Consequences

- One migration (0029): the `object` kind list and one table.
- A server that does not know the kind reads it as `unknown` (0015) and
  an older client simply does not show the list.
- The CLI covers it whole: `samu checklist list|add|check|uncheck|edit|
  move|rm|to-subtask|from-subtask`, and `show --full` carries the list.

## Changes

- 2026-11-21: draft.
