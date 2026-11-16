-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- Pairing codes (ADR 0031): traded once, within ten minutes, for a
-- session. Only the SHA-256 is kept, as for sessions; at most one open per
-- account, a new one replaces it.
CREATE TABLE pairing_code (
    account     INTEGER PRIMARY KEY REFERENCES account (id) ON DELETE CASCADE,
    code_hash   BLOB NOT NULL UNIQUE CHECK (length(code_hash) = 32),
    device      TEXT NOT NULL CHECK (length(device) BETWEEN 1 AND 64),
    created     TEXT NOT NULL,
    expires     TEXT NOT NULL
);
