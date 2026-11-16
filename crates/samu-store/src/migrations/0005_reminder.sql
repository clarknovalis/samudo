-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- Reminders (ADR 0030): a new kind of object, and the time of day a
-- reminder on a date without a time goes off.
--
-- `object.kind` has a CHECK listing the kinds, and SQLite cannot change a
-- CHECK. So the table is rebuilt the way SQLite documents it (ALTER TABLE,
-- "Making Other Kinds Of Table Schema Changes"): this migration runs with
-- foreign keys off, so dropping the old table deletes nothing below it, and
-- the runner checks the foreign keys before it commits.
CREATE TABLE object_new (
    id          INTEGER PRIMARY KEY,
    uuid        BLOB NOT NULL UNIQUE CHECK (length(uuid) = 16),
    kind        TEXT NOT NULL
                    CHECK (kind IN ('folder', 'project', 'task', 'tag', 'link', 'clock',
                                    'reminder')),
    created     TEXT NOT NULL,
    modified    TEXT NOT NULL,
    -- The account's change number of the last change to this object.
    seq         INTEGER NOT NULL,
    -- A tombstone: kept until compaction so a client that was away learns
    -- of the deletion (ADR 0004 §7).
    deleted_at  TEXT
);

INSERT INTO object_new (id, uuid, kind, created, modified, seq, deleted_at)
    SELECT id, uuid, kind, created, modified, seq, deleted_at FROM object;

DROP TABLE object;
ALTER TABLE object_new RENAME TO object;
CREATE INDEX object_by_seq ON object (seq);

CREATE TABLE reminder (
    object      INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    owner       BLOB NOT NULL CHECK (length(owner) = 16),
    anchor      TEXT NOT NULL CHECK (anchor IN ('at', 'due', 'planned', 'defer')),
    at          TEXT,
    before      TEXT,
    CHECK ((anchor = 'at') = (at IS NOT NULL))
);

CREATE INDEX reminder_by_owner ON reminder (owner);

ALTER TABLE account ADD COLUMN remind_time TEXT NOT NULL DEFAULT '09:00';
