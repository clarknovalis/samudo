---
number: 0028
title: Android app – Kotlin/Compose over samu-mobile, mirror plus outbox, sync in Rust
status: accepted
date: 2026-11-08
adds_to: ["0004", "0008", "0016", "0020", "0026", "0027"]
---

# 0028 – Android app: Kotlin/Compose over `samu-mobile`, mirror plus outbox, sync in Rust

## Context

ADR 0008 §3 sets the Android app as a native client: Kotlin/Compose over
`samu-mobile` (uniffi), `samu-core` for availability, a read cache plus
outbox per ADR 0004. ADR 0020 §1 describes its shape (bottom navigation,
swipe completes, capture as a sheet, outbox visible). catastr has built
this path before (its `android/` and `rust.sh`).

Open were where sync lives, how to log in, and the scope of the first
version. Chosen: sync entirely in Rust, login with account and password,
a first version that reads, checks off and captures.

## Decision

1. **Structure as in catastr:** the app in Kotlin/Compose under
   `android/`, outside the Rust check; the Rust side in
   `crates/samu-mobile` over uniffi (proc macros). The bindings are
   generated on every build from the built library (`android/rust.sh`) and
   are not checked in, so Rust and Kotlin cannot drift apart. `make
   android` builds the APK. App id `de.samudo.app` (ADR 0110; debug builds
   `de.samudo.app.dev`, ADR 0118 §3), minSdk 26, only `arm64-v8a`.
2. **`samu-client`:** the CLI's HTTP client moved into a crate of its own
   that depends only on `samu-api`; CLI and app share it. On Android it
   reads the root certificates from the system's folders
   (`/apex/com.android.conscrypt/cacerts`, otherwise
   `/system/etc/security/cacerts`), as catastr does; no new dependency.
3. **`samu-mobile`** depends on `samu-core` and `samu-client`, not on
   `samu-store`: the store's schema is the server's, with stamps per
   field, while the phone receives whole objects. It keeps a small cache
   of its own in SQLite (rusqlite like the store): the **mirror** of the
   server's objects with the `seq` it reaches, the **outbox**, and the
   login. Mirror, outbox and sync later moved into `samu-replica`, shared
   with the local replica (ADR 0066 §4).
4. **What the phone shows is the mirror with the outbox laid over it.**
   Laying a change over is a pure function in `samu-core` (`overlay`),
   the same for every interface that writes offline. Availability is
   computed by `samu-core` on that result (ADR 0006).
5. **Sync:** push first (`POST /changes` with own HLCs from `next_stamp`,
   ADR 0016); per answer the change leaves the outbox: `applied` and
   `superseded` silently, `rejected` with a visible error. Then pull (`GET
   /changes?after=<seq>`, on `reset` everything anew, ADR 0004 §5). Sync
   runs on opening, after every write, on request, and every 15 minutes
   with a network (WorkManager). A 401 leads back to the login; the outbox
   stays.
6. **Interface:** `GET /session` and the answer to `POST /login` also
   carry the account's `zone` and `due_soon`, so the phone computes
   "today" and "due soon" as the server does (ADR 0007, ADR 0017 §2).
   Optional fields in `samu-api` with a fixture. Without them (an older
   server) the app takes the phone's zone and says so.
7. **Capture offline:** `parse` and `resolve` from `samu-core` run on the
   phone against the mirror. New tags, the task, and one `add` per tag go
   into the outbox in the same order as `POST /capture` (ADR 0027).
8. **First version**, styled with the tokens of ADR 0026, light and dark
   following the system:
   - login with server, account, password and device name;
   - bottom navigation with **Inbox, Available, Projects**;
   - rows with a round checkbox, due date and first reason below, grouped
     by project; **swiping right completes**;
   - tapping opens the task: flag, complete, all reasons, note;
   - **Capture** opens a sheet with a preview from the parser;
   - above the list **"Offline · n changes in the outbox"** while the
     outbox is not empty.
9. **Later steps**, each a step of its own: forecast and search, tag
   filters as chips, sharing (ADR 0013), widget, reminders, QR pairing
   (ADR 0031). Each has since come with its own record.
10. **Delivery:** first by `adb install`; release builds are signed and
    published as described in ADR 0118.

## Consequences

- Workspace: `samu-client` joined; direction `samu-api` ← `samu-client`
  ← `samu-cli`, `samu-mobile`; `samu-mobile` also depends on `samu-core`.
- New dependencies, each justified when added: `uniffi` in
  `samu-mobile`, `rusqlite` there as in the store.
- The check builds and tests `samu-client` and `samu-mobile` on the host;
  the tests of `samu-mobile` run against a real `samu-server`, like those
  of the CLI.
- No function exists only in the app: every change is an ordinary change
  over `POST /changes` (ADR 0019 §7).
- A change the server rejects is shown by the app; what was checked off
  appears open again after the pull.

## Changes

- 2026-11-10 (ADR 0066): §3 kept mirror, outbox and sync inside
  `samu-mobile`.
- 2026-11-15 (ADR 0110): §1 used an earlier app id.
- 2026-11-16 (ADR 0118): §10 had only installation by `adb`.
