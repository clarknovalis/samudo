---
number: 0066
title: Offline on the computer – a local replica with mirror and outbox answers the API on 127.0.0.1
status: accepted
date: 2026-11-10
adds_to: ["0004", "0006", "0025", "0028"]
---

# 0066 – Offline on the computer: a local replica

## Context

samu in daily use needs a fallback on the computer when the server is
unreachable (network down, travelling, maintenance). Only the app worked
offline: `samu-mobile` keeps mirror and outbox in SQLite (ADR 0028) and
lays the outbox over the mirror with `samu_core::overlay`. Web, CLI and
Emacs were online clients (ADR 0004 §6) and failed without the server.

Considered: the CLI with a mirror and outbox of its own; a Tauri shell
with a local mirror; `samu-core` as WebAssembly in the browser with a
service worker and IndexedDB. Each makes only one client offline and
needs code per client. The last one also breaks "no build step" (ADR
0025), and the browser may discard its data.

Chosen: a replica as below, as a subcommand of the server, with the
browser on the computer always going through it.

## Decision

1. **A local replica** runs on the computer as a user service and listens
   only on `127.0.0.1`. It answers the same HTTP API as the server and
   serves the same embedded web UI. Browser, CLI (`--server
   http://127.0.0.1:<port>`) and Emacs talk to it instead of the server
   and stay as they are. **The browser on the computer always goes
   through the replica**, not only during an outage: so all windows see
   the same local state, and an outage needs no switching. The server's
   own address stays for other devices.
2. **It is a client, not a second server.** It keeps a mirror of the
   server's objects and an outbox, exactly like the app (ADR 0028 §3–5).
   Reading means mirror with outbox laid over it (`samu_core::overlay`),
   from which `samu-core` computes as on the server. Writing means into
   the outbox, with an HLC stamp and the computer's device. Only the
   server decides who wins (ADR 0004 §4).
3. **Sync:** it pushes the outbox and pulls `/changes?after=<seq>` as soon
   as the server is reachable, and listens on the server's `/events`. It
   tells its own clients about changes through its own `/events`, those
   from syncing included.
4. **Shared code, not a copy:**
   - Mirror, outbox and sync moved out of `samu-mobile` into a crate of
     their own, `samu-replica`, without uniffi; app and replica use it.
   - What the routes compute from a snapshot (such as `status_of`, the
     filters of `/available`, forecast, folders and projects) moved from
     `samu-server` into `samu-core`. Server and replica call the same
     function; only the snapshot's source differs (store here, mirror with
     outbox there). This fits ADR 0006: computed in exactly one place.
5. **Every route has a class** in a table next to `samu_api::ROUTES`
   (`replica::locality`), and a test demands an entry for every route:
   - **local:** answered from mirror and outbox;
   - **online only:** answered locally with 503 and the error code
     `offline`, which the web UI shows as "only with a connection", such
     as hooks and pairing; blobs come from the replica's cache, otherwise
     also 503;
   - **relay** (ADR 0096): passed to the server with the replica's own
     session, offline `503 offline`; the session routes, the calendar
     routes (ADR 0100 §9) and the account-wide logbook (ADR 0123 §4) are
     of this class.
   Login has its own rule (ADR 0067).
6. **The web UI shows the state** in its status bar, "Offline · n changes
   in the outbox", like the app (ADR 0028 §8).
7. **Form:** a subcommand of the server, `samu-server replica --upstream
   <server URL>`. It reuses router and embedded web UI; the mirror takes
   the place of the account database. Several accounts on one computer
   are profiles, one service each (ADR 0086). A Tauri shell can later
   start and show exactly this replica.
8. **The replica protects itself like a server:** it accepts only
   requests whose `Host` is `127.0.0.1` or `localhost` with its port, or
   `<profile>.samu.localhost` (ADR 0087 §3), against DNS rebinding from a
   foreign page in the same browser, and demands a login like the server.
   Login is relayed to the server and gives a local token that keeps
   working offline (ADR 0067).

## Consequences

- Offline on the computer holds for browser, CLI and Emacs at once, with
  one piece of code.
- Every new route needs a decision "local, online only or relay". The test
  of §5 enforces it; that is the lasting cost per feature.
- Something completed offline creates its repetition's follow-up only
  after syncing (as in the app, ADR 0004).
- The computer's device needs a session of its own on the server.
- The mirror keeps all the account's objects on the computer, completed
  tasks with notes included: a few MB.
- Calendar subscriptions: appointments come with the mirror; only the
  server fetches.

## Changes

- 2026-11-10 (ADR 0067): §8 left open how login is checked offline.
- 2026-11-12 (ADR 0086, 0087): §7 knew one account per computer; §8
  accepted only `127.0.0.1` and `localhost`.
- 2026-11-13 (ADR 0096, ADR 0100 §9): §5 had two classes; session and
  calendar routes were "online only" and always answered 503 at the
  replica.
