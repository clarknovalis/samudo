# AGENTS.md

Conventions for changing samudo, for people and coding agents alike.
[`docs/architecture.md`](docs/architecture.md) explains how the pieces fit;
[`CONTRIBUTING.md`](CONTRIBUTING.md) how to send a change.

## Names

The product is called **samudo** (README, web UI, app). Everything inside
is `samu`: the crates, the command, the `samu:` URIs.

## Architecture rules

- **The server holds the truth.** Clients keep a read cache and an outbox
  of field changes stamped with a hybrid logical clock and their device;
  the server decides last-writer-wins per field. No second operation log.
- **One account, one database**: separate SQLite files, not an owner
  column.
- **Availability is computed, never stored**, in exactly one place:
  `samu-core`. No client computes it on its own.
- **Time:** a whole day is a local date without zone; a time is an instant
  plus zone. Repetition is an RRULE plus a mode.
- SQLite through rusqlite, internal rowids, UUIDv7 outside, timestamps as
  ISO 8601 text with `Z`.
- Dependencies point one way (see the architecture page). `samu-api`
  depends on nothing in this workspace, because other projects build it
  inside their own; `samu-client` depends on `samu-api` only, because
  both may also be used under MIT (`make license-check`); nothing
  depends on `samu-server`; no cycles.
- `samu-core` has no I/O and no async runtime.

## Rust

- Stable toolchain, edition 2024. `tokio` and `axum` for async and HTTP.
- Keep it simple: a `clone()` rather than lifetime acrobatics, no custom
  proc macros, no generic abstractions in advance.
- Errors: `thiserror` in libraries, `anyhow` only in binaries.
- No `unsafe` except at FFI boundaries, there with a `// SAFETY:` comment.
- No `unwrap()` or `expect()` outside tests, except for real invariants
  with the reason in the `expect` text. The workspace lints enforce this.
- A new dependency needs a short reason in `Cargo.toml` (purpose,
  maintenance, license). Few, well-maintained crates.
- Schema changes only through numbered migrations in `samu-store`.

## Definition of done

A change is done when `make check-public` passes (CI runs it; the
maintainer's `make check` adds checks of parts that are not public). It
runs:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo package -p samu-api     # builds without the rest of the workspace
scripts/license-check.sh      # MIT crates depend on no AGPL-only crate
cargo deny check              # no known vulnerability, allowed licenses
reuse lint                    # every file names its copyright and license
```

plus the web UI's and the Emacs package's tests.

- Every new feature gets tests; every bug fix a regression test.
- Every new source file starts with an SPDX header (`SPDX-FileCopyrightText`
  and `SPDX-License-Identifier`, in `samu-api` and `samu-client`
  `AGPL-3.0-or-later OR MIT OR Apache-2.0`); data, images and
  third-party code are covered by `REUSE.toml` instead.
- Tests use synthetic data only (`testdata/`), never real notes or mail,
  and never call network services or language models.
- **Every API route has a command in the CLI**, in the same commit. No
  feature exists in only one interface.
- CLI output: data on stdout, messages on stderr,
  `--format plain|json|jsonl|tsv`, defined and tested exit codes
  (`crates/samu-cli/src/main.rs`, `mod exit`).
- **Every change to `samu-api` comes with a JSON fixture** under
  `crates/samu-api/tests/fixtures/`. A fixture changes only together with
  a new version of `samu-api`, which has its own semver: removing or
  reinterpreting a field is a breaking change.

## Language and commits

- Code, identifiers, comments, commit messages, error messages and the
  documentation are in English (`docs/de/` is the German translation).
- Small, self-contained commits whose message says what changed and why.
- Comments that cite `ADR NNNN` point to the maintainer's design records;
  the reasons that matter for a change belong in the code or the commit.
