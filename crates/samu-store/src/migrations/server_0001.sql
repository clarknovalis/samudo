-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- samu, the server file: accounts and sessions (ADR 0019 §2, §3).
-- Conventions as in the account database (0001_initial.sql).

CREATE TABLE schema_history (
    version     INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    applied_at  TEXT NOT NULL
);

-- An account is a sign-in and a database of its own (ADR 0005); its name is
-- also the database's file name, hence the narrow alphabet.
CREATE TABLE account (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE
                        CHECK (length(name) BETWEEN 1 AND 32
                               AND name NOT GLOB '*[^a-z0-9_-]*'),
    -- Argon2id with parameters and salt inside (catastr ADR 0022).
    password_hash   TEXT NOT NULL,
    created         TEXT NOT NULL,
    disabled_at     TEXT
);

-- A session is a row the server can end. Only the SHA-256 of the token is
-- stored, so a copy of this file hands over no live session.
CREATE TABLE session (
    token_hash  BLOB PRIMARY KEY CHECK (length(token_hash) = 32),
    account     INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    device      TEXT NOT NULL,
    created     TEXT NOT NULL,
    expires     TEXT NOT NULL
) WITHOUT ROWID;

CREATE INDEX session_by_account ON session (account);
