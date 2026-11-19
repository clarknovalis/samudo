---
number: 0054
title: Properties in the interface – frontmatter, link templates per key, opening, inheritance, search chip
status: accepted
date: 2026-11-09
adds_to: ["0006", "0023", "0042", "0043"]
---

# 0054 – Properties in the interface

## Context

The wish: properties as in org-mode, to attach catastr or Jira links,
ticket numbers and other free fields to tasks; unobtrusive like a
frontmatter, foldable; a click on a value should, on the desktop, run a
freely chosen script (or `xdg-open`) with the value.

The model exists since ADR 0023: `properties` on task and project, LWW
per key, `samu set <id> properties.<key>`. The web showed them read-only
at the bottom of the inspector, the app not at all, and a click did
nothing. ADR 0006 had inheritance on its list.

A browser may not start local programs. It may open an address with a
foreign scheme, though; the operating system hands it to the program
registered for `x-scheme-handler/<scheme>`, and the browser asks first.

## Decision

1. **Links and properties stay separate.** A link (ADR 0006) is a
   reference with meaning (identity by UUID, `blocked_by`, back-references
   from catastr). A property is a free field. If its value is a URI
   (starts with `<scheme>:` per RFC 3986, such as `https:`, `catastr:`,
   `samu:`, `mailto:`), it is still clickable.
2. **Shown as frontmatter.** In web and app a foldable block stands right
   below the title: folded, the count and the first keys; open, one line
   `key  value` per property. The fold state is per device, not on the
   server. Values are edited in the block, `×` removes a property, "+
   Property" suggests the keys already used in the account. System keys
   (`org_id`, `org_origin`, `source`; ADR 0023 §5, ADR 0051) stand faint
   at the end. The same in the project inspector. In the table view a
   property can be a column (ADR 0115).
3. **A link template per key in the account.** `AccountSettings` carries
   `property_links`, a map key → template with `{value}`, such as `jira`
   → `https://jira.example/browse/{value}`. `{value}` is inserted
   URL-encoded. Written per key with `set <settings>
   property_links.<key>`, `null` removes; LWW per key as ADR 0023 §2. The
   account database keeps them in the table `property` with the settings
   object as `owner`.
4. **Where a value leads** (`samu-core`, one place): a URI value leads to
   itself; otherwise to its key's template; otherwise nowhere, and it is
   no link. For `catastr:` ADR 0006 "opening links" still holds.
5. **A command per key, only on the device.** The CLI reads from its
   configuration file

   ```toml
   [open]
   jira = ["my-script", "{value}"]
   ```

   a command as argv, without a shell; `{value}` is inserted unchanged.
   `samu open <id> <key>` runs the command, otherwise `xdg-open` on the
   target of §4. The server knows no commands: a server that hands
   commands to clients would be a back door.
6. **The bridge from the web: scheme `samu-open:`.** The address is
   `samu-open:<uuid of task or project>/<key>` and **never carries the
   value**. The handler `samu open-uri` fetches the value itself from the
   server (own or inherited) and proceeds as in §5. A foreign page can
   thus only open what really is in samu. `make install` registers the
   handler as a `.desktop` file. In the web a switch per browser ("Use
   this computer's commands") turns clicks to `samu-open:`; off, the
   target of §4 applies. Android knows no local commands.
7. **Inheritance.** A task inherits the properties of its action groups
   and its project; the nearest wins, an own one overrides. System keys do
   not inherit. Computed in `samu-core`, never stored; `TaskStatus`
   carries `inherited`: key → `{ value, from }` for every inherited key
   the task does not have itself. Editing an inherited value creates an
   own property. Folders have none.
8. **Search chip `prop:`** (ADR 0042 §2): `prop:jira` (has it, own or
   inherited), `prop:jira=ABC-1` (equal, ignoring case), negated with
   `-`. In scopes as `ScopeChipKind::Property` with the value `jira` or
   `jira=ABC-1` (ADR 0043 §2).
9. **No types per key.** Every value is text, as in org-mode.

## Consequences

- `samu-api` 0.1.20: `AccountSettings.property_links`,
  `TaskStatus.inherited`, `ScopeChipKind::Property`, all additive, with
  fixtures. catastr reads none of it; an older reader sees `property` as
  an unknown chip and lets every task through (ADR 0015).
- CLI: `samu prop <id>`, `samu settings link …`, `samu open <id> <key>`,
  `samu open-uri <uri>`.
- The JavaScript search and `link_for` in the web rebuild the rules of
  `samu-core`, with tests on both sides, as for the search (ADR 0042).

## Changes

None yet.
