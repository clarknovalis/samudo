# Search language

One small language filters tasks everywhere: the search field of the web
UI and the app, a scope's filter and a perspective. It is made of **free
text** and **chips**. Every word of the language has a German and an
English form, and both are understood whatever language the interface
speaks; this page uses the English ones (see
[German and English](#german-and-english)).

On the command line, `samu list --search <words>` takes free text only
and refuses a chip (exit 2); in double quotes (`--search '"#bath"'`) a
chip is searched for as text. Chips go into scopes (`samu scope filter`) and perspectives
(`samu perspective add`), and `samu list --perspective <name>` shows one.

## Free text

Every word that is not a chip must appear in the title, the note or the
id. The last eight characters of an id, as the command line prints it,
find their task.

A word with a leading `-` must not appear: `milk -oat`. Words in double
quotes are one phrase and must appear together, in that order:
`"by Friday"`, and `-"not today"` leaves out what has it. To search for
the word `or` itself, put it in quotes: `"or"` (see [Or](#or)).

## Chips

A word becomes a chip once you take it from the list under the field (in
the web ↑ ↓, Tab or Enter; in the app a tap) or finish it with a space.
While you type, the list offers what fits the word: the kinds, then
projects, tags, folders, states and the values of a date. With an empty
field the web lists the kinds; the app shows quick filters, recent
searches and perspectives (see [Android app](android.md#search)).

| Chip | Finds tasks |
|---|---|
| `#Website`, `#Inbox` | in the project, or in the inbox |
| `@Place`, `@Place/Office` | with the tag or a tag below it |
| `folder:Work` | in projects of the folder and its subfolders |
| `state:"In progress"` | in that state of their workflow |
| `due:`, `planned:`, `defer:` + range | with that date in the range (see below) |
| `done:`, `created:`, `modified:` + range | completed, created or modified in the range |
| `is:` + word | `flagged`, `available`, `waiting`, `open`, `done` (completed), `dropped`, `group` (action group), `note` (has a note), `repeating`, `clocked` (has clocks), `blocker` (another open task waits for it), `blocked` (waits for an open blocker, its own or its project's), `hidden` (open, but under a completed or dropped task; see [Concepts](concepts.md#completing-a-task-with-open-tasks-below)) |
| `is:appointment` | appointments instead of tasks, by title, place and note; a repeating one once, with its next occurrence; date chips apply to the start (web only, not in a scope's filter) |
| `~<15m`, `~>=1h`, `~30m`, `~any`, `~none` | by estimate (`<`, `<=`, `>`, `>=`, equal; with or without one) |
| `prio:a`, `prio:b`, `prio:c`, `prio:any`, `prio:none` (or `prio:-`) | by priority |
| `prop:jira`, `prop:jira=ABC-1` | with the property, own or inherited; with a value, ignoring case |
| `id:1a2b3c4d`, `id:` + a whole id | the task with that id, by its last eight characters (as the command line prints them) or the whole id; `id:1a2b3c4d id:5e6f7a8b` finds both, so a search can be a list of tasks |

### Ranges

| Range | Means |
|---|---|
| `today`, `tomorrow`, `yesterday` | today, tomorrow, yesterday |
| `week` | today up to Sunday |
| `7d`, `30d` | today and the next 7 or 30 days |
| `-7d`, `-30d` | the last 7 or 30 days up to today |
| `overdue` | overdue |
| `any`, `none` | has the date, has none |
| `2026-11-01` | that day |
| `2026-11-01..2026-11-30`, `..2026-11-30`, `2026-11-01..` | a span, open at one end if you like |

Days are counted in the account's time zone.

## Combining

- Chips of the **same kind** are alternatives: `@phone @computer` finds
  either.
- Chips of **different kinds** must all hold: `@phone due:week`.
- `-` before a chip negates it: `-@Waiting`. A click (in the app a tap)
  on a chip toggles the negation, × removes it; in the web Backspace in
  an empty field removes the last one, Escape clears everything.

Asking for completed tasks (`is:done`, `done:…`) or for a task by its
id (`id:…`) widens the view to "All"; asking for waiting tasks (`is:waiting`, `defer:…`) widens
"Available" to "Remaining". This holds when any chip asks for it, in
whichever group.

## Or

The word `or` (German `oder`) between chips splits them into **groups**,
and a task is found when it meets one group:

- `is:flagged or due:today` – flagged, or due today.
- `#Website or @phone -is:waiting` – in the project Website, or tagged
  @phone and not waiting.

Inside a group everything combines as above. Free text holds for every
group: `is:flagged or due:today invoice` finds the flagged and the due
tasks that mention an invoice. The same chip may stand in two groups.

`or` becomes a chip of its own in the field and is offered in the list
under it once a chip stands there. A new chip goes where the insertion
point is, at the end unless you move it: in the web ← at the start of
the field moves it back across the chips and → forward again, Backspace
there removes the chip before it; a click (in the app a tap) on an `or`
puts it in front of that `or`, so the next chip joins the group before
it, and a second click moves it back to the end. Typing `or` in the
middle of a group splits it.
Removing `or` joins its two groups. An `or` at the start or twice in a
row means nothing and is dropped; one at the end waits for the next chip. Groups cannot be nested,
and there is no "and" within one kind: `@a @b` stays either tag.

Scopes and perspectives keep the groups, and the command line takes and
prints them the same way: `samu scope filter Work "is:flagged or due:today"`.

## German and English

| English | German |
|---|---|
| `due:` `planned:` `defer:` `done:` `created:` `modified:` | `fällig:` `geplant:` `start:` `erledigt:` `erstellt:` `geändert:` |
| `folder:` `state:` `is:` | `ordner:` `zustand:` `ist:` |
| `today` `tomorrow` `yesterday` `week` `overdue` `any` `none` | `heute` `morgen` `gestern` `woche` `überfällig` `mit` `ohne` |
| `7d` `-30d` | `7t` `-30t` |
| `is:flagged` `available` `waiting` `open` `done` `dropped` `group` `note` `repeating` `clocked` `blocker` `blocked` `hidden` `appointment` | `ist:markiert` `verfügbar` `wartet` `offen` `erledigt` `verworfen` `gruppe` `notiz` `wiederholt` `geclockt` `blocker` `blockiert` `versteckt` `termin` |

`#`, `@`, `~`, `prio:`, `prop:` and `id:` are the same in both. A chip is
**stored in its German form**: a scope or perspective keeps `heute`,
`markiert`, `7t`, and the interface shows it in its own language. So a
perspective made in English reads the same on a German phone, and the
command line, which writes German, takes both.

## Arranging

The web's "Anordnen" button groups (by project, folder, tag, state,
priority, date or not at all) and sorts (by own order, due, planned,
defer, title, created, modified, estimate or priority, ascending or
descending), with a second key for ties. The choice is kept per view in
the browser. A perspective stores the search together with its filter,
arrangement and list or board.

```sh
samu perspective add "Quick calls" '@phone ~<15m'
samu perspective arrange "Quick calls" --help     # grouping and sorting
samu list --perspective "Quick calls"
```
