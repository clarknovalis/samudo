-- The account's settings as an object (ADR 0045 §4), so they sync and can
-- be changed offline like anything else.
--
-- `object.kind` has a CHECK listing the kinds, so the table is rebuilt as in
-- 0007_comment.sql, with foreign keys off and checked before the commit.
-- The one settings object is made by the store when it opens the file, with
-- a UUIDv7 and the values of account.due_soon and account.remind_time;
-- those two columns stay, unread from then on.
CREATE TABLE object_new (
    id              INTEGER PRIMARY KEY,
    uuid            BLOB NOT NULL UNIQUE CHECK (length(uuid) = 16),
    kind            TEXT NOT NULL
                        CHECK (kind IN ('folder', 'project', 'task', 'tag', 'link', 'clock',
                                        'reminder', 'scope', 'comment', 'attachment',
                                        'settings')),
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

CREATE TABLE settings (
    object              INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    due_soon            TEXT NOT NULL,
    remind_time         TEXT NOT NULL,
    forecast_days       INTEGER NOT NULL DEFAULT 7 CHECK (forecast_days BETWEEN 1 AND 366),
    forecast_flagged    INTEGER NOT NULL DEFAULT 1 CHECK (forecast_flagged IN (0, 1)),
    forecast_tag        BLOB CHECK (forecast_tag IS NULL OR length(forecast_tag) = 16),
    forecast_defer      INTEGER NOT NULL DEFAULT 1 CHECK (forecast_defer IN (0, 1)),
    forecast_done       INTEGER NOT NULL DEFAULT 0 CHECK (forecast_done IN (0, 1))
);
