---
number: 0129
title: Capturing several lines – each line a task, indentation makes subtasks, split in samu-core, POST /capture/lines in one go
status: accepted
date: 2026-11-18
adds_to: ["0011", "0027", "0062"]
---

# 0129 – Capturing several lines

## Context

Capture takes one line (ADR 0011, 0027). A list from a mail or a note had
to be entered line by line. OmniFocus turns pasted plain text into one
entry per line, TaskPaper with indentation included; Todoist offers "Add
multiple tasks at once" when pasting into Quick Add.

`POST /capture` may already contain line breaks: after `//` they belong to
the note (the Emacs package appends a region that way). That meaning
stays.

## Decision

1. **Splitting happens in one place**, `samu-core` (`capture::lines`):
   lines at `\n` or `\r\n`; empty lines are dropped. At the start of a line
   list marks are dropped: `-`, `*`, `+`, `•`, `1.`, `1)` and a box `[ ]`
   or `[x]`. An `[x]` does not complete; that is what `x:` is for.
2. **Indentation makes subtasks:** a line indented deeper than the one
   before becomes its child. Depth is relative (a stack of indentations, a
   tab counting as one character); a jump of several levels counts as one.
   A line that goes back attaches to the nearest shallower line above it,
   or stands at the top.
3. **Every line is a quick capture of its own** with everything of ADR
   0011, `// note` only for its own line. Project, parent, scope and
   template of the request apply to the top lines as for a single line. A
   child goes below its parent line's task; if it names a project itself,
   that wins as in ADR 0062 §2.
4. **`POST /capture/lines`**, a route beside `POST /capture`:
   `CaptureLinesRequest { text, project, parent, scope, template, preview
   }` → `CaptureLinesResponse { tasks: [CaptureResponse] }`, in the order
   of the lines. It is computed in `samu-core` (`views::capture_lines`) line
   by line over a working copy. Nothing is written until every line has
   succeeded and the grant check (ADR 0095 §4) allows all changes; one
   failing line prevents all. A new minor version of `samu-api`.
5. **Interfaces:**
   - CLI: `samu add --lines`, the text from the arguments or with `-` from
     stdin, with `--dry-run`, `--project` and `--parent`.
   - Web: pasting text with at least two non-empty lines makes the window
     multi-line. The preview shows the tasks indented, with chips. ↵
     creates all, ⇧↵ starts a new line.
   - App: with at least two lines in the field, the preview shows the list
     and the button reads "Create N tasks". Computed offline in
     `samu-mobile` with `capture::lines` and the same chain as for one
     line.
   - Emacs: `samu-capture-lines` for a region or an input; pasting several
     lines into `samu-capture` lands there.
6. **Shared text stays as it is:** what is shared into the app is never
   parsed (ADR 0013).

## Consequences

- No TaskPaper project from a line ending in `:`, no `@tag(value)`
  syntax.
- No confirmation before creating; the preview shows what will be
  created.
- Creating a workflow uses the same line splitting for its states (ADR
  0132).

## Changes

None yet.
