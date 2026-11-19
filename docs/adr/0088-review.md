---
number: 0088
title: Review – due date computed in samu-core, "reviewed" as an ordinary change, a review view in the web
status: accepted
date: 2026-11-12
adds_to: ["0006", "0020", "0025", "0037"]
---

# 0088 – Review

## Context

Since ADR 0006 a project has `review_interval`, `next_review` and
`last_reviewed`. ADR 0037 §3 put the interval into the project settings
(none, 1 week, 2 weeks, 1 month, 3 months) and deferred the review itself.
ADR 0020 §1 and ADR 0025 foresee a view "Review" in the sidebar; it
follows the scope like every view from `/available/projects` (ADR 0094
§7).

Nobody computed a due date, no interface showed one, and nothing set
`last_reviewed`; whoever set an interval saw nothing of it. The model is
OmniFocus's review perspective: one project after another, "Mark
Reviewed".

Stuck is the pattern for a computed project view: computation in
`samu-core::availability`, a flag in `ProjectAvailability`, a filter
`?stuck=true` on `/available/projects`, a view in the web, a hint in app
and Emacs.

## Decision

1. **When a review is due is computed by `samu-core`**, in one place
   beside `stuck`, never stored (as ADR 0006 for availability):
   - Without `review_interval` there is no review.
   - Only active and on-hold projects; reviewing a project on hold is
     exactly deciding whether it starts again.
   - **Date of the next review** = `next_review` if set; otherwise the
     local date (account zone) of `last_reviewed`, without it of `created`,
     plus `review_interval`.
   - **Due** when today's local date has reached that date. An old project
     that just got an interval is due at once: its first review.
2. **`next_review` is only a date set by hand** that overrides §1 ("not
   again before 1 December"). It is no longer advanced.
3. **"Reviewed" is an ordinary field change over `/changes`:**
   `last_reviewed = now`, `next_review = null`, in one batch. No new route,
   LWW per field, offline from the app's outbox, and no client computes a
   date. A changed interval takes effect at once.
4. **Wire form** (`samu-api`, a new version with a fixture):
   `ProjectAvailability` has `review: Option<Date>` (the date of §1, absent
   without an interval) and `review_due: bool`. `/available/projects`
   takes `?review=true` like `?stuck=true` and then returns only the due
   ones, ordered by `review`, otherwise by position. The scope applies as
   for every view from `/available/projects`.
5. **Web, the view "Review"** in the sidebar after "Stuck", key `g r`,
   with a count of due projects:
   - The list shows the due projects one after another, each as a group
     with a header (title, status, "last reviewed … ago", due since) and
     its open tasks below as in the project view, so one can change,
     complete or drop while reviewing.
   - Each group's header has a button **"Reviewed"**; the group disappears
     afterwards. The action `review.done` with the key `R` works on the
     project of the selected task (ADR 0079), also in a project's context
     menu in the sidebar.
   - Empty: "No review due."
   - **A project's inspector:** under "Review" (ADR 0037) also "Next
     review" as a date, changeable and clearable (§2), "Last reviewed"
     read-only, and the button "Reviewed".
   - In the sidebar a due project carries a small mark.
6. **CLI:** `samu projects --review` lists the due ones (columns as with
   `--stuck`, plus `review`); `samu review <id>…` applies §3 to each named
   project over `/changes`; `samu set <id> next_review <date>` exists
   already.
7. **App:** the project tree (ADR 0085) shows "review due" like "no next
   action", and the long-press menu has "Reviewed". A view of its own in
   the app comes later.
8. **Emacs:** the projects view shows the column `review` and marks what
   is due; `samu-review` opens the due projects, and a key runs `samu
   review` on the project at point.
9. **No reminder, no notification** and nothing in the desktop widget: a
   review is something one sets out to do, not something that calls (as
   ADR 0030 §7). The count in the sidebar is enough.
10. **No default interval.** A new project has none; whoever wants reviews
    sets them per project (ADR 0037).

## Consequences

- The meaning of `next_review` narrows (§2). Existing values stay valid
  and act as an override until "Reviewed" clears them.
- `samu-api` has a new version with a fixture; catastr does not read
  `ProjectAvailability`.
- The computation needs the account's zone, which `samu-core`'s settings
  already carry for today and forecast.
- After switching on an interval many projects are due at once. That is
  intended; whoever does not want it sets `next_review`.
- A review flow with its own focus, one project at a time as in OmniFocus,
  is not part of this; the list of §5 is the view.

## Changes

None yet.
