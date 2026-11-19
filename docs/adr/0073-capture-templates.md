---
number: 0073
title: Capture templates – a quick-capture line as defaults plus reminders, invoked with /name
status: accepted
date: 2026-11-11
adds_to: ["0006", "0007", "0011", "0030", "0043"]
---

# 0073 – Capture templates

## Context

ADR 0006 names a `Template` object as a capture template with target
project, tags, fields and a title pattern; it was not built. ADR 0011
foresees that templates may carry the quick-capture syntax as defaults.

The first real case was habits: capturing one from any scope without
giving folder, tag, repetition or reminder each time, just the title.

What existed:

- **Quick capture** (ADR 0011): `#project @tag ! !A d: s: p: ~15m every:
  every!: // note`; dates with a time (`p:today 21:00`) work. `every:` is
  `fixed`, `every!:` is `due_after_completion`; `catch_up` could not be
  expressed.
- **`Scope.capture`** (ADR 0043 §6): project and tags for everything
  captured in a chosen scope; no dates, no repetition.
- **Reminders** are objects of their own (ADR 0030), anchored at `at`,
  `due`, `planned` or `defer` with optional `before`; a repetition's
  follow-up copies them.
- **org-capture** knows target, skeleton and prompts (`%^{Prompt}`, `%t`,
  `%a`).

## Decision

### 1. Object `template`

A new kind beside `scope` and `reminder`, synced over `/changes` like
every object (ADR 0004); the app has it in its mirror and captures with
it offline. Fields:

- **`title`**: the name, such as "Habit", which is also the invocation
  (§3). Unique ignoring case; letters, digits, `-` and `_`, no spaces.
- **`text`**: a line in quick-capture syntax, such as `#Habits @habit
  p:today 21:00 every+:1d`. Free text other than the note is a title
  prefix (ADR 0120); `/name` inside a template and anything the parser
  warns about are rejected. Relative dates (`today`, `fri`, `tomorrow
  9:00`) are computed at the moment of capture in the account's zone, so
  a template means "today 21:00", not a fixed date.
- **`reminders`**: a list of `{anchor, before}` without `at` (ADR 0030
  §1), or `{anchor: daily, time}` (ADR 0093 §5). They become reminders on
  the new task, for example `[{anchor: planned}]`.
- **`key`**: one character or none, for invocation by key (ADR 0084).
- **`position`**: order in lists and buttons.

Every field has its stamp; `reminders` changes as a whole. Unknown values
follow ADR 0015: an unknown anchor is read, never written.

### 2. New short form `every+:` for `catch_up`

Quick capture has `every+:` for `fixed` with `catch_up` (ADR 0007 §3,
org's `++`), such as `every+:1d`, everywhere, not only in templates. A
missed day then jumps to the next day in the future on completion.

### 3. Invocation

- **In the line:** `/name` anywhere in the capture line, such as `Smoke
  free /habit`, alike in web, app, CLI and the Emacs package, recognised
  by `samu-core`.
  - Only a name that exists as a template is recognised.
  - Otherwise `/word` stays text and capture warns `unknown_template`,
    as for an unknown project (ADR 0011 §5).
  - At most one template per line.
- **Web:** the capture window shows the templates as buttons below the
  field. A click puts `/name` into the line, so the preview shows what
  will be created; `/name` is highlighted like `#` and `@`. A key prefix
  opens capture with a template (ADR 0084 §2).
- **CLI:** `samu add --template habit "Smoke free"`, the same as `samu add
  "Smoke free /habit"`.
- **App:** like the web, buttons in capture.
- A template can also be applied to an existing task in the edit dialog
  (ADR 0078).

### 4. Precedence

**Line, then template, then `Scope.capture`.** What the line names wins;
what it leaves open the template fills; what the template leaves open the
chosen scope fills (ADR 0043 §6). Per field: project, tags, flag,
priority, defer, due, planned, estimate, repetition, note. Tags do not
mix: if the line names `@x`, only the line's tags apply. Reminders and
the title prefix (ADR 0120) always come from the template.

### 5. One computation, in `samu-core`

`samu-core::capture` applies the template: parse line, parse template,
merge by §4, reminders as part of the result.

- The server does this on `POST /capture` and creates task and reminders
  in **one** transaction.
- The app runs the same function offline and writes the result to its
  outbox.

`CaptureRequest.template` (an id) serves interfaces that pick the
template by button instead of `/name`; if the line names one as well,
the line wins. `CaptureResponse` has `reminders` (those created) and the
meaning `Meaning::Template`.

### 6. Managing

- **Web:** Settings > Templates: list, create, rename, delete, the key;
  the `text` field with the same recognition as capture; reminders as a
  small list (anchor, "before").
- **CLI:** `samu template list|add|text|remind|rename|delete`.
- The server creates no template on first start; the settings offer a
  "Habit" example while there is none.

### 7. Not now

- Prompts like `%^{Prompt}` and title patterns with placeholders.
- `%a`, a reference to what is open right now.
- Several tasks from one template (checklists, projects from templates).
- A default template per scope; `Scope.capture` covers the simple case.

## Consequences

- `samu-api`: `Template`, `TemplateReminder`, `ObjectKind::Template` and
  `AnyObject`; `CaptureRequest.template`, `CaptureResponse.reminders`,
  `Meaning::Template`, the warning `unknown_template`; fixtures. catastr
  does not read templates and skips an unknown kind (ADR 0015).
- `samu-store`: migration with the table `template`; the store checks the
  text with the parser from `samu-core` on write.
- `samu-core`: `every+:`, recognising `/name`, merging by §4, table tests
  as in ADR 0011.
- Server: `POST /capture` creates the reminders in the same transaction.

## Changes

- 2026-11-12 (ADR 0084): §1 had no `key`.
- 2026-11-13 (ADR 0093): §1's reminders had only date anchors.
- 2026-11-17 (ADR 0120): §1 rejected free text in `text`; the title came
  only from the line.
