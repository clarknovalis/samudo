# Android app

A native app (Kotlin and Jetpack Compose) on top of the same Rust core as
the server, so availability, quick capture, clocks and habits are computed
the same way on the phone, also offline. The interface speaks German or
English (see [Language](#language)); this page uses the English labels.
Building it is described in [`android/README.md`](../android/README.md).

## Signing in

Either with server, account, password and a device name, or by **pairing**:
in the web UI open the account menu, "Pair a phone", and scan the QR code
with the app (or use `samu pair phone` on the command line). The code works
once, for ten minutes. The app keeps only the session token.

## Offline first

Every change goes into the **outbox** first and is sent with the next
sync: when the app opens, after every change, when you pull a list down
and every 15 minutes while there is a network. A dot at the title shows
that something waits; the menu (⋮) says how much. Files wait on the phone
and go up before the changes that refer to them. Only deleting needs a
connection.

## The head

One line: the title, the [scope](concepts.md#scopes) as a chip (a tap
picks another) and a menu (⋮) with "Details", "Sync", "Settings" and, in
the projects, "New project" or folder. **Pull a list down** to
sync. The tag chips are the first row of the list and scroll away with
it; each filters by one tag and everything below it.

## The bar

The bar at the bottom shows five entries at a time; swipe it sideways for
more, a dot at its edge says it goes on. Without a setting it has
**Inbox**, **Available**, **Forecast**, **Projects** (a tree of folders and
projects that can be created, moved and folded) and **Search**.

Settings › "Bar" decides what it holds, on this phone only: drag an entry
by its handle, hide it with its switch, remove it with ×. "Add" offers
the fixed views (also **Today**, **Flagged**, **Stuck**, **Review** and
**Habits**), your perspectives, projects (search by name) and tags. Inbox
can only be moved. A project, tag or perspective that is deleted or
dropped leaves the bar by itself.

- A **perspective** shows its tasks as a list, with its scope and filter
  as on the web; its chips stand under the title, to look at. Changing a
  perspective stays in the web and the [command line](cli.md). It shows
  its emoji in the bar, without one a funnel.
- A **project** opens as from the tree, a **tag** shows the available
  tasks with it.

## Search

Search takes the [search language](search.md), in German or English.
While you type, the list under the field offers kinds, projects, tags,
folders, states and dates for the word at the cursor, also offline; a tap
takes one. A recognised word becomes a **chip in the field**: a tap
negates it, × removes it.

The empty field shows five quick filters ("Flagged", "Due today",
"Overdue", "No project", "Done today"), the last five searches on this
phone and your perspectives. With at least one chip or word, "☆ As
perspective" saves the search as a perspective (emoji and name) and opens
it.

## Working with tasks

- **Rows** have a round check; swipe right or tap the check to complete
  (or reopen). **Swipe left** flags or unflags. Tap a row to open the task.
- **Undo:** after completing, flagging, unflagging or dropping from a
  list, a bar at the bottom says so ("3 completed" for a selection) and
  offers "Undo" for a few seconds. Until it is gone the change waits on
  the phone; undo takes it back out of the outbox, so the server never
  sees it (no logbook entry, no next task of a repetition). If a sync has
  already sent it, undo writes the opposite change instead.
- **Long press** opens the task's menu: "Edit", "Add subtask", "Flag",
  "Defer until …", "Move …" (into the inbox or a project, children go
  along) and "Select". A habit keeps its own menu.
- **Edit** shows the title in its own field, what is set as chips (✕
  marks one for removal, ↶ takes that back) and a line in
  [quick capture](quick-capture.md) syntax with offers and a preview of
  what it adds. "Save" writes it all at once, also offline.
- **Selecting many:** "Select" in a task's menu, or in the menu (⋮) of the
  head, starts picking; taps then add or remove tasks until ✕ or Back.
  The bar offers complete, flag, move, and under "⋯" reopen, drop, plan,
  dates, tags, state and delete; with exactly one task also edit and the
  clock.
- **The task page** is editable: title, note, defer, due and planned dates,
  estimate, tags and state as chips, the project line moves the task, and
  repetition and reminders.
- **Comments and attachments** at the bottom: text, a **photo** from the
  camera app, a **recording** ("● Record", asks for the microphone the
  first time) or any **file**. With text it becomes a comment with the
  files on it; without, the files hang on the task. Images preview,
  recordings play, other files open in another app.
- **Clock** ("Time"): total against the estimate, start/stop and the last
  clocks. A running clock shows as a permanent notification with the time
  and "Stop".
- **Habits:** see [Habits](habits.md#app).

## Capture and sharing

- **Capture** opens a sheet with [quick capture](quick-capture.md);
  the preview is computed on the phone, also offline.
- **Share to samudo** from any app that shares text: the subject or first
  line becomes the title (editable), below it a quick-capture field for
  project, tags and dates. The rest of the text becomes the note, and every
  `https://`, `mid:` or `catastr:` address a link. Without a session the
  shared text waits until you sign in.
- **`samu:<id>` links** in other apps open the task.

## Widgets

- **"Today"**: overdue, today, then flagged available tasks. The
  check completes through the outbox, "+" opens capture, a row opens the
  task. Redrawn after every sync and when you leave the app.
- **"Habits"**: every open habit; see [Habits](habits.md#widget-habits).

## Reminders and calendar

Reminders of tasks are set as alarms on the phone and show as
notifications, also without a network. Appointments show in the forecast
and in the calendar view (list, day, week); a tap opens the appointment's
sheet, read only, with "Open in calendar app".

**Reminders of appointments** (Settings › "Reminders") are off at first
and apply to this phone only. Switched on, the reminders an appointment
carries in its calendar (VALARM) go off like those of tasks, with title,
time and place; "Later" asks again in an hour, a tap opens the
appointment. If the phone already syncs the calendars through DAVx5, leave
it off, or every reminder comes twice. The appointment's sheet names its
reminders ("15 min before") whether the switch is on or not.

## Language

Settings › "Language": "System", "Deutsch" or "English", on this phone
only. "System" follows the phone's language and the app language in
Android's settings (Android 13 and later). A change redraws at once;
widgets and notifications follow when they are next drawn. Error messages
from the server stay English.

## Automation with Tasker

Two broadcasts let other apps (for example Tasker) act on the app, whose
package name is `de.samudo.app`.

**Scope** (follow your context: Do Not Disturb, Wi-Fi, location):

| Field | Value |
|---|---|
| Action | `de.samudo.app.action.SCOPE` |
| Package | `de.samudo.app` |
| Extra | `scope`: a scope's title or id; empty = no scope; `auto` = follow the schedule |

**Capture into the inbox** (as the ring does, also offline): action
`de.samudo.app.action.RING`, extra `text`, optional `recorded_at` in Unix
milliseconds. See [Capture from the Index ring](ring.md#offline-through-tasker).

Try them over adb:

```sh
adb shell am broadcast -a de.samudo.app.action.SCOPE -p de.samudo.app --es scope Work
adb shell am broadcast -a de.samudo.app.action.RING -p de.samudo.app --es text "'Buy milk'"
```

Back to the sign-in screen: `adb shell pm clear de.samudo.app`.
