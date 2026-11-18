// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Editing a clock (ADR 0046): what to send and what to point out. Without
// the DOM, so web/tests/clocks.mjs runs it in node. The server refuses an
// end before the start; this only keeps a step in between from tripping
// over it.

// The fields that change from `before` to `after` as [field, value], in an
// order the server takes one by one: a start moved past the old end goes
// after the new end.
export function clockEdits(before, after) {
  const same = (a, b) => (a ?? null) === (b ?? null);
  const start = same(before.start, after.start) ? null : ["start", after.start];
  const end = same(before.end, after.end) ? null : ["end", after.end ?? null];
  const startLast = before.end && Date.parse(after.start) > Date.parse(before.end);
  const edits = (startLast ? [end, start] : [start, end]).filter(Boolean);
  if ((before.note || null) !== (after.note || null)) edits.push(["note", after.note || null]);
  return edits;
}

// The other clocks `clock` overlaps, a running one counted up to `now`.
// Allowed; the inspector only says so.
export function overlapping(clock, clocks, now = Date.now()) {
  const span = (c) => [Date.parse(c.start), c.end ? Date.parse(c.end) : now];
  const [from, to] = span(clock);
  return clocks.filter((c) => {
    if (c.id === clock.id) return false;
    const [a, b] = span(c);
    return a < to && from < b;
  });
}
