---
number: 0116
title: Block perspectives – several queries in one view like org's block agenda, built from references to perspectives and fixed views
status: accepted
date: 2026-11-16
adds_to: ["0090"]
---

# 0116 – Block perspectives

## Context

ADR 0090 §9 deferred "several queries in one view like org's block
agenda" to a decision of its own.

In org an agenda command is a list of blocks, `(agenda "")`, `(tags-todo
"@office")`, `(todo "WAITING")`, each with its own heading and settings,
all in one buffer. A task that two blocks hit stands in both. A typical
morning: first the day, then the short things at this place, then what
waits. OmniFocus and Todoist have nothing equivalent; there one clicks
through several perspectives.

samu has perspectives (ADR 0090) with search, filter, arrangement and
display (list, board, table per ADR 0115) and the fixed views inbox,
today, flagged and forecast; each shows one query.

Chosen: blocks as references, not as a second definition; duplicates by
a switch, by default shown once; a block as list or table; web and CLI,
evaluated in `samu-core`.

## Decision

1. **A block perspective** is a perspective (ADR 0090 §1) with the field
   `blocks`, a list, LWW as a whole. When it is not empty the perspective
   is a block perspective; `text`, `chips`, `filter`, `arrange`, `view`,
   `board` and `table` then do not apply and are not offered in web and
   CLI. Also `dedupe` (boolean, default `true`, meaningful only for block
   perspectives).
2. **A block** is `{perspective: <id>}` or `{view: <name>}`, each with an
   optional `title` as its own heading. Fixed views as blocks: `inbox`,
   `today`, `flagged`, those whose matches `samu-core` already computes
   per task (`computed.inbox`, `computed.today`, `flagged`). Forecast with
   appointments and the calendar stay out; appointments in a block agenda
   need a decision of their own.
   - A block never refers to a block perspective. The store rejects that
     on write; a reader that meets it anyway (over `/changes` from another
     device) skips the block.
   - A reference to a deleted or unknown perspective, and an unknown fixed
     view (ADR 0015), stay in the list; the block is empty and says so in
     its head ("missing"). So nothing is lost when an older client writes.
3. **Evaluation in `samu-core`** in one place (ADR 0006): per block the
   matches exactly as the referenced perspective or view shows them alone
   (ADR 0090 §2), with the current scope as the lens. With `dedupe` a task
   appears only in the first block that hits it; without, in each. The web
   computes in the browser from the same fields (ADR 0090 §3); shared test
   cases keep both equal.
4. **Interface:** `GET /blocks?perspective=<id>` (with `scope=`) answers
   with the sections `[{title, icon, block, missing, tasks}]`, `tasks` as
   in `/available`; the local replica (ADR 0066) computes the same. `GET
   /available?perspective=<block id>` still answers a flat list: the union
   of all blocks, without duplicates, in block order. A client that does
   not know `blocks` thus shows a sensible list instead of nothing. For a
   simple perspective `/blocks` answers with a single section.
5. **Display in the web:** per block a section with icon, heading (its
   own, otherwise the name of the perspective or view) and count,
   collapsible (per browser). Inside it the block's arrangement and
   display: list or table of its perspective; a board is shown as a list
   in a block; fixed views like their list. A click on the heading opens
   the referenced perspective or view. Keyboard, selection and editing run
   across all blocks as over one list. The block perspective's count in
   the sidebar counts the union.
6. **Creating and editing in the web:** "+" in the head of the section
   "Perspectives" asks "Perspective" or "Block perspective". The row menu
   of a block perspective has "Blocks …": a dialog with the list of
   blocks, adding from perspectives and fixed views, dragging to order,
   removing, an own heading, the switch "Show duplicates once". Inside a
   block perspective the search field stays the search over everything,
   not its definition.
7. **CLI:** `samu perspective blocks <name> <block>…` sets the blocks,
   each a perspective name or a fixed view as `:inbox`, `:today`,
   `:flagged`; without blocks it becomes a simple perspective again.
   `samu perspective dedupe <name> on|off`; `samu perspective add` takes
   `--blocks`. `samu list --perspective <block>` fetches `/blocks` and
   prints in plain a heading per block (part of the data, so on stdout),
   in json/jsonl the sections, in tsv a leading column `block`. The
   commands that concern only simple perspectives (`text`, `chips`,
   `filter`, `arrange`, `view`) reject a block perspective with the exit
   code for invalid input.
8. **Later, each with its own decision:** block perspectives in the app
   (done by ADR 0147) and in the Emacs package (until then it shows
   the union of §4);
   appointments and forecast as blocks; blocks carrying a search of their
   own, without a perspective.
9. **One filter for all blocks, for the moment.** Above a block
   perspective web and app offer "Per block", "Available", "Remaining"
   and "All". "Per block" is the default: each block its own filter
   (§1, §5). Any other choice holds for every block in place of its own,
   the fixed views included, and is lifted by a block's chips as the
   block's own filter would be (ADR 0042 §4). Nothing keeps the choice:
   it is not written to the perspective, and leaving the perspective ends
   it. `GET /blocks` takes it as `filter=`, `samu list --perspective
   <block>` as `--available`, `--remaining` or `--all`. `/available` and
   the count in the sidebar keep each block's own filter.

## Consequences

- `samu-api` 0.1.54 has `PerspectiveBlock` (`perspective` or `view`,
  `title`), `PerspectiveBlockView` with `Unknown` (ADR 0015), the fields
  `blocks` and `dedupe` on `Perspective`, and `BlockSection` for
  `/blocks`, with fixtures. catastr needs no change.
- `samu-store` migration with two columns; on write it checks that no
  block points to a block perspective and that a perspective that blocks
  point to does not get blocks itself.
- Older web clients and the app show a block perspective as the union
  list; fields of a simple perspective they write to it have no effect
  but do no harm.

## Changes

- 2026-11-24: §9 added. Until then a block perspective could be
  filtered only block by block, by editing each block's perspective or
  search; §1 and §5 still hold for what is saved.

- 2026-11-24 (ADR 0147): §8 left block perspectives in the app open;
  the app showed the union of §4.
