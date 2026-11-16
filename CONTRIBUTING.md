# Contributing

Thank you for looking. samudo is a small project with one maintainer, so
please open an issue before a larger change: a short note about what you
want to do saves us both a pull request that goes in another direction.

## Building and checking

You need a stable Rust toolchain (`rust-toolchain.toml` picks it), a C
compiler for the bundled SQLite, the system's time zone database (tzdata;
the tests use named zones) and Node.js for the web UI's tests.
Emacs is optional; without it the Emacs package's checks are skipped.

```sh
make check-public
```

runs everything a change has to pass: `cargo fmt --check`, `cargo clippy`
with warnings as errors, `cargo test`, packaging `samu-api` on its own,
the web UI's tests and the Emacs package's tests. CI runs the same target,
plus a secret scan (`make secret-scan`, needs gitleaks). The maintainer's
`make check` adds checks of parts that are not public. The Android app
(`android/`) is built with Gradle and is outside these targets; see
[`android/README.md`](android/README.md).

## Conventions

The rules for code, tests, the API and the command line are in
[`AGENTS.md`](AGENTS.md). They apply to people and coding agents alike.
The short version:

- Code, identifiers, comments, commit messages and error messages are in
  English.
- Every new feature gets tests, every bug fix a regression test. Tests use
  synthetic data only and never call network services.
- Every API route has a command in the CLI. A change to `samu-api` comes
  with a JSON fixture.
- Small, self-contained commits with a message that says what changed and
  why.

[`docs/architecture.md`](docs/architecture.md) explains how the pieces fit.

## Pull requests

Development happens in the maintainer's own repository; this one on
GitHub receives it as snapshots, one commit per published state. A pull
request is applied there as a patch with your authorship kept, and its
change appears here with the next snapshot; the pull request itself is
then closed rather than merged, with a pointer to that snapshot. That is
expected, not a rejection.

Most of the code is written with Claude Code, Anthropic's coding agent; a
snapshot names Claude as co-author when commits in it were co-authored.

The desktop widget lives in
[samudo-dms](https://github.com/clarknovalis/samudo-dms); pull requests
for it are merged there directly.

## License

samudo is licensed under the [GNU Affero General Public License v3.0 or
later](LICENSE). The wire types in `crates/samu-api` and the HTTP client
in `crates/samu-client` may in addition be used under the MIT or
Apache-2.0 license; they must therefore depend on nothing else in this
workspace (`make license-check`). Every file names its copyright and
license, in an SPDX header or in `REUSE.toml` (`make reuse-lint`). By submitting a contribution
you agree that it is licensed under the same terms as the files it
changes.
