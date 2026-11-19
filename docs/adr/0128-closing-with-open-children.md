---
number: 0128
title: Completing or dropping with open children – ask, then close them along or lift them out; the server computes, the clients ask
status: accepted
date: 2026-11-18
adds_to: ["0004", "0033", "0036", "0126"]
---

# 0128 – Completing with open children

## Context

Completing was the same plain field change in every interface: set
`completed_at` (`samu done`, `samu bulk done`, the checkbox, menu,
inspector, board column "Done" and selection in the web, swipe, selection
bar, widget and notification in the app, `samu-toggle-done` in Emacs).
The server did not look at children; it checked only repetition (an
action group does not repeat, ADR 0033).

What happened to open children followed from availability (`samu-core`,
rule 7): under an unavailable group a child counts as
`container_unavailable` and is missing from every view except `--all`.
Nobody deleted it and nobody decided it was done. In practice a dozen
open entries ended up under completed tasks this way, none of them on
purpose. The same holds for `dropped_at`.

Other programs: OmniFocus completes the children along and can complete
a group with its last action (samu's model has
`complete_with_last_child`, unused); Todoist completes subtasks along;
Things 3 asks for a project with open to-dos; org-mode with
`org-enforce-todo-dependencies` blocks DONE while children are open.

Whoever checks off a group usually wants everything below done too. But
the other case exists: the task is built and a remainder stays open that
should live on by itself. So one fixed answer is not enough.

## Decision

1. **Ask instead of hiding silently.** Completing or dropping a task with
   open descendants offers three answers:
   - **complete along** (when dropping: **drop along**): all open
     descendants get the same instant as the task;
   - **lift out**: the open direct children move one level up with their
     subtrees, then the task is completed;
   - **cancel**: nothing changes.

   Without open descendants it completes without asking, as before. In
   the dialog "complete along" is preselected.
2. **Lifting out in detail.** The target is the task's `parent`. Without
   one the children go to the top level of its project, for an inbox task
   into the inbox. They keep their order and stand right after the task;
   following siblings move down. Completed and dropped children stay
   where they are: they belong to the task's history. Every move is a
   `moved` in the logbook (ADR 0123), as when converting (ADR 0126).
3. **The server computes, as for deletion.** Route `POST
   /objects/{id}/close` with `CloseRequest { field, children, preview }`:
   - `field` is `completed` or `dropped`;
   - `children` is `ask`, `close` or `lift`, default `ask`.

   `samu-core` (`samu_core::close::plan`) computes the changes; the server
   writes them in **one** transaction, like `/objects/{id}/delete` (ADR
   0036). With `ask` and open descendants nothing is written; the answer
   `CloseResponse { open, closed, lifted, seq }` says how many are open,
   and the client asks when `open > 0`. `preview: true` never writes.
   `samu-api` has a new minor version with fixtures.
4. **The field change stays allowed.** `POST /changes` with
   `completed_at` is not rejected. Outboxes written offline, old clients,
   syncs (Jira, import, hook) and repetition keep writing this way. An
   outbox must not fail afterwards on a question nobody can answer any
   more. The rule of §1 is for the interfaces, not a lock in the store.
5. **The interfaces.**
   - **Web:** every way to complete and drop goes through one helper that
     uses the route: checkbox, inspector, menu, shortcut, board column
     "Done" (dragging there asks too), selection. With `open > 0` a dialog
     shows the three answers with the number ("3 open subtasks"). A
     selection gets **one** question for all affected tasks ("2 of the 5
     tasks have open subtasks").
   - **App:** the same dialog as a sheet. Offline `samu-mobile` computes
     with `samu_core::close::plan` and queues the result in the outbox (as
     for deleting and converting). Undo takes back everything the answer
     changed. **Widget and notification** cannot ask: with open
     descendants they open the task in the app instead of completing it.
   - **CLI:** `samu done` and `samu drop` have `--children close|lift`.
     Without it and with open descendants nothing changes: the children
     are listed on stderr and the exit code is "needs a decision" (code 7
     in `mod exit`). `samu bulk done` and `samu bulk drop` behave the same
     for the whole selection; then nothing at all is written, not even to
     tasks without children.
   - **Emacs:** `samu-toggle-done` asks with `read-multiple-choice` and
     calls the CLI with `--children`.
   - **Desktop widget:** like the app's widget, open instead of complete.
6. **Reopening reopens only the task.** What was completed along stays
   completed; lifted children stay where they went. Undo (web and app) is
   the way back while it is still possible.
7. **What is already hidden.** The search word `ist:versteckt`
   (`is:hidden`) finds open tasks under a completed or dropped group, in
   web, app and `samu list --search`, and can be saved as a perspective.
   No hint in the sidebar. New cases arise after §4 only from syncs and
   outboxes written offline.
8. **Not for now:**
   - evaluating `complete_with_last_child`, a group completing itself;
   - projects: a completed project also hides its open tasks
     (`project_not_active`), but a project has a status and is not checked
     off in passing. If §1 proves itself, projects get the same question
     in a step of their own;
   - an account default "always complete along".

## Consequences

- Completing and dropping take the same path with the same question in
  every interface; a child no longer disappears unnoticed.
- `samu-api` gains `CloseRequest` and `CloseResponse`, the CLI
  `--children` and an exit code.

## Changes

None yet.
