// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The arranged sidebar (ADR 0142) in web/lib/arrange.js: what this browser
// kept, made to count, and the moves on it. Run by `make check` with node,
// no packages; synthetic data only.

const root = new URL("../../", import.meta.url);
const { arrangementOf, isHidden, lowerPerspectives, moveBefore, setHidden, showAll, toPerspectives, VIEWS } = await import(
  new URL("web/lib/arrange.js", root)
);

let failed = 0;
let checks = 0;
const check = (what, got, want) => {
  checks += 1;
  const g = JSON.stringify(got);
  const w = JSON.stringify(want);
  if (g !== w) {
    failed += 1;
    console.error(`${what}\n  got  ${g}\n  want ${w}`);
  }
};

const kinds = ["inbox", "today", "forecast", "flagged"];
const ids = ["p1", "p2", "p3"];
const views = kinds.map((k) => `view:${k}`);

// Nothing kept: the fixed views in their order, nothing hidden (§1).
check("nothing kept", arrangementOf(null, kinds, ids), { order: views, hidden: [] });
check("broken JSON", arrangementOf("{", kinds, ids), { order: views, hidden: [] });
check("no order", arrangementOf('{"hidden":["view:today","x"]}', kinds, ids), { order: views, hidden: ["view:today"] });

// Kept: unknown keys and deleted perspectives drop out, each key once.
check(
  "cleaned",
  arrangementOf(
    JSON.stringify({
      order: ["view:inbox", "perspective:p2", "perspective:gone", "view:old", "view:today", "view:forecast", "view:flagged", "view:inbox", 7],
      hidden: ["view:today", "perspective:gone", "perspective:p3"],
    }),
    kinds,
    ids,
  ),
  { order: ["view:inbox", "perspective:p2", "view:today", "view:forecast", "view:flagged"], hidden: ["view:today", "perspective:p3"] },
);

// A view this browser has not stored yet goes in behind the one that
// precedes it by default; the first one at the start (§4).
check(
  "new view behind its predecessor",
  arrangementOf(JSON.stringify({ order: ["view:flagged", "perspective:p1", "view:today", "view:inbox"], hidden: [] }), kinds, ids).order,
  ["view:flagged", "perspective:p1", "view:today", "view:forecast", "view:inbox"],
);
check(
  "new first view at the start",
  arrangementOf(JSON.stringify({ order: ["view:today", "view:forecast", "view:flagged"], hidden: [] }), kinds, ids).order,
  ["view:inbox", "view:today", "view:forecast", "view:flagged"],
);

// Inbox can only be moved (§2).
check("inbox never hidden when kept", arrangementOf('{"order":[],"hidden":["view:inbox"]}', kinds, ids).hidden, []);
const start = arrangementOf(null, kinds, ids);
check("inbox cannot be hidden", setHidden(start, "view:inbox", true).hidden, []);

// Moving: before another entry, to the end, a perspective into the list
// and back (§3).
check("before", moveBefore(start, "view:flagged", "view:today").order, ["view:inbox", "view:flagged", "view:today", "view:forecast"]);
check("to the end", moveBefore(start, "view:inbox", null).order, ["view:today", "view:forecast", "view:flagged", "view:inbox"]);
const today = moveBefore(start, "perspective:p2", "view:today");
check("perspective in", today.order, ["view:inbox", "perspective:p2", "view:today", "view:forecast", "view:flagged"]);
check("lower without it", lowerPerspectives(today, ids), ["p1", "p3"]);
check("back", toPerspectives(today, "perspective:p2").order, views);
check("a view does not leave", toPerspectives(start, "view:today").order, views);

// A customised Today (ADR 0142, Consequences): the perspective in, Today hidden.
const custom = setHidden(today, "view:today", true);
check("today hidden", isHidden(custom, "view:today"), true);
check("hidden perspective not below", lowerPerspectives(setHidden(custom, "perspective:p3", true), ids), ["p1"]);
check("show all", showAll(setHidden(custom, "perspective:p3", true)).hidden, []);
check("show again", setHidden(custom, "view:today", false).hidden, []);

// What is saved reads back the same.
check("round trip", arrangementOf(JSON.stringify(custom), kinds, ids), custom);

// Every fixed view of the page, Inbox first.
check("views", VIEWS[0].kind, "inbox");

if (failed) {
  console.error(`arrange: ${failed} of ${checks} checks failed`);
  process.exit(1);
}
console.log(`arrange: ${checks} checks passed`);
