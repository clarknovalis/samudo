---
number: 0148
title: Properties in quick capture – `prop:key=value` in the line and in a template's text
status: accepted
date: 2026-11-24
adds_to: ["0011", "0054", "0073", "0078"]
---

# 0148 – Properties in quick capture

## Context

Properties are free fields on tasks and projects (ADR 0023, ADR 0054):
a ticket number, a link to catastr or Jira. The search has the chip
`prop:jira` and `prop:jira=ABC-1` (ADR 0054 §8), and the search page
says `prop:` reads the same in search and capture. Quick capture
(ADR 0011) could not set one, though: a task like `ABC-345` that should
carry its ticket number as a property had to be captured first and edited
after. A template (ADR 0073) could not carry one either.

## Decision

### 1. The mark

`prop:key=value` in a capture line sets the property `key` to `value`.
Several are allowed, one per key. The key follows the rules of the
search chip (`properties::is_key`: a lowercase letter, then lowercase
letters, digits, `_` and `-`, at most 64); the system keys (`org_id`, `org_origin`, `source`; ADR 0023 §5,
ADR 0051) are not set by capture. The value runs to the next white
space; `prop:key="two words"` holds spaces, as `#"New flat"` does. The
value is taken as typed, so a URI stays clickable (ADR 0054 §1).

`prop:` is the same in German and English, like in the search.

### 2. What does not read

`prop:key` without `=`, an empty value, an invalid key or a system key
stays in the title with the warning `unknown_property`. A second
`prop:` for the same key stays in the title with `twice`.

### 3. Templates

A template's `text` may carry `prop:key=value`, a fixed value. The line
and the template are merged per key (adds to ADR 0073 §4): what the line
names wins, the template fills in the other keys. Unlike tags, properties
from line and template mix. `Scope.capture` has no properties.

### 4. On an existing task

A line applied to an existing task (ADR 0078 §2) sets each named
property, overwriting the task's value. Removing a property stays with
the edit dialog and `samu set <id> properties.<key> null`.

### 5. Not now

- Deriving a property from the title or the note by a pattern, such as
  a ticket number into a URL (ADR 0073 §7, "title patterns"). It needs
  a source, a pattern and a value template per rule and is decided on
  its own.

## Consequences

- `samu-api` 0.1.67: `CaptureMeaning::Property` and
  `CaptureWarning::UnknownProperty`, additive, with fixtures. An older
  reader sees an unknown meaning or warning (ADR 0015).
- `samu-core::capture` reads the mark, merges templates per key and
  writes the properties on the new task or as `properties.<key>` on an
  existing one; the app runs the same code offline.
- Web, app, CLI and the Emacs package word the new warning; the capture
  preview shows the properties.

## Changes

None yet.
