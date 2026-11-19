---
number: 0097
title: Hardening a public instance – a login brake, limits on attachments, allowed calendar hosts
status: accepted
date: 2026-11-13
adds_to: ["0019", "0039", "0047"]
---

# 0097 – Hardening a public instance

## Context

samudo runs publicly as a demo on a small machine (half a CPU, 256 MB).
A regular reset limits the damage but protects neither against load nor
against a full disk between two resets. Three open flanks:

1. `POST /login` checks every password with Argon2 without a limit: a
   visitor can guess and, more importantly, bind half the demo's CPU.
   `POST /pair` is just as unlimited.
2. `PUT /blobs/{sha256}` takes up to 50 MB per file (ADR 0039 §4), without
   a limit per account.
3. With `SAMU_SECRET_KEY` set (for the demo's calendars) the server
   fetches any address someone sets as a calendar source, including
   addresses inside the host's private network (SSRF).

## Decision

### §1 Login brake

Always on, in memory only; a restart forgets it.

- Per account name (lower case, also for unknown names): after **5 failed
  attempts in 15 minutes** `/login` answers that name with **429** and
  `Retry-After` until the window ends, without computing Argon2. A
  successful login resets the counter.
- At most **2 password checks at a time** in the whole server; more wait.
  This bounds the CPU even with changing names.
- `/pair`: after **20 failed attempts in 15 minutes** in the whole server,
  429 until the window ends. Registration has a brake of its own (ADR 0099
  §3).
- No new `ErrorCode`, so no new version of `samu-api`: 429 carries
  `unauthorized` and the message "too many failed sign-ins, try again in N
  s", as 413 carries `bad_request`.
- No brake per IP: behind a reverse proxy that would need a trusted
  `X-Forwarded-For`. Later, if needed.

### §2 Limits for attachments

Server configuration; without it everything stays as before.

- `SAMU_BLOB_MAX_BYTES`: limit per file, at most and by default 50 MB.
- `SAMU_BLOB_QUOTA_BYTES`: limit per account, the sum of `blob.size`;
  without it unlimited.
- An upload is checked against the smaller of both, the per-file limit and
  what is still free in the account; above it 413 as before. A file the
  account already has does not count again.

### §3 Allowed calendar hosts

- `SAMU_CALENDAR_HOSTS=host1,host2`: when set, the server fetches only
  addresses on these hosts. Another one is refused when setting the source
  (`PUT /calendars/{id}/source`, 400), on discovery (`POST
  /calendars/discover`, also after every redirect), on fetching and on
  writing back (ADR 0068): a redirect to another host is not followed.
- Without the variable any host, as before.

### §4 The demo

The public demo sets `SAMU_CALENDAR_HOSTS` to its own CalDAV host, a
per-file limit of 5 MB and a quota of 50 MB per account. Its deployment is
the maintainer's installation and not part of this repository.

## Consequences

- Private servers and local profiles change only through the login brake.
- Whoever mistypes five times waits up to 15 minutes; `samu-server
  account passwd` does not lift that, a restart does.
- Someone can block the login of a known account for 15 minutes by
  sending wrong passwords. Existing sessions are not affected; for the
  demo, whose password is public, only the CPU matters anyway.

## Changes

None yet.
