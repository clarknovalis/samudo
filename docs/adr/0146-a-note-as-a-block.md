---
number: 0146
title: A note as a block – a block perspective's block can show the note of a project or task, rendered and editable in place
status: accepted
date: 2026-11-24
adds_to: ["0058", "0116", "0143"]
---

# 0146 – A note as a block

## Context

A block perspective (0116) is a morning in one view: the day, the short
things here, what waits. What it cannot hold is the text that belongs
before those lists: the focus of the day, the checklist of the weekly
review, the rules of a context. In org that text sits in the agenda file
above the blocks; in samu it has no place in a block perspective.

samu already has such text: every project and task has a note, rendered
as Markdown or Org (0058), with checkboxes that can be ticked in the
rendered text. A block that shows one of these notes needs no new kind of
object.

Deliberately not: importing a folder of org or Markdown files into samu.
Notes live in catastr, and for tasks org files are no source (0002); a
second copy would drift. A block that shows a note from catastr and edits
it through catastr's write path touches the interface between the two
projects (auth, rendering, conflicts) and needs a decision of its own,
with its counterpart in catastr (0003). It is taken up only when it shows
that the notes wanted are really catastr's.

## Decision

1. **A fourth form of block:** `{note: <id>}`, the id of a project or a
   task, with the optional `title` the other forms have. A block is
   exactly one of `perspective`, `view`, `query` and `note`.
2. **What it shows:** the note of that project or task, rendered (0058),
   under a heading that is its own `title`, else the title of the
   project or task, with the project's icon. It shows no tasks: it adds
   nothing to `dedupe` and nothing to the union of `/available` (0116 §4),
   and its head has no count.
   - **The scope does not apply.** The block names its object; it shows
     the note in every scope. A checklist must not vanish because the
     scope of the moment leaves out its project.
   - It is **missing** (0116 §2) when the object is deleted, unknown, not
     a project or task, or not visible to the session. Completed or
     dropped does not make it missing.
   - An empty note shows a quiet line that invites writing one.
3. **Editing in place, in the web,** as the inspector edits a note
   (0058 §6): the pencil beside the text or a double click on it
   turns it into the field, and leaving the field saves. A
   checkbox is ticked in the rendered text, as in the inspector. The
   write is the object's field `note`, as from the inspector: no new
   route. A click on the heading opens the project or task.
4. **As a line** (0143 §3) and in the CLI (0143 §6): `:note #Project`
   for a project by its name as the search field writes it, `:note
   <id>` for a project or task by its full id or the short end of it
   (as the CLI takes ids), each with ` | Heading`. A line `:note` that
   names nothing, or names more than one, is refused. A note block is
   written back as `:note #Project` or `:note <short id>`.
   The list mode of the dialog "Blocks …" shows a note block by its
   object's title and orders, retitles and removes it like any block;
   it adds one by picking a project. A task's note is added by its line.
5. **Interface:** no new route, no migration (`blocks` is JSON).
   `PerspectiveBlock` gets `note` (an id) and `BlockSection` gets `note`
   (the text, empty when there is none) in a new version of `samu-api`.
   The store refuses a note block whose id is not a live project or
   task, as it refuses a reference to a missing perspective. Evaluation
   stays in `samu-core` (0116 §3); it is given the notes it may show and
   does not read them itself. The web computes from its cache the same
   way, held equal by the shared cases.
6. **CLI:** `samu list --perspective <block perspective>` prints a note
   block in plain as its heading followed by the note's text, indented,
   as it is written (not rendered); in json and jsonl the section with
   `note`; in tsv nothing, since tsv has a row per task.
7. **Older readers and writers** as in 0143 §7: an older reader shows a
   note block as missing; an older writer of blocks drops `note`, the
   block is of no form, and the server refuses the whole write.

## Consequences

- A block perspective can open with the text it needs before its lists,
  kept where samu keeps text already.
- The app and the Emacs package still have no block perspectives of
  their own (0116 §8); a note block adds nothing to the union they show.
- A block that shows a note from catastr stays open (Context).

## Changes

None yet.
