-- Named hooks a sender outside samu delivers to (ADR 0010, ADR 0022 §3).
--
-- In the server file, not an account database: the URL `/hooks/ring` names
-- no account, so a name is unique on the server and the hook says whose
-- inbox it fills.
CREATE TABLE hook (
    id          INTEGER PRIMARY KEY,
    -- A path segment: `POST /hooks/{name}`.
    name        TEXT NOT NULL UNIQUE
                    CHECK (length(name) BETWEEN 1 AND 64
                           AND name NOT GLOB '*[^a-z0-9_-]*'),
    account     INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    format      TEXT NOT NULL CHECK (format IN ('index-v1')),
    -- The HMAC key, hex. In the clear, unlike a session token, which is
    -- kept as a hash: checking a signature needs the key itself (catastr
    -- migration 0061). It can deliver to this one hook and nothing else.
    secret      TEXT NOT NULL CHECK (length(secret) = 64),
    created     TEXT NOT NULL
);

CREATE INDEX hook_by_account ON hook (account);
