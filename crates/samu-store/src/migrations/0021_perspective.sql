-- Perspectives (ADR 0090): a new kind of object.
--
-- `object.kind` has a CHECK listing the kinds, so the table is rebuilt as in
-- 0017_template.sql, with foreign keys off and checked before the commit.
CREATE TABLE object_new (
    id              INTEGER PRIMARY KEY,
    uuid            BLOB NOT NULL UNIQUE CHECK (length(uuid) = 16),
    kind            TEXT NOT NULL
                        CHECK (kind IN ('folder', 'project', 'task', 'tag', 'link', 'clock',
                                        'reminder', 'scope', 'comment', 'attachment',
                                        'settings', 'calendar', 'appointment', 'template',
                                        'perspective')),
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

-- The title is unique without regard to case among live perspectives, which
-- the store checks (a tombstone keeps its row). Chips name projects, tags,
-- folders and states by id; one that is deleted later counts for nothing,
-- as in a scope's filter (ADR 0043 §2). `filter` and `view` hold the wire
-- word; `arrange` and `board` their JSON.
CREATE TABLE perspective (
    object      INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    title       TEXT NOT NULL CHECK (length(title) > 0),
    icon        TEXT,
    text        TEXT,
    chips       TEXT,
    filter      TEXT,
    arrange     TEXT,
    view        TEXT,
    board       TEXT,
    position    INTEGER NOT NULL DEFAULT 0
);
