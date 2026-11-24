# Quick capture

One line makes a task: the title in plain words, the rest as marks. The
same parser runs on the server and in the Android app, so a line means the
same thing everywhere: in the web's capture window, in the app, in Emacs,
in the desktop widget and on the command line.

```sh
samu add "Call the tiler #Bathroom @phone d:fri 9:00 ~15m !A ! // ask about grout"
```

makes the task *Call the tiler* in the project *Bathroom*, tagged *phone*,
due next Friday at 9:00, estimated 15 minutes, priority A, flagged, with
the note "ask about grout". Try a line with `--dry-run` first: it shows
what would be made and writes nothing. In the web the marks light up as you
type and warnings appear under the field.

## Marks

| Mark | Meaning |
|---|---|
| `#Project` | the project, by name or a unique beginning of it (`#bath` finds *Bathroom*); `#"New flat"` for a name with spaces. Unknown or ambiguous: the task goes to the inbox and `#…` stays in the title, with a warning |
| `@tag` | a tag; several allowed. `@Place/Office` is the tag *Office* under *Place*; `@"two words"` with spaces. An unknown tag is created |
| `!` | flag |
| `!A` `!B` `!C` | priority |
| `d:` or `due:` | due date |
| `s:` or `start:` | defer date (not available before) |
| `p:` or `plan:` | planned date |
| `~15m` | estimate: `30m`, `45min`, `2h`, `1h30`, `1h30m` |
| `every:` | repeat, fixed: `every:1d`, `every:2w`, `every:1m`, `every:1y`, `every:mon` |
| `every+:` | repeat, fixed, skipping missed occurrences up to today |
| `every!:` | repeat, due counted from the day you complete it |
| `prop:key=value` | set a property: `prop:ticket=ABC-345`, `prop:place="two words"`; the key in lower case, several allowed. A key that does not read or a system key (`org_id`, `source`) stays in the title, with a warning |
| `habit:` | make it a [habit](habits.md): `habit:` (daily), `habit:3/w`, `habit:2/m`, `habit:8/d:glass`, `habit:avoid` |
| `z:` | book time already spent: `z:1h30` (ending now), `z:yesterday 2h`, `z:14-15:30`, `z:yesterday 14:00-15:30` |
| `x:` | made already completed: `x:` (now), `x:yesterday` |
| `/name` | apply a [template](#templates) |
| `// text` | everything after it is the note |
| `\#5` | a backslash keeps the next word as text |

Each mark may appear once (tags excepted); a second project, date, estimate
or repeat stays in the title with a warning.

## Dates

A date is read only after a prefix (`d:`, `s:`, `p:`, …), never freely in
a sentence, and it is never guessed: a value that is no date stays in the
title with a warning. German and English words both work.

| Written | Means |
|---|---|
| `today` `heute`, `tomorrow` `morgen`, `übermorgen` | |
| `mon` … `sun`, `monday` …, `mo` … `so`, `montag` … | the next such weekday, never today |
| `next week` `nächste woche` | next Monday |
| `end of month` `monatsende` | the last day of this month |
| `in 3 days`, `in 2 weeks`, `in 1 month` (`tage`, `wochen`, `monate`, `jahre`) | counted from today |
| `+3d`, `+2w`, `+1m`, `+1y` | the same, short |
| `2026-12-24` | a full date |
| `24.12.` or `24.12.2026` | the next 24 December (today included), or that year's |

A time after the date makes it an instant in the account's time zone:
`d:fri 9:00`, `p:tomorrow 14:30`, `s:mon 9h`. Without a time the date is a
whole day.

For `z:` and `x:` the day lies in the past: `today`, `yesterday`
(`gestern`), `vorgestern`, a weekday (the last one, today included), `d.m.`
or a full date. A `z:` without a range goes after that day's last clock, or
from 09:00.

## Editing with marks

`samu edit <id> <marks>` (key `e` in the web and in Emacs) applies a line
with only marks to an existing task:

```sh
samu edit 4f2a91c3 "@errand !A d:mon // ask for the price"
```

Tags are added (a state of a workflow replaces its sibling), a project,
priority, date, estimate, repeat, flag or `prop:key=value` overwrites, `// text` is appended
to the note, `/name` applies a template. Free text is refused, since the
title has its own field: `samu set <id> title "New title"`.

## Several lines

A text of several lines makes a task per line, each line with its own
marks and `// note`. Blank lines are skipped, and list markers at the
start of a line are removed: `-`, `*`, `+`, `•`, `1.`, `1)` and a box `[ ]`
or `[x]`. A line indented deeper than the one before goes under it as a
subtask:

```text
Groceries #Home
  - Milk @shop
  - Bread
Call the tiler d:fri
```

makes four tasks, "Milk" and "Bread" under "Groceries". The tasks are made
all at once or not at all: if one line fails, for example a project the
session may not write to, none is made. Paste such a text into the web's
capture window or the app's capture field, or switch the window to
"Several lines" and type them (in the web also Shift+Enter, in the app
also "Capture several" in the ⋮ menu). On the command line, use
`samu add --lines -` with the text on stdin. In Emacs, use
`samu-capture-lines`: on the region, or without one in a buffer of its own.
A text shared to the app stays one task.

## Templates

A template is a line of marks under a name, with optional reminders and a
key:

```sh
samu template add habit '#Habits @habit habit:1/d'
samu template add evening '#Home p:today 21:00 every+:1d'
samu template key evening e          # key e after the capture key: C e in the web
samu add "Water the plants /evening"
```

Free text in a template goes before the title of what is captured:
with `samu template add epic '[EPIC] @epic'`, the line `Publish the docs
/epic` makes `[EPIC] Publish the docs`. A template applied to an existing
task leaves its title alone.

Relative dates in a template count from the moment of capture. The line
wins field by field: what it names overrides the template, tags are not
mixed. Properties mix per key: `prop:team=infra` in a template and
`prop:ticket=ABC-345` in the line set both. A scope's capture settings fill in only what both leave open.

## Scope and place

While a [scope](concepts.md#scopes) is chosen, a line that names no project
goes to the project of the view you are in, else to the scope's capture
project; a line without tags gets the scope's capture tags. What the line
names always wins. `samu add --no-scope` leaves the scope out;
`--parent <id>` puts the new task under another one. With several lines,
`--parent` takes the top lines.
