-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- Checklists (ADR 0141): a new kind of object, one item of a task's list.
--
-- `object.kind` has a CHECK listing the kinds, so the table is rebuilt as in
-- 0022_habit.sql, with foreign keys off and checked before the commit.
CREATE TABLE object_new (
    id              INTEGER PRIMARY KEY,
    uuid            BLOB NOT NULL UNIQUE CHECK (length(uuid) = 16),
    kind            TEXT NOT NULL
                        CHECK (kind IN ('folder', 'project', 'task', 'tag', 'link', 'clock',
                                        'reminder', 'scope', 'comment', 'attachment',
                                        'settings', 'calendar', 'appointment', 'template',
                                        'perspective', 'habit_entry', 'checklist_item')),
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

-- One item of a task's checklist (ADR 0141 §1).
CREATE TABLE checklist_item (
    object      INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    owner       BLOB NOT NULL CHECK (length(owner) = 16),
    text        TEXT NOT NULL,
    checked     INTEGER NOT NULL DEFAULT 0 CHECK (checked IN (0, 1)),
    position    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX checklist_item_by_owner ON checklist_item (owner, position);
