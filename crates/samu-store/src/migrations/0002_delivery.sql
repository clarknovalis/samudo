-- What a hook delivered, by the sender's own id (ADR 0022 §1): the same
-- delivery again finds the task it made instead of making a second one.
-- Written in the same transaction as the task. Bookkeeping of the server;
-- it does not travel.
CREATE TABLE delivery (
    -- Whose id it is: `pebble-index` for the ring.
    system      TEXT NOT NULL,
    delivery_id TEXT NOT NULL CHECK (length(delivery_id) BETWEEN 1 AND 128),
    object      BLOB NOT NULL CHECK (length(object) = 16),
    received    TEXT NOT NULL,
    PRIMARY KEY (system, delivery_id)
) WITHOUT ROWID;
