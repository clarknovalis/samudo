# Search language

One small language filters tasks everywhere: the search field of the web
UI and the app, a scope's filter and a perspective. It is made of **free
text** and **chips**. Every word of the language has a German and an
English form, and both are understood whatever language the interface
speaks; this page uses the English ones (see
[German and English](#german-and-english)).

On the command line, `samu list --search <words>` takes free text only;
chips go into scopes (`samu scope filter`) and perspectives
(`samu perspective add`), and `samu list --perspective <name>` shows one.

## Free text

Every word that is not a chip must appear in the title, the note or the
id. The last eight characters of an id, as the command line prints it,
find their task.

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
| `is:` + word | `flagged`, `available`, `waiting`, `open`, `done` (completed), `dropped`, `group` (action group), `note` (has a note), `repeating`, `clocked` (has clocks), `blocker` (another open task waits for it), `blocked` (waits for an open blocker, its own or its project's) |
| `is:appointment` | appointments instead of tasks, by title, place and note; a repeating one once, with its next occurrence; date chips apply to the start (web only, not in a scope's filter) |
| `~<15m`, `~>=1h`, `~30m`, `~any`, `~none` | by estimate (`<`, `<=`, `>`, `>=`, equal; with or without one) |
| `prio:a`, `prio:b`, `prio:c`, `prio:any`, `prio:none` (or `prio:-`) | by priority |
| `prop:jira`, `prop:jira=ABC-1` | with the property, own or inherited; with a value, ignoring case |

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

Asking for completed tasks (`is:done`, `done:…`) widens the view to
"All"; asking for waiting tasks (`is:waiting`, `defer:…`) widens
"Available" to "Remaining".

## German and English

| English | German |
|---|---|
| `due:` `planned:` `defer:` `done:` `created:` `modified:` | `fällig:` `geplant:` `start:` `erledigt:` `erstellt:` `geändert:` |
| `folder:` `state:` `is:` | `ordner:` `zustand:` `ist:` |
| `today` `tomorrow` `yesterday` `week` `overdue` `any` `none` | `heute` `morgen` `gestern` `woche` `überfällig` `mit` `ohne` |
| `7d` `-30d` | `7t` `-30t` |
| `is:flagged` `available` `waiting` `open` `done` `dropped` `group` `note` `repeating` `clocked` `blocker` `blocked` `appointment` | `ist:markiert` `verfügbar` `wartet` `offen` `erledigt` `verworfen` `gruppe` `notiz` `wiederholt` `geclockt` `blocker` `blockiert` `termin` |

`#`, `@`, `~`, `prio:` and `prop:` are the same in both. A chip is
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
