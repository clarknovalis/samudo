---
number: 0115
title: Table view – a third display beside list and board, choosable columns including properties, sums
status: accepted
date: 2026-11-16
adds_to: ["0021", "0046", "0054", "0061", "0064", "0081", "0090"]
---

# 0115 – Table view

## Context

The wish started as "a column or table view with properties and estimate
against actual" and was widened to a table as a general display.

Models:

- **org-mode's column view:** columns from properties, `Effort` against
  `CLOCKSUM`, sums per level of the tree, values edited in place.
- **Todoist's table layout** (web and desktop, Pro): one row per task,
  columns date, deadline, priority, description, created, creator,
  assignees; show and hide, order by dragging, sort by column, group. The
  app shows the same project as a list. It serves for overview and
  tidying (where is a date missing, what is old) and for comparing several
  fields side by side.
- **Tana:** a table per supertag.

samu had list and board (ADR 0061) as the display of every view, the
details line (ADR 0053) with four fixed fields, and properties only in the
inspector (ADR 0054). Estimate and clocks (ADR 0046) were both in the
browser, but the actual time only in the report.

## Decision

1. **A third display.** *List | Board | Table* in every web view, where
   list and board switch today. `PerspectiveView` has `table`; older
   clients read it as `list` (ADR 0015); a new version of `samu-api` with
   a fixture.
2. **Columns.** The title is always there. Choosable: state, priority,
   project (path), tags, defer, due, planned, created, modified,
   completed, estimate, actual, remaining (estimate − actual), blockers,
   and every property as a column of its own (`prop:<key>`; a value that is
   a URI stays clickable as in ADR 0054). Shown and hidden in a menu
   "Columns", ordered by dragging the header. Default: state, priority,
   due, estimate, actual.
3. **Rows as in the list.** Action groups indented with an arrow (ADR
   0021); grouping and sorting from "Arrange" (ADR 0064) apply unchanged; a
   group header is a row of its own.
4. **Sums** as in org: estimate, actual and remaining of an action group
   count its subtree, a group header counts its rows, a last row the
   whole. Actual is the sum of all the task's clocks, a running one up to
   now (ADR 0046); booked time is a clock too (ADR 0081), so the number is
   the same as in the report.
5. **Sorting at the header.** A click on a column header sets the sorting
   of "Arrange", a second reverses it; columns without a sort kind
   (properties, actual, tags) do not sort in this round. Sorting by
   property and actual needs a sort kind with a key in `samu-api` and
   comes in a second round.
6. **Editing in the cell.** State, priority, the dates, estimate, tags and
   property values with the editors the inspector already has, over
   `/changes`. Keyboard: arrows move from cell to cell, Enter opens, Esc
   cancels. Actual, created, modified and project are read-only (a project
   changes by moving).
7. **Remembered.** In a perspective the columns are part of the object
   (`table: { columns: [...] }`, over `/changes`, on every device); in the
   other views per browser like the board choice (ADR 0061).
8. **CLI.** `samu list --columns
   state,priority,due,estimate,worked,prop:jira` for
   `plain`/`tsv`/`json`, the same names as in the web, actual as `worked`;
   `samu perspective view <name> table --columns …`.
9. **Not in the app for now.** The app shows a table perspective as a list,
   as Todoist does; a table needs width. The Emacs package can follow later
   through `--columns`.

## Consequences

- `samu-api`: `PerspectiveView::Table`, `Perspective.table`, with
  fixtures.
- Web: the module `web/lib/table.js`, the menu "Columns", tests under
  `web/tests/`.
- The actual sum per task and subtree is computed by `samu-core` for the
  CLI; the web computes it in the browser from the same clocks, with
  shared test cases under `testdata/`.
- The details line (ADR 0053, now the row fields of ADR 0130) stays; the
  table does not replace it and keeps its own columns.

## Changes

None yet.
