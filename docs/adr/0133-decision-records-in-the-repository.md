---
number: 0133
title: Decision records live in this repository, in English, as living documents
status: accepted
date: 2026-11-19
replaces: ["0001 (language and immutability of records)", "0108 §4"]
---

# 0133 – Decision records in the repository

## Context

Until now the records were written in German and kept outside the
repository, and a record was never rewritten: a changed decision got a
new record that replaced the old one in part.

Two outside reviews of the published code, on 16 and 19 November 2026,
found the same gap. The code cites its records about 2,950 times in 330
files, 119 different records, and none of them can be read. The reasons
for the design are therefore missing for a contributor, and for an agent
that is asked a year from now to turn a decision back.

Immutable records have a cost of their own: what holds today has to be
read from a chain. By record 0132 none was replaced as a whole, but many
were replaced in single sections.

## Decision

1. **One version of every record, in English, under `docs/adr/`.** There
   is no second copy elsewhere. A draft is written here as well, with
   status `draft`.
2. **Records are living documents.** A changed decision changes its
   record. Every such change adds a dated line under "Changes" that says
   what held before, so that the history can be read without git.
   Rewording that changes no decision needs no entry.
3. **Numbers and section numbers are stable.** A section that no longer
   holds keeps its number and says so; it is not renumbered away, because
   the code cites sections.
4. **A new record only for a new subject.** What changes an existing
   decision is written into its record.
5. **The existing records 0001 to 0132 are translated and tightened**,
   keeping number and sections, the 40 most cited first (they carry about
   two thirds of the citations). Sections that were replaced later are
   worked in during translation. The German originals are frozen and stay
   outside the repository as an archive.
6. **What concerns only the maintainer's own installation** (hosts,
   domains, signing keys, private tools) is not a record of this
   repository.
7. **No quotations and no names of people** in a record; the check for
   private words covers `docs/adr/`.

## Consequences

- A record can no longer be cited as proof of what was decided on a given
  day; the "Changes" section and git carry that.
- Until the translation is complete, the index says which records are
  missing.
- `AGENTS.md` and `docs/architecture.md` point here.

## Changes

None yet.
