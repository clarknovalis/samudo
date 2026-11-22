---
number: 0021
title: Action groups in the lists – indented, foldable, with a counter
status: accepted
date: 2026-11-07
adds_to: ["0006", "0017", "0020"]
---

# 0021 – Action groups in the lists: indented, foldable, with a counter

## Context

Subtasks are in the model already: `parent` makes a task a child, and a
task with children is an action group with its own `kind` and
`complete_with_last_child` (ADR 0006). Availability takes them into
account (ADR 0017 §5). ADR 0020 names action groups only as a target of
the refile palette, not what they look like in a list.

## Decision

1. **A group is a row like any task**, with an arrow to fold and unfold,
   its own check mark, its title in bold, its kind (sequential or
   parallel) and a counter done/all ("1/4"). The children stand below it,
   indented. Groups inside groups are indented further.
   In the app the arrow stands at the right edge of the row, where the
   thumb is, and only on a group that has children in this list.
2. **The filter of the list decides which children appear:**
   - "Available": only the available children. The group stands above
     them as context, also when it is not available itself (ADR 0006
     rule 6).
   - "Remaining": all open children, those that wait dimmed with their
     reason, as ADR 0020 §2 says.
   - "All": the completed ones as well.

   The web UI has one filter for all its lists. The app keeps one per
   list on the phone, next to the list's arrangement (ADR 0125), and each
   list starts with what it showed before there was a choice: the inbox,
   a project and the search what remains, the others what is available.
   The views named after their filter, Available and Today, have none.
3. **Folded**, the group shows a summary instead of its children: a
   sequential one its next action, a parallel one the number of available
   children ("3 available"). Both come from the result of availability in
   `samu-core`, not from the interface.
4. **Default:** unfolded, in the web UI and in the app. What is folded
   belongs to the view on the device and is not synchronised. It is no
   field of the task.
5. **Adding and moving children:** through refile with "as a child"
   (ADR 0020 §4), and in the web UI with keys to indent and outdent in
   the list.
6. **A group that is a hit brings its children.** Where a search, a tag
   or a perspective finds a group but not its children, the children
   stand below it as context, dimmed, and theirs below them, as far as
   the filter of the list (§2) lets them through. Context thus goes both
   ways: the group above a hit, the children below one.
   The account says how such a group starts, in the setting
   `hit_children`: `open` (the default), or `folded`, where the arrow
   brings the children. The arrow turns that round per device, as in §4.
   It is one setting for the web UI and the app, set in either or with
   `samu settings set hit-children open|folded`. In a flat list (see
   Consequences) the setting does not count: the children stand on their
   own there already.

## Consequences

- The summary of a folded group needs, per group, the next action or the
  number of available children in the result of availability
  (`GroupSummary` in `samu-api`, `group_summary` in `samu-core`).
- A list that is grouped or sorted by anything but the order of its
  projects is flat: a child stands on its own, without its group above
  it.
- `hit_children` is a field of the account's settings, since `samu-api`
  0.1.64, additive, with migration 0030. A reader that does not know it
  draws such a group as before.

## Changes

- 2026-11-22: §4 had groups folded on Android, except the one holding
  the chosen task; a child was then only visible after a tap. §2 did not
  say where the app keeps its filter; it had a fixed one per list.
- 2026-11-22: §6 is new. A group found by a search or a tag stood alone
  in the app, though "all" was chosen, and folded with an arrow in the
  web UI; its children were context nowhere. Now they come along, and
  the account chooses between unfolded and folded.
