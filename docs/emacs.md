# Emacs

`emacs/samu.el` brings samudo into Emacs: views in the style of org-agenda,
actions at point, a buffer per task, quick capture, the running clock in
the mode line and the Org link type `samu:`. Everything goes through the
[command line](cli.md) with `--format json`, so configuration, session and
server are those of `samu`, a local [profile](profiles.md) included. Emacs
writes as the person signed in there.

## Loading

```elisp
(load "/path/to/samu/emacs/samu.el")
(global-set-key (kbd "C-c s") #'samu)          ; forecast
(global-set-key (kbd "C-c S") #'samu-capture)  ; quick capture
(samu-clock-mode-line-mode 1)                  ; running clock
```

Options (`M-x customize-group RET samu`):

| Option | |
|---|---|
| `samu-executable` | the command line, default `samu` |
| `samu-profile` | as `--profile`; `nil` = the command line's default profile |
| `samu-server`, `samu-account` | as `--server` / `--account`; `nil` = as the command line |
| `samu-scope` | `nil` = as the command line, `none` = no scope, else a title; `S` sets it for this Emacs session |
| `samu-link-target` | `emacs` (default) or `browser`; `C-u` when following takes the other |
| `samu-web-url` | the web UI, for the browser and for export |
| `samu-capture-link` | capture from a file notes the place |
| `samu-live` | open buffers and the clock follow changes (default on); `samu-toggle-live` |
| `samu-clock-interval` | seconds between updates of the clock |

`samu-switch-profile` and `samu-switch-account` change profile or account
for this Emacs session.

## Views

| Command | |
|---|---|
| `samu`, `samu-forecast` | forecast: overdue, today, flagged, the next days, later |
| `samu-inbox` | tasks without a project |
| `samu-available` | available, by project |
| `samu-today` | planned for today or earlier |
| `samu-flagged` | flagged |
| `samu-project` | one project |
| `samu-tag` | one tag with everything below it |
| `samu-search` | words in title, note, id |
| `samu-projects` | all projects; `RET` opens one |
| `samu-review` | projects due for review, oldest first |
| `samu-show` | one task in its own buffer |
| `samu-habit-show` | one habit: streak, rate, the last 12 weeks, the entries |
| `samu-clock-report` | the time clocked |

Under Evil the buffers start in Emacs state.

## Keys

The keys are those of the [web UI](web.md#keys), the navigation that of
Emacs. Tasks marked with `M` take `x f m t s X DEL , : C-c C-s C-c C-d e`
together.

| Key | |
|---|---|
| `n` `p`, `j` `k` | next / previous task |
| `RET` / `TAB` | show the task / in the other window |
| `x` | complete / reopen |
| `s` / `t` | next state / state by key |
| `f` | flag on/off |
| `c`, `I` / `O` | clock toggle; in / out |
| `m`, `C-c C-w` | move: inbox or project, with `C-u` under a task |
| `e` | edit with marks: `@tag #project !A d:fri ~30m every:1w // note /template z:30m` |
| `,` | priority A/B/C, Space: none |
| `:` | tags; missing ones are created |
| `C-c C-s` / `C-c C-d` | planned / due, `C-u` clears |
| `a` / `A` | capture / as a child task |
| `C` | capture with a template: then the template's key, as in org-capture |
| `/` | search |
| `X` / `DEL` | drop / delete, both ask first |
| `w` | copy the `samu:` link |
| `M` `u` `U` `B` | mark, unmark, unmark all, action on the marked |
| `S` | scope |
| `R` | mark the project reviewed |
| `h` | a [habit](#habits), followed by a key |
| `g` `q` `?` | reload, quit, help |

In a task's buffer also:

| Key | |
|---|---|
| `T` | title |
| `N` | edit the note (Org or Markdown); `C-c C-c` saves, `C-c C-k` discards |
| `C` | write a comment, likewise |
| `+` | attach a file |
| `RET` | on a child: show it; on an attachment: open it; on a property: `samu open` |
| `o` | open a property |
| `l` | show/hide the logbook |

Action keys act on the child task at point, else on the buffer's task.

## Habits

A row of a [habit](habits.md) shows, before the title, what the running
period is: ✓ once the goal is met, ✗ for not done today, ‖ for a pause,
○ for not yet. With a unit or a goal above one, the amount follows, like
`5/8 Glas` or `2/3 Woche`, and then the streak, like `🔥4`
(`samu-habit-streak-mark`). A habit to avoid shows `frei seit n T`. The
figures come from `samu habit list`, once each time a buffer is filled,
through the same scope as the view.

| Key | |
|---|---|
| `h h`, `h d` | done today: +1. `C-u 3 h h` enters 3, `C-u h h` asks for an earlier day |
| `h f` | not done today; for a habit to avoid, a relapse. `C-u` asks for the day |
| `h p` | pause from a day to a day; the streak does not break |
| `h u` | delete the entry made last, after asking |
| `h s` | the habit's buffer |

Entering never completes the habit; `x` does, and ends it. With tasks
marked, `h h`, `h f` and `h p` enter for each of them.

The task buffer of a habit shows its goal, today, the streak and the
rates. The habit's buffer (`h s`, `samu-habit-show`) adds the last 12
weeks as a calendar, `#` done, `+` partial, `x` not done, `=` pause, `.`
nothing, and the entries of that time. `DEL` on an entry deletes it, `RET`
opens the task, `g` reloads.

## Capture

`samu-capture` reads a line of [quick capture](quick-capture.md). `TAB`
completes `#project`, `@tag` and `/template`. An active region becomes the
note. From a file, an Org link to the line is added (`samu-capture-link`).

`samu-capture-template` (in the views `C`, elsewhere `C-u C-u a`) works
like `org-capture`: a small window lists the templates with their keys, a
key picks one, and the line starts with its `/name`. Set a key with
`samu template key <name> <key>` or under "Templates" in the web settings.

## Org links

`samu:<id>` is an Org link type once `ol` is loaded.

- **Following** opens a task or project in a task buffer, a tag or folder
  as a view; with `samu-link-target` = `browser` or `C-u` in the web UI.
- **`org-store-link`** in a samu buffer stores the task at point with its
  title.
- **`C-c C-l samu: RET`** completes over the open tasks.
- **Export** gives `<samu-web-url>/o/<id>`.

## Live

While a samu buffer is open or the clock shows in the mode line, a process
`samu events` runs. Every change, also from the web, the app or the command
line, refreshes the open buffers and the clock, after one second of quiet
and not while the minibuffer is reading. Point, marks and each window's
first line stay where they were. If the process ends, it reconnects after
5 seconds, then with doubling delays up to a minute.
