---
number: 0135
title: One table of modules cuts both what is shown and where code lives
status: accepted
date: 2026-11-19
adds_to: ["0072", "0079"]
---

# 0135 – Modules as the one cut

## Context

Every feature is a vertical slice through about ten places: core, a
migration, the wire types, CLI, mobile facade, app, web UI, Emacs, the
documentation in two languages. Each feature has a file of its own in
most of them, and still most of its code is elsewhere. Measured for
habits on 19 November 2026: 399 lines that mention them in their own
files, 734 in 63 others, most in the Emacs package, the capture parser,
the store, the reminders and the model.

Taking a feature out is therefore not deleting five files but hand work
in sixty places. A draft about showing features step by step had already
grouped them into modules; this record makes that grouping the one cut.

## Decision

1. **A table of modules in `samu-core`** names every module, what belongs
   to it and what it needs. It is also written out under `testdata/`, so
   that web UI, app and Emacs read the same table.

   | Module | Contains | Needs |
   |---|---|---|
   | core (always on) | inbox, projects, due, completing, note, search, quick capture | |
   | `tags` | tags, tag groups | |
   | `defer` | start date, availability, deferred items | |
   | `planned` | planned date, forecast | |
   | `structure` | sequential projects, action groups, `blocked_by`, stuck | |
   | `flow` | workflows and their states, board | `tags` |
   | `priority` | priority A/B/C | |
   | `review` | review interval, review view | |
   | `perspectives` | perspectives | |
   | `scopes` | scopes, `exclusive`, shape | |
   | `properties` | properties, templates per key | |
   | `time` | clocking, report, estimate, table | |
   | `habits` | habits, daily reminder | |
   | `reminders` | reminders | |
   | `calendar` | calendars | `planned` |
   | `templates` | capture templates | |

   There should not be more modules than these.
2. **A module has a file of its own in every interface**, and shared
   files know it only through a registration: the capture parser through
   a list of prefixes per module, the server through a router per module,
   the mobile facade through a file per module. The large shared files
   are split along the modules, not before.
3. **A probe in the check** counts, per module, the mentions outside its
   own files and registrations. The count is recorded and may only fall.
4. **A module may be experimental.** Its fields in the wire types are
   optional, and it may leave without a major version of `samu-api`
   (0136 §4).
5. **Every new feature belongs to a module or to the core**; that is
   part of the definition of done.
6. **No compile-time features per module.** Every combination would be a
   build and a test state of its own.
7. **Switching modules on and off in the interface is not decided
   here.** It needs only this table and gets a record of its own.

## Consequences

- The work adds no function and touches most large files. It goes module
  by module and can stop after each.
- The probe of §3 starts from today's counts, which are high.

## Changes

None yet.
