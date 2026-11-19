---
number: 0074
title: Export and restore – a directory of JSON and files, readable without samu; Org only as an output, a git mirror as operations
status: accepted
date: 2026-11-11
adds_to: ["0002", "0004", "0009", "0039"]
---

# 0074 – Export and restore

## Context

The aim: getting out, with all data readable and complete without samu.
The server holds the truth (ADR 0004). If it fails, or samu, the apps and
agents stop running, the data is reachable only through the SQLite file,
which is unreadable without programs. Attachments lie under their hash in
`<account>.blobs/`, without a name.

First an Org mirror was planned that the server would keep writing into a
git repository. That was dropped since samudo was going to be published:
an Org mirror inside the server would be a niche everybody else would have
to carry. Chosen: an ordinary export as the core; Org and git only
optional and outside the server.

What existed: `GET /changes` returns all objects in wire form
(`AnyObject`), without stamps and logbook; `GET /objects/{id}/logbook`
and `GET /blobs/{sha256}` per object; `samu import org` (ADR 0002 §4,
0051), which is an import, not a restore: ids, stamps and logbook are new;
and `sqlite3 .backup` by hand before migrations.

## Decision

1. **An export is a directory**, not a single file and not an archive, so
   it can be read with any editor, put into git and compared with `diff`:

   ```
   samu-export/
     README.md          what is here and how to read it (English)
     manifest.json      format version, samu-api version, account, instant,
                        change number `seq`, count per kind
     folders.json  projects.json  tasks.json  tags.json  links.json
     clocks.json  reminders.json  scopes.json  comments.json
     attachments.json  calendars.json  appointments.json
     templates.json  settings.json  perspectives.json
     habit_entries.json
     logbook.json
     files/<sha256>     the bytes of every attachment, once per hash
   ```

   One file per kind of object; kinds added later get their own file. An
   object of kind `Unknown` is an error the export reports; it is not
   skipped.
2. **The form is `samu-api`'s wire form** (ADR 0014) plus the stamps:
   every object is the `AnyObject` of `/changes` with an extra field
   `stamps` (field → HLC and device, ADR 0004 §1). There is no second data
   model. The format version in the manifest is the version of `samu-api`;
   a new version reads old exports.
3. **Stable for diffs:** arrays sorted by id (UUIDv7, so by creation),
   keys in a fixed order, two spaces of indentation, a newline at the end.
   An unchanged state gives the same export byte for byte; only
   `manifest.json` carries the instant.
4. **What is included:** all live objects of the account, completed and
   dropped ones too, the whole logbook with author (ADR 0039 §1), and all
   attachments.

   **Not included:** tombstones (`deleted_at`, they exist only for sync),
   sessions, hooks, pairing codes and everything from `server.db`; and the
   sealed calendar sources (`calendar_source`, ADR 0047 §3), since an
   export contains no secrets. A calendar is exported with name and
   settings; its source must be set again after a restore. README and
   manifest say so.
5. **Two ways to export, one code:** writing lives in `samu-store` (it
   reads the account file), the types in `samu-api`.
   - `samu-server export --account <name> <directory>` reads the file
     directly; the way for schedules on the server and for emergencies,
     needing neither a session nor a running server.
   - `GET /export` returns the same over the API (manifest, objects,
     stamps, logbook as one JSON; the client fetches the files over
     `/blobs`). `samu export <directory>` writes the directory from it; the
     way for every user without a shell on the server. The local replica
     (ADR 0066) treats the route as online only; without a connection it
     answers 503 `offline`, since it has no logbook. Restricted sessions
     may not export (ADR 0095 §4).
6. **Restore only into an empty account:** `samu-server restore --account
   <name> <directory>` creates the account from an export. Ids, fields,
   stamps, `created`/`modified`, logbook and files stay as they were. If
   the account is not empty, the command aborts and changes nothing. There
   is no route: restoring is an act of whoever runs the server, not a
   client function. A partial import into an existing account is a
   different question.
7. **A round trip in the check:** a test fills an account with every kind
   of object, logbook and attachments, exports it, restores it into an
   empty account and exports again; apart from the manifest's instant both
   exports must be equal. A second test demands a file in the export for
   every kind of `ObjectKind` (except `Unknown`); a new kind missing from
   the export turns it red.
8. **Org is only an output:** `samu export --org` writes a `samu.org`
   beside the JSON files; a command of its own does the same from an
   existing export directory, without a server. It has a heading per
   folder, project and task in their order, a keyword from state and
   checkbox, `SCHEDULED`/`DEADLINE`, tags, `:PROPERTIES:` with `ID` and the
   properties, the note, comments as sub-items and attachments as
   `file:files/<sha256>` links. It is for reading only (with org-agenda,
   say, if samu fails) and is never read back; ADR 0002 §1 stays: samu
   reads Org only on an explicit `samu import org`. The renderer lives in
   `samu-cli`, not in the server or `samu-core`.
9. **A git mirror is operations, not a function of samu:** `deploy/` has a
   script with systemd timers. It runs `samu-server export` (optionally
   with `--org`), commits only when something changed, and pushes the
   result where configured. The files under `files/` do not go into git;
   they stay beside it for the ordinary backup. The interval is a setting
   of the timer (15 minutes suggested). The server knows nothing of git;
   whoever runs samudo without the timer loses nothing but the mirror. Local
   profiles get the same per profile (ADR 0086).
10. **Documentation:** the user documentation describes how to export,
    what the directory looks like, how to restore and what is missing
    (calendar sources, sessions).

## Consequences

- `samu-api` has the export types (manifest, object with stamps) with
  fixtures and `GET /export` in `ROUTES`. catastr is not affected.
- `samu-store` has export and restore. Restore writes past the rules of
  `/changes` (stamps, logbook and `seq` taken over); that is allowed only
  here and only into an empty account.
- Every new kind of object must appear in the export; §7 enforces it.
- The git mirror gives a readable history; its commit author is the timer,
  not the logbook's author.
- CSV or Markdown can later be added as outputs beside Org.
- Open: whether `GET /export` must stream for large accounts. At today's
  sizes it returns one document.

## Changes

None yet.
