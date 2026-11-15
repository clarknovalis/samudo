-- Scopes (ADR 0035): a new kind of object, its fields and its members.
--
-- `object.kind` has a CHECK listing the kinds, so the table is rebuilt as in
-- 0005_reminder.sql, with foreign keys off and checked before the commit.
CREATE TABLE object_new (
    id          INTEGER PRIMARY KEY,
    uuid        BLOB NOT NULL UNIQUE CHECK (length(uuid) = 16),
    kind        TEXT NOT NULL
                    CHECK (kind IN ('folder', 'project', 'task', 'tag', 'link', 'clock',
                                    'reminder', 'scope')),
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

CREATE TABLE scope (
    object      INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    mode        TEXT NOT NULL CHECK (mode IN ('only', 'except')),
    position    INTEGER NOT NULL DEFAULT 0
);

-- The members of a scope: folders, projects and tags by uuid. A set like
-- `tagging`, each element with its own stamp; a removed one stays with
-- `present = 0`, so an older `add` cannot bring it back (ADR 0004).
CREATE TABLE scope_member (
    owner       INTEGER NOT NULL REFERENCES object (id) ON DELETE CASCADE,
    member      BLOB NOT NULL CHECK (length(member) = 16),
    present     INTEGER NOT NULL CHECK (present IN (0, 1)),
    wall        INTEGER NOT NULL,
    counter     INTEGER NOT NULL,
    device      TEXT NOT NULL,
    seq         INTEGER NOT NULL,
    PRIMARY KEY (owner, member)
) WITHOUT ROWID;
