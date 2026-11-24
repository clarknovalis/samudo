---
number: 0147
title: Block perspectives in the app – block by block as in the web, and every group heading of the app's lists can be folded
status: draft
date: 2026-11-24
adds_to: ["0102", "0116", "0146"]
---

# 0147 – Block perspectives in the app

## Context

ADR 0116 §8 left block perspectives in the app to a decision of their
own. Until now the app shows a block perspective as the union of its
blocks (0102 §4, 0116 §4): every task once, grouped by the arrangement
of the block perspective itself. That loses what a block perspective is
for: a heading per block, each block in its own arrangement, the note
blocks of 0146. A morning view on the phone becomes one long list.

No heading of the app can be folded either. In the web every heading of
a list and every block folds (per browser), and an empty block starts
folded. On a phone, where the screen is short, folding matters more.

## Decision

1. **Block by block.** The app shows a block perspective as the web
   does (0116 §5): per block a heading with icon, its own title, else
   the name of the perspective or view, or the words of its search
   (0143), and the count of its tasks; "missing" when the block is
   (0116 §2). Inside a block its tasks in the arrangement of its
   perspective (0116 §5); a board or a table is shown as a list (0102
   §4). With `dedupe` a task stands only in the first block that hits
   it. The evaluation is `samu_core::views::blocks` through
   `samu-mobile`, with the scope and tag chosen in the app as the lens
   (0116 §3); the app computes nothing on its own (0006).
2. **Note blocks** (0146) as in the inspector of the app: the note
   rendered, its checkboxes ticked in place, the pencil turns it into
   the field, and leaving the field saves. No count, the scope does not
   apply (0146 §2).
3. **Tapping a block's heading folds it.** An icon at its right end
   opens the perspective, view, project or task it refers to; a missing
   block has none. A long press on a heading leaves only that block
   open, as the alt-click in the web does.
4. **Every group heading folds.** The same holds for the headings of
   every list of the app (project, tag, perspective, today, flagged and
   the other lists drawn as groups): a tap folds or unfolds the group,
   a long press leaves only this one open. Forecast stays as it is; its
   strip jumps between the days.
5. **Folds are kept per device**, across restarts, as the web keeps them
   per browser. The key names the list and the group, so a group folded
   in one list stays open in another. An empty block starts folded and
   does not count for "only this one open"; opened, it stays open until
   the list is left.
6. **Interface:** no new route, no migration, no new version of
   `samu-api`. `samu-mobile` gets a function that answers a block
   perspective with its blocks, each with its sections as the other
   lists have them; `perspective()` keeps answering the union, as
   `/available?perspective=` does.

## Consequences

- ADR 0102 §4 changes: a block perspective no longer shows the union in
  the app.
- ADR 0116 §8 is done for the app; the Emacs package still shows the
  union.
- Folding is new in every list of the app, not only in block
  perspectives.

## Changes

None yet.
