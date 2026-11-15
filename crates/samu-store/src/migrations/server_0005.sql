-- The grant of a restricted session (ADR 0095 §1): the folders and projects
-- it may reach and whether it may write, as the JSON of `samu_api::Grant`.
-- NULL is a session of the whole account, as every session before.
-- Set at sign-in only; it never changes after.
ALTER TABLE session ADD COLUMN "grant" TEXT;
