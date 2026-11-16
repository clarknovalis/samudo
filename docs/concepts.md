# Concepts

samudo borrows from several to-do worlds: projects, availability and
perspectives from OmniFocus, capture, refile and clocking from org-mode,
and workflow states from Jira. This page explains the pieces once; the
pages for each client only say where to find them. The reasons behind
the design are in [Architecture](architecture.md).

## One server, many clients

- **The server is the single writer.** Every task lives in one database on
  the server. The command line, the web UI, the Android app, Emacs and the
  desktop widget are clients of its HTTP API.
- **Offline is a queue, not a second truth.** The Android app and the
  optional local replica on a computer keep a copy and an *outbox* of
  changes. When they reach the server again, it decides per field: the
  later change wins (last writer wins, stamped with a hybrid logical clock).
- **One account, one database.** Private and work are two accounts with
  two separate SQLite files, not a field on each task. A computer can be
  signed in to several accounts at once through [profiles](profiles.md).
- **Availability is computed, never stored.** Whether a task can be done
  now is worked out on the server (and, with the same code, in the app
  while offline). No client decides it on its own.

## Places: inbox, folders, projects, action groups

| Thing | What it is |
|---|---|
| **Inbox** | Tasks without a project. Everything captured without `#project` lands here. |
| **Folder** | A tree of folders holds projects. A folder is active or dropped; dropping it hides everything below. |
| **Project** | A goal with tasks. Its *kind* is **parallel** (every open task is available), **sequential** (only the first open task is) or **single actions** (a list of loose tasks that is never "stuck"). Its *status* is active, on hold, completed or dropped. |
| **Action group** | A task with children. It is not itself available while it has open children; it can be sequential, and can complete itself with its last child. |

Folders, projects and tags may carry an emoji as icon. A folder, project
or tag can name its **own scope** ("Only in scope: Work"): it then appears
only in that [scope](#scopes), whether the scope shows *only* what it ticks
or *everything except* it, and nowhere else, not even without a scope.
That is useful for a work area you do not want to see at home, or for
habits. In its own scope it needs no tick; ticked in an "everything except"
scope it is left out like anything else. Inside a folder with an own
scope, the nearest folder with one decides.

## Tasks

A task has a title, a note (Org or Markdown), a place, and optionally:

- **Dates**, each either a whole day (a date without zone) or an instant
  (a time with the account's zone):
  - **defer**: not available before this date;
  - **due**: must be done by then; it is *due soon* a set time
    before (two days by default, per task adjustable) and *overdue* after;
  - **planned**: when you intend to do it. With a time and a
    length it is a block in the calendar view.
- **Flag**, **priority** A, B or C (none sorts after C), and an
  **estimate** (`~30m`).
- **Repetition**: an RRULE (RFC 5545) with one of three modes:
  - *fixed* (`every:`): the next dates follow the rule, however late you
    completed it; with catch-up (`every+:`) missed occurrences are skipped
    up to today;
  - *due after completion* (`every!:`): the next due date counts from the
    day you completed it;
  - *defer after completion*: the next defer date counts from completion.
  Completing a repeating task makes the next one as a new task.
- **Tags** in a tree, with an order on each task. A tag is active, on hold
  or dropped; a task with an inactive tag is not available.
- **Properties**: free keys and values, inherited from the action group and
  project. A property key can have a link template, so `jira=ABC-1` opens
  your tracker.
- **Links** to other objects: `samu:<id>`, `https:`, `mid:` (a mail),
  `catastr:` (the sister project) and others. A link may point to
  something that does not resolve.
- **Blockers**: "blocked by" another task. The task waits until that one
  is done; samudo refuses a task blocking itself or a circle.

### Availability and its reasons

Every view shows tasks with their availability and **every reason** that
applies, so you can see *why* something is not available: completed,
dropped, deferred until a date, project not active, folder dropped, tag not
active, waiting in a sequence, has open children, its action group is not
available, blocked by another task.

The views offer three filters: **available**, **remaining**
(everything open, also what waits) and **all** (also
completed and dropped).

## Workflows and states

A tag can be *exclusive*: then it is a **workflow** and its children are
the **states**, for example `Status` with `Next`, `In progress`, `Waiting`,
`Done`. A task is in at most one state of a workflow. Each state can have a
key (for the `t` key in the web), can ask for a note when entered, and the
board view uses states as columns. A project names the workflow its tasks
use; otherwise the account's default applies.

## Views

| View | Shows |
|---|---|
| **Inbox** | tasks without a project |
| **All projects** | the folder and project tree with their tasks |
| **Today** | planned for today or earlier |
| **Forecast** | overdue, today, flagged, the next days and later, each row with why it is there; appointments of your calendars beside the tasks |
| **Calendar** | day, week, month or list with appointments and planned tasks |
| **Flagged** | flagged tasks |
| **Stuck** | active projects with no available task (single-action lists never count) |
| **Review** | projects whose review is due, oldest first |
| **Report** | what was done and worked on, and time clocked, over days |
| **Perspectives** | your saved searches, see below |

**Habits** are tasks with a goal that you enter day by day; they show in
the lists like other tasks, with a button instead of the circle. See
[Habits](habits.md).

### Review

Each project can have a review interval. The review view lists projects
whose next review has come; marking one reviewed sets the next
review from today.

### Perspectives

A perspective is a saved search: words and chips of the
[search language](search.md), a filter (available, remaining, all), a
grouping and sorting, and list or board. Perspectives appear in the
sidebar; the command line shows one with `samu list --perspective <name>`.

A **block perspective** shows several perspectives and the views Inbox,
Today and Flagged one below the other, like org-mode's block agenda: say
first today, then the short tasks at the office, then what waits for
others. Each block keeps its own search, filter and arrangement and may
have a heading of its own. A task that several blocks show stands only in
the first, unless "Duplicates once" is off.

## Scopes

A **scope** is a lens over every view: it names folders, projects and tags
(each with everything below it), and says whether the views show **only**
what those cover, or **everything except** it. On top it can have:

- a filter in the search language, which every task must also meet;
- whether the inbox belongs to it;
- what a task captured while the scope is chosen gets (a project, tags);
- a schedule (`mo-fr 08:00-17:00`), so devices set to "auto" switch scopes
  by themselves;
- which calendars it shows, a colour, and a layout of the task inspector.

The chosen scope belongs to the **device**, not the account: the laptop can
be in "Work" while the phone shows everything.

## Capture

Most tasks are made with **quick capture**: one line with marks for
project, tags, dates and more, for example
`Call the tiler #Bathroom @phone d:fri ~15m`. See
[Quick capture](quick-capture.md). **Templates** store a line of marks
under a name (`/habit`) and optional reminders; a template can have a key
for fast capture.

Other ways in: share text from any Android app to samudo, a voice note in
the web UI, a webhook (for example [from a smart ring](ring.md)), and the
[Org import](cli.md#import-and-export).

## Time: clocks and the report

A **clock** records time spent on a task or project. At most one clock runs
per account; starting another stops the running one, completing a task
stops its clock. Time can be booked afterwards (`z:yesterday 2h` in quick
capture). The **report** shows, over a range of days, what was completed,
what was worked on and the time clocked, grouped by task, project, folder,
tag or day.

## Calendars and appointments

samudo reads calendars from elsewhere (an ICS subscription or CalDAV) and
shows their **appointments** in the forecast and calendar views. A CalDAV
calendar can be marked writable; then samudo can create, change and delete
appointments there too. The addresses and passwords of calendar sources are
stored encrypted on the server.

## Reminders

An open task or project with a due date reminds at that date by itself.
More reminders can be set: at an instant, or a length of time before its
due, planned or defer date. A date without a time reminds at the account's
reminder time (09:00 by default). A **daily** reminder goes off every day
at its time while the task is open; on a habit only on days that still
need something (see [Habits](habits.md)). The Android app shows reminders
as notifications.

## Comments, attachments and authors

Tasks and projects have **comments** (Org or Markdown, rendered) and
**attachments** (any file; photos and voice recordings straight from the
web UI or app). Every object records **who made it**: the device and
whether it was a person or an **agent**. Agents (such as an AI assistant
using the command line) sign in with a session of their own, so their
comments and changes are marked as such.

## History and deletion

Every object has a **logbook** of what happened to it. Deleting leaves
nothing dangling: a project's tasks go to the inbox (or are deleted with
it, if you say so), a folder's content moves up, a task takes its children
with it, and a tag comes off everything first. The deleted object stays as
a tombstone, so other devices learn about the deletion.

## Your data

Everything can be exported as a directory of plain JSON files (and an Org
outline) that you can read without samudo, and restored into an empty
account. See [Self-hosting](self-hosting.md#export-and-restore).
