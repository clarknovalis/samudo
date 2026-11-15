-- Reminders of appointments, read from their VALARMs (ADR 0101 §2): the
-- server's, a JSON list, NULL when there are none.
ALTER TABLE appointment ADD COLUMN alarms TEXT;

-- The next fetch of every calendar reads it whole, so that `alarms` is
-- filled (ADR 0101, Konsequenzen): the version it was read at is forgotten.
UPDATE calendar_source SET etag = NULL, last_modified = NULL, next_fetch = NULL;
