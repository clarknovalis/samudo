# samudo

A self-hosted task manager for Linux, the web and Android, with the best
parts of org-mode: capture templates, refile, clocking and free properties.

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

**Status:** server, command line, web UI and Android app are in daily use;
there is no release yet.

## Documentation

Everything is in [`docs/`](docs/README.md):
[concepts](docs/concepts.md), [quick capture](docs/quick-capture.md),
the [web UI](docs/web.md), the [Android app](docs/android.md), the
[command line](docs/cli.md), [Emacs](docs/emacs.md),
[self-hosting](docs/self-hosting.md) and the
[architecture](docs/architecture.md), which also gives the reasons behind
the design.

## A first look

```sh
make install                                       # the command line
samu login --server https://samu.example.org --account private
samu add "Call the tiler #Bathroom @phone d:fri ~15m"
samu add "Buy milk" --dry-run                      # show, write nothing
samu list --available
samu done 4f2a91c3
```

Running the server: [Self-hosting](docs/self-hosting.md). Checking a
change: `make check-public`.

## Sister project

samudo grew alongside *catastr*, a document and knowledge store. Tasks
link to catastr objects by stable URIs, and catastr reads samudo as a
source. The two talk only through versioned APIs.

## Contributing and license

Issues and pull requests are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md)
and, for security problems, [SECURITY.md](SECURITY.md).

samudo is licensed under the [GNU Affero General Public License v3.0 or
later](LICENSE). The wire types in `crates/samu-api` may in addition be
used under the MIT or Apache-2.0 license, so clients can embed them
without copyleft.
