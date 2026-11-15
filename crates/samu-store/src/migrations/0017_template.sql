-- Capture templates (ADR 0073): a new kind of object.
--
-- `object.kind` has a CHECK listing the kinds, so the table is rebuilt as in
-- 0010_calendar.sql, with foreign keys off and checked before the commit.
CREATE TABLE object_new (
    id              INTEGER PRIMARY KEY,
    uuid            BLOB NOT NULL UNIQUE CHECK (length(uuid) = 16),
    kind            TEXT NOT NULL
                        CHECK (kind IN ('folder', 'project', 'task', 'tag', 'link', 'clock',
                                        'reminder', 'scope', 'comment', 'attachment',
                                        'settings', 'calendar', 'appointment', 'template')),
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

-- The title is the call `/title`: letters, digits, `-`, `_`; unique without
-- regard to case among live templates, which the store checks (a tombstone
-- keeps its row). The text holds only marks; samu-core checks it on write.
CREATE TABLE template (
    object      INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    title       TEXT NOT NULL CHECK (length(title) > 0),
    text        TEXT NOT NULL DEFAULT '',
    reminders   TEXT,
    position    INTEGER NOT NULL DEFAULT 0
);
