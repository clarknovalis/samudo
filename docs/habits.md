# Habits

A habit is a task with a goal that you enter day by day: drink eight
glasses, stretch every day, train three times a week, stay away from
cigarettes. samudo counts the entries and computes the streak and the rate.
Each tap is an entry of its own and does not complete the habit. Decisions
and reasons: ADR 0091 (habits) and ADR 0092 (the app), in German.

## Making a habit

A task becomes a habit through `habit:` in quick capture:

| Written | Means |
|---|---|
| `habit:` or `habit:1/d` | once a day |
| `habit:3/w` | three times a week (Monday to Sunday) |
| `habit:2/m` | twice a calendar month |
| `habit:8/d:glass` | an amount with a unit: eight glasses a day |
| `habit:avoid` | something to avoid: only relapses are entered |

```sh
samu add 'Water #Habits @habit habit:8/d:glass'
samu habit set <id> 3/w        # make an existing task a habit, or change its goal
samu habit set <id> none       # make it a plain task again
```

A [template](quick-capture.md#templates) makes this a single key, for
example `samu template add habit '#Habits @habit habit:1/d'` with
`samu template key habit h`, then `C h` in the web.

A habit has no due date and no repeat. Its project, tags, note, scope and
links work as for any task. **Completing or dropping a habit ends it.** Its
entries stay.

## Entering

There are three kinds of entry:

- **done:** done once. With a unit, each entry counts 1, so tap again for
  the next glass. On the command line `--amount 2` enters two at once.
- **fail:** not done, on purpose. This is different from entering nothing.
  For a habit to avoid it is a relapse.
- **pause:** holiday or illness. The streak does not break. On the command
  line a pause can last several days.

An entry is for today or a day before, never a later day. A wrong entry is
deleted, never changed. Two phones that each tap +1 offline add up to 2.

### Web

- In every list the row shows a button instead of the circle: ✓, or **+1**
  with `5/8` beside it. The button turns green once the goal is met. A habit
  to avoid shows "free for n d" and a button **Relapse**. The small figure
  beside it is the streak.
- The context menu has "Done today" (not for a habit to avoid),
  "Not done today" (for a habit to avoid "Relapse today") and
  "Pause today".
- The inspector's part **Streak** shows the goal, the streak, the longest
  streak, the rates and a calendar of the last 12 weeks. Click a day to see
  its entries, delete one, or enter done, not done or a pause for that day.

### App

- **Row:** ✓ or **+1** with `5/8` and 🔥 with the streak. A habit to avoid
  shows "free for n d" and has no button.
- **Swipe right** means "today done" and never completes the habit. For a
  habit to avoid, swiping does nothing, so a relapse cannot happen by
  accident.
- **Long press** opens a menu: "Done today" or "+1 today", "Not done
  today" (for a habit to avoid "Relapse today"), "Pause today", and
  "Complete", which ends the habit. While you are picking tasks, a
  long press picks as usual.
- **Task page, section "Streak":** the same figures and calendar as in the
  web. Tap a day to delete an entry or to enter one for that day. For a
  habit to avoid, the day's sheet offers **Free from here** instead of
  "Done": it enters done for that day, so "free for" counts from
  there when the day lies before the habit's beginning. This is the only
  place in the app that enters done for a habit to avoid.
- Everything works offline through the outbox and counts at once.

### Widget "Habits"

Drag "samudo · Habits" onto the home screen. It lists every open habit, in
the order of projects and positions, including habits that are only shown
in their own scope.

- **✓ or +1** enters today as done. The row fades once the goal is met.
- **The title** opens the task with its section "Streak", where you take
  back a wrong tap.
- **A habit to avoid** shows "free for n d" without a button. Enter a
  relapse in the app.
- It is drawn anew after every sync and at midnight in the account's time
  zone, when a new day begins.

In the widget "Today", the ring of a habit also enters today as done and
does not complete it.

### Emacs

A row shows ✓, ✗, ○ or the amount like `5/8 Glas`, and the streak; a
habit to avoid shows "frei seit n T". `h h` enters done, `h f` not done
or a relapse, `h p` a pause, `h u` deletes the last entry, and `h s` shows
the streak, the rates, the last 12 weeks and the entries. See
[Emacs](emacs.md#habits).

### Command line

```sh
samu habit list [--scope Habit]          # status today and streak
samu habit show <id> [--days 28]         # streak, rate, the last days as marks
samu habit done <id> [--day D] [--amount N]
samu habit fail <id> [--day D]           # for a habit to avoid: a relapse
samu habit pause <id> [--from D] [--until D]
samu habit entries <id>
samu habit undo <entry>
```

`samu habit list` goes through the chosen scope. Habits that are shown only
in their own scope are missing without it, so use `--scope Habit`.

## How it counts

samudo computes this on the server and in the app, offline:

- **A day** of a habit to build is *done* once the day's goal is reached,
  *partial* below it, *fail* with a fail entry, *pause* in a pause, and
  *none* with no entry. For a habit to avoid, every day without a relapse is
  done, counted from the day the habit was made.
- **A period** (day, week or month) is met once its done amounts reach the
  goal.
- **The streak** counts the periods met in a row up to today. The running
  period never breaks it, and once met it counts. A missed period with at
  least one pause day neither breaks nor counts. Fail and no entry both
  break it. For a habit to avoid, the streak is "free for n days": the
  days since the last relapse, pauses left out.
- **The rates** of the last 4 weeks and of this year divide the periods met
  by the closed periods, without paused ones.
- **Entering for an earlier day** than the habit's creation moves its
  beginning back. For a habit to avoid this sets where "free for" counts
  from: in the app with **Free from here** on the day's sheet, on the command
  line with `samu habit done <id> --day 2026-10-01`, or in the web.

## Reminders

A habit reminds with a **daily** reminder: "daily at" in the inspector or
the app, `samu remind <id> --daily 20:00`, or `daily@20:00` in a template
(so `/habit` can bring one). It goes off at that time in the account's
zone, but only on a day that still needs something: not once the day's
goal is reached, not after a fail or during a pause, not once a weekly or
monthly goal is met, and never for a habit to avoid. Entering before the
time keeps it quiet. Like every reminder it rings on the phone.

## Not yet

Some of this follows later, with an ADR each:

- a limit ("at most n"),
- minutes from the clock,
- fixed weekdays,
- a note on an entry,
- a challenge with an end.
