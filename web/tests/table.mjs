// The table's numbers of web/lib/tabledata.js against the cases samu-core
// meets as well (ADR 0115 §4): testdata/table/worked.json. Run by
// `make check` with node, no packages.

import { readFileSync } from "node:fs";

const root = new URL("../../", import.meta.url);
const { worked, estimated, sums, estimateSeconds, hoursText, columnsOf, DEFAULT_COLUMNS, editable, stepColumn, cellWrite } = await import(new URL("web/lib/tabledata.js", root));
const doc = JSON.parse(readFileSync(new URL("testdata/table/worked.json", root), "utf8"));

let failed = 0;
const check = (what, got, want) => {
  const g = JSON.stringify(got);
  const w = JSON.stringify(want);
  if (g !== w) {
    failed += 1;
    console.error(`${what}\n  got  ${g}\n  want ${w}`);
  }
};
const plain = (map) => Object.fromEntries([...map].sort(([a], [b]) => a.localeCompare(b)));
const sorted = (object) => Object.fromEntries(Object.entries(object).sort(([a], [b]) => a.localeCompare(b)));

if (doc.cases.length < 3) {
  console.error(`only ${doc.cases.length} cases: the file is not what it should be`);
  process.exit(1);
}
let found = 0;
for (const c of doc.cases) {
  const got = worked(doc.tasks, doc.clocks, Date.parse(c.now));
  found += [...got.values()].filter((w) => w.subtree).length;
  check(`worked: ${c.about}`, plain(got), sorted(c.worked));
}
if (!found) {
  console.error("no case finds any time: the file is not what it should be");
  process.exit(1);
}
check("estimated", plain(estimated(doc.tasks)), sorted(doc.estimated));

// Sums: an action group and its child as rows side by side count once.
const [first, second, third] = doc.tasks;
const parentOf = (id) => doc.tasks.find((t) => t.id === id)?.parent ?? null;
const w = worked(doc.tasks, doc.clocks, Date.parse(doc.cases[0].now));
const e = estimated(doc.tasks);
const alone = sums([first.id], parentOf, w, e);
check("sums: a group and its children", sums([first.id, second.id, third.id], parentOf, w, e), alone);
check("sums: the group's subtree", alone, {
  estimate: e.get(first.id).subtree,
  worked: w.get(first.id).subtree,
  remaining: e.get(first.id).subtree - w.get(first.id).subtree,
});
check("sums: without an estimate", sums([doc.tasks[5].id], parentOf, w, e).remaining, null);

check("estimateSeconds", ["PT1H30M", "PT45M", "P1D", "P1DT2H", "PT90S", "", "x"].map(estimateSeconds), [5400, 2700, 86400, 93600, 90, null, null]);
check("hoursText", [3900, -1800, 0, null].map(hoursText), ["1:05", "−0:30", "0:00", ""]);
check("columnsOf", [columnsOf(null), columnsOf({ columns: ["due", "nope", "due", "prop:jira", "prop:"] })], [DEFAULT_COLUMNS, ["due", "prop:jira"]]);

// Editing in a cell (§6).
check("editable", ["state", "priority", "due", "estimate", "tags", "prop:jira", "worked", "project", "created", "blocked_by", "prop:", null].map(editable), [
  true, true, true, true, true, true, false, false, false, false, false, false,
]);
const cols = ["state", "due", "worked"];
check("stepColumn", [stepColumn(cols, null, 1), stepColumn(cols, "due", 1), stepColumn(cols, "worked", 1), stepColumn(cols, "state", -1), stepColumn(cols, null, -1), stepColumn(cols, "gone", 1)], [
  "state", "worked", "worked", null, null, "state",
]);
const parse = (t) => ({ "": null, "1h30": "PT1H30M" })[t];
check("cellWrite", [
  cellWrite("due", "2026-11-20", "", parse),
  cellWrite("due", "", "2026-11-20", parse),
  cellWrite("due", "2026-11-20", "2026-11-20", parse),
  cellWrite("planned", "morgen", "", parse),
  cellWrite("estimate", "1h30", "", parse),
  cellWrite("estimate", "", "1 h", parse),
  cellWrite("estimate", "lang", "", parse),
  cellWrite("prop:jira", " ABC-1 ", "", parse),
  cellWrite("prop:jira", "", "ABC-1", parse),
  cellWrite("worked", "1:00", "", parse),
], [
  { field: "due", value: { date: "2026-11-20" } },
  { field: "due", value: null },
  null,
  { error: "date" },
  { field: "estimate", value: "PT1H30M" },
  { field: "estimate", value: null },
  { error: "estimate" },
  { field: "properties.jira", value: "ABC-1" },
  { field: "properties.jira", value: null },
  { error: "column" },
]);

console.log(`web table: ${doc.cases.length} cases, ${failed} failed`);
process.exit(failed ? 1 : 0);
