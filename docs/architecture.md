# Architecture

For contributors: how the pieces fit, and why. The maintainer keeps a
design record for every decision outside this repository; a comment that
cites `ADR NNNN` points to one. What matters for a change is said here, in
the code, or in the commit message.

## Principles

- **The server holds the truth.** Clients keep a read cache and an outbox
  of field changes stamped with a hybrid logical clock (HLC) and their
  device. The server decides last-writer-wins per field. There is no
  second operation log.
- **One account, one database.** Each account is its own SQLite file;
  `server.db` holds accounts and sessions.
- **Availability is computed, never stored**, in exactly one place:
  `samu-core`. The server and the phone call the same code.
- **Time:** a whole day is a local date without zone; a time is an instant
  plus zone. Repetition is an RRULE plus one of three modes.
- **The API is the core.** Every interface is a client; every route has a
  command in the CLI.

## Key decisions

**Sync.** Each account has a change number, `seq`, and every field the
stamp of its last change: a hybrid logical clock `{wall, counter, device}`.
A change is one of five operations, `create`, `set`, `add`, `remove` and
`delete`, and the server answers each with `applied`, `superseded` (a newer
change already holds, and the client is told so rather than left guessing)
or `rejected`. Clients pull by arrival, not by clock:
`GET /changes?after=<seq>` returns everything since, or `reset` when the
client has been away too long. A deleted object stays as a tombstone, so a
client that was offline learns about it. New objects get their UUIDv7 on
the client, and a repeated `create` is harmless. Online clients (web, CLI,
Emacs) write directly and the server stamps for them; only the app and the
replica keep an outbox. Last-writer-wins is one pure function in
`samu-core` (`clock::wins`).

**Unknown values.** Every enum on the wire has a fallback `unknown`. It is
read, never written: a client that shows an object with a value it does
not know leaves that field alone, so an old client never overwrites what a
newer one set. A new enum value is therefore a minor version of
`samu-api`, like a new field; removing or reinterpreting one is a major
version.

**Storage.** One table `object` for all kinds carries the id, `seq` of the
last change and the tombstone; each kind has a table with typed columns,
and `stamp` holds the stamp of each field. One process writes a database
file at a time (a file lock).

**Authors.** The server sets the author of a change, `{device, kind}`,
from the session that brings it, also for changes from an outbox; what a
client claims is ignored. `kind` is `human` or `agent`, chosen at sign-in
(`samu login --agent`), so the logbook and the report tell a person's work
from an agent's.

**Calendars.** A calendar's address and credentials are a secret: sealed
with XChaCha20-Poly1305 under `SAMU_SECRET_KEY`, never in `/changes`, never
in an answer. The server fetches the calendars; appointments of a
writable CalDAV calendar can be changed through `/changes` like any
object, and the server writes them back to the calendar's own server.

**Leaving.** `samu export` writes a directory, one JSON file per kind in
the wire form of `samu-api`, sorted and stable for `diff`, attachments
included. It is a backup and a way out that needs no samudo to read.

**Clients.** The web UI is plain JavaScript modules with no build step,
embedded in the server binary. The Android app is Kotlin and Compose over
`samu-mobile` (uniffi). On a computer the replica answers the same API on
the loopback from a mirror, also while the server is away. The interface
texts are German and serve as keys; an English table translates them.

## Crates

```
crates/
  samu-api/      wire types (serde only), versioned and published on its own
  samu-core/     model rules, availability, repetition, quick capture,
                 search, forecast, reminders, report, habits; no I/O, no async
  samu-store/    SQLite (rusqlite), numbered migrations
  samu-server/   axum: API, server-sent events, hooks, calendars, the web UI
                 embedded; binary `samu-server` (serve, replica, profile, front)
  samu-client/   HTTP client (rustls) on samu-api; shared by CLI and app
  samu-cli/      binary `samu`
  samu-replica/  mirror, outbox and sync of a client, without uniffi;
                 shared by the app and the local replica
  samu-mobile/   Rust side of the Android app (uniffi bindings)
```

Dependencies point one way: `samu-api` ← `samu-core` ← `samu-store` ←
`samu-server`; `samu-api` ← `samu-client` ← `samu-cli`, `samu-mobile`;
`samu-core`, `samu-client` ← `samu-replica` ← `samu-mobile`,
`samu-server`; `samu-core` ← `samu-cli`, which reads the search language
with it. Nothing depends on `samu-server`; `samu-api` depends on
nothing in the workspace.

## Other parts

| Directory | |
|---|---|
| `web/` | the web UI: ES modules, no framework, no build step; embedded in the server binary (`crates/samu-server/src/web.rs` lists the files) |
| `android/` | Kotlin and Compose over `samu-mobile` |
| `emacs/` | `samu.el`, over the CLI |
| `deploy/` | systemd units, export script |
| `docs/` | this documentation; `docs/de/` is the German translation |
| `site/` | the documentation site (Hugo), built from `docs/` |
| `testdata/` | synthetic data only |

## Data flow

```
 web UI ─┐
 CLI ────┼── HTTPS ──► samu-server ──► accounts/<name>.db
 Emacs ──┤  (via CLI)      │
 widget ─┘                 └─ GET /events (SSE) ──► every client
 Android app: local mirror + outbox ──► POST /changes, GET /changes
 replica on a computer: the same API on 127.0.0.1, mirror + outbox
```

## Building and checking

Stable Rust (edition 2024), see `rust-toolchain.toml`;
`make check-public` also needs Node.js (no packages) for the web's tests.

```sh
make check-public # fmt, clippy -D warnings, tests, samu-api packaging, web, samu.el
make install      # samu into ~/.local/bin
cargo build -p samu-server
target/debug/samu-server --data /tmp/samu serve --listen 127.0.0.1:8492
```

Rules the project keeps:

- Every new feature gets tests, every bug fix a regression test. Tests use
  synthetic data only and call no outside services.
- Every API route gets a CLI command in the same commit. CLI output: data
  on stdout, messages on stderr, `--format plain|json|jsonl|tsv`, defined
  exit codes.
- Every change to `samu-api` comes with a JSON fixture under
  `crates/samu-api/tests/fixtures/`.
- Schema changes only as numbered migrations in `samu-store`.
- No `unsafe` outside FFI, no `unwrap()` outside tests.

## Sister project

samudo has a sister project, *catastr*, a document and knowledge store.
Tasks link to its objects by URI (`catastr:<id>`), and catastr reads samudo
as a source through the same API and the `samu-api` crate. Neither touches
the other's database.
