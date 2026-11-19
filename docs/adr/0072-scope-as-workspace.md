---
number: 0072
title: A scope as a workspace – a scope presets how the interface looks, starting with the inspector
status: accepted
date: 2026-11-11
adds_to: ["0035", "0043", "0058", "0060"]
---

# 0072 – A scope as a workspace

## Context

The wish came up with habits and with objects shown only in their own
scope: in a scope "Habits" the inspector should behave differently,
showing only the note as rendered Markdown and what managing a habit
needs, with everything else folded away unobtrusively, chosen in the
scope's settings. Later this should hold for every scope and the whole
interface: left bar, right bar, detail views, filters. Start small.

A scope decides **what** one sees: members and mode (ADR 0035), inbox,
filter and capture (ADR 0043), colour (ADR 0060). **How** one sees it
depended on the device: sidebar stage (ADR 0055), arrangement (ADR 0064),
list or board (ADR 0061), all remembered in the browser. The inspector
showed everything in a fixed order (`web/lib/detail.js`): place, title,
id, properties, availability, state, the fields project to tags, note,
links, reminders, clock, comments and attachments, actions, logbook.

## Decision

### 1. A scope carries a layout

- New field `layout` on `Scope`, an object `ScopeLayout`. Without it
  everything looks as before.
- It changes as a whole, LWW like `capture` (ADR 0043 §6).
- **It lives on the server, not on the device**, so web and app show the
  same layout, as with the colour (ADR 0060). What one changes by hand on
  a device, such as unfolding something, stays on that device (§4).
- "No scope" has no layout.
- `ScopeLayout` is open for later parts (§6). A part a reader does not
  know is skipped and everything stays as before; an older client shows
  more, never less (ADR 0015).

### 2. First part: the inspector

`ScopeLayout.inspector` (`InspectorLayout`) has two fields:

- **`shown`**: a list of inspector parts (`InspectorPart`) with fixed
  names: `properties`, `availability`, `state`, `project`, `priority`,
  `defer`, `due`, `planned`, `repeat`, `series` (ADR 0091 §6),
  `estimate`, `tags`, `note`, `links`, `reminders`, `clock`, `comments`,
  `attachments`, `logbook`. An unknown name is read as `unknown` and never
  written (ADR 0015).
- **`note`**: how the note appears, `rendered` or `default`. Absent means
  `default`, the usual behaviour (ADR 0058): rendered while it has text,
  an input field when empty. With `rendered` an empty note appears not as
  an input field but as a quiet line "Write a note". Editing is as before,
  by double click or a button.

The order stays the inspector's; `shown` only selects.

**Always visible** and not part of the list: place and title, the actions
with complete or reopen and drop, and the buttons ‹ list and ⤢. Without
them a task cannot be handled.

### 3. What is not in `shown` is folded, not hidden

- Below the shown parts stands a quiet line, such as "More: Due, Tags,
  Logbook …". A click unfolds the rest in its usual place.
- **A folded part that has a value is named first and by name in that
  line**, so a due date or a tag does not vanish unnoticed; the same care
  as the header hint (ADR 0035 §6, ADR 0094 §4).
- Parts without a value are only counted.

### 4. On the device

- Unfolding holds per scope and device: the web keeps it in
  `localStorage` (`samu.inspectorOpen.<scope>`), the app in its cache.
- The same line folds again ("Less").
- Nothing of it goes to the server.

### 5. Setting it

- **Web:** in the scope's settings a section "Layout" with a checkbox per
  part of §2 in the inspector's order, the choice for the note, "Show
  everything" (removes `layout.inspector`), and a preset **"Habit"**:
  `note`, `repeat`, `planned`, `reminders`, note `rendered`. The preset
  only fills the checkboxes; what is stored is the list, not a name.
- **App:** reads the field and follows it in its task page.
- **CLI:** `samu scope layout <scope>` prints the layout;
  `--inspector note,repeat,planned,reminders` sets the list; `--note
  rendered|default` sets the note; `--clear` removes `layout`. `samu set
  <id> layout '<json>'` works as for any field.

### 6. Outlook, not decided

Every further part gets a record of its own that adds to this one.
Candidates: the left bar (its stage on entering the scope, ADR 0055;
which built-in views appear); a view on entering (such as "Today" in a
scope "Habits"); the list (arrangement default, ADR 0064; list or board,
ADR 0061; visible columns); a preset, removable search, which is not the
scope's fixed filter (ADR 0043 §2). The inspector part `series` for
habits came with ADR 0091 §6.

The same rule holds for all: the scope's layout is the default; what one
changes by hand on a device stays there until the scope changes.

## Consequences

- `samu-api` has `Scope.layout` and the types `ScopeLayout`,
  `InspectorLayout`, `InspectorPart` (with `Unknown`) and `NoteMode`
  (with `Unknown`), with fixtures `scope/layout.json` and one with an
  unknown part. catastr does not read scopes.
- `samu-store`: numbered migration with the column `layout` (JSON) on
  `scope`; the store checks it through serde on write, like `capture`.
- `samu-core` is untouched: the layout is presentation, not computation.
  The rule "a folded part with a value is named" lives in each client; it
  is small, and if it grows it belongs in `samu-core`.
- Web: `detail.js` builds its parts from a table name → block instead of a
  fixed sequence, plus the "More" line; `settings.js` has the section
  "Layout".
- Tests: fixture round trip; unknown part and unknown note mode read and
  skipped; the CLI setting, printing and clearing a layout; the web in
  headless Chromium.

## Changes

- 2026-11-13 (ADR 0091): §2 had no part `series`.
