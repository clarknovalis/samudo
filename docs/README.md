# samudo documentation

How to run and use samudo. The reasons behind the design are in
[Architecture](architecture.md).

## Start here

1. [Concepts](concepts.md): inbox, projects, availability, scopes,
   perspectives, the model in one page.
2. [Quick capture](quick-capture.md): one line makes a task.
3. [Search language](search.md): chips and free text, for search, scopes
   and perspectives.

## Clients

- [Web UI](web.md)
- [Android app](android.md)
- [Command line](cli.md)
- [Emacs](emacs.md)
- [Desktop widget](desktop-widget.md) for DankMaterialShell

## Features

- [Habits](habits.md): goals entered day by day, with streak and rate.
- [Capture from the Index ring](ring.md): webhook, and offline through
  Tasker and the app.

## Running

- [Self-hosting](self-hosting.md): Docker Compose, accounts, backup,
  export and restore.
- [Profiles](profiles.md): several accounts on one computer, offline
  through a local replica, or an account that stays on the computer.

## Developing

- [Architecture](architecture.md): crates, data flow, rules.
- [HTTP API](api.md): sign-in, reading, changes, live events.
- Building the app: [`android/README.md`](../android/README.md); the web
  UI's internals: [`web/README.md`](../web/README.md) (both German).
