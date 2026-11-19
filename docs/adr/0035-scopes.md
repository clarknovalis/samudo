---
number: 0035
title: Scopes – one lens over all views, object scope, chosen per device
status: accepted
date: 2026-11-08
adds_to: ["0006", "0019", "0020"]
---

# 0035 – Scopes: one lens over all views

## Context

The wish: choose a scope such as "private with IT" or "private without
IT", after which every view stays as it is but shows only what the rules
let through. The model is OmniFocus's Focus, which works on folders and
projects only, applies to all perspectives and since 4.3 can follow the
system's focus modes.

Work and private remain two accounts (ADR 0005); a scope applies within
one account. Simple scopes first, with a settings menu in web UI and app
to define them.

## Decision

1. **Object `scope`**, a new kind beside `tag` and `reminder`, with:
   - `title`;
   - `mode`: `only` or `except`. An unknown value is read as `unknown`
     and never written (ADR 0015); a scope with an unknown mode filters
     nothing;
   - `members`: a set of ids of folders, projects and tags. Each element
     has its own stamp and changes with `add` and `remove`, like a task's
     `tags` (ADR 0004);
   - `position`.

   Later fields: `inbox`, `filter`, `capture`, `schedule` (ADR 0043),
   `calendars` (ADR 0047 §12), `color` (ADR 0060), `layout` (ADR 0072).
   A scope travels over `/changes` like every object, so the app has it
   in its mirror and filters offline (ADR 0028 §3). No nested rules.
2. **The rule lives in one place, `samu-core`** (as ADR 0006 for
   availability). A task *matches* the scope when one of these is a
   member:
   - its project,
   - a folder above the project, at any depth,
   - one of its tags or a tag above it,
   - one of its project's tags or a tag above it.

   A tag as member counts with all its children, a folder with everything
   below. With `only` the views show what matches, with `except` the
   rest. For a project the same rule applies without a task's tags. A
   deleted member does not count. A cycle among folders or tags is
   detected and ends the walk. A scope without members and without the
   inbox checkbox restricts nothing (ADR 0044); `scope_only` objects are
   hidden unless a scope opens them (ADR 0069, ADR 0094); a filter
   narrows further (ADR 0043 §3).
3. **The inbox** follows the checkbox `inbox` (ADR 0043 §1). Originally
   the inbox was never filtered.
4. **The choice is per device.** The web UI keeps it in the browser, the
   app in its cache, the CLI in its configuration (per profile, ADR
   0086). The server holds only the definitions. So "Work" can hold on
   the computer while "Private" holds on the phone. A scope that no
   longer exists counts as not chosen. A device may follow the scope's
   schedule instead ("automatic", ADR 0043 §7).
5. **Routes:** `GET /available` and `GET /available/projects` take
   `scope=<id>` and then show only what lies in the scope; an unknown
   scope gives 404. Other views that list tasks or appointments take the
   same parameter (forecast, appointments, report). Scopes are created and
   changed over `POST /changes`; the only route of their own is `GET
   /scopes/scheduled`, the schedule's current choice for clients without
   `samu-core` (ADR 0043 §7).
6. **What is due outside stays visible as a number.** With `scope=`,
   `/available` also returns the open tasks outside the scope that are
   overdue or due soon. They carry `outside: true` (field on
   `TaskStatus`, absent when false). Clients show them in no view; they
   only count them for the hint in the header, such as "2 due outside".
7. **Header:** web UI and app always show the chosen scope at the top,
   with a menu to switch and "no scope", and the hint of §6 beside it. A
   rule with a gap would otherwise hide tasks unnoticed.
8. **Settings:** web UI (from the account menu) and app (a screen of its
   own) have a settings menu. Its first section is "Scopes": create,
   rename, choose the mode, tick members in the tree of folders, projects
   and tags, delete. Further sections come with their own records (ADR
   0036, 0037, 0045).
9. **Not for now:**
   - Capture defaults from the scope: since ADR 0043 §6, explicitly per
     scope.
   - Switching by time of day or by Android's focus mode: since ADR 0043
     §7–8.
   - Full rules in the query language of perspectives, joining scope and
     perspective.

## Consequences

- `samu-api` has `Scope`, `ScopeMode`, the kind `scope` and
  `TaskStatus.outside`, with fixtures (ADR 0014). An older reader sees
  the kind as `unknown` (ADR 0015). catastr needs no change.
- `samu-store` migration: rebuild `object` so the CHECK on `kind` knows
  the new kind (as for reminders, ADR 0030), and tables `scope` and
  `scope_member`. Deleting a folder, project or tag removes it from every
  scope's members first (ADR 0036, 0076).
- CLI: `samu scope` (create, list, show, rename, mode, members, delete,
  choose) and `--scope` on `samu list` and `samu projects`.
- A scope is stronger than OmniFocus's Focus: it knows tags and
  exclusion, has a name, and its definition is the same on every device.
  The price is one more concept (account, scope, perspective); header and
  hint (§6–7) keep it visible.

## Changes

- 2026-11-08 (ADR 0043 §1): §3 said the inbox is never filtered; a task
  without project was always in the scope.
- 2026-11-08 (ADR 0043 §6–8): §9 excluded capture defaults and automatic
  switching.
- 2026-11-09 (ADR 0044): §2 let an `only` scope without members show
  nothing.
- 2026-11-11 (ADR 0069, 0070, 0094): §2 knew no objects that only their
  own scope shows.
