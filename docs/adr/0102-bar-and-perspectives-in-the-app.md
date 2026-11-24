---
number: 0102
title: Bar and perspectives in the app – a configurable bottom bar, perspectives as lists, a slimmer header
status: accepted
date: 2026-11-14
adds_to: ["0020", "0028", "0052", "0090"]
---

# 0102 – Bar and perspectives in the app

## Context

After a walk through the app the wish was: the bottom bar (inbox,
available, forecast, projects, search; ADR 0020) should be extendable
with a swipe as in OmniFocus 4, and the settings should say what stands
there and in which order, perspectives included.

What the app had and lacked:

- **Five fixed tabs** (`enum class Tab`) with Unicode characters as icons
  (▤ ◉ ◷ ▦ ⌕): small and uneven. The web knows more fixed views (`VIEWS`
  in `web/lib/sidebar.js`): today, flagged, stuck, review, calendar,
  report; the app did not, though `samu-mobile` already offered `today()`
  for instance.
- **Perspectives** (ADR 0090) existed only in web and CLI; §9 there asks
  for a decision of its own for the app. This is it.
- **The header ate space:** title, "Details", "Sync", gear, a line for
  the scope and a line of tag chips; about a quarter of the screen before
  the first task.
- **Long press** on a task started multi-selection (ADR 0052), leaving no
  room for a context menu of a single task (such as "add subtask").

Search with suggestions while typing is ADR 0103.

## Decision

### 1. The bar is a list of entries

An entry is one of:

| Kind | Example | Opens |
|---|---|---|
| fixed view | Inbox, Available, Today, Forecast, Flagged, Stuck, Review, Projects, Search, Habits | the view as before, or as in the web |
| perspective | "Calls" | the perspective as a list (§4) |
| project | "Website" | the project's list, as from the tree |
| tag | "@office" | the available tasks with this tag |

New fixed views in the app are today, flagged, stuck, review and habits.
Each is computed through `samu-mobile` from `samu-core`, like the
existing ones (ADR 0006); none computes on its own.

Default without a setting: the five of before in the order of ADR 0020.
Whoever sets nothing sees no difference.

### 2. Room for more than five entries

**Swiping:** the bar shows five entries at once and scrolls sideways; a
dot at the edge shows there is more. The active view scrolls into the
visible part. No "More".

### 3. Settings page "Bar"

Under settings a page "Bar": at the top the bar's entries with a handle
to drag and a switch to hide; below "Add" with the fixed views,
perspectives, projects (search by name) and tags. A deleted or dropped
project, a deleted tag or perspective disappears quietly from the bar.
Inbox cannot be removed, only moved.

The setting is **per device**, in the cache like "Details" and the
reminder switch (`set_meta`), without a change to `samu-api`; it is not
synced and does not change the web's sidebar.

### 4. Perspectives in the app

- A perspective shows its matches as a **list, without board** (ADR 0090
  §9). The evaluation comes from `samu-core` through `samu-mobile`
  (`perspective(id)`), with scope and filter as `GET
  /available?perspective=`; the perspective's grouping and sorting are
  applied by the app. A block perspective (ADR 0116) shows its blocks
  one by one, as ADR 0147 decides.
- The header shows the perspective's chips. Changing its arrangement in
  the app came with ADR 0125 §4; changing chips, text and name, creating
  and renaming stay in web and CLI. "Save as perspective" from the search
  came with ADR 0103 §5.
- An icon (emoji, ADR 0090) stands in the bar, without one a funnel.

### 5. Icons

Material symbols instead of Unicode characters, the same motifs as in the
web's sidebar (`VIEWS`), so web and app look alike. Perspectives carry
their emoji (without one the funnel), projects the project icon from the
tree, tags an `@`.

### 6. A slimmer header

- One line: title, on the right a menu (⋮) with "Details", "Sync",
  "Settings" and, in projects, "New project/folder"; the arrange button
  `⇅` beside it (ADR 0125).
- **Pull to refresh** instead of the "Sync" button. When something waits
  in the outbox, a small dot at the title says so.
- The **scope** stands as a chip in the title instead of a line of its
  own; tapping opens the choice as before.
- The **tag chips** stay but scroll away with the list instead of standing
  fixed.

### 7. Gestures on a task

- **Swipe right** completes as before (ADR 0020).
- **Swipe left** flags or unflags.
- **Long press** opens the task's context menu: edit (ADR 0078 §6), add
  subtask, flag, defer until …, move, select. **"Select"** starts
  multi-selection (ADR 0052); after that tapping selects more as before.
  This changes ADR 0052 for the app: long press no longer selects
  directly.
- Every gesture can be undone (ADR 0104).

## Consequences

- No change to `samu-api` or the server; catastr is not affected.
- `samu-mobile` has `perspectives()`, `perspective(id, tag)`, `flagged`,
  `stuck`, `review` and `habits` as lists, and `bar()`/`set_bar()` for
  the entries (§3).
- A widget with a perspective and perspectives in the Emacs package stay
  wishes of their own.

## Changes

- 2026-11-14 (ADR 0103, 0104): §4 had no "save as perspective" in the
  app; gestures had no undo.
- 2026-11-18 (ADR 0125): §4 left arranging a perspective to web and CLI.
- 2026-11-24 (ADR 0147): §4 showed a block perspective as the union of
  its blocks.
