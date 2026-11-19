---
number: 0099
title: Registration (demo accounts or open) and demo content on request
status: accepted
date: 2026-11-13
adds_to: ["0019", "0074", "0095", "0097", "0098"]
replaces: ["0019 §3 (accounts only through the command line)"]
---

# 0099 – Registration and demo content

## Context

Until now only `samu-server account add` created accounts; ADR 0019 §3
said accounts are managed by `samu-server account …`, never over HTTP.
The public demo (ADR 0097, 0098) had two shared accounts, `demo-de` and
`demo-en`, refilled every hour by a script over the CLI. Everybody trying
the demo worked in the same data.

The wish: registration, configurable as demo accounts only or open; a new
account created with demo content, or demo content fetched into an
existing account later. Chosen: demo accounts expire after a period of
rest; a "Try it" button next to a form; fetching later brings only
fenced-off content; the demo content is built in Rust in the server, the
seeding script goes.

samu could not delete an account (only `disable`). Expiring demo accounts
and a reset without wiping the volume need it.

## Decision

### §1 Three modes

`SAMU_REGISTRATION=off|demo|open`, default `off`.

- `off`: as before, accounts only through `samu-server account …`.
- `demo`: anyone can create a **demo account** (§4); it always gets demo
  content and expires.
- `open`: anyone can create an ordinary account, demo content optional.
  Meant for self-hosters (family, club), **not** for a public demo, where
  it would put strangers' real data on the server.

This changes ADR 0019 §3: with `demo` or `open`, `POST /register` creates
accounts over HTTP. Changing, disabling and setting passwords stay with
`samu-server account …`; deleting one's own account is §5.

### §2 The route

- `GET /register` without a session: `RegistrationInfo { mode, languages
  }` (`languages` = `["de","en"]`), so the login page knows what to show.
  With `off` only `mode: "off"`.
- `POST /register` without a session, body `Register { name?, password?,
  demo? }`, `demo` being the language of the demo content or empty:
  - `demo` mode without a name: the server makes `demo-` plus six
    characters and a password and returns both **once** ("Try it"). With
    a name a password is needed (at least 8 characters). Empty `demo`
    means the language from `Accept-Language`, otherwise English.
  - `open` mode: name and password required, `demo` optional.
  - Response `Registered { account, password?, token, expires }` and the
    cookie `samu_session` as for `POST /login`; one is logged in at once.
- Errors: `off` → 403; name taken → 409; invalid name or short password →
  400; a limit of §3 → 429 or 503. No new `ErrorCode`.
- CLI: `samu register [<name>] [--password-stdin] [--demo de|en]`,
  `samu register --info`.

### §3 Limits

- At most 10 registrations per hour per server, kept in memory like the
  brake on `/pair` (ADR 0097) → 429 with `Retry-After`.
- `SAMU_REGISTRATION_MAX` (default 100): this many registered accounts at
  once; above it 503 with a message. Accounts from `account add` do not
  count.
- The limit on concurrent password checks and `SAMU_BLOB_*` apply as to
  every account.

### §4 Demo accounts expire after rest

- Server migration 6: columns `account.registered` (how it was created:
  `cli`, `demo`, `open`) and `account.last_active` (login, otherwise
  updated at most hourly while in use).
- A demo account is deleted (§5) `SAMU_DEMO_ACCOUNT_DAYS` days (default
  7) after `last_active`, checked hourly by the server.
- `SessionInfo.demo_until` (a date) carries the expiry of a demo account.
  The banner of ADR 0098 shows, in one's own demo account, instead of
  "shared data": "Your demo account; deleted on … unless you log in
  before". Without `SAMU_DEMO_RESET_MINUTES` this banner also appears as
  soon as `demo_until` is set.

### §5 Deleting accounts

- `samu-server account delete <name>` ends the sessions and deletes the
  row in `server.db`, `accounts/<name>.db` with its WAL, and the
  account's blobs. Only with the server stopped or under the same lock as
  the running server; the server itself deletes expired demo accounts
  (§4).
- A **registered** account can delete itself: `POST /account/delete`
  with the password in the body, `samu account delete`, "Delete account"
  in the web's account menu. For accounts from `account add` 403: those
  are managed by whoever runs the server. Not for restricted sessions
  (ADR 0095), not at the local replica.

### §6 The demo content in the server

- The content lives in `crates/samu-server/demo/{de,en}.toml`, embedded
  in the binary; format: `{+N}` for dates relative to today, `line` as
  quick-capture text, `history`, and so on.
- Module `demo` in `samu-server` reads it, computes dates from today in
  the account's zone, builds the objects through the rules of
  `samu-core` (quick capture, habits, clock) and writes them the same way
  as `POST /changes`, with server stamps and device `demo`; attachments
  as blobs. A test seeds both languages into an empty account and counts
  per kind.
- `samu-server account add <name> --demo de|en [--caldav <base>]`:
  account with content. Only there, never over HTTP, are there calendars
  and appointments (`[[calendars]]`, `[[appointments]]`); the
  collections must exist on the CalDAV server, whose password comes from
  `SAMU_DEMO_CALDAV_PASSWORD`.
- Resetting the shared demo accounts is `account delete` followed by
  `account add --demo`, without wiping the volume, so registered demo
  accounts survive the hourly reset.

### §7 Demo content later

- `POST /demo { lang }` with a session, `samu demo de|en`, button "Fetch
  demo content" under Settings > Account.
- An **empty** account (no tasks, no projects) gets everything, as on
  registration, including settings, scopes, perspectives and templates.
- An account with content gets only **fenced-off content**: a folder
  "Demo" with the folders and projects below it, the inbox as the project
  "Demo/Inbox", tags under a tag "Demo", habits with their history,
  comments, clock entries, attachments, blockers. No settings, property
  links, scopes, perspectives or templates. If a folder "Demo" exists →
  409. Removed again with "delete project/folder with its tasks".
- Not for restricted sessions, not at the replica (online only).

### §8 Web

When `mode` is not `off`, the login page shows "Create account" below the
form (name, password, language of the demo content; in `open` also
"none"), and in `demo` mode also the button "Try it". After "Try it" a
dialog shows name and password once, to write down.

### §9 The public demo

The public demo runs with `SAMU_REGISTRATION=demo` next to the shared
`demo-de`/`demo-en`; the banner before login adds "or try an account of
your own". `docs/self-hosting.md` describes the three modes, for `open`
with the note that strangers' data then sits on the server (backup,
deletion, privacy). Where the demo runs is the maintainer's installation
and not part of this record.

## Consequences

- `samu-api` has `RegistrationInfo`, `Register`, `Registered`,
  `DemoRequest` and `SessionInfo.demo_until`, each with a fixture.
- The demo content has exactly one version, in Rust; changing it means
  changing the TOML files in the server crate.
- Registered accounts get no calendar; they can add their own CalDAV
  server like any account.
- A demo account without login is gone with everything after a week; the
  banner says so.

## Changes

None yet.
