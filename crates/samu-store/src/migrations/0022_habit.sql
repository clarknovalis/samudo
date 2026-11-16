-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- Habits (ADR 0091): a field on the task and a new kind of object for the
-- entries.
--
-- `object.kind` has a CHECK listing the kinds, so the table is rebuilt as in
-- 0021_perspective.sql, with foreign keys off and checked before the commit.
CREATE TABLE object_new (
    id              INTEGER PRIMARY KEY,
    uuid            BLOB NOT NULL UNIQUE CHECK (length(uuid) = 16),
    kind            TEXT NOT NULL
                        CHECK (kind IN ('folder', 'project', 'task', 'tag', 'link', 'clock',
                                        'reminder', 'scope', 'comment', 'attachment',
                                        'settings', 'calendar', 'appointment', 'template',
                                        'perspective', 'habit_entry')),
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

INSERT INTO object_new (id, uuid, kind, created, modified, seq, deleted_at, author_device, author_kind)
    SELECT id, uuid, kind, created, modified, seq, deleted_at, author_device, author_kind FROM object;

DROP TABLE object;
ALTER TABLE object_new RENAME TO object;
CREATE INDEX object_by_seq ON object (seq);

-- What makes a task a habit, as its JSON (ADR 0091 §1); NULL for every
-- other task.
ALTER TABLE task ADD COLUMN habit TEXT;

-- One entry per tap (ADR 0091 §2). `day` is the account's date as
-- `YYYY-MM-DD`; the store checks that it is not after today and that
-- `amount` and `until` go with their status.
CREATE TABLE habit_entry (
    object      INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    habit       BLOB NOT NULL CHECK (length(habit) = 16),
    day         TEXT NOT NULL,
    status      TEXT NOT NULL,
    -- The number as JSON text; the store checks it is at least 1.
    amount      TEXT,
    until       TEXT
);
CREATE INDEX habit_entry_by_habit ON habit_entry (habit, day);
