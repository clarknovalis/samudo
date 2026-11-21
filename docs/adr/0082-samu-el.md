---
number: 0082
title: samu.el as a client – through the CLI, views like org-agenda, the web's keys with Emacs navigation, a task's buffer, capture with completion, the clock in the mode line, `samu:` opens in Emacs
status: accepted
date: 2026-11-12
adds_to: ["0008", "0011", "0046", "0075", "0078", "0079"]
---

# 0082 – samu.el as a client

## Context

The wish was a complete samudo for Emacs. ADR 0008 §4 says how:
`samu.el` through the CLI, in the style of org-agenda. Until then
`emacs/samu.el` knew only the Org link `samu:` (ADR 0075 §3).

Settled before this record:

- **The first round:** core, views, actions, a task's buffer, capture,
  clocking and links. Live updates over `samu events` follow as a task
  of their own.
- **This record first**, then the code.
- **Evil:** samu buffers start in Emacs state, like the org-agenda.

What the CLI gives for this it prints with `--format json`. `samu list`,
for one, gives each row its short form (`short`, `st`, `state`,
`priority`, `project`, `tags`, `due_text`, `why`) and under `status` the
task with what `samu-core` computed. `samu forecast` gives rows with
`section`, `date` and `why`.

## Decision

### 1. Through the CLI, as the client of the person signed in

- samu.el calls `samu … --format json` and parses what comes back. It
  speaks no HTTP itself and keeps no cache of tasks. So everything the
  CLI can do holds here too: configuration, session, the local replica
  (ADR 0066) and working offline.
- **Writing waits for the CLI, reading does not.** A key that changes
  something is one call, and its error belongs to that key. What a
  buffer shows takes several calls; they run in the background, side by
  side, and the buffer is drawn when all have answered. Until then it
  keeps what it shows, and a new buffer says that it is loading. A
  buffer in no window is not filled at all; it is noted as out of date
  and fills once a window shows it. `samu events` runs in the
  background as well.
- **Errors:** an exit code other than 0 becomes an error with the last
  line of stderr; for a call in the background, a message. Exit code 3
  ("nothing there", as for `clock out`) is no error where it is an
  answer.
- The settings `samu-executable`, `samu-server` and `samu-account` go
  along as `--server` and `--account`. `nil` means: what the CLI's
  configuration says. `samu-switch-account` changes the account for this
  Emacs session.
- **Emacs writes as the person signed in**, through the CLI's session on
  this device. It gets no session of its own.
- **Scope:** `samu-scope` is `nil` (then the CLI's choice on this device
  holds), the title of a scope, or `none`. When set it goes along as
  `--scope` or `--no-scope`. `samu-choose-scope` sets it for the Emacs
  session and leaves the CLI's choice alone. The active scope stands in
  the header of every view.

### 2. Views in the style of org-agenda

One buffer per view, `samu-list-mode` (derived from `special-mode`):

| Command | View | CLI |
| --- | --- | --- |
| `samu` / `samu-forecast` | forecast, by `section` | `samu forecast` |
| `samu-inbox` | tasks without a project | `samu list`, field `project` empty |
| `samu-available` | available | `samu list --available` |
| `samu-today` | today | `samu list --today` |
| `samu-flagged` | flagged | `samu list --flagged` |
| `samu-project` | one project, chosen with completion | `samu list --project` |
| `samu-tag` | one tag | `samu list --tag` |
| `samu-search` | words in title, note, id | `samu list --search` |
| `samu-projects` | projects with their availability, `RET` opens one | `samu projects` |

- **A row:** state, priority, flag, title, then `#project`, `@tags` and
  the date (`due_text`). The forecast adds `why`. Subtasks are indented
  by `parent`. What is not available is dimmed (`shadow`). Overdue and
  due soon have faces of their own. Every face can be customized.
- Lists other than the forecast have **groups by project**, like the
  categories of org-agenda.
- **What is computed comes from the server.** samu.el computes no
  availability (ADR 0006). The inbox is a plain filter on a field
  (`project` empty), not a computation.
- The id is a text property on the row. After an action the view loads
  again, and point stays on the same task.

### 3. Keys: those of the web, navigation as in Emacs

The buffers start in Emacs state under Evil. The keys for actions are
those of the web UI, so the same fingers do the same everywhere.
Navigation and frame follow Emacs and org-agenda. `settings.keys`
(ADR 0079) does not apply here: in Emacs one rebinds through the keymap.

| Key | Action | From |
| --- | --- | --- |
| `n` `p`, `j` `k` | next / previous task | Emacs, web |
| `RET` | the task in its own buffer (§4) | |
| `TAB` | the same in the other window, point stays | org-agenda |
| `x` | complete / open again | web |
| `s` | next state (`samu state --next`) | web |
| `t` | pick a state by its key, as in the web | web, org-agenda |
| `f` | flag on/off | web |
| `c`, `I` / `O` | clock toggle; in / out | web, org-agenda |
| `m`, `C-c C-w` | move to the inbox, a project or under a task | web, org-agenda |
| `e` | edit by marks (`samu edit`, ADR 0078) | web |
| `,` | priority A/B/C/none | org-agenda |
| `:` | set and take off tags | org-agenda |
| `C-c C-s` / `C-c C-d` | planned / due, with `org-read-date` | org-agenda |
| `a` / `A` | capture / a subtask of the task here (§5) | |
| `/` | search | web |
| `X` | drop (after asking) | |
| `DEL`, `<deletechar>` | delete (after asking, ADR 0036) | web `Del` |
| `w` | copy the `samu:` link | |
| `M` / `u` / `U` / `B` | mark / unmark / unmark all / act on the marked (`samu bulk`) | org-agenda |
| `S` | choose the scope | |
| `g` / `q` / `?` | reload / quit / help | Emacs |

So `n` does not capture as in the web but goes to the next row. Capture
is on `a`, because `k` (org-agenda) means "up" in the web. `m` moves, as
in the web; marking is therefore on `M`, unlike org-agenda. With marks,
`x`, `f`, `m`, `t`, `X`, `DEL`, `,` and `:` act on all marked tasks, as
in the web (ADR 0052).

### 4. A task's buffer

`samu-show` opens `*samu: <title>*` (`samu-task-mode`). The buffer shows:

- the head: title, place (project, group), state, priority, flag, tags,
  dates, estimate, repetition, reminders and the id;
- the properties with where they come from (`samu prop`); `o` opens one
  (`samu open`);
- the note as text. An Org note is shown in Org's faces, a Markdown note
  in those of `markdown-mode` when it is loaded. `samu render` says
  which it is, that is `samu-core` (ADR 0058);
- the subtasks as rows like in §2, with the same keys;
- the comments, the attachments and, unfolded with `l`, the logbook.

The keys for actions of §3 act on the task shown, and also:

- `T` changes the title (minibuffer);
- `N` edits the note in a buffer of its own, in the mode of its format.
  `C-c C-c` saves (`samu set <id> note`), `C-c C-k` discards;
- `C` writes a comment the same way (`samu comment <id> -`);
- `+` attaches a file (`samu attach`). `RET` on an attachment fetches it
  with `samu blob get` into the cache and opens it.

Projects get the same buffer without the rows that only tasks have.

### 5. Capture

- `samu-capture` reads a line of quick capture in the minibuffer
  (ADR 0011). At `#`, `@` and `/`, `completion-at-point` offers
  projects, tags with their path, and templates. These lists come from
  the CLI once per session; `g` in a view fetches them again.
- An active region becomes the note with `//`. In a file's buffer a
  reference to the place is added, as `org-capture` does with `%a`.
- `A`, or `C-u samu-capture`, captures a subtask of the task at point
  (`--parent`).
- Warnings in the answer (`warnings`, `new_tags`) show in the echo area,
  so one sees when a tag was created.
- This record proposes no global key. That is the user's configuration.

### 6. Clocking

- `c`/`I`/`O` in a view and in a task's buffer, `samu-clock-out`
  anywhere.
- **Mode line:** while a clock runs, `samu-clock-mode-line-mode` (a
  global minor mode) shows "⏱ 0:42 title". `samu clock status` is asked
  after each action of one's own and once a minute by a timer. With
  live updates the timer goes.
- `samu-clock-report` shows `samu clock report` in a text buffer.
- `z:` to book time afterwards (ADR 0081) goes through `e` and capture,
  without a command of its own.

### 7. Links

- Following a `samu:` link opens the object **in Emacs**: a task or
  project in a task's buffer, a folder or tag as a view.
  `samu-link-target` = `browser` brings back the earlier behavior
  (ADR 0075 §3). `C-u` when following takes the other target.
- `org-store-link` knows the samu buffers: it stores `samu:<id>` with
  the title as description. `samu-copy-link` (`w`) copies the link.
- `C-c C-l` → `samu:` completes over the open tasks (`:complete`).
- Export stays as in ADR 0075 §3.

### 8. Files and tests

- `emacs/samu.el` stays one file while it is below about 1500 lines;
  after that it is split. Dependencies: Emacs 28.1 and `org` (for dates
  and links), nothing else. `markdown-mode` is optional.
- `emacs/test-samu.el` tests with ERT. The call of the CLI is replaced
  by synthetic JSON, so no test needs a CLI or a server. The tests of
  the calls in the background run a shell script in its place.
- `make emacs-check` compiles without warnings
  (`byte-compile-error-on-warn`) and runs the tests, with
  `emacs -Q --batch`. `make check` calls it. Without `emacs` the step is
  skipped, and says so.
- `emacs/README.md` describes loading, commands and keys.

### 9. Not in this round

Live updates over `samu events` (a task of their own), calendars and
appointments, the report (ADR 0080), settings, managing scopes,
templates, workflows and folders, setting reminders (showing them, yes),
import and export. What of that Emacs needs becomes a task of its own.

## Consequences

- No Rust and no new route: samu.el needs only what the CLI can do
  today. If something is missing on the way, it becomes a finding for
  the CLI, not a workaround in Elisp.
- The web's keys become the default for a second client. When a default
  changes in the web, samu.el has to follow. A sentence about that
  stands at the head of the table in `web/lib/keymap.js`.
- `make check` depends on an installed `emacs`, softly: without it the
  step is skipped.
- ADR 0075 §3 changes: following a link now opens in Emacs.
- Reading in the background means that, right after a key, a buffer
  shows the state before it for a moment, until the CLI has answered.

## Changes

- 2026-11-21: §1 said that every call is synchronous and only
  `samu events` runs in the background, on the ground that a call takes
  milliseconds through the local replica and tenths of a second against
  a server. A task's buffer takes nine calls and one more per note and
  comment; against a server on the local network Emacs stood still for
  3.6 s when opening one, and again after every key and every live
  update, for each open buffer. Now only writing waits, and buffers in
  no window are not filled. §8 adds the tests that run a script in place
  of the CLI.
