-- How an account came to be and when it was last used (ADR 0099 §4).
--
-- `registered`: `cli` from `samu-server account add`, `demo` and `open`
-- from `POST /register` in that mode. Accounts from before are `cli`.
-- `last_active`: the last sign-in, otherwise moved on at most hourly while
-- the account is used; a demo account is deleted some days after it.
-- NULL until the first sign-in: read as `created`.
ALTER TABLE account ADD COLUMN registered TEXT NOT NULL DEFAULT 'cli'
    CHECK (registered IN ('cli', 'demo', 'open'));
ALTER TABLE account ADD COLUMN last_active TEXT;
