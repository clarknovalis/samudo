// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// What web/lib/close.js finds open below tasks before closing them
// (ADR 0128), on synthetic tasks: the same counting as `samu bulk done`.
// Run by `make check` with node, no packages.

const root = new URL("../../", import.meta.url);
const { deepestFirst, openBelow, waiting } = await import(new URL("web/lib/close.js", root));

let failed = 0;
const same = (what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    failed += 1;
    console.error(`not ok  ${what}: ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
  }
};

const DONE = "2026-11-18T10:00:00Z";
// g ── a (open) ── a1 (open)
//   ├─ b (completed) ── b1 (open)
//   └─ c (dropped)
// h (open, alone); x ── x1 (completed)
const tasks = [
  { id: "g", parent: null },
  { id: "a", parent: "g" },
  { id: "a1", parent: "a" },
  { id: "b", parent: "g", completed_at: DONE },
  { id: "b1", parent: "b" },
  { id: "c", parent: "g", dropped_at: DONE },
  { id: "h", parent: null },
  { id: "x", parent: null },
  { id: "x1", parent: "x", completed_at: DONE },
];
const ids = (list) => list.map((t) => t.id).sort();

same("open below at any depth, closed ones left out", ids(openBelow(tasks, "g")), ["a", "a1", "b1"]);
same("nothing below", openBelow(tasks, "h"), []);
same("only closed below", openBelow(tasks, "x"), []);

same("one group waits with its count", waiting(tasks, ["g"]), [{ id: "g", open: 3 }]);
same("a task without open ones does not wait", waiting(tasks, ["h", "x"]), []);
same("picked ones below do not count", waiting(tasks, ["a", "a1"]), []);
same("picked ones below lower the count", waiting(tasks, ["g", "a", "a1"]), [{ id: "g", open: 1 }]);

same("deepest first, ties keep their order", deepestFirst(tasks, ["g", "h", "a1", "a"]), ["a1", "a", "g", "h"]);
const cycle = [
  { id: "p", parent: "q" },
  { id: "q", parent: "p" },
];
same("a cycle ends", deepestFirst(cycle, ["p", "q"]), ["p", "q"]);
same("a cycle ends below too", ids(openBelow(cycle, "p")), ["q"]);

if (failed) {
  console.error(`close: ${failed} failed`);
  process.exit(1);
}
console.log("close: ok");
