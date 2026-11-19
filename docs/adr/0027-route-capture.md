---
number: 0027
title: Quick capture over the route POST /capture – preview, tags created on the way, suggestions while typing
status: accepted
date: 2026-11-08
adds_to: ["0011", "0019", "0020", "0026"]
---

# 0027 – Quick capture over `POST /capture`

## Context

The parser of ADR 0011 lives in `samu_core::capture`. The CLI depends
only on `samu-api` and cannot call it itself; the web UI is not to compute
anything on its own anyway (ADR 0025 §3). Both need it through the
server. Building the parser settled a few points differently from ADR
0011; they are recorded here.

Chosen: an unknown tag is created on the way; tags (and projects) are
suggested while typing, so typos do not arise and input stays fast; ⇥
creates the task and opens it in the inspector.

catastr does not call this route.

## Decision

1. **Route** `POST /capture` with `{text, project?, preview?}`. The server
   parses in the account's zone (ADR 0007), resolves names and creates.
   `project` is the default when the text has no `#` (in the web UI the
   view's project). With `preview: true` it computes the same and writes
   nothing. Later fields: `scope` (ADR 0043 §6), `template` (ADR 0073 §5),
   `parent` (ADR 0062), `task` to edit an existing task (ADR 0078 §3);
   several lines go to `POST /capture/lines` (ADR 0129).
2. **Answer:** the task as created or as it would be; `created`; the names
   of the tags created (or, in a preview, to be created); the recognised
   spans (`start`, `end`, `meaning`); the warnings as codes. Later also
   the reminders (ADR 0073) and clocks (ADR 0081) created. Types in
   `samu-api` with fixtures, unknown values per ADR 0015.
3. **Tags:** an `@name` is resolved by name like a project (exact,
   otherwise a unique prefix; ADR 0011 §2), `@group/child` by path. If it
   hits no tag, the tag is **created**: active, not exclusive, at the top
   or below the existing group. Never below an exclusive group, whose
   children are a workflow's states (ADR 0024); then `@…` stays in the
   title with a warning. A prefix that hits several tags also stays in the
   title with a warning. Projects are still never created (ADR 0011 §5).
4. **Departures from the wording of ADR 0011:**
   - `parse` has no language parameter; warnings are codes and every
     interface words them itself.
   - Spans are UTF-8 byte offsets into the text.
   - `every!:` is the mode `due_after_completion` (ADR 0007).
   - `20.11.` without a year is the next such day, today included.
   - A second project, date, estimate or repetition stays in the title
     with a warning.
5. **Web UI** (capture window, ADR 0026 §5): while typing the window asks
   for the preview with a delay and shows the spans as chips with meaning
   and resolved value ("Project: New bathroom", "Tag new: …", "Due: Fri
   13.11."), plus the warnings in words. With the cursor in a `#…`, `@…`
   or `/…`, a suggestion list of known projects, tags or templates appears;
   ↑/↓ chooses, ⇥ takes it. Without open suggestions ↵ creates, and ⇥
   creates and opens the task in the inspector. That is the "⇥ opens the
   form" of ADR 0020 §3; there is no form of its own.
6. **CLI:** `samu add <text…>` goes over `POST /capture`. `--dry-run`
   shows the preview, `--literal` takes the text literally as a title
   (over `POST /changes`), `--project` is the default, `--flag` and
   `--note` stay. Warnings and new tags go to stderr; exit 0.

## Consequences

- `samu-core` has, beside `parse`, the pure function `resolve`, which
  resolves names against projects and tags and describes new tags; the
  ids come from outside, so `samu-core` stays without randomness.
- The server creates by applying changes as `POST /changes` would, with
  server stamps: new tags, the task, then one `add` per tag. So the store's
  rules for workflows hold here too.
- The suggestion list is display from the page's cache, not computation
  (ADR 0025 §3); the suggestions of the search are computed in
  `samu-core` (ADR 0103).

## Changes

None yet.
