---
number: 0015
title: Unknown enum values are read as unknown; the HLC is an object on the wire
status: accepted
date: 2026-11-07
adds_to: ["0003", "0014"]
replaces: ["0014 (HLC in its display form)"]
---

# 0015 – Unknown enum values become `unknown`, HLC as an object on the wire

## Context

ADR 0003 §2 lets a reader skip unknown *fields*, so samu can add a field
without breaking catastr. Nothing covered unknown *values*: the
cross-check while building ADR 0014 showed that a new `LinkKind` made an
old reader fail. The choice was between a major version for every new
value and a fallback value in every enum.

ADR 0014 gave the HLC in catastr's display form `wall.counter@device`,
but catastr serialises a stamp as an object (`Hlc` in catastr's core
crate) and uses that form only for display.

## Decision

1. **Every enum on the wire has a fallback value `unknown`.** A value the
   reader does not know is read as `unknown`, not rejected. This holds for
   the date form `When` too: an unknown form (`{"week": …}`) becomes
   `{"unknown": null}`. The same goes for kinds of object
   (`ObjectKind::Unknown`).
2. **`unknown` is read, never written.** The server rejects a change that
   would set `unknown`; a client that shows an object with `unknown` does
   not write that field back. Otherwise an old client would overwrite a
   new value.
3. **A new enum value is therefore a minor version** of `samu-api`, like
   a new field. Removing or reinterpreting a value stays a major version.
4. **The HLC is an object on the wire**, in catastr's form: `{"wall":
   1762502400000, "counter": 3, "device": "pixel"}`, `wall` in
   milliseconds since the epoch. `wall.counter@device` is only the
   display form.

## Consequences

- Every interface needs an `unknown` branch for every enum (shown as
  "unknown", not editable). Kotlin over uniffi gets it automatically.
- For availability (ADR 0006) `samu-core` counts an `unknown` status like
  the most cautious known one (not available), with the reason "unknown
  value" (ADR 0017).
- A test checks for every enum in the fixtures that a foreign value is
  read as `unknown`.

## Changes

None yet.
