# Roadmap

What is being worked on and what may come next. There are no dates:
samudo is built alongside daily use, and the order follows what that use
asks for. `main` is what runs; whatever is listed as done there is usable
by building from source.

## In progress

- **English interface.** The web UI and the Android app follow the system
  language or a choice of German or English. Both are translated; the last
  German strings that slip through are being caught.
- **Table view.** A perspective can show its tasks as a table, with
  columns for properties and an estimate against the time clocked, sums
  and sorting by column. Done in the web UI and the command line; the app
  shows such a perspective as a list for now.

## Planned

- **The app catches up with the web UI**: capture templates, booking time
  afterwards and the report, and arranging a list in three steps (group,
  sort, sort again).
- **The Android app as a release download**, built on GitHub and attached
  to a release, so it can be installed without building it.
- **The command line on Windows and macOS.** It should build on macOS but
  is untested; on Windows it does not build yet.

## Considering

These may happen, or may not; nothing about them is decided.

- **An interface in stages**: features that are seen only once they are
  wanted, and sets of workflows to switch on, so that a new account starts
  small.
- **Sharing folders and projects** between accounts on one server.

## Not planned

- **An iOS app.** Contributions are welcome; the [HTTP API](api.md) is
  what an app talks to.

Ideas and wishes are welcome as issues on GitHub.
