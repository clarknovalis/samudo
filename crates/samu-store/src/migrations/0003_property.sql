-- Properties of a task or project (ADR 0023): free keys, each with its own
-- stamp, so two devices changing different keys both win. A removed key
-- keeps its row with `value IS NULL` and its stamp, as `tagging` does with
-- `present = 0`, so an older `set` cannot bring it back.
CREATE TABLE property (
    owner       INTEGER NOT NULL REFERENCES object (id) ON DELETE CASCADE,
    key         TEXT NOT NULL CHECK (
                    length(key) BETWEEN 1 AND 64
                    AND key GLOB '[a-z]*'
                    AND key NOT GLOB '*[^a-z0-9_-]*'
                ),
    value       TEXT,
    wall        INTEGER NOT NULL,
    counter     INTEGER NOT NULL,
    device      TEXT NOT NULL,
    seq         INTEGER NOT NULL,
    PRIMARY KEY (owner, key)
) WITHOUT ROWID;

-- Tasks the ring delivered before properties existed get `source =
-- index-ring` (ADR 0022 §2, ADR 0023 §5), with the stamp they were created
-- with. Each gets a change number of its own, so clients see them change.
CREATE TEMP TABLE ring_task AS
    SELECT o.id AS owner, row_number() OVER (ORDER BY o.id) AS n
    FROM delivery d JOIN object o ON o.uuid = d.object
    WHERE d.system = 'pebble-index' AND o.deleted_at IS NULL;

INSERT INTO property (owner, key, value, wall, counter, device, seq)
    SELECT r.owner, 'source', 'index-ring', s.wall, s.counter, s.device,
           (SELECT seq FROM account WHERE id = 1) + r.n
    FROM ring_task r JOIN stamp s ON s.object = r.owner AND s.field = '@create';

UPDATE object SET
    seq = (SELECT p.seq FROM property p WHERE p.owner = object.id AND p.key = 'source'),
    modified = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE id IN (SELECT owner FROM ring_task);

UPDATE account SET seq = seq + (SELECT count(*) FROM ring_task) WHERE id = 1;

DROP TABLE ring_task;
