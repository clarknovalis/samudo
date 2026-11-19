---
number: 0058
title: Notes and comments rendered – Org or Markdown detected per text in samu-core, a document tree in samu-api, POST /render
status: accepted
date: 2026-11-10
adds_to: ["0006", "0019", "0025", "0028", "0039"]
---

# 0058 – Notes and comments rendered

## Context

The wish: Markdown and org-mode rendering in notes and comments, with an
unobtrusive edit symbol to switch between input and rendered page. Notes
and comments were plain text everywhere; in the web the note was always
an input field.

catastr solved the same with a renderer in its core crate that turns Org
or Markdown into a document tree (no HTML), with each node's range in the
text and links classified by the renderer, not by the interface. The
difference here: notes and comments have no media type. Imported notes
(ADR 0051) are Org; what one types in web or app tends to be Markdown.

Chosen: detect the format per text; render in `samu-core` as a tree; a
first round for web and app, note and comment, clickable checkboxes,
links to tasks.

## Decision

1. **A module `render` in `samu-core`**, taken over from catastr's core:
   Markdown with `pulldown-cmark` (the same crate and version as catastr,
   without default features), Org with its own line-based subset. No I/O.
   What the parser does not know stays text. It is a copy, not a
   dependency: samu does not reach into catastr's code (ADR 0003). When one
   side improves the parser, the other follows; shared test cases keep
   them together.
2. **The format is detected per text**, in `samu-core` and nowhere else
   (`render::detect`). Org signs: a line with `#+`, a drawer
   (`:PROPERTIES:` … `:END:`), a link `[[…]]`, a heading with two or more
   stars. Markdown signs: a heading `#`, a link `[…](…)`, `**bold**`, a
   code fence ```` ``` ````. The one with more signs wins; a tie or no
   signs means Markdown. Detection is deterministic (same text, same
   format), tested, and not stored.
3. **The tree is in `samu-api`** (`samu_api::document`): blocks and
   inlines as in catastr, each node with its range in code points. New
   compared to catastr:
   - A list item with a checkbox carries the range of the checkbox itself
     (`box`, the three characters `[ ]`), so an interface can toggle it in
     the text without parsing again.
   - Link classes are `web` (`http`, `https`, `mailto`), `task`, `app`
     (an address another app on the device opens; for now only `catastr:`,
     ADR 0075 §5) and `inert` (everything else, explicitly `elisp:`,
     `shell:`, `javascript:`, `data:`, `file:`).
   - `samu:<uuid>` and `id:<x>` are resolved while rendering (server: in
     the account's database; app: in its cache): `samu:` by id against
     every live task, project, folder and tag; `id:` by `org_id` on tasks
     and projects (ADR 0023 §5), otherwise by id. A resolved link carries
     `object` (`{id, kind}`, ADR 0077) and, for a task, also `task` with
     the UUID, and opens it in samu; an unresolved one stays text with the
     target as a hint.
   - Images are not loaded but shown as links (as catastr does).

   A new version of `samu-api` with a fixture.
4. **Route `POST /render`** takes a list of texts and returns a tree per
   text with the detected format; one call for the note and all comments
   of a task. An optional `format` per text forces Org or Markdown. CLI:
   `samu render [FILE|-] [--as org|markdown]`; with `--format json` format
   and tree, with `plain` only the detected format.
5. **Safe by construction**: no HTML in the tree, raw HTML becomes text;
   the web builds the tree with `el()`, never through `innerHTML`; only
   `web`, `app` and resolved `samu:` links are clickable.
6. **Web.** Note and comments appear rendered. An unobtrusive pencil at the
   note's top right (also a double click on a spot without a link)
   switches to the input field; leaving the field or Esc saves as before
   and switches back. An empty note is the input field right away, unless
   the scope's layout says otherwise (ADR 0072 §2). Comments keep their
   "Edit" (ADR 0039 §6) and show the text rendered. A click on a checkbox
   replaces the three characters at `box` (`[ ]` ⇄ `[x]`, `[-]` becomes
   `[x]`) and writes the field like any other change; on comments only
   where one may edit them. A resolved link opens its object at `/o/<id>`
   (ADR 0075 §1): a task in the inspector, a project as a view, folder and
   tag as a search chip.
7. **App.** The same natively in Compose: `samu-mobile` renders from
   `samu-core` and resolves links in the cache (offline, ADR 0028); the
   app shows the tree, a pencil switches to the input field, checkboxes
   toggle as in the web.
8. **CLI and Emacs** keep showing note and comment as text; a terminal
   renderer is not part of this record.

## Consequences

- New dependency `pulldown-cmark` in `samu-core` (MIT, the parser of
  rustdoc, actively maintained, the same as in catastr).
- About 1,700 lines of parser copied from catastr. Two copies can drift
  apart; that is the price of neither reaching into the other's code.
- Detection can be wrong, for instance for a note with a single `[[…]]`.
  Visibly wrong is better than a field to maintain; a switch per note
  would be a new record.
- The web makes one more request per opened task.

## Changes

- 2026-11-11 (ADR 0075): §3 had no link class `app`; `catastr:` was
  inert.
- 2026-11-11 (ADR 0077): §3 resolved `samu:` only against tasks.
