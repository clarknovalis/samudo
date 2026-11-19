---
number: 0004
title: Sync – the server is the truth, clients keep a cache and an outbox
status: accepted
date: 2026-11-07
---

# 0004 – Sync: the server is the truth, clients keep a cache and an outbox

## Context

Two proposals were on the table:

- **A:** the server holds the truth; clients replay a queue of offline
  changes; last-writer-wins per field.
- **B:** extract catastr's operation log, HLC and device register into a
  shared crate.

Against B: catastr's operation log is written by its store in the same
transaction as the change, and cursors, deferred entries and unknown
fields hang on its schema. A shared crate would couple both release
cycles. And samu needs no peer-to-peer: the phone talks only to the
server, as catastr's phone talks only to its master.

Tasks are small field changes; two devices changing the same field at the
same time is rare. But a widget and an offline view need more than a
queue: they need a read cache.

## Decision

1. **The server is the only place where a change holds.** Per account it
   keeps a running change number (`seq`) and for every field the stamp of
   its last change.
2. **A client without network** (the Android app, ADR 0028, and the local
   replica on a computer, ADR 0066) has
   - a **read cache** (mirror): the objects it shows, with the `seq` up to
     which it is current;
   - an **outbox**: field changes `{object, field, value, hlc}`, the device
     being part of the HLC (ADR 0016 §2).
3. **The stamp is an HLC**, computed as in catastr (a pure function in
   `samu-core`, state in the device's database). Rebuilt, not shared; the
   function is small.
4. **The server decides last-writer-wins per field** by HLC, on a tie by
   device name (part of the stamp's order, ADR 0016 §4). A superseded
   change is discarded and reported to the client as such, not silently.
   Sets (such as a task's tags) are kept per element with add and remove,
   each element with its own stamp; an exclusive tag group is a register
   per object (ADR 0024 §2).
5. **Pull by arrival, not by clock:** `GET /changes?after=<seq>` returns
   everything after it. If `after` is too old (the server has compacted),
   it answers `reset` and the client loads everything once.
6. **Online clients** write directly, without an outbox; the server stamps
   for them. Live updates come over `GET /events` (ADR 0003). On a
   computer, web, CLI and Emacs talk to the local replica, which has an
   outbox (ADR 0066).
7. **No hard deletion:** a deleted object stays as a tombstone until
   compaction, so a client that was away for long learns of it.
8. **New objects** get their UUIDv7 on the client. A creation in the
   outbox is idempotent by that id.

## Consequences

- No conflict copies: for tasks "the newer holds" is right. Lists are the
  exception (a task's tags, the order in a project): they are sets with
  add and remove per element, not one field, since repeatable fields get
  lost in sync otherwise. Every element has its own stamp; order is a field
  `position` per element. The wire form is fixed in `samu-api` with
  fixtures (ADR 0014, 0016).
- Completing a repeating task is *one* operation the server carries out
  (create the next instance), not two field changes from the client;
  otherwise the next instance would be created twice offline (ADR 0033).
- A client that completes offline sees the follow-up only after syncing.
  For the widget that is enough; it computes availability locally with
  `samu-core` over uniffi.

## Changes

- 2026-11-07 (ADR 0016): §2 named the device next to the HLC in an outbox
  change; it lives only in the HLC.
- 2026-11-10 (ADR 0066): §2 and §6 knew only Android as an offline
  client.
