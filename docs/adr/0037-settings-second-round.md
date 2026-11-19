---
number: 0037
title: Settings, second round – moving, project properties, navigation with inspector
status: accepted
date: 2026-11-08
adds_to: ["0035", "0036"]
---

# 0037 – Settings, second round

## Context

After ADR 0036 was in use:

- A newly created, empty folder did not appear in the web UI's sidebar,
  though ADR 0035 §4 and `samu_core::scope::Lens::folder` foresee it: the
  web UI computed on its own and hid every folder without a project.
- Projects could not be moved into a folder; ADR 0036 §9 deferred that to
  this record.
- A miscellany in a folder is, as in OmniFocus, a project of the kind
  "single actions", but web and app could not choose the kind.
- The settings page worked but was hard to survey; that one can rename
  was not visible.

Chosen: navigation, tree and inspector as in the main view. A project's
inspector shows name, kind, place, status, workflow and review interval.
The app gets screens instead of dialogs.

## Decision

1. **Moving is an ordinary change** over `/changes`, in the app offline
   through the outbox too:
   - a project into a folder (`folder`, or to the top without);
   - a folder into another (`parent`);
   - a tag below another (`parent`).

   The object lines up after the last of its new siblings (`position`).
   Tasks directly in a folder still do not exist.
2. **No cycles.** The store refuses a `parent` that would put a folder
   below itself or one of its descendants; the same for tags. The rule
   thus holds for every client; the interfaces do not even offer such
   targets. A cycle that still arises in a race between two devices stays
   broken data as before (`Reason::Cycle`, ADR 0017 §9).
3. **Project properties** in the inspector, each an ordinary change:
   - **Kind:** parallel, sequential, single actions.
   - **Status:** active, on hold, completed, dropped. "Completed" sets
     `status` and `completed_at`, "dropped" sets `status` and
     `dropped_at`, like `samu done` and `samu drop`; back to active or on
     hold clears both instants. The logbook records it as for tasks (ADR
     0024 §5).
   - **Workflow:** an exclusive tag group or none, in which case the
     default applies (ADR 0024 §3).
   - **Review interval:** none, 1 week, 2 weeks, 1 month or 3 months
     (`review_interval`); the review itself is ADR 0088.

   Completed and dropped projects show in the tree only when a switch
   reveals them.
4. **Visible folders are computed by `samu-core`.** Route `GET
   /available/folders[?scope=]`: the folders the views show; without a
   scope all of them, with a scope those `Lens::folder` shows. The sidebar
   takes this list and the projects in it from `/available/projects`. An
   empty folder appears unless the scope hides it. The CLI command is
   `samu folder list --scope`.
5. **Shape of the web UI's settings:**
   - On the left a navigation: scopes, projects and folders, tags.
   - In the middle the tree. Clicking a row selects it; above it "+
     Project" and "+ Folder", or "+ Tag". New objects are created at the
     selected place.
   - On the right the inspector of the selected object: name as a labelled
     field, place as a choice; for projects also the fields of §3; delete
     with the preview of ADR 0036.
   - Scopes get the same shape.
   - Below 760 px width everything stacks: navigation as tabs, below it the
     tree, below that the inspector.

   Creating, renaming and moving projects and folders also work directly
   in the sidebar (ADR 0041).
6. **App:** the settings are a list of sections. Each section is a screen
   with its tree, and a tap opens the object's screen with the same fields
   as the inspector. The dialogs of ADR 0036 are gone. Deleting stays
   online (ADR 0036 §7). The projects tab itself is a tree as well (ADR
   0085).
7. **Not for now:**
   - moving by dragging (since ADR 0041 in the web's sidebar);
   - projects as target or subject of the refile palette;
   - tags and folders with a status of their own in the inspector (tags are
     put on hold through the CLI).

## Consequences

- `samu-api` 0.1.12 has the route in `ROUTES`. It answers with a list of
  `Folder`, so no new type. catastr needs no change.
- `samu-store` checks cycles in `parent` of folders and tags. No schema
  change.
- CLI: `samu project move|set`, `samu folder move`, and `--scope` on
  `samu folder list`.
- `samu-mobile` has moving, project properties and an object's details;
  the app has new screens.

## Changes

- 2026-11-08 (ADR 0041): §7 excluded dragging and left the sidebar
  without creating and moving.
- 2026-11-12 (ADR 0085, 0088): §3 deferred the review; the app's project
  tab was a list.
