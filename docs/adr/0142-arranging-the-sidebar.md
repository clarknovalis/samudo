---
number: 0142
title: Arranging the sidebar in the web UI – fixed views and perspectives in one order, each can be hidden, kept per browser
status: accepted
date: 2026-11-23
adds_to: ["0020", "0055", "0090", "0102", "0116"]
---

# 0142 – Arranging the sidebar in the web UI

## Context

The sidebar of the web UI has three parts: the fixed views (Inbox, All
projects, Today, Forecast, Calendar, Flagged, Stuck, Review, Report) in a
fixed order, the perspectives in their own order (0090), and the tree of
folders and projects. Nothing can be hidden, and a perspective cannot
stand among the fixed views.

Block perspectives (0116) make a customised "Today" possible without a
new mechanism: a block perspective of `:today`, `:flagged` and a few
perspectives. What is missing is that it can take the place of "Today"
in the sidebar, and that "Today" itself can go. The perspectives such a
block perspective is made of are often needed only as blocks; in the
sidebar they are noise.

The app has had an arranged sidebar since 0102 §3: a list of entries
with a handle to drag and a switch to hide, per device.

## Decision

1. **The upper part of the sidebar is one ordered list of entries.**
   An entry is a fixed view or a perspective, plain or of blocks. By
   default it holds the fixed views in today's order; whoever arranges
   nothing sees no difference. Below it, the section "Perspectives"
   shows every perspective that is neither in the upper list nor
   hidden, in its own order as before. The tree of folders and projects
   stays as it is.
2. **Every entry can be hidden**, except Inbox, which can only be
   moved. A hidden entry stands nowhere in the sidebar, neither wide nor
   narrow (0055), but it stays what it is: a hidden perspective works as
   a block of a block perspective (0116), keeps its count and its
   definition, and opens from the palette like before; a
   hidden fixed view opens by its key and from the palette.
3. **Where it is arranged:**
   - In the row menu of a perspective: "Move to the views" (into the
     upper list, at its end), "Back to the perspectives", and "Hide in
     the sidebar". A fixed view gets a row menu with "Hide in the
     sidebar". Dragging moves entries within the upper list and from the
     section "Perspectives" into it.
   - In the settings, a page "Sidebar", as in the app (0102 §3): the
     entries of the upper list with a handle and a switch, below them
     the perspectives with a switch, and "Show all again".
4. **Kept per browser**, in `localStorage` under `samu.sidebarEntries`,
   like the sidebar's step (0055) and the row fields (0131): the keys of
   the upper list in order (`view:<kind>`, `perspective:<id>`) and the
   hidden keys. No change to `samu-api`, no migration, nothing in
   `samu-core`: the sidebar computes nothing new.
   - A deleted perspective drops out silently. A fixed view this browser
     has not stored yet (a newer samu) goes in at its default place. An
     unknown key is dropped.
   - Each browser is arranged on its own. A synced arrangement is a
     decision of its own; it would also move the app's arrangement
     (0102 §3), so that there are not two ways to keep it.
5. **The CLI and samu.el** have no sidebar; nothing changes there. The
   app keeps its own arrangement (0102 §3).

## Consequences

- A customised "Today" is a block perspective moved to the place of
  "Today", with "Today" hidden; no option on the fixed views is needed,
  and there is still one block mechanism (0116).
- A view that is open when it is hidden stays open; the sidebar marks
  nothing as current then.
- Forecast as a block, with its days and appointments, stays deferred
  (0116 §8). Its tasks can already be had as a perspective whose
  chips ask for overdue, due in the next seven days or planned in them,
  arranged by date; what it lacks are the appointments and a task
  standing on both its planned and its due day.

## Changes

None yet.
