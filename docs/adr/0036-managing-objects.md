---
number: 0036
title: Managing projects, folders and tags – deletion as a route, orphans count as inbox
status: accepted
date: 2026-11-08
adds_to: ["0004", "0006", "0019", "0035"]
---

# 0036 – Managing projects, folders and tags

## Context

The settings menu of ADR 0035 §8 was to create, rename and delete
projects and tags, and deleting a project was to offer a choice whether
its tasks go along or stay.

`samu delete <project>` only put a tombstone on the project (ADR 0004
§7). Its tasks then pointed to a project that no longer existed;
`samu-core` read that as `UnknownValue`, never available, and the web UI
showed them under "?". Tags were cleared before deletion, but only by
the CLI itself. Web and app could not delete at all. Tasks without a
valid project can also arise without the menu: a phone files a task
offline into a project another device has just deleted.

Chosen: the deletion dialog offers the choice; the server composes the
deletion; orphaned tasks count as inbox; folders are included.

## Decision

1. **Route `POST /objects/{id}/delete`** with a session, body
   `DeleteRequest { tasks, preview }`.
   - `tasks` is allowed only for a project: `inbox`, `{ "project": id }`
     or `delete`.
   - `preview: true` writes nothing and gives the same answer, as for
     `/capture` (ADR 0027).

   The server reads the state and computes in `samu-core`
   (`samu_core::delete`) which changes are needed. It stamps them with its
   HLC and writes them in **one** transaction; if one change is rejected,
   none holds. The response `DeleteResponse` counts what is deleted, what
   moves and from what a tag is removed, and names the `seq` written up
   to. Every change is an ordinary field change (ADR 0004) and appears in
   `/changes` and the event stream. Deleting the same object again counts
   as applied and changes nothing. The deletion is one entry in the
   logbook (ADR 0123 §2).
2. **Project.** Without `tasks`, `inbox` applies. The choice covers all
   the project's tasks, completed and dropped ones too.
   - `inbox` sets `project` to `null`.
   - `{ "project": id }` sets the target project; the top-level tasks
     line up after the target's last top-level tasks. The target must be
     another existing project.
   - `delete` puts a tombstone on every task.

   Action groups stay whole: `parent` and the order below do not change.
   Then the project itself gets its tombstone.
3. **Folder.** Its subfolders and projects move up into its parent folder,
   or to the top if it has none, after the parent's content. `tasks` is
   rejected.
4. **Tag.** A tag with children is rejected, as is one that serves a
   project as workflow (ADR 0024). Otherwise it is first removed from all
   tasks and projects and from the members of every scope (ADR 0035), then
   deleted. The server computes this for all clients. Deleted projects and
   folders likewise leave every scope's members (ADR 0076 §1).
5. **Other kinds.**
   - A task takes its descendants along; before, they were left with a
     `parent` pointing nowhere.
   - A scope, a reminder or an unknown object gets a plain tombstone.
   - `samu delete` always goes through this route.
   The store itself refuses to delete a folder, project or tag while a
   live object points to it; this route clears the references first and
   deletes last, so it gets through (ADR 0076 §2).
6. **Orphans count as inbox.** For a task whose `project` points to
   nothing or to a tombstone, `samu-core` takes the inbox as its
   *effective project*. Computed, not stored (ADR 0006), so it covers
   today's orphans and future ones from a race. It follows:
   - the task is available by the inbox's rules;
   - scopes treat it as inbox (ADR 0043 §1);
   - rule 7 of availability compares the effective project of group and
     child, so an orphaned group with its children stays whole.

   `TaskStatus` has `inbox: bool`, set from the effective project.
   Clients group by it, not by `task.project`. The stored field stays as
   it is until someone moves the task.
7. **Creating and renaming** stay ordinary changes over `/changes`, so in
   the app they also work offline through the outbox. Deleting needs a
   connection, since only the server knows the whole state; the app says
   so instead of queueing the deletion.
8. **Settings.** Beside "Scopes", in web UI and app:
   - "Projects and folders", a tree: create, rename, delete;
   - "Tags": create, also as a child, rename, delete.

   The deletion dialog fetches the preview first and shows its numbers;
   for a project it offers the three choices of §2, inbox preselected.
   Navigation, tree and inspector replaced the first dialogs (ADR 0037).
9. **Not for now:**
   - deleting a folder with its projects;
   - deleting a tag with its children;
   - moving projects or folders in the menu (since ADR 0037 §1 an
     ordinary change; in the sidebar ADR 0041).

## Consequences

- `samu-api` 0.1.11 has `DeleteRequest`, `TaskFate`, `DeleteResponse`,
  `TaskStatus.inbox` and the route in `ROUTES`, with fixtures. catastr
  needs no change; an older reader skips `inbox`.
- `samu-store` applies a batch of changes in one transaction. No schema
  change.
- CLI: `samu delete` and `samu tag delete` go through the route; `samu
  delete` has `--tasks` and `--dry-run`. New: `samu project add|rename`
  and `samu folder add|rename|list`.
- Existing orphans appear in the inbox after the update without anything
  being written.

## Changes

- 2026-11-08 (ADR 0037): §9 excluded moving projects and folders; the
  settings were dialogs.
- 2026-11-11 (ADR 0076): §4 removed only tags from scopes; deleting a
  project or folder could leave scope members and references behind, and
  the store did not refuse that.
- 2026-11-18 (ADR 0123): a deletion with consequences wrote no logbook
  entry.
