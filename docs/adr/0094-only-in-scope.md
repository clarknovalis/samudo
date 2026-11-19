---
number: 0094
title: "Only in scope X" – an object names its own scope
status: accepted
date: 2026-11-13
replaces: ["0069", "0070"]
---

# 0094 – "Only in scope X": the object names its own scope

## Context

ADR 0069 (field named `scope_only` by ADR 0070) let a folder, project or
tag be shown only in its own scope: a scope with `only` that ticks the
object or something below it. A scope with `except` never showed it.

That failed a simple case: a scope "Client" defined as "all except
Private, Other", for an exclusive project "Client" in the root. The
project did not appear. With `only` and a tick on it, it did, but then
every new project had to be ticked by hand. A run with synthetic data
confirmed it.

The expectation: "all except" shows what is not ticked, the exclusive
project too, while habits stay hidden as before. Ticks that mean the
opposite on exclusive objects, or a second list of "unlocks" on the
scope, would confuse new users.

The core of the problem: the switch said "only in its own scope" but not
which scope that is. The scope had to guess from its ticks, and that
works only with `only`.

## Decision

1. **The object names its scope.** Folder, project and tag have the
   field `own_scope`: the id of a scope, or absent. A field with its own
   stamp, LWW, `set` and `unset` (ADR 0004). In words: "Client appears
   only in the scope Client."
2. **What it hits:** a task hits an object with `own_scope` through its
   project, a folder above the project at any depth, one of its tags or a
   tag above it, or one of its project's tags or a tag above it (the rule
   of ADR 0035 §2). A project hits through itself, the folders above and
   its tags; a folder through itself and the folders above. Along the
   path of folders and project only the **nearest** object with
   `own_scope` counts: a habit in the folder "Habits" (own scope
   "Habits") under the folder "Private" (own scope "Private") belongs to
   the scope "Habits". Among tags each counts, and **one is enough**.
3. **Where it appears.** Something that hits such objects is shown only by
   a scope that is the own scope of one of them. There it counts like any
   other object, with one exception: a scope with `only` treats it as if
   it were ticked.
   - Scope "Client" as "all except Private, Other": the project is not
     ticked, so it appears.
   - Scope "Client" as "only what is ticked": the project appears even
     without a tick.
   - Scope "Habits" with `only` and the folder Habits: unchanged.

   The scope's mode thus plays no part in "exclusive or not"; ticks
   always mean "only these" or "all except these". The filter (ADR 0043
   §2) narrows further, as before.
4. **Where it does not appear:** in every other scope, in a scope without
   ticks that is not its own, and with "no scope".
   - **Precedence over the inbox:** a task in the inbox with such a tag is
     hidden like any other; the inbox checkbox (ADR 0043 §1) and "no
     scope" do not show it. For this case the tags of an inbox task do
     count.
   - **The header hint** (ADR 0035 §6) also counts such tasks outside the
     current scope that are overdue or due soon, also with "no scope", so
     a task that carries such a tag by mistake does not vanish unnoticed
     once it is due.
   - **Not hidden:** reminders (they do not follow the scope anyway), the
     tree in the scope settings, access by id (`samu show`, a `samu:`
     link, opening a task from a comment), and calendar appointments.
   - Where a hidden task lives can be shown on request (`elsewhere`, ADR
     0124).
5. **An own scope that does not exist** (deleted, or from another file)
   counts as none: the object is then not exclusive. Showing more beats
   showing less. The inspector says "scope missing".
6. **Transition from `scope_only`.** A numbered migration set `own_scope`
   on every object with `scope_only`: the `only` scope with the smallest
   `position` that unlocked it. Where none was found the field stayed
   empty, and `samu-server` names the object in its log at start.
   `scope_only` remains in `samu-api` as a derived, read-only field
   (`own_scope` set) for older readers; the server rejects a `set` on it.
7. **The rule stays in one place**, `samu_core::scope`, checked before the
   member rule; since it also applies without a scope, "no scope" goes
   through the same function. It covers every view from `/available` and
   `/available/projects`: inbox, today, forecast, available, flagged,
   review, stuck, board, and the projects and folders of the sidebar. The
   app computes offline with the same function.
8. **Interfaces:**
   - Web and app: in the inspector of folder, project and tag a choice
     **"Only in scope:"** with "–" and the scopes. The mark in the sidebar
     and in the scope settings' tree names the scope ("only in Client").
     The help text on ticks no longer mentions exclusive objects.
   - CLI: `samu set <id> own_scope <scope>` (title, id or its end), `samu
     unset <id> own_scope`. The lists of folders, projects and tags show
     "only in <scope>".
   - Desktop widget and Emacs package read through the CLI and
     `/available`; nothing changes there.

## Consequences

- `samu-api` 0.1.44 with `own_scope` on three types, with fixtures (ADR
  0014). catastr reads neither scopes nor this field.
- `samu-store`: migration with the column `own_scope` and the transfer of
  §6.
- Tests in `samu-core`: an exclusive project in an `except` and an `only`
  scope as its own scope, hidden in every other and without a scope;
  habits unchanged; the nearest exclusive object on the folder path
  counts; one tag is enough; a missing own scope makes nothing exclusive;
  and a test of the migration.
- New projects appear in an `except` scope without anyone maintaining
  ticks.

## Changes

- 2026-11-11 (ADR 0069, 0070): the field was a boolean `scope_only`
  ("only in its own scope"); a scope with `only` unlocked it when it
  ticked the object or something below it, a scope with `except` never
  showed it, and a member above the object did not unlock it.
- 2026-11-18 (ADR 0124): §4 had no way to tell where a hidden task is
  shown.
