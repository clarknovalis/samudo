---
number: 0105
title: Interface language – English beside German in web and app, chosen per device, a bilingual search language
status: accepted
date: 2026-11-15
adds_to: ["0026", "0042", "0079", "0103"]
---

# 0105 – Interface language

## Context

The wish: English for search, shortcuts, dialogs and the rest, and,
besides following the system, a switch in the settings to choose the
language, in app and web.

The interface was German throughout: about 960 texts in the web
(`web/index.html`, `web/app.js`, `web/lib/*.js`), about 430 in the app,
date formats fixed to `de-DE` or `Locale.GERMAN`. The search language (ADR
0042) knew only German words (`fällig:`, `ist:markiert`, `heute`, `7t`),
in `samu_core::query` and `web/lib/query.js`. Quick capture already
understood English (`today`, `monday`, `3 days`). The CLI, its messages
and the server's errors were English.

## Decision

### 1. German is the key

The German text stays in the code, wrapped in `t("…")` (web) or `tr("…")`
(app). One table per interface maps it to English (`web/lib/en.js`, the
app's `En.kt`). Placeholders are `{name}` and are replaced after lookup:
`t("{n} Aufgaben", { n })`. A missing entry shows German.

No invented ids and no `strings.xml` for the app: the code stays as
readable as before, and a third language is one more table. Plurals
through separate keys (`"1 Aufgabe"`, `"{n} Aufgaben"`), no plural
library.

### 2. Chosen per device: system, German, English

Like the theme (ADR 0026 §2), the language holds per browser or phone and
is not synced; nothing in `samu-api`, no migration.

- **Web:** `localStorage` `samu.lang` (`system` | `de` | `en`), in the
  account menu as a second row below system/light/dark. "System" follows
  `navigator.language` (`de…` → German, otherwise English). `<html lang>`
  is set too; `Intl` formats take the chosen language. A change redraws
  the page.
- **App:** a settings page "Language" with the same three choices, in the
  phone's cache. "System" follows the context's locale, so Android's own
  per-app language works too (Android 13+, `res/xml/locales_config.xml`
  with `de` and `en`). The language is Compose state; a change redraws at
  once. Widgets and notifications take it on their next drawing.

### 3. The search language always understands both

English words are aliases of the German ones, whatever the interface
language:

| German | English |
|---|---|
| `fällig:` `geplant:` `start:` `erledigt:` `erstellt:` `geändert:` | `due:` `planned:` `defer:` `done:` `created:` `modified:` |
| `ordner:` `zustand:` `ist:` | `folder:` `state:` `is:` |
| `heute` `morgen` `gestern` `woche` `überfällig` `mit` `ohne` | `today` `tomorrow` `yesterday` `week` `overdue` `any` `none` |
| `7t` `-30t` | `7d` `-30d` |
| `ist:markiert` `verfügbar` `wartet` `offen` `erledigt` `verworfen` `gruppe` `notiz` `wiederholt` `geclockt` `blocker` `blockiert` `versteckt` `termin` | `is:flagged` `available` `waiting` `open` `done` `dropped` `group` `note` `repeating` `clocked` `blocker` `blocked` `hidden` `appointment` |

`prop:`, `prio:`, `#`, `@`, `~` are the same in both; the group separator
is `oder` or `or` (ADR 0121 §3), the author chip `autor:` or `author:`
(ADR 0080 §2).

**What is stored is always the German form.** A chip in a scope or a
perspective keeps `heute`, `markiert`, `7t`: that is its wire form, so old
perspectives and catastr stay valid. English is mapped to it on reading
and made from it on display.

- `samu_core::query` has `Lang { De, En }`. `parse` takes both languages;
  `words(chips, snapshot, lang)` writes in the chosen one;
  `OfferOptions.lang` gives the kinds, values and hints of the suggestions
  in it (inserted text and label), while the chips stay German.
- `web/lib/query.js` does the same; the shared test cases
  `testdata/query/offers.json` include English cases (ADR 0103 §2).
- The CLI writes chips in German and accepts both.

### 4. Shortcuts

The keys stay the same, also where they were thought in German. Titles,
places and hints of the table in `web/lib/keymap.js` (ADR 0079) are
translated; help, menu and settings show them in the chosen language.
Emacs stays as it is.

### 5. A check in the build

A `node` script (`web/tests/i18n.mjs`) collects every key from `t("…")` in
`web/` and `tr("…")` in `android/`, and reports keys without an English
entry and entries without a key. It also fails when it finds no key at
all. A key must be a string literal; a composed text is built with
placeholders, not with `t(a + b)`.

### Not here

- Error messages of the server and of `samu-mobile` stay English.
- Names the user gives (the inbox, a workflow's states) are not
  translated; the fixed "Inbox" is called so in both languages.
- The documentation has its own bilingual setup (`docs/de/`).

## Consequences

- Every new text in web or app is a key in German with an English entry;
  §5 enforces it.
- Search chips stay German on the wire; readers in either language agree.

## Changes

- 2026-11-17 (ADR 0121): the separator `oder`/`or` joined the language.
- 2026-11-18 (ADR 0128 §7): `versteckt`/`hidden` joined the `is:` words.
