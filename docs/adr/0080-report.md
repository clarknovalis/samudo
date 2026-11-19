---
number: 0080
title: Report – done, worked on and time over a period, filtered with the search language; GET /report and a view "Report"; "worked on" configurable, by default only a person's own work
status: accepted
date: 2026-11-12
adds_to: ["0042", "0043", "0045", "0046", "0066"]
---

# 0080 – Report

## Context

The wish: a report page of work done, worked on and time, filterable by
period (last week, last N days, last month) and in detail, somewhat like
Jira.

What existed:

- **The view "Time"** in the web (ADR 0046 §7): periods today,
  yesterday, this week, last week, this month, 30 days and free; grouped
  by task, project, folder, tag or calendar day; data from `GET
  /clocks/report`, computed in `samu-core`, with scope; the choice kept
  in the browser (`samu.time`).
- **The search language** (ADR 0042), parsed and evaluated in
  `samu-core` (`query::parse`): `#project @tag folder: state: prio:
  is:flagged is:clocked due: done:-7d ~<15m`, with negation; also a
  scope's filter (ADR 0043).
- **The logbook** per object (table `logbook` with `at`, `kind`): state,
  completed, dropped, reopened, with author `human`/`agent`; indexed by
  `(object, at)`, not by time alone.
- **Comments** with time and author (ADR 0039).

Missing: "what did I work on" and "what got finished" as a list over a
period, and filters beyond scope and grouping.

## Decision

### 1. Three parts, one period, one filter

A report has a period (local dates `from` to `to` inclusive, bounds in
the account's zone as in ADR 0046 §5), a filter and a grouping. It has
three parts:

- **Done:** tasks (and projects) with `completed_at` in the period; with
  `--dropped` dropped ones too, marked as such.
- **Worked on:** tasks where something happened in the period, each with
  **what** happened as a list of reasons with counts:
  - `clocked`: time clocked in the period,
  - `state`: state changed (logbook),
  - `completed`/`reopened`/`dropped` (logbook),
  - `commented`: a comment written,
  - `child_completed`: a subtask completed;

  plus the latest instant. Completed tasks appear here too, since
  completing is work.

  What counts as "worked on" is an **account setting**, `report_worked`
  in `settings` (ADR 0045 §4), changeable in web, app and CLI:
  - `events` (default): the reasons above;
  - `fields`: also every field change, reason `changed`, from the field
    stamps (table `stamp`: `wall`, `device`). Stamps keep only the
    **last** change per field, so a field changed in the period and again
    later does not count for the period. That is knowingly imprecise; a
    history of all field changes would be a record of its own. Stamps of
    the server itself (follow-ups, clock ends, ADR 0033, 0046) do not
    count.
- **Time:** as `/clocks/report`, with the same filter.

### 2. Filter in the search language

The filter is a text in the search language of ADR 0042 (`q`), parsed
and evaluated in `samu-core` like a scope's filter, and the chosen scope
applies as everywhere (ADR 0035). So filters like `#website @today
prio:A -state:Wish`, `folder:Work -@agenda` or `is:flagged` need no new
language. The language gains one chip:

- **`autor:<account>|agent|alle`** (English `author:<account>|agent|all`;
  `ich`, `mensch`, `me` and `human` stand for the account's person): only
  events of this author count, from `author_kind` in the logbook and on
  comments. For field stamps the device decides: the device of an agent
  session counts as `agent`. For "Done" the author of the `completed`
  entry counts. Time has no author and always counts.
  **Without `autor:` the filter means the account's person**: the report
  shows that person's work. `all` adds the agents' work, `agent` shows it
  alone. In the web the chip stands visibly in the filter field, so the
  default does not look hidden.

A `done:` or `due:` chip in the filter applies **in addition** to the
period; the period stays the field above.

### 3. Periods

The presets of "Time" (today, yesterday, this week, last week, this
month), plus **last month**, **last N days** with a number field, and
**free** (from–to). "Last N days" is computed by the client for display
only; the route always receives `from` and `to`.

### 4. API: `GET /report`

`GET /report?from=&to=&q=&by=&scope=&parts=done,worked,time` answers
`Report { from, to, done: Vec<ReportRow>, worked: Vec<ReportRow>, time:
ClockReport }`. A `ReportRow` holds task or project (id, title, project,
folder), `at` (completion or latest event), `reasons` (`{reason, count}`)
and `seconds` in the period. With `by` (`task`, `project`, `folder`,
`tag`, `day`) the rows come grouped as `groups` with sums, as in
`ClockReport`. Unknown values are read as `unknown` (ADR 0015).

The computation is a pure function in `samu-core` (`report::compute`)
over the period's tasks, clocks, logbook entries and comments. The store
delivers only the period's entries; a migration adds an index `logbook
(at)`.

The replica (ADR 0066) has no logbook in its mirror. It answers `503
offline` for `worked` and computes `done` and `time` itself.

`/clocks/report` stays for catastr and existing clients.

### 5. Web: the view "Report"

The view **"Report"** in the sidebar has three tabs: **Done | Worked on |
Time**; it replaced the older view "Time" (ADR 0081 §1), whose choice in
`samu.time` was taken over on first opening. Below the tabs:

- the bar with period (§3), grouping and filter field (the same chip
  field as the search, `chipfield.js`, with suggestions);
- the sums: number done, number worked on, total time;
- the rows. A click opens the task in the inspector. "Worked on" shows
  the reasons as small marks (⏱ 1:20, ⇄ 2, 💬 1, ✓), "Done" the date.

Period, tab, grouping and filter are kept in the browser (`samu.report`).
There are no saved reports; whoever wants to keep a filter makes a scope.

### 6. CLI

`samu report [--from] [--to] [--days N] [--q '<filter>'] [--by …]
[--part done|worked|time]`; `samu settings set report-worked
fields|events`; with `--format plain|json|jsonl|tsv`. TSV is the way into
a spreadsheet; the web has no CSV export.

### 7. App

Later, over the same route.

## Consequences

- `samu-api`: `Report`, `ReportRow`, `ReportReason`, `ReportPart`,
  `AccountSettings.report_worked` (unknown values as `unknown`), with
  fixtures. catastr is untouched and can read it later.
- `samu-core`: `report::compute` with tests (bounds in the zone, a clock
  across midnight, filter, author, groups); `query` knows `autor:`.
- `samu-store`: migration with the index `logbook (at)` and the column
  `report_worked` on `settings`; queries by period for logbook, comments
  and field stamps.
- Server: `GET /report`; the replica partly (§4), with its entry in the
  per-route table (ADR 0066 §5).
- Web: `report.js` for the view; `report_worked` in the settings menu.

## Changes

- 2026-11-12 (ADR 0081 §1): §5 added "Report" beside "Time", which stayed
  as it was.
