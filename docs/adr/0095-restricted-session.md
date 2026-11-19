---
number: 0095
title: Restricted session – a session sees and changes only granted folders and projects
status: accepted
date: 2026-11-13
adds_to: ["0005", "0019", "0039", "0066", "0067"]
---

# 0095 – Restricted session: only granted folders and projects

## Context

The wish was to let agents touch only one folder of an account, and
behind it the broader wish to share folders and projects between
accounts, read-write or read-only.

Real sharing between accounts contradicts ADR 0005 §1 (no query ever
crosses an account's boundary), and almost everything is built for one
account per session: `AppState.account()` opens the session's one file,
`/changes` counts with that file's `seq`, the mirror in replica and app
holds one account, and `samu-core` computes availability and scopes over
one account's objects. A foreign task also points to its account's tags
and workflows; sharing would mean mounting foreign objects with
everything they point to, with a composite cursor and a write path into
the owner's file.

The aim is narrower: an agent should be able to touch only some folders.
So in stages:

- **Stage A**, this record: a session can be restricted to folders and
  projects of its own account. ADR 0005 stays as it is.
- **Stage B**, later with a record of its own: real sharing between
  accounts, when a case needs it.

## Decision

1. **A grant on the session.**
   - A session may carry a grant: a list of folders and projects of its
     account plus `rw` or `ro`. Without a grant it is unrestricted.
   - The grant is stored in a column `grant` of the `session` table in
     `server.db` (server migration 5).
   - It is set **only at login**, with `Login.only` (optional, `{
     objects: [Id], access: rw|ro }`), and never changes afterwards; a
     different grant needs a new login.
   - It is shown in `SessionInfo.grant` and in the account menu; both are
     additional optional fields in `samu-api`.
   - Any kind of session may carry one; it is meant for `agent` (ADR 0039
     §2).
   - Pairing (ADR 0031) always gives an unrestricted session.
2. **What is visible.** The rule lives in one place, the module `grant`
   in `samu-core`, without I/O, like the scope in `scope.rs`. Visible
   are:
   - every granted folder with all its subfolders;
   - every project in them and every explicitly granted project;
   - their tasks with subtasks;
   - what hangs on these objects: comment, attachment, reminder, clock
     entry, habit entry, logbook;
   - **read-only**, all tags and workflows, since state, priority and
     tagging cannot be read without them.

   Everything else is invisible, among it the inbox, scopes,
   perspectives, templates, settings, calendars and appointments. What
   is invisible does not exist for the session: `not_found`, not
   `forbidden`.
3. **What is writable.** With `ro` nothing. With `rw`:
   - changes to visible objects except tags and workflows;
   - an object the session creates or moves must lie in the grant
     **afterwards**; moving a task to a project outside is rejected;
   - tagging a visible task is allowed, a state change too; creating new
     tags is not (a tag belongs to the whole account).

   What is not allowed `POST /changes` answers per change with
   `ChangeOutcome::Rejected`, every other route with 403 `forbidden`.
   The server checks before the store, with the rule of §2.
4. **Every route has a rule.** Like the replica's table (ADR 0066 §5),
   every route of `samu_api::ROUTES` is classed in `samu-server`'s
   `grant::restriction`, and a test fails for a route without one:
   - **filtered:** the routes that read or write tasks, projects and
     folders and what hangs on them (`/changes`, objects, logbooks,
     deletion, `/available…`, forecast, reports, habits, the event
     stream, capture, blobs). Capture works only with a target inside the
     grant; without a target it would go to the inbox and is
     `forbidden`. `PUT /blobs` needs `rw`; `GET /blobs` serves only a
     blob that a visible attachment points to.
   - **forbidden** (403): scope schedule, export, hooks, pairing codes,
     everything about calendars, appointments and alarms, session
     management (§7), account deletion and demo content (ADR 0099).
   - **free:** `GET /session`, `POST /logout`, `POST /render`, and the
     routes without a session.
5. **`GET /changes` filtered.**
   - The `seq` stays the account's.
   - An object changed since `after` and visible now is in `objects`.
   - One changed and **not** visible now is in `gone`, also a project
     moved out of the granted folder: it vanishes for the agent too.
   - For ids the session never saw, `gone` is harmless.
   - The event stream reports only changes to visible objects.
6. **Only directly at the server.** The local replica (ADR 0066) and the
   app mirror a whole account; they reject a login with `only` with
   `bad_request`. Agents log in at the server directly anyway (ADR 0067).
7. **Seeing and ending sessions.** Two routes, both only for an
   unrestricted session of kind `human`:
   - `GET /sessions` lists the account's sessions with device, kind,
     grant, creation and expiry;
   - `POST /sessions/{id}/end` ends one.

   The `id` is a hex prefix of the `token_hash`, which does not reveal
   the token. Commands `samu sessions` and `samu session end <id>`; in
   the web a list in the account menu with "End". At the replica both
   routes are relayed to the server (ADR 0096).
8. **Command line.**
   - `samu login --agent --only <folder|project>… [--read-only]`.
   - A folder or project is given as for `samu folder list` and `samu
     projects`, as a path (`work/ai`) or an id. The CLI resolves the path
     before login; the server receives the id.
   - `samu session` shows the grant.
9. **Switching an agent over:** log the agent in again with `--only`,
   then end its old unrestricted session with `samu sessions` and `samu
   session end <id>` from a person's own session. How the maintainer did
   this for their own installation is not part of this record.

## Consequences

- ADR 0005 is untouched: no query across an account's boundary, no
  second cursor.
- A grant hangs on folders and projects. Moving a folder into or out of
  the grant changes what the agent sees; that is intended and needs no
  new login.
- The agent does not see the inbox. It files findings as entries in a
  project it may see.
- Without a scope, `scope_only` objects stay hidden for the agent too
  (ADR 0069, 0094).
- `samu-api` has the new fields and types and the two routes of §7, with
  fixtures. catastr is not affected: it does not log in with `only` and
  does not read the new fields.
- Stage B remains a wish. It does not build on this record but can use
  §2 to answer what lies below a shared folder.
- Decided with this record: a `rw` session creates no new tags; the inbox
  cannot be granted until a case needs it; sessions are listed in the
  account menu, not on a page of their own.

## Changes

- 2026-11-13 (ADR 0096): §7's routes were "online only" at the replica
  and always answered 503 there.
- 2026-11-13 and later (ADR 0099, 0100, 0101, 0123, 0126, 0129): new
  routes were classed under §4 as they came; the table in the code is
  the current list.
