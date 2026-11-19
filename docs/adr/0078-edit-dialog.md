---
number: 0078
title: The "Edit" dialog – change an existing task in quick-capture syntax, with its values as removable chips
status: accepted
date: 2026-11-12
adds_to: ["0011", "0026", "0027", "0062", "0073"]
---

# 0078 – The "Edit" dialog

## Context

The wish: an edit dialog for a task, reachable by a key, showing values
already set as chips with a remove "×", where tags and the like can be
added.

What existed:

- **The inspector** (ADR 0026, 0072): every field has its own row and
  control; complete, but made for the mouse and spread over the page.
- **Quick capture** (ADR 0011, 0027): `#project @tag ! !A d: s: p: ~15m
  every: // note` in one line, parsed by `samu_core::capture::parse`, with
  chips previewed through `POST /capture` with `preview`; only for **new**
  tasks.
- **Templates** (ADR 0073): `/name` in the same line.
- **The context menu** (ADR 0062) with single entries for state, date and
  moving.

The fast way to change an existing task by keyboard, the way one captures
it, was missing.

## Decision

### 1. A dialog like capture, for an existing task

In the web the key **`e`** on the selected task opens the dialog "Edit";
so does the entry "Edit …" in the context menu, with the key beside it
(ADR 0062 §4). It looks like the capture window (ADR 0026 §5):

- **Title** as its own field at the top, prefilled and editable.
- **Chips** for everything that is set: project, tags, state, flag,
  priority, defer, due, planned, estimate and repetition, looking like
  capture's preview. Each chip has an **✕** that marks it for removal,
  shown struck through with ↶ to take it back.
- **An input line** below the chips in quick-capture syntax, tokens only:
  `@tag d:fri !A ~30m every:1w #project /template`. While typing, the
  preview shows new chips (highlighted) and suggestions for `#`, `@` and
  `/` as in capture. ⌫ in the empty line marks the last chip for removal,
  as in the search field.
- **Enter** saves everything at once, **Esc** discards.

The note stays in the inspector. `// …` in the line appends text to the
note instead of replacing it.

### 2. What the line means when editing

The rule lives in `samu-core` as `capture::apply_to(task, draft)` and is
applied alike by server, replica (ADR 0066) and app:

- **Tags** are added; existing ones stay. A tag from an exclusive group
  (ADR 0024) replaces the previous tag of the same group.
- **Single fields** (project, priority, dates, estimate, repetition, flag)
  overwrite the previous value.
- **A template** `/name` sets its tokens by the same rules. Its reminders
  are added where the task has none with the same anchor. Its free text
  (a title prefix, ADR 0120) leaves the title alone here and is neither an
  error nor a change.
- **Free text** in the line is refused (warning `free_text`), as in a
  template's text; the title has its own field.
- **Booked time** `z:` adds clocks and `x:` completes (ADR 0081 §2).
- Unknown `#project` and `@tag` are handled as in capture (ADR 0011 §5).

### 3. API: `CaptureRequest.task`

`CaptureRequest` has the field `task: Option<Id>`. When set, `POST
/capture` applies the line to that task instead of creating a new one;
with `preview` it only computes. The answer is the same `CaptureResponse`:
`task` is the task afterwards, `created` is `false`. Together with
`parent`, `task` is refused (400).

Title and removals go as ordinary changes over `/changes`, in the same
save and **before** the `POST /capture`. A ✕ is an `unset` (field) or the
removal of a tagging. Every change carries its HLC stamp, LWW as always
(ADR 0004).

### 4. CLI

`samu edit <id> "<line>"` (with `--preview`) calls the same route. `samu
set`/`unset` stay for single fields.

### 5. Several tasks

Later, as a step of its own: only the line then, without a title, and the
chips show what all have in common.

### 6. App

With the same `CaptureRequest.task`: "Edit" in a task's menu (long press,
ADR 0102 §7). Offline `capture::apply_to` runs in `samu-mobile`.

## Consequences

- `samu-api`: `CaptureRequest.task`, warning `free_text`, with a fixture.
  catastr is untouched; it calls no capture.
- `samu-core`: `capture::apply_to` with tests (tags added, a group tag
  replaced, fields overwritten, template, free text).
- Server and replica: `POST /capture` with `task`, offline at the replica
  too since the computation is in the core.
- Web: `edit.js` beside `capture.js`, key `e`, the context menu entry, a
  line in the help (`KEYS`).
- CLI: `samu edit`. No migration.

## Changes

- 2026-11-12 (ADR 0081): §2 knew no booked time.
- 2026-11-17 (ADR 0120): §2 did not say what a template's free text does
  on an existing task.
