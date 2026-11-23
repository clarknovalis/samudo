---
number: 0143
title: Blocks with a search of their own – a block perspective's block can carry its search instead of pointing at a perspective, written line by line like org-super-agenda
status: draft
date: 2026-11-23
adds_to: ["0090", "0116", "0142"]
---

# 0143 – Blocks with a search of their own

## Context

A block of a block perspective (0116 §2) is a reference: to a perspective
or to a fixed view. For every query in a block agenda a perspective has
to be made first and then added as a block. That is twice the work, and
it leaves perspectives behind that exist only for that one block; 0142
can hide them in the sidebar, but they stay. 0116 §8 deferred "blocks
carrying a search of their own, without a perspective" to a decision of
its own; this is it.

In org-super-agenda a block agenda is written as text, one group per
line, each a query with a heading. The search language of samu (0042,
0105) already writes a whole query on one line: `@Office ~<15m`,
`#Garden is:waiting`.

## Decision

1. **A third form of block:** `{query: {...}}`, with the optional
   `title` the other two forms have. `query` holds what a plain
   perspective holds (0090 §1, 0115 §7): `text`, `chips`, `filter`,
   `arrange`, `view`, `board` and `table`, with the same types and
   defaults. It is embedded in the block perspective's `blocks` and
   changes with it, as a whole. A block is exactly one of
   `perspective`, `view` and `query`.
   - A board in a query is shown as a list, as for a referenced
     perspective (0116 §5); it is kept so that nothing is lost between
     the forms (§4).
2. **Evaluation** as for a referenced perspective (0116 §3): the matches
   are what a plain perspective with these fields would show alone, in
   the current scope, in `samu-core` and in the web from the same
   fields, held equal by the shared cases. `dedupe` treats it like any
   other block. Without a `title` its heading is the search as it reads
   in the search field, and "All" for an empty one. Its heading opens
   nothing; it has a menu instead (§4).
3. **Editing as text.** The dialog "Blocks …" (0116 §6) gets a second
   mode beside the list: a text with one line per block.
   - `:inbox`, `:today`, `:flagged` is that fixed view.
   - A line that is exactly the name of a plain perspective (ignoring
     case and surrounding space; quotes allowed) is a reference to it.
     A line that names a block perspective is refused, as 0116 §2
     refuses such a block.
   - Any other line is a search, read as the search field reads it:
     chips and free text.
   - ` | ` ends the block and starts its heading: `#Garden is:waiting |
     Waiting`. `#` cannot start a comment, since it marks a project.
   - Blank lines do not count.
   - A preview below shows for every line what it becomes (view,
     reference, search) and how many tasks it shows now. Saving with a
     refused line is not possible.
   - A search line that is unchanged from an existing query block keeps
     that block's `filter`, `arrange`, `view` and `table`; the text sets
     only the search, the list mode sets the rest per block.

   The list mode stays and edits a query block's search, filter,
   arrangement and view, as for a perspective.
4. **Between the forms:**
   - "Detach" turns a reference into a query block with a copy of the
     perspective's fields; its heading stays what it was (the
     perspective's name becomes the `title` unless one was set).
   - "Save as perspective" turns a query block into a new plain
     perspective with its fields, under a name asked for, and the block
     into a reference to it; the heading stays.
   - "Add as block to …" in the menu of the search bar puts the current
     search, with the list's filter, arrangement and view, as a query
     block at the end of a chosen block perspective, or of a new one.
5. **Interface:** no new route. `PerspectiveBlock` gets `query`
   (`BlockQuery`) in a new version of `samu-api`; `/blocks` returns the
   block in each section as before, `/available` the union (0116 §4).
   No migration: `blocks` is stored as JSON. The store refuses a block
   that is not exactly one form, and checks a query's values as it
   checks a perspective's (a blank heading, unknown table columns).
6. **CLI:** `samu perspective blocks <name> <block>…` (and `add
   --blocks`) reads each argument as one line of §3: `:today`,
   `Waiting`, `"@Office ~<15m | Short here"`. The older `Name=Heading`
   keeps working where the part before `=` is a perspective or a view;
   otherwise the whole word is a search (`prop:jira=HAUS-12` is a
   chip). `samu perspective show` prints a query block as its line.
   New: `samu perspective detach <name> <n>` and `samu perspective
   extract <name> <n> <new name>`, §4 for the n-th block, counted from
   1. Both are writes of `/changes`; there is no route of their own.
7. **Older readers and writers.** An older reader sees a block that is
   none of the forms it knows and shows it as missing (0116 §2); the
   app shows the union from `/available` as before. An older writer of
   blocks (an open web tab of an older build, an older CLI) drops
   `query` when it writes the list again; the block is then no form at
   all and the server refuses the whole write. A refusal, not a loss:
   the blocks stay as they were.

## Consequences

- No more perspectives made only to be a block. Those made so far can
  be turned into query blocks with "Detach" and then deleted.
- The app and the Emacs package still have no block perspectives of
  their own (0116 §8); they show the union.
- A query block cannot be shared between two block perspectives; a
  search wanted in several stays a perspective.

## Changes

None yet.
