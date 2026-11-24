---
number: 0090
title: Perspectives – saved views of search, filter, arrangement and display as a server object, evaluated in samu-core
status: accepted
date: 2026-11-12
adds_to: ["0042", "0043", "0055", "0059", "0061", "0064", "0075"]
---

# 0090 – Perspectives

## Context

The web UI can show a lot: search with chips (ADR 0042), the switch
available / remaining / all (ADR 0020 §1), arranging with grouping and
two sort levels (ADR 0064), the board (ADR 0061). But it was remembered
only per view and per browser in `samu.order`, and a search had no name:
whoever wants `@office ~<15m is:available` by estimate every morning
types it every morning.

OmniFocus Pro has custom perspectives with rules, icon, grouping,
sorting and layout in the sidebar; Todoist has filters in a query
language as favourites; org-mode has custom agenda commands. ADR 0042 §8
deferred perspectives to a record of their own with the evaluation in
`samu-core`; since ADR 0043 that evaluation exists for scope filters
(`samu-core::query`).

Chosen: an object on the server; evaluation in `samu-core`, the web keeps
computing in the browser; the scope stays a lens over it; web and CLI
first.

## Decision

1. **Object `perspective`** in the account, over `/changes` like scope
   and template, each field LWW (ADR 0004), lists as a whole:

   | Field | Content |
   |---|---|
   | `title` | name, not empty, unique ignoring case among live ones |
   | `icon` | an emoji as in ADR 0059, at most 32 bytes, optional |
   | `text` | free text of the search, must occur in title or note (ADR 0042 §2), optional |
   | `chips` | chips `{kind, value, not, group}` as a scope's filter (ADR 0043 §2, ADR 0121): project, tag, folder, state as id |
   | `filter` | `available`, `remaining` or `all` (default `available`) |
   | `arrange` | `{group, sort, desc, then, then_desc}` with the values of ADR 0042 §5–6 and 0064 (default project, order) |
   | `view` | `list`, `board` or `table` (default `list`; `table` per ADR 0115) |
   | `board` | `{source, workflow, tags, wip, done}` as in the browser (ADR 0061), optional |
   | `table` | `{columns}` (ADR 0115), optional |
   | `blocks`, `dedupe` | a block perspective of several queries (ADR 0116); when `blocks` is not empty the fields from `text` to `table` do not apply |
   | `position` | order in the sidebar |

   A value a reader does not know (a future grouping, sort or view) falls
   back to the default for it; a chip of unknown kind matches every task
   (ADR 0043 §2). An older reader thus shows more, never invisibly less.
2. **What a perspective shows** is computed by `samu-core` in one place
   (ADR 0006): the tasks in the chosen scope (the scope stays a lens, also
   for objects with an own scope, ADR 0094), that satisfy the chips (one
   kind OR, kinds AND, negation, groups OR; ADR 0043 §3, ADR 0121) and
   contain the free text, then the filter. As in the search, a chip for
   finished tasks (`ist:erledigt`, `ist:verworfen`, `erledigt:`) lifts
   the filter to "all", and one for waiting tasks (`ist:wartet`,
   `start:`) lifts "available" to "remaining" (ADR 0042 §4). A word of
   the free text that names a task by its id (its end from 8 characters,
   the whole id, `samu:<id>`, a web address that opens it) lets that task
   alone pass the filter, as an `id:` chip, which takes the same forms,
   does for all. `GET
   /available?perspective=<id>` answers this way, with `scope=` beside
   it; the local replica (ADR 0066) computes the same. Unknown id: 404.
3. **The web computes in the browser** like the search (ADR 0042 §1,
   0043 §4), from the same fields; shared test cases keep
   `web/lib/query.js` and `samu-core::query` equal, now including free
   text. Grouping and sorting stay presentation and thus each client's
   business; `samu-core` returns the matches in the order of
   `/available`.
4. **Sidebar in the web:** a section "Perspectives" below the fixed
   views, each row with icon (otherwise a funnel) and name, with a count
   as for projects; in the narrow sidebar the icon (ADR 0055, 0059).
   Dragging orders (`position`). Row menu: rename with icon, duplicate,
   delete. With no perspective only "+" stands in the section's head.
5. **Created from what one sees:** "Save as perspective" at the search
   field and in the arrange popover takes the current view's chips, free
   text, switch, arrangement and list/board; it asks for name and icon.
   "+" in the sidebar creates an empty one and opens it.
6. **Inside a perspective** its chips and free text stand in the search
   field. Arrange, board and switch change the perspective itself (over
   `/changes`), not `samu.order`. Changing chips or text shows "Save" and
   "Discard" at the search field; without saving, the perspective stays
   as it was when one leaves it. Details (ADR 0053) stay per device.
7. **Address:** `/o/<uuid>` opens a perspective as a view (adds to ADR
   0075 §1). Jumping by keyboard is a separate feature.
8. **CLI:** `samu perspective
   list|add|search|text|chips|filter|arrange|view|blocks|dedupe|rename|icon|move|delete`,
   chips in the search language like `samu scope filter` (names in, ids
   on the wire, `query::parse`); `samu list --perspective <name|id>`.
   Output and exit codes like `samu template`.
9. **Later, each with its own decision:** perspectives in the app (ADR
   0102, arranging there ADR 0125) and in the Emacs package; several
   queries in one view like org's block agenda (ADR 0116); a perspective
   as the "view on entering" a scope (ADR 0072); project perspectives
   (rules over projects instead of tasks).

## Consequences

- `samu-api` has `Perspective` with `PerspectiveFilter`,
  `PerspectiveArrange`, `PerspectiveView`, `PerspectiveBoard`, the kind
  and `?perspective=` on `/available`, with fixtures (ADR 0014). catastr
  needs no change.
- `samu-store` migration with the table; on write the store checks title,
  icon and chips (`query::check`).
- `samu.order` stays for the fixed views and projects.

## Changes

- 2026-11-14 (ADR 0102): §9 left the app without perspectives.
- 2026-11-16 (ADR 0115): §1 had only `list` and `board` as views.
- 2026-11-16 (ADR 0116): §1 had no `blocks`; a perspective was one query.
- 2026-11-17 (ADR 0121): chips had no `group`; §2 had no OR across kinds.
- 2026-11-24: §2: a free word naming a task by its id did not lift the
  filter for it, and `id:` took only the last 8 characters or the whole id.
