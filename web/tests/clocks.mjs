// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Editing a clock (web/lib/clockedit.js). Run by `make check` with node,
// no packages.

const root = new URL("../../", import.meta.url);
const { clockEdits, overlapping } = await import(new URL("web/lib/clockedit.js", root));

let failed = 0;
const check = (what, got, want) => {
  const g = JSON.stringify(got);
  const w = JSON.stringify(want);
  if (g !== w) {
    failed += 1;
    console.error(`${what}\n  got  ${g}\n  want ${w}`);
  }
};

const clock = { id: "a", start: "2026-11-09T08:00:00Z", end: "2026-11-09T09:00:00Z" };
check(
  "both ends past the old end: the end first",
  clockEdits(clock, { ...clock, start: "2026-11-09T10:00:00Z", end: "2026-11-09T11:00:00Z", note: "Belege" }),
  [
    ["end", "2026-11-09T11:00:00Z"],
    ["start", "2026-11-09T10:00:00Z"],
    ["note", "Belege"],
  ],
);
check(
  "both earlier: the start first",
  clockEdits(clock, { ...clock, start: "2026-11-09T07:00:00Z", end: "2026-11-09T07:30:00Z" }),
  [
    ["start", "2026-11-09T07:00:00Z"],
    ["end", "2026-11-09T07:30:00Z"],
  ],
);
check("nothing changed", clockEdits(clock, { ...clock, note: "" }), []);
check("running again", clockEdits(clock, { ...clock, end: null }), [["end", null]]);

const others = [
  { id: "b", start: "2026-11-09T08:30:00Z", end: "2026-11-09T08:45:00Z" },
  { id: "c", start: "2026-11-09T09:00:00Z", end: "2026-11-09T10:00:00Z" },
  { id: "d", start: "2026-11-09T07:00:00Z" },
];
check(
  "overlaps: inside and running, not touching",
  overlapping(clock, [clock, ...others], Date.parse("2026-11-09T12:00:00Z")).map((c) => c.id),
  ["b", "d"],
);

if (failed) process.exit(1);
console.log("clocks: ok");
