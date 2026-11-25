---
number: 0145
title: Copying and exporting tasks – Ctrl+A picks every row; the picks or one task go to the clipboard or a file as IDs, a Markdown list, TSV or JSON
status: accepted
date: 2026-11-24
adds_to: ["0052", "0062"]
---

# 0145 – Copying and exporting tasks

## Context

The web UI picks rows for one action on many (0052): Ctrl or ⌘ and a
click, Shift and a click, the space. There is no way to pick every row
at once, so a list of forty tasks takes forty clicks.

What is picked can be completed, moved, dated and deleted, from the
context menu (0062) and from the bar above the list, but it cannot leave
the web UI. Pasting a set of tasks into a mail, a chat, a spreadsheet,
a note or a terminal means copying titles one by one. The only thing
that can be copied is the link of one task (`samu:<id>`).

The CLI already prints lists as `json` and `tsv`, and its commands for
many tasks (`samu bulk …`) take IDs.

## Decision

1. **Ctrl+A picks every row drawn**, in the order drawn: in a list,
   Forecast, the table and a perspective. When every row is picked
   already, it takes the picks away. In a text field the browser keeps
   its own Ctrl+A. It is an action of the key table (0079), so it can be
   bound to another key.
2. **Five forms**, the same for one task and for many:
   - **IDs**: the full UUIDs, separated by a space, ready to follow
     `samu bulk …` on a command line.
   - **Text**: a Markdown task list, `- [ ] Title`, `- [x]` for one that
     is completed or dropped; a task whose parent is copied with it is
     indented below it.
   - **Org**: a heading per task as `samu export-org` writes one
     (0074 §8): keyword `TODO`, `DONE` or `CANCELLED`, priority, tags,
     `CLOSED`, `DEADLINE` and `SCHEDULED`, a drawer with `ID`,
     `CREATED`, `DEFER` and the task's properties, then its note; one
     level deeper below a parent copied with it.
   - **TSV**: a header row, then one row per task: `id`, `title` and the
     columns of the table (0115) under the names the CLI and the table
     use. Dates as `YYYY-MM-DD`, with ` HH:MM` when they carry a time
     (0144), stamps as their day, durations as `H:MM` as the table
     shows them. TSV rather than CSV, because a spreadsheet splits
     pasted tabs into columns by itself and the CLI prints the same
     format; a tab or a line break inside a value becomes a space.
   - **JSON**: an array shaped like `samu show --format json`, one
     element `{id, kind, short, title, object}` per task, `object` as
     the server sent it.
3. **Two ways out**: *Copy* puts the form on the clipboard; *Export*
   saves it as a file named after the view and the day: `.txt` for the
   IDs, `.md`, `.org`, `.tsv`, `.json`. The file is made in the browser
   and handed to a download link as a `blob:` URL, revoked at once; the
   page's policy still refuses `blob:` as a source of scripts, styles
   and images, and the test that keeps blob URLs out of the scripts
   allows this one module. Without a clipboard (a page not served over
   HTTPS) the IDs are shown in the message line to copy by hand, as the
   link of a task is today, and the longer forms offer to be saved.
4. **Where**: a submenu *Copy* and a submenu *Export* in the context
   menu of one task and in that of the picks, and both as choices in
   the bar above the picks. *Copy link* of one task moves into *Copy*.
5. **The order is the order drawn**, never the order of picking; what a
   filter hides is not copied (0052 §4).
6. Nothing changes on the wire: no route, no field in `samu-api`, no
   migration. The forms are built in the browser from what it has, or
   from what `GET /available` and `GET /export` already answer (§7).
   The app and the Emacs package are not part of this decision.
7. **Export in the settings**: a section that saves a file without
   picking rows first. It asks for a source and a form:
   - the inbox, a project or a perspective (not a block perspective,
     which the CLI refuses as well, §8), open tasks or with the
     completed and dropped ones too: fetched with `GET /available`,
     which applies the project, the perspective and the scope on the
     server as the list does, then written in one of the forms of §2;
   - everything: the account as `GET /export` answers it (0074), one
     JSON file without the attachments' files; `samu export` remains
     the backup with files.
8. **The CLI writes the same text forms**: `samu list` takes
   `--format org` and `--format md`, with every filter it has
   (`--project`, `--perspective`, `--all` …). Org is written by the
   code that writes `samu export-org`, one heading per task without
   its comments. Every other command refuses these two formats with
   exit code 2. The web and the CLI are held to the same cases in
   `testdata/export/`, as the table's numbers are (0115).

## Consequences

- The five forms are built in the web by one module without DOM
  (`web/lib/exporttext.js`), tested with node like the other pure parts
  of the web UI. Markdown and Org exist twice, in JavaScript and in
  Rust; the shared cases keep them alike.
- TSV, JSON and Org are for programs; the Markdown text is for people
  and may change its look without notice.

## Changes

- 2026-11-25: §8 gained a full text form. `samu list --format md|org
  --full` writes each task's note, comments and attachments below it,
  and `samu show <id> --format md|org` writes a task with the tasks
  below it, or a project with its tasks, in the same way (`--all` adds
  the completed and dropped ones). The full form is the CLI's alone; the
  short form without `--full` stays the one the web writes and the
  shared cases hold. Attachments are named, not linked, because there is
  no `files/` folder next to the output.
