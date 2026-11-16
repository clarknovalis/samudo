-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- Comments, attachments, blobs and the author of an object (ADR 0039).
--
-- `object.kind` has a CHECK listing the kinds, so the table is rebuilt as in
-- 0006_scope.sql, with foreign keys off and checked before the commit. The
-- author comes from the session that made the object; what was made before
-- has none (both columns NULL).
CREATE TABLE object_new (
    id              INTEGER PRIMARY KEY,
    uuid            BLOB NOT NULL UNIQUE CHECK (length(uuid) = 16),
    kind            TEXT NOT NULL
                        CHECK (kind IN ('folder', 'project', 'task', 'tag', 'link', 'clock',
                                        'reminder', 'scope', 'comment', 'attachment')),
    created         TEXT NOT NULL,
    modified        TEXT NOT NULL,
    -- The account's change number of the last change to this object.
    seq             INTEGER NOT NULL,
    -- A tombstone: kept until compaction so a client that was away learns
    -- of the deletion (ADR 0004 §7).
    deleted_at      TEXT,
    author_device   TEXT,
    author_kind     TEXT CHECK (author_kind IN ('human', 'agent')),
    CHECK ((author_device IS NULL) = (author_kind IS NULL))
);

INSERT INTO object_new (id, uuid, kind, created, modified, seq, deleted_at)
    SELECT id, uuid, kind, created, modified, seq, deleted_at FROM object;

DROP TABLE object;
ALTER TABLE object_new RENAME TO object;
CREATE INDEX object_by_seq ON object (seq);

-- A comment on a task or project (ADR 0039 §3).
CREATE TABLE comment (
    object      INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    owner       BLOB NOT NULL CHECK (length(owner) = 16),
    body        TEXT NOT NULL
);

CREATE INDEX comment_by_owner ON comment (owner);

-- A file kept by the server, named by its SHA-256 (ADR 0039 §4). The bytes
-- are in `<account>.blobs/` next to this file; a row here means the file is
-- there and whole.
CREATE TABLE blob (
    hash        TEXT PRIMARY KEY
                    CHECK (length(hash) = 64 AND hash NOT GLOB '*[^0-9a-f]*'),
    size        INTEGER NOT NULL CHECK (size >= 0),
    media_type  TEXT NOT NULL,
    created     TEXT NOT NULL
) WITHOUT ROWID;

-- A file on a task, a project or a comment (ADR 0039 §5).
CREATE TABLE attachment (
    object      INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    owner       BLOB NOT NULL CHECK (length(owner) = 16),
    blob        TEXT NOT NULL REFERENCES blob (hash),
    name        TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 255),
    media_type  TEXT NOT NULL,
    size        INTEGER NOT NULL CHECK (size >= 0)
);

CREATE INDEX attachment_by_owner ON attachment (owner);

-- Whether a person or an agent made the change (ADR 0039 §1); NULL before.
ALTER TABLE logbook ADD COLUMN author_kind TEXT CHECK (author_kind IN ('human', 'agent'));
