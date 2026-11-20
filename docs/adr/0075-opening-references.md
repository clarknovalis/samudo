---
number: 0075
title: Opening references everywhere – addresses in the web UI, a handler for `samu:` on the desktop, in Emacs and on Android, `catastr:` as link class `app`
status: accepted
date: 2026-11-11
adds_to: ["0006", "0012", "0054", "0058", "0066"]
---

# 0075 – Opening references everywhere

## Context

Goal: `samu:` and `catastr:` references open with one click everywhere.
The state on 2026-11-11:

- **samu web:** `samu:` in a note jumps to the task (ADR 0058 §3, link
  class `task`). But a task had no address of its own, so no URL could be
  shared, and a handler could not hand a task to the browser. `catastr:`
  was inert in text. In properties it was an `href` that only opened when
  the computer had a handler for it.
- **Desktop:** `x-scheme-handler/catastr` opens `catastr open`. There was
  no handler for `x-scheme-handler/samu`, only for `samu-open:`
  (ADR 0054 §6, `samu open-uri`).
- **Emacs:** catastr.el knows `catastr:`; there was no link type `samu:`.
- **Android:** neither app accepted its scheme.
- **catastr web:** no address per object either; `/o/<uuid>` is planned
  there.

On 2026-11-20 the views themselves got addresses as well (§7): until
then the inbox, Forecast, the calendar, a search or the scope could not
be bookmarked, and a list of tasks could not be handed over as a link.

## Decision

1. **An address per object in the web UI: `/o/<uuid>`**, in the same
   form catastr plans for itself. Server and local replica (ADR 0066)
   serve the same `index.html` under `/o/{id}` as under `/`, without a
   session; the page therefore refers to its files by absolute paths. On
   start the web UI reads the path:
   - a task opens the inspector, as a `task` link does, also across the
     border of a scope (ADR 0069, "access by id");
   - a project or a perspective opens as its view, a folder or a tag as
     its chip in the search;
   - an unknown id gives a message.
   The open task is kept in the address as `?open=<uuid>` on the view it
   is open in (§7), so `/o/<uuid>` is what is handed out, and the address
   bar is what is bookmarked.
2. **`samu:` on the computer:** `samu open-uri` takes `samu:<uuid>` besides
   `samu-open:<uuid>/<key>` (ADR 0054 §6) and opens `<web>/o/<uuid>` with
   `xdg-open`. `<web>` is a key `web` in the CLI's configuration, default
   `server`; with the replica running it is `web = "http://127.0.0.1:8490"`.
   `make install` registers a `.desktop` file for both schemes.
3. **Emacs:** `emacs/samu.el` brings the Org link type `samu:`. Following
   it calls `samu open-uri samu:<uuid>`; export gives `<web>/o/<uuid>`.
4. **Android:** `MainActivity` accepts `samu:` as `VIEW`/`BROWSABLE` and
   opens the task the same way as `samu-widget:task/<id>`. Projects follow
   once the app can open a project page by id.
5. **`catastr:` becomes a link class of its own, `app`** (`samu-api`
   `LinkClass::App`, rule in `samu_core::render::link_class`): an address
   another app on the device opens. Only `catastr:` belongs to it for
   now. The web makes it `<a href="catastr:…">`; the browser asks and
   passes it to the system's handler. The app opens it with `ACTION_VIEW`
   and says so when no app takes it. An older client reads `unknown` and
   shows the link inert. Every other scheme stays inert (ADR 0058 §5).
   It is mapped to catastr's web address only once catastr has
   `/o/<uuid>`.
6. **What catastr has to change** (`/o/<uuid>` in its web UI, `catastr:`
   in its app) is a task in catastr's project and is built there
   (ADR 0003).
7. **Addresses for views.** The address bar always carries what the
   middle shows, so every state is a bookmark:
   - **The path is the view**, under `/v/` in English words whatever the
     language of the interface: `/` or `/v/inbox`, `/v/projects`,
     `/v/today`, `/v/forecast`, `/v/calendar`, `/v/flagged`, `/v/stuck`,
     `/v/review`, `/v/report`. `/v/` beside `/o/`, because `/forecast`
     and `/report` are API routes, and no later route may take a view's
     place. A project or a perspective is `/o/<uuid>` (§1), so renaming
     breaks nothing. The inbox is written as `/`.
   - **The query adds to any view**, written only where it differs from
     the default: `q=<search>` (chips as their English words, then the
     free text; read back with the search's own parser once the names
     are loaded, a word it does not know stays free text; in a
     perspective only while its search is edited), `scope=<uuid>` or
     `scope=none` (missing: the device's own choice; read as a choice by
     hand, ADR 0043 §7), `show=remaining|all` (not in a perspective,
     whose filter is its own, ADR 0090 §6), in the calendar
     `mode=day|week|month|list` and `day=<date>`, and `open=<uuid>` for
     the open task with `full` when it fills the window (ADR 0055 §6).
   - **History:** changing the view, project or perspective adds an entry,
     so Back returns to the previous view; the search, the filter, the
     scope, the open task and paging the calendar replace it.
   - A path the page does not know is not a page (404), like `/o/` with
     something that is not an id.
   Names in `q` are resolved when the address is read: a renamed project
   or tag leaves its word as free text. That is the price of an address
   a person can read and edit.

## Consequences

- `samu-api` 0.1.34 with `LinkClass::App` and a fixture. catastr is not
  affected as long as it does not render samu's documents.
- `/o/{id}` and the view paths are pages, not API routes. They are not in
  `ROUTES` and need no CLI command.
- A `samu:` link from any app (mail, calendar, catastr) leads into the
  browser via the replica on the computer and into the app on the phone.
- An address scheme is a promise to the outside: bookmarks live long.
  Paths and parameter names change only with an entry below.

## Changes

- 2026-11-20 (§7): views had no address; selecting a task wrote
  `/o/<id>` into the address bar in place of the view (§1), by
  `history.replaceState`, without a back stack. Folders and tags under
  `/o/` already opened as chips; §1 now says so.
