---
number: 0039
title: Comments, attachments and author – the server sets the author from the session
status: accepted
date: 2026-11-08
adds_to: ["0006", "0019", "0024", "0038"]
---

# 0039 – Comments, attachments and author

## Context

Two wishes: **comments**, with attachments such as pasted images, enough
to carry an optional Jira-like workflow; and **attachments** (audio,
photos and more) on objects, with camera and recording in the Android
app and recording in the web UI. Both together, plus an author that
tells an agent from a person. samu knew only the device, and only from a
change's stamp, which the client writes itself for changes from its
outbox. Agents were to write their reports as comments and images as
attachments (ADR 0038 §4).

The earlier research proposed attachments as links into catastr instead
of blobs of samu's own. That would have needed app and web UI to log in
to catastr just to take a photo, and made samu depend on catastr for
attachments.

## Decision

1. **Author.** `Author { device, kind }` with `kind` = `human` or
   `agent`.
   - **The server sets the author from the session** that brings the
     change, also when it comes from an outbox. An author sent by a
     client is ignored.
   - Every new object gets the author of its creation: columns
     `author_device` and `author_kind` in `object`; in the API the field
     `created_by`, on task, project, comment and attachment.
   - Every logbook entry (ADR 0024 §5) carries the `kind` next to
     `device`.
   - What existed before this record has no author (`created_by` absent).
2. **Kind of session.**
   - Set **at login**: `Login.kind`, optional, default `human`. `samu
     login --agent` sets `agent`.
   - It is in `SessionInfo.kind` and in the column `session.kind` of
     `server.db` (server migration 4). Existing sessions became `human`.
   - A session from pairing (ADR 0031) is always `human`.
   - The account menu shows the kind.
   - An agent is trusted to log in honestly as an agent and to use only
     its own session. A session can additionally be restricted to some
     objects (ADR 0095).
3. **Comment:** a new kind `comment`.
   - Fields: `owner` (task or project), `body` (text like the note) and
     `created_by`.
   - Editing is `set body`; `modified` shows the change. Deleting leaves
     a tombstone (ADR 0004 §7).
   - **Only the same kind of session that wrote a comment may change or
     delete it.** An agent changes no comment of a person and vice versa;
     otherwise the store answers `Rejected`. Comments without an author
     count as `human`.
   - The interfaces show comments below the note, oldest first, with
     author and time. An author that is an agent gets an "Agent" mark.
4. **Blobs: samu stores files itself.** Links to `catastr:<uuid>` remain
   possible beside them (ADR 0012).
   - A blob is stored under its SHA-256 and never changes, at
     `<SAMU_DATA>/accounts/<account>.blobs/<ab>/<hash>`, so separate per
     account (ADR 0005), and is backed up with the volume as a file.
   - Table `blob(hash, size, media_type, created)` in the account
     database.
   - `PUT /blobs/{sha256}` with a session:
     - The server streams the upload into a temporary file without
       holding it in memory, checks the hash and then renames it.
     - A wrong hash is rejected with `bad_request`.
     - A blob that exists is not written again; the response is the
       same.
     - Limit: 50 MB per file; a server may lower it with
       `SAMU_BLOB_MAX_BYTES` and cap an account with
       `SAMU_BLOB_QUOTA_BYTES` (ADR 0097 §2). Above it 413.
   - `GET /blobs/{sha256}` with a session returns the file with its media
     type. The web UI passes with its cookie, so `<img>` and `<audio>`
     suffice.
   - Blobs that no attachment points to stay for now.
5. **Attachment:** a new kind `attachment`.
   - Fields: `owner` (task, project or comment), `blob` (the hash),
     `name`, `media_type`, `size` and `created_by`.
   - The store rejects an attachment whose blob is not there.
   - Changing (`name`) and deleting follow §3.
   - Deleting removes the attachment only; the blob stays.
6. **Interfaces.**
   - **CLI:** `samu comment <id> <text|->` and `samu comments <id>`;
     `samu attach <id> <file> [--to-comment <id>]` and `samu attachments
     <id>`; `samu blob get <hash>` writes the file to stdout. Every new
     route has its command.
   - **Web UI:** sections "Comments" and "Attachments" in the inspector.
     A pasted or dropped image becomes an attachment of the comment being
     written. A button records audio with `MediaRecorder`
     (`audio/webm`). Images show as a preview, audio with a player.
     Comments can be edited in place.
   - **Android:** the same sections on the task page. Photos through
     `TakePicture` with a FileProvider, audio with `MediaRecorder`.
     Offline a blob waits in `blob_outbox` in the cache of
     `samu-mobile` and is uploaded before the outbox sends the
     attachment.
7. **Agents** (adds to ADR 0038 §4) write their reports as comments and
   images as attachments, not as paragraphs at the end of the note, and
   log in with `--agent`.
8. **Later, each with its own record:** transcripts of audio; preview
   images; cleaning up orphaned blobs; mentions; showing comments in
   catastr.

## Consequences

- `samu-api` 0.1.13 has `Author`, `AuthorKind`, `Comment`, `Attachment`,
  `created_by`, `Login.kind`, `SessionInfo.kind`, `LogEntry.kind` and the
  new routes, each with fixtures. Readers of `samu-api`, catastr among
  them, must skip the new kinds; `ObjectKind::Unknown` covers that (ADR
  0015).
- Account migration 7 rebuilds `object` as migrations 5 and 6 did: new
  kinds in the CHECK and the author columns. It was rehearsed on a copy
  of a real database first.
- `apply_all` in the store receives the session's author. The rules of
  §3 and §5 are checked by the store, not the client.
- The data volume grows with attachments; the per-file limit and backup
  as plain files keep that manageable.
- An existing agent session had to log in again with `--agent`.

## Changes

- 2026-11-13 (ADR 0097 §2): §4 had a fixed limit of 50 MB per file and
  no limit per account.
