# samudo

**[samudo.de](https://samudo.de)** · [demo](https://demo.samudo.de) ·
[documentation](https://docs.samudo.de) ·
[desktop widget](https://github.com/clarknovalis/samudo-dms)

> [!WARNING]
> **Early stage.** Development started on 7 November 2026. Working, but
> not feature-complete. No warranty.

A self-hosted task manager for Linux, the web and Android. It takes from
OmniFocus availability and reviews, from Todoist quick capture in one
line, and from org-mode capture templates, refile, clocking and free
properties. Workflows with states and time tracking are built in.

The name is samu plus *dō*, the way (as in Aikidō); in English the "do"
reads as a nod to getting things done. Inside, everything is still called
`samu`: the command, the crates, the `samu:` URIs.

- Defer and due dates kept apart; availability is computed, never stored,
  and every view says why a task is not available.
- Sequential and parallel projects, action groups, single-action lists,
  workflows with states, a board.
- Reviews, perspectives (saved searches), scopes, forecast, a real
  calendar view with your appointments.
- Quick capture in one line: `Call the tiler #Bathroom @phone d:fri ~15m`.
- Clocking and a report, reminders, habits, comments and attachments.
- One server is the single writer of every task. Clients (web, Android,
  command line, Emacs, a desktop widget) talk to its API; offline changes
  wait in an outbox on the device.
- Agents, such as AI assistants, work through the same API with sessions
  of their own, so their changes are marked as theirs.
- Your data is never locked in: export to plain JSON files and an Org
  outline, readable without samudo.

**Status:** server, command line, web UI, Android app and Emacs package
are in daily use. The Android app is released on
[GitHub Releases](https://github.com/clarknovalis/samudo/releases)
([installing](docs/android.md#installing)), the server as a container
image for amd64 and arm64 ([self-hosting](docs/self-hosting.md#the-image));
everything else is built from `main`. What
is being worked on and what may come next is on the
[roadmap](docs/roadmap.md).

**Platforms:** built and used on Linux and Android. The server runs
anywhere through Docker. On macOS the command line should build, but is
untested; on Windows it does not build yet. An iOS app is not planned so
far; contributions are welcome.

**How it is made:** samudo is developed by its maintainer together with
Claude Code, Anthropic's coding agent. Most commits are co-authored by
Claude and say so.

## Try it

The [public demo](https://demo.samudo.de) needs no installation: sign in
as `demo-en` (or `demo-de`) with the password `demo4samudo`, or make a
demo account of your own there. The shared accounts start fresh every
hour, an own one is deleted seven days after its last use. In the Android
app or the command line the server is `https://demo.samudo.de`.
Screenshots of the web UI and the app are on [samudo.de](https://samudo.de/en/).

## Documentation

The documentation is at **[docs.samudo.de](https://docs.samudo.de/en/)**
(English and German), built from the Markdown in [`docs/`](docs/README.md),
which reads just as well here:
[concepts](docs/concepts.md), [quick capture](docs/quick-capture.md),
the [web UI](docs/web.md), the [Android app](docs/android.md), the
[command line](docs/cli.md), [Emacs](docs/emacs.md), the
[desktop widget](docs/desktop-widget.md),
[self-hosting](docs/self-hosting.md) and the
[architecture](docs/architecture.md), which also gives the reasons behind
the design.

## A first look

```sh
make install                                       # the command line
samu login --server https://demo.samudo.de --account demo-en   # password demo4samudo
samu add "Call the tiler #Bathroom @phone d:fri ~15m"
samu add "Buy milk" --dry-run                      # show, write nothing
samu list --available
samu done 4f2a91c3
```

Running your own server: [Self-hosting](docs/self-hosting.md). Checking
a change: `make check-public`.

## Repositories

- [clarknovalis/samudo](https://github.com/clarknovalis/samudo), this one:
  server, web UI, Android app, command line, Emacs package, documentation
  (`docs/`, `site/`).
- [clarknovalis/samudo-dms](https://github.com/clarknovalis/samudo-dms):
  samuAgenda, the widget for the bar of DankMaterialShell, MIT licensed.

## Sister project

samudo grew alongside *catastr*, a document and knowledge store. Tasks
link to catastr objects by stable URIs, and catastr reads samudo as a
source. The two talk only through versioned APIs.

## Contributing and license

Issues and pull requests are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md)
and, for security problems, [SECURITY.md](SECURITY.md).

samudo is licensed under the [GNU Affero General Public License v3.0 or
later](LICENSE). The wire types in `crates/samu-api` and the HTTP client
in `crates/samu-client` may in addition be used under the MIT or
Apache-2.0 license, so other programs can talk to a samudo server without
copyleft.
