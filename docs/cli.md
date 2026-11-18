# Command line

`samu` speaks to the server over its API, and covers all of it: every
route has a command. Other clients build on it too: [Emacs](emacs.md) and
the [desktop widget](desktop-widget.md) call `samu` with `--format json`.

`samu help` and `samu <command> --help` list everything; this page shows
the commands by task. Ids can be given in full or by their last eight
characters, as `samu list` prints them.

## Setup

```sh
make install                               # samu into ~/.local/bin
vim ~/.config/samu/config.toml
```

```toml
server = "https://samu.example.org"
account = "private"
device = "laptop"               # the name of this device; the hostname without it
web = "https://samu.example.org" # where samu: links open; the server without it

# What opens a property's value on this device: argv, no shell
[open]
jira = ["my-jira-script", "{value}"]
```

```sh
samu login                        # asks for the password
samu session                      # who and where
samu health                       # does the server answer?
```

Several accounts on one computer are [profiles](profiles.md) in the same
file; `--profile <name>` or `SAMU_PROFILE` picks one.

### Options and environment

| Option | Environment | |
|---|---|---|
| `--server` | `SAMU_SERVER` | the server URL |
| `--account` | `SAMU_ACCOUNT` | the account |
| `--profile` | `SAMU_PROFILE` | a profile of the configuration file |
| | `SAMU_DEVICE` | the device name |
| | `SAMU_CONFIG` | the configuration file (default `~/.config/samu/config.toml`) |
| | `SAMU_STATE` | where session tokens are kept (default `~/.local/state/samu`) |
| | `SAMU_WEB` | the web UI for `samu:` links |
| `--format` | | `plain` (default), `json`, `jsonl`, `tsv` |

Data goes to stdout, messages to stderr.

### Exit codes

| Code | Means |
|---|---|
| 0 | done |
| 1 | something else went wrong |
| 2 | wrong invocation, or a profile that is missing or malformed |
| 3 | not found (also: `samu clock out` with no running clock) |
| 4 | the server could not be reached |
| 6 | not signed in, or signing in failed |

### Pairing a phone

```sh
samu pair phone          # asks for the password, prints a code and a QR code
```

The code is valid once, for ten minutes. Scan it in the Android app, or use
it on another computer with `samu login --code <code>`.

### Agents

An AI assistant or a script should have a session of its own, so that what
it writes is marked as an agent's:

```sh
SAMU_STATE=~/.local/state/samu-agent SAMU_DEVICE=assistant samu login --agent
```

Such a session can be restricted to some folders and projects with all
below them; it then sees nothing else, not even the inbox. Name them by
path as `samu folder list` shows it, a project also by its title, or by
id. A restricted session signs in directly at the server, not through
the local replica.

```sh
samu login --agent --only work/ai                  # read and write there
samu login --agent --only work/ai --read-only      # only read
samu session                                       # shows the grant
```

`samu sessions` lists the account's sessions (never their tokens), and
`samu session end <id>` ends one, from a person's session of the whole
account.

On a server that takes registrations ([self-hosting](self-hosting.md#registration)):

```sh
samu register --info                               # what the server allows
samu register                                      # a demo account; name and password printed once
samu register alice --password-stdin --demo en     # an account of your own
samu demo en                                       # demo content into the signed-in account
samu account delete                                # a registered account, for good
```

## Capturing

```sh
samu add "Call the tiler #Bathroom @phone d:fri ~15m"
samu add "Buy milk" --dry-run          # show, write nothing
samu add --literal "#hashtag in a title"  # the text as the title, word for word
samu add "Sub-step" --parent 4f2a91c3
samu add "Draft /weekly"               # with a template
```

The marks are on [Quick capture](quick-capture.md). Templates:
`samu template list | add | text | remind | key | rename | delete`; a
template's reminder is `planned`, `due:1h`, `defer:30m` or `daily@20:00`.

## Looking

```sh
samu list                      # open tasks with their availability
samu list --available --tag phone
samu list --today              # planned for today or earlier
samu list --flagged
samu list --project Bathroom --all
samu list --search tiler
samu list --perspective "Quick calls"
samu list --columns state,due,estimate,worked,prop:jira   # as a table
samu projects                  # projects with their availability
samu projects --stuck          # active, with no available task
samu projects --review         # review due
samu forecast                  # overdue, today, flagged, next days, later
samu show 4f2a91c3             # one object, all fields
samu log 4f2a91c3              # its logbook
samu logbook --since 1h --kind deleted,moved   # the account's
samu prop 4f2a91c3             # its properties, inherited ones too
samu blockers 4f2a91c3         # the tasks it waits for
```

The chosen [scope](concepts.md#scopes) applies; `--scope <name>` picks
another, `--no-scope` none. Without a scope, what has an own scope is
missing; choose that scope to see it.

`--columns` prints a table with the same column names as the web UI
(`worked` is the time on the clocks, `remaining` estimate minus worked;
an action group counts its subtree); without a value the default columns,
and a table perspective brings its own.

A block perspective prints a heading before each block (`▸ Title (n)`), in
tsv the heading as the first column, in json the blocks with their tasks.

## Changing

```sh
samu edit 4f2a91c3 "@errand !A d:mon // ask for the price"
samu set 4f2a91c3 title "Call the tiler again"
samu set 4f2a91c3 flagged true          # values are JSON, else a string
samu set 4f2a91c3 properties.jira ABC-1
samu unset 4f2a91c3 due
samu done 4f2a91c3
samu drop 4f2a91c3 --note "no longer needed"
samu state 4f2a91c3 Waiting            # a state of its workflow, by title or key
samu state 4f2a91c3 --next
samu block 4f2a91c3 9e01d2aa            # wait until 9e01d2aa is done
samu unblock 4f2a91c3 9e01d2aa
samu remind 4f2a91c3 --before 1d        # a day before due
samu remind 4f2a91c3 --at 2026-11-20T08:30
samu remind 4f2a91c3 --daily 20:00      # every day; a habit only while open
samu reminders
samu delete 4f2a91c3 --dry-run          # count what would go
```

`samu delete` on a project moves its tasks to the inbox; `--tasks delete`
deletes them along with it, `--tasks <project>` moves them there.

### Many at once

```sh
samu list --tag phone --format tsv | samu bulk done -
samu bulk move --to Bathroom 4f2a91c3 9e01d2aa
samu bulk date due tomorrow -          # ids from stdin
```

`samu bulk` takes `done`, `drop`, `reopen`, `flag`, `unflag`, `move`,
`tag`, `state`, `date` and `delete`.

### Places and tags

```sh
samu folder add Home
samu project add Bathroom --folder Home
samu tag add Place/Office              # Office under Place
samu tag add Status --exclusive --default-workflow  # a workflow; its children are states
samu tag add Status/Waiting --key w    # --ask-note asks for a note on entry
samu tag list
samu review Bathroom                   # mark reviewed
```

`rename`, `move` and `delete` work on folders, projects and tags as
`--help` shows.

## Comments and files

```sh
samu comment 4f2a91c3 "Called, back on Monday"
echo "longer text" | samu comment 4f2a91c3 -
samu comments 4f2a91c3
samu attach 4f2a91c3 photo.jpg
samu attachments 4f2a91c3
samu blob get <sha256> > file
```

Comments are Org or Markdown; `samu render` shows how a text is read.

## Time

```sh
samu clock in 4f2a91c3
samu clock status
samu clock out
samu clock add 4f2a91c3 "yesterday 2h"
samu clock list 4f2a91c3
samu clock set 7c01 --start 2026-11-09T08:30 --note "call"
samu report --days 7 --by project
samu report --q "#Bathroom" --part time
```

`clock set` changes a clock's start, end or note; `--running` lets a
stopped one run again. In the web, click a clock in the inspector's
“Time” section.

The report counts your own work; `--q "author:agent"` shows the agents'
work, `author:all` everyone's (the German `autor:agent`, `autor:alle`
work as well).

## Calendars

```sh
samu calendar discover https://dav.example.org --username alice --password-stdin --add
samu calendar add Holidays --url https://example.org/holidays.ics
samu calendar discover --from Personal --add Work      # more calendars on the same server
samu calendar import Personal export.ics
samu calendar list
samu calendar refresh <id>
samu appointments --from 2026-11-16 --to 2026-11-22
samu appointment add Dentist --start 2026-11-20T10:00 --end 2026-11-20T11:00 --calendar Personal
```

`--from` takes user and password from a CalDAV calendar already there;
they stay on the server. `samu calendar import` sends each event of the
file to the calendar's server as it is, replaces one with the same UID,
and reports what was left out or failed on stderr (exit 1 when anything
failed). Fetching calendars needs a secret key on the server (see
[Self-hosting](self-hosting.md#the-secret-key)).

```sh
samu alarms                                  # reminders of the next 24 hours
samu alarms --to 2026-11-22 --no-tasks       # only appointments, up to that day
```

## Scopes, perspectives, settings

```sh
samu scope add --only Work Work      # only what the folder Work covers; --except for the rest
samu scope use Work                    # on this device
samu scope use auto                    # follow the scopes' schedules
samu scope use --none
samu scope schedule Work "mo-fr 08:00-17:00"
samu set <id> own_scope Work           # a folder, project or tag only in Work
samu unset <id> own_scope              # everywhere again
samu perspective add "Quick calls" '@phone ~<15m'
samu perspective view "Quick calls" table --columns state,due,estimate,worked
samu perspective add Morning --blocks :today "Quick calls=At the desk" :flagged
samu perspective blocks Morning                  # prints its blocks; --clear makes it plain
samu perspective dedupe Morning off              # a task in every block that shows it
samu settings                          # the account's settings
samu settings set remind-time 08:00
samu settings set due-soon P3D
```

## Habits

`samu habit` makes, enters and shows [habits](habits.md).

## Live and raw

```sh
samu events                    # one line per change, as it happens
samu changes --after 0         # what changed after a change number
samu apply changes.json        # send raw changes; see API
```

## Import and export

```sh
samu import org ~/org --dry-run        # the open tasks of an Org agenda directory
samu import org ~/org
samu export ~/samu-export --org        # the account as JSON files, and samu.org
samu export-org ~/samu-export          # samu.org again from the directory alone
```

The import can run again: what an earlier run brought in stays as it is.
The export is described under
[Self-hosting](self-hosting.md#export-and-restore).

## Hooks

`samu hook add | list | secret | remove | send` manage webhooks that an
outside sender delivers to; see [Capture from the Index ring](ring.md).

## Opening links

`samu open <id> <key>` opens a property's value with this device's command
from `[open]`, else `xdg-open`. `samu open-uri` handles `samu:<id>` and
`samu-open:` addresses from the browser; `make install` registers it.

## zsh completion

```sh
source <(COMPLETE=zsh samu)      # in ~/.zshrc, after compinit
```

It completes commands, options, task ids with titles, states, and inside
`samu add` the `@tags`, `#projects`, dates, estimates and repetitions,
live from the server.
