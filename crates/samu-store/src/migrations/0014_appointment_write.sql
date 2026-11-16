-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- Writing appointments back (ADR 0068). Columns that may be NULL or have a
-- default: no rebuild.

-- The server's: whether the source lets samu write (§1), whether an
-- appointment is an invitation (§2).
ALTER TABLE calendar ADD COLUMN writable INTEGER NOT NULL DEFAULT 0 CHECK (writable IN (0, 1));
ALTER TABLE appointment ADD COLUMN meeting INTEGER NOT NULL DEFAULT 0 CHECK (meeting IN (0, 1));

-- Where the VEVENT lives on the CalDAV server, and the ETag it had when it
-- was last read or written (§6). The server's alone, never in a change.
ALTER TABLE appointment ADD COLUMN href TEXT;
ALTER TABLE appointment ADD COLUMN etag TEXT;

-- The calendar a new appointment goes into (§10).
ALTER TABLE settings ADD COLUMN default_calendar BLOB
    CHECK (default_calendar IS NULL OR length(default_calendar) = 16);

-- What is still to be written, one row per resource: a calendar and a UID
-- (§6). `fields` lists what clients changed and the server has not written,
-- as JSON [[object, field, hlc], ...], for the rule of §7. A move from
-- another calendar keeps where the resource was, to delete it there.
-- `version` goes up with every change, so the writer finishing an older
-- one leaves a newer one in place.
CREATE TABLE appointment_write (
    calendar        INTEGER NOT NULL REFERENCES object (id) ON DELETE CASCADE,
    uid             TEXT NOT NULL,
    from_calendar   INTEGER REFERENCES object (id) ON DELETE SET NULL,
    from_href       TEXT,
    fields          TEXT NOT NULL DEFAULT '[]',
    version         INTEGER NOT NULL DEFAULT 1,
    since           TEXT NOT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    next_try        TEXT NOT NULL,
    last_error      TEXT,
    PRIMARY KEY (calendar, uid)
);

-- A change to an appointment in the logbook names its field (§3).
ALTER TABLE logbook ADD COLUMN field TEXT;
