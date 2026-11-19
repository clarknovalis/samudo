---
number: 0043
title: Scopes, second round – inbox as a checkbox, filter in the search language, capture defaults, schedule, choice over Tasker
status: accepted
date: 2026-11-08
adds_to: ["0011", "0035", "0042"]
replaces: ["0035 §3", "0035 §9 item 1"]
---

# 0043 – Scopes, second round

## Context

A scope (ADR 0035) has folders, projects and tags as members, reads them
as "only these" or "all except these", and never hides the inbox. Three
wishes went further:

- the inbox as a member like a folder, checked or not;
- filters by flag, by date spans relative to today (due, planned …), by
  "has been worked on";
- new tasks that take something from the scope, and switching scopes by
  time of day and by Android's focus modes (ADR 0035 §9).

Chosen: filters in the search language of ADR 0042, evaluated in
`samu-core`; members and filter apply together (AND); "worked on" means
"has clock entries"; capture defaults, schedule and Android modes in the
same round, the modes through Tasker.

## Decision

1. **Inbox as the field `inbox`** on the scope, a boolean, LWW like
   `title`. Members stay ids; a pseudo-member "Inbox" would need a
   special value on the wire. With `only` the views show the inbox only
   when `inbox` is set; with `except` they then leave it out. Orphans
   still count as inbox (ADR 0036 §6). The migration set `inbox` on every
   scope with `only`, so existing scopes stayed as they were. Tags of a
   task in the inbox do not count for membership, with one exception: a
   `scope_only` tag (ADR 0069 §5, ADR 0094). **Replaces ADR 0035 §3.**
2. **Filter** as the field `filter`: a list of chips `{kind, value, not}`
   in the language of ADR 0042 §2, without free text. On the wire,
   project, tag, folder and state are **ids**, not names, so renaming
   does not break a filter; every other kind is the word (`7t`, `woche`,
   `markiert`, `<15m`). It is the form the web UI already builds for its
   chips. The field changes as a whole (LWW). A chip of unknown kind
   matches every task, so filters nothing (like `ScopeMode::unknown`, ADR
   0015). Chips may carry a `group` for OR groups (ADR 0121 §2).
3. **In the scope** is a task that the member rule (ADR 0035 §2 with the
   mode, inbox per §1; a scope without members and without the inbox
   checkbox restricts nothing, ADR 0044) **and** the filter let in. Inside
   a filter as in the search: chips of one kind OR, different kinds AND,
   a negated chip must not hold; several groups OR (ADR 0121 §1). The
   evaluation lives in `samu-core` in one place (ADR 0006); server and app
   ask it. Relative spans (`heute`, `7t`, `woche` to Sunday) are computed
   in the account's zone. For a project or folder only the member rule
   applies; the filter acts on tasks. The hint "due outside" (ADR 0035 §6)
   also counts what the filter leaves out.
4. **The web search stays in the browser** (ADR 0042 §1). The evaluation
   in `samu-core` serves scopes, perspectives (ADR 0090) and reports (ADR
   0080); shared test cases keep `web/lib/query.js` and `samu-core` equal.
5. **Clocking as the chip `ist:geclockt`** (English `is:clocked`): matches a task
   that has clock entries. Reserved here, built with clocking (ADR 0046
   §6).
6. **Capture inherits**, explicitly: field `capture` with a project and
   tags. With a scope chosen, a new task gets its project when the line
   names no `#project`, and its tags when it names no `@tag`. Nothing is
   guessed from the members. `POST /capture` takes `scope`; the app
   applies the same function from `samu-core::capture` offline. Capture
   templates build on this (ADR 0073). **Replaces ADR 0035 §9 item 1.**
7. **Schedule** as the field `schedule`: windows of weekdays and a time
   from–to in the account's zone (`mo–fr 08:00–17:00`); a window across
   midnight ends the next day. The windows are part of the definition and
   the same on every device. **Whether a device follows them is the
   device's choice** ("automatic", ADR 0035 §4 stays). If it follows,
   **the latest event wins**: a manual choice, the start or end of a
   window, a mode switched on or off (§8). In several windows at once the
   scope with the smaller `position` wins; in none, "no scope". The
   computation lives in `samu-core`.
8. **Android modes over Tasker.** Android gives other apps no focus filter
   like iOS; an app sees only its own modes. So **Tasker** chooses the
   scope, as it already files into the inbox (ADR 0032): a broadcast with
   the explicit package `de.samudo.app`, action
   `de.samudo.app.action.SCOPE`, extra `scope` with the title or id of a
   scope; empty means "no scope", `auto` means "follow the schedule". An
   unknown scope changes nothing and shows a message. A Tasker profile
   can react to Do Not Disturb, system modes, Wi-Fi, location or time.
   The broadcast is an event under §7. Device only, no change to
   `samu-api`. The receiver is open like the one of ADR 0032; another app
   can only switch this phone's view with it, not read or change
   anything.
9. **Not yet:** full rules tied to a perspective (ADR 0035 §9 item 3).

## Consequences

- `samu-api` has on `Scope` the fields `inbox`, `filter` (`ScopeChip`),
  `capture` (`ScopeCapture`) and `schedule` (`ScopeWindow`), and on the
  `/capture` request the field `scope`, with fixtures (ADR 0014). An older
  reader skips the new fields; its views then show the inbox and do not
  filter: visibly more, never invisibly less. catastr needs no change.
- `samu-store` migration with the columns and the checkbox for every
  scope with `only`.
- CLI: `samu scope inbox`, `samu scope filter`, `samu scope capture`,
  `samu scope schedule`, and `auto` as a choice; `samu add` sends the
  chosen scope.
- Settings in web and app per scope: inbox checkbox above the tree, a
  filter field with chips and completion like the search, project and
  tags for new tasks, windows; the scope menu in the header gets the
  "automatic" switch. The app has the Tasker receiver, documented like
  the ring (ADR 0032).
- A more powerful scope is an easier trap: the header (ADR 0035 §7) shows
  when the schedule has chosen.

## Changes

- 2026-11-09 (ADR 0044): §3 did not say what a scope without members and
  without the inbox checkbox does; an `only` scope then let nothing in.
- 2026-11-09 (ADR 0046 §6): §5 was a reservation; no client offered the
  chip until clocking existed.
- 2026-11-11 (ADR 0069, renamed in ADR 0070, sections 3–4 in ADR 0094):
  §1 said without exception that tags of an inbox task do not count.
- 2026-11-15 (ADR 0110): §8 used an earlier package name, also in
  the action.
- 2026-11-17 (ADR 0121): §2 and §3 had a single group of chips; there
  was no OR across kinds.
- §4 said the search would follow once perspectives existed; perspectives
  (ADR 0090) use the same evaluation in `samu-core`.
