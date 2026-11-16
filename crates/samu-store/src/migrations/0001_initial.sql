-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- samu, the account database (ADR 0018).
--
-- Conventions, as in catastr (catastr ADR 0016):
--
--   * Every table has an INTEGER PRIMARY KEY that never leaves this file.
--     What travels is the uuid, stored as BLOB(16); a UUIDv7 sorts correctly
--     as a big-endian blob.
--   * Timestamps are ISO 8601 text with a Z suffix.
--   * Enums are the snake_case strings serde produces, with CHECK
--     constraints, so a typo — or an `unknown` from a newer client — fails on
--     write instead of silently matching nothing on read.
--   * A reference from one object to another (a task's project, a tag's
--     parent, a link's owner) is the other object's uuid, not a rowid: the
--     model allows a reference to something not (or no longer) there, and
--     the store checks it when it writes (ADR 0017 §7, ADR 0018 §6).

-- ---------------------------------------------------------------- migrations

CREATE TABLE schema_history (
    version     INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    applied_at  TEXT NOT NULL
);

-- ------------------------------------------------------------------- account

-- One row. The change number, the settings ADR 0017 needs, and the highest
-- stamp seen from any device (catastr ADR 0029: kept in a row, not searched
-- for in the stamps).
CREATE TABLE account (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),
    seq                 INTEGER NOT NULL DEFAULT 0 CHECK (seq >= 0),
    zone                TEXT NOT NULL DEFAULT 'Europe/Berlin',
    due_soon            TEXT NOT NULL DEFAULT 'P2D',
    observed_wall       INTEGER NOT NULL DEFAULT 0,
    observed_counter    INTEGER NOT NULL DEFAULT 0,
    observed_device     TEXT NOT NULL DEFAULT ''
);

INSERT INTO account (id) VALUES (1);

-- ------------------------------------------------------------------- objects

-- Every object, whatever its kind: identity, change number, tombstone.
CREATE TABLE object (
    id          INTEGER PRIMARY KEY,
    uuid        BLOB NOT NULL UNIQUE CHECK (length(uuid) = 16),
    kind        TEXT NOT NULL
                    CHECK (kind IN ('folder', 'project', 'task', 'tag', 'link', 'clock')),
    created     TEXT NOT NULL,
    modified    TEXT NOT NULL,
    -- The account's change number of the last change to this object.
    seq         INTEGER NOT NULL,
    -- A tombstone: kept until compaction so a client that was away learns
    -- of the deletion (ADR 0004 §7).
    deleted_at  TEXT
);

CREATE INDEX object_by_seq ON object (seq);

-- The stamp of the last change to each field (ADR 0004 §1). The pseudo-field
-- '@create' holds the stamp of the creation, which a field never set since
-- is compared against.
CREATE TABLE stamp (
    object      INTEGER NOT NULL REFERENCES object (id) ON DELETE CASCADE,
    field       TEXT NOT NULL,
    wall        INTEGER NOT NULL,
    counter     INTEGER NOT NULL,
    device      TEXT NOT NULL,
    -- The change number this stamp was applied at, for answering a change
    -- sent twice with the same `applied` (ADR 0018 §7).
    seq         INTEGER NOT NULL,
    PRIMARY KEY (object, field)
) WITHOUT ROWID;

-- ----------------------------------------------------------------- the kinds

CREATE TABLE folder (
    object      INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    parent      BLOB CHECK (parent IS NULL OR length(parent) = 16),
    status      TEXT NOT NULL CHECK (status IN ('active', 'dropped')),
    position    INTEGER NOT NULL DEFAULT 0
);

-- Dates in two forms (ADR 0007 §1): a local date, or an instant in UTC with
-- its zone. At most one form per date.
CREATE TABLE project (
    object              INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    title               TEXT NOT NULL,
    note                TEXT,
    folder              BLOB CHECK (folder IS NULL OR length(folder) = 16),
    kind                TEXT NOT NULL
                            CHECK (kind IN ('parallel', 'sequential', 'single_actions')),
    status              TEXT NOT NULL
                            CHECK (status IN ('active', 'on_hold', 'completed', 'dropped')),
    defer_date          TEXT,
    defer_at            TEXT,
    defer_zone          TEXT,
    due_date            TEXT,
    due_at              TEXT,
    due_zone            TEXT,
    planned_date        TEXT,
    planned_at          TEXT,
    planned_zone        TEXT,
    due_warning         TEXT,
    planned_duration    TEXT,
    repeat_rule         TEXT,
    repeat_mode         TEXT CHECK (repeat_mode IN
                            ('fixed', 'due_after_completion', 'defer_after_completion')),
    repeat_catch_up     INTEGER CHECK (repeat_catch_up IN (0, 1)),
    review_interval     TEXT,
    next_review         TEXT,
    last_reviewed       TEXT,
    flagged             INTEGER NOT NULL DEFAULT 0 CHECK (flagged IN (0, 1)),
    estimate            TEXT,
    completed_at        TEXT,
    dropped_at          TEXT,
    position            INTEGER NOT NULL DEFAULT 0,
    CHECK (defer_date IS NULL OR defer_at IS NULL),
    CHECK ((defer_at IS NULL) = (defer_zone IS NULL)),
    CHECK (due_date IS NULL OR due_at IS NULL),
    CHECK ((due_at IS NULL) = (due_zone IS NULL)),
    CHECK (planned_date IS NULL OR planned_at IS NULL),
    CHECK ((planned_at IS NULL) = (planned_zone IS NULL)),
    CHECK ((repeat_rule IS NULL) = (repeat_mode IS NULL))
);

CREATE TABLE task (
    object                  INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    title                   TEXT NOT NULL,
    note                    TEXT,
    project                 BLOB CHECK (project IS NULL OR length(project) = 16),
    parent                  BLOB CHECK (parent IS NULL OR length(parent) = 16),
    defer_date              TEXT,
    defer_at                TEXT,
    defer_zone              TEXT,
    due_date                TEXT,
    due_at                  TEXT,
    due_zone                TEXT,
    planned_date            TEXT,
    planned_at              TEXT,
    planned_zone            TEXT,
    due_warning             TEXT,
    planned_duration        TEXT,
    repeat_rule             TEXT,
    repeat_mode             TEXT CHECK (repeat_mode IN
                                ('fixed', 'due_after_completion', 'defer_after_completion')),
    repeat_catch_up         INTEGER CHECK (repeat_catch_up IN (0, 1)),
    flagged                 INTEGER NOT NULL DEFAULT 0 CHECK (flagged IN (0, 1)),
    estimate                TEXT,
    completed_at            TEXT,
    dropped_at              TEXT,
    repeat_of               BLOB CHECK (repeat_of IS NULL OR length(repeat_of) = 16),
    sequential              INTEGER NOT NULL DEFAULT 0 CHECK (sequential IN (0, 1)),
    complete_with_last_child INTEGER NOT NULL DEFAULT 0
                                CHECK (complete_with_last_child IN (0, 1)),
    position                INTEGER NOT NULL DEFAULT 0,
    CHECK (defer_date IS NULL OR defer_at IS NULL),
    CHECK ((defer_at IS NULL) = (defer_zone IS NULL)),
    CHECK (due_date IS NULL OR due_at IS NULL),
    CHECK ((due_at IS NULL) = (due_zone IS NULL)),
    CHECK (planned_date IS NULL OR planned_at IS NULL),
    CHECK ((planned_at IS NULL) = (planned_zone IS NULL)),
    CHECK ((repeat_rule IS NULL) = (repeat_mode IS NULL))
);

CREATE INDEX task_by_project ON task (project);
CREATE INDEX task_by_parent ON task (parent);

CREATE TABLE tag (
    object      INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    parent      BLOB CHECK (parent IS NULL OR length(parent) = 16),
    status      TEXT NOT NULL CHECK (status IN ('active', 'on_hold', 'dropped')),
    exclusive   INTEGER NOT NULL DEFAULT 0 CHECK (exclusive IN (0, 1)),
    position    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE link (
    object      INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    owner       BLOB NOT NULL CHECK (length(owner) = 16),
    uri         TEXT NOT NULL,
    label       TEXT,
    kind        TEXT NOT NULL DEFAULT 'ref' CHECK (kind IN ('ref', 'blocked_by'))
);

CREATE INDEX link_by_owner ON link (owner);

CREATE TABLE clock (
    object      INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    owner       BLOB NOT NULL CHECK (length(owner) = 16),
    start_at    TEXT NOT NULL,
    end_at      TEXT,
    note        TEXT
);

CREATE INDEX clock_by_owner ON clock (owner);

-- ------------------------------------------------------------------- tagging

-- Tags on a task or project: a set, each element with its own stamp
-- (ADR 0004, ADR 0016). A removed element stays with `present = 0`, so an
-- older `add` from another device cannot bring it back.
CREATE TABLE tagging (
    owner       INTEGER NOT NULL REFERENCES object (id) ON DELETE CASCADE,
    tag         BLOB NOT NULL CHECK (length(tag) = 16),
    position    INTEGER NOT NULL DEFAULT 0,
    present     INTEGER NOT NULL CHECK (present IN (0, 1)),
    wall        INTEGER NOT NULL,
    counter     INTEGER NOT NULL,
    device      TEXT NOT NULL,
    seq         INTEGER NOT NULL,
    PRIMARY KEY (owner, tag)
) WITHOUT ROWID;

-- ------------------------------------------------------------------- logbook

-- What happened to a task, beyond the last state the stamps keep (ADR 0006).
-- Written by the server in the same transaction as the change; empty until
-- the server does.
CREATE TABLE logbook (
    id          INTEGER PRIMARY KEY,
    object      INTEGER NOT NULL REFERENCES object (id) ON DELETE CASCADE,
    at          TEXT NOT NULL,
    kind        TEXT NOT NULL,
    note        TEXT,
    old_value   TEXT,
    new_value   TEXT
);

CREATE INDEX logbook_by_object ON logbook (object, at);
