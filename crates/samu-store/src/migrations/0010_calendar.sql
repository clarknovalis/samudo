-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- Calendars read from elsewhere and their appointments (ADR 0047, 0048).
--
-- `object.kind` has a CHECK listing the kinds, so the table is rebuilt as in
-- 0009_settings.sql, with foreign keys off and checked before the commit.
CREATE TABLE object_new (
    id              INTEGER PRIMARY KEY,
    uuid            BLOB NOT NULL UNIQUE CHECK (length(uuid) = 16),
    kind            TEXT NOT NULL
                        CHECK (kind IN ('folder', 'project', 'task', 'tag', 'link', 'clock',
                                        'reminder', 'scope', 'comment', 'attachment',
                                        'settings', 'calendar', 'appointment')),
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

-- The user's: everything but fetched_at, error and host, which the server
-- writes (ADR 0047 §2).
CREATE TABLE calendar (
    object          INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    title           TEXT NOT NULL,
    kind            TEXT NOT NULL CHECK (kind IN ('ics', 'caldav')),
    color           TEXT,
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    -- Minutes; a week at most.
    refresh         INTEGER NOT NULL DEFAULT 15 CHECK (refresh BETWEEN 5 AND 10080),
    link_template   TEXT,
    position        INTEGER NOT NULL DEFAULT 0,
    fetched_at      TEXT,
    error           TEXT,
    host            TEXT
);

-- Written by the server alone (ADR 0047 §4). One row per VEVENT: the main
-- one (no recurrence_id) or a changed instance of it.
CREATE TABLE appointment (
    object              INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    calendar            BLOB NOT NULL CHECK (length(calendar) = 16),
    uid                 TEXT NOT NULL,
    recurrence_id_date  TEXT,
    recurrence_id_at    TEXT,
    recurrence_id_zone  TEXT,
    title               TEXT NOT NULL,
    start_date          TEXT,
    start_at            TEXT,
    start_zone          TEXT,
    end_date            TEXT,
    end_at              TEXT,
    end_zone            TEXT,
    location            TEXT,
    note                TEXT,
    url                 TEXT,
    status              TEXT NOT NULL CHECK (status IN ('confirmed', 'tentative', 'cancelled')),
    busy                INTEGER NOT NULL CHECK (busy IN (0, 1)),
    rrule               TEXT,
    -- JSON lists of dates in the wire form, NULL when empty.
    rdate               TEXT,
    exdate              TEXT,
    CHECK ((start_date IS NULL) <> (start_at IS NULL))
);
CREATE INDEX appointment_by_calendar ON appointment (calendar, uid);

-- Where a calendar comes from (ADR 0047 §3): never in a change, never in an
-- answer. `sealed` is XChaCha20-Poly1305 over the JSON of the source, the
-- 24-byte nonce first, the calendar's uuid as associated data; the key is
-- the server's, from its environment.
CREATE TABLE calendar_source (
    calendar        INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    sealed          BLOB NOT NULL CHECK (length(sealed) > 40),
    -- For the next conditional GET.
    etag            TEXT,
    last_modified   TEXT,
    -- When the server fetches it next; NULL: at once.
    next_fetch      TEXT
);

-- ADR 0047 §12: the calendars a scope shows, a JSON list of ids; NULL: all.
ALTER TABLE scope ADD COLUMN calendars TEXT;
