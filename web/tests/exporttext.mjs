// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The forms tasks leave the web UI in (web/lib/exporttext.js, ADR 0145):
// IDs, the Markdown list, TSV, JSON and the file name. Synthetic tasks
// only. Run by `make check` with node, no packages.

// The stamps of Org are written in the zone of the runtime.
process.env.TZ = "UTC";

const root = new URL("../../", import.meta.url);
const { dateText, fileName, ids, json, org, orgActive, orgTag, outline, render, taskRows, text, tsv } = await import(new URL("web/lib/exporttext.js", root));

let failed = 0;
let checks = 0;
const same = (what, got, want) => {
  checks += 1;
  if (got !== want) {
    failed += 1;
    console.error(`${what}\n  got  ${JSON.stringify(got)}\n  want ${JSON.stringify(want)}`);
  }
};

const A = "01900000-0000-7000-8000-00000000000a";
const B = "01900000-0000-7000-8000-00000000000b";
const C = "01900000-0000-7000-8000-00000000000c";
const D = "01900000-0000-7000-8000-00000000000d";

const rows = [
  { id: A, parent: null, title: "Plan the trip", closed: false, cells: { due: "2026-03-02", priority: "A" }, object: { id: A, title: "Plan the trip" } },
  { id: B, parent: A, title: "Book a train", closed: true, cells: { due: "2026-03-01 14:30" }, object: { id: B, title: "Book a train" } },
  { id: C, parent: B, title: "Seat\twith\ta table", closed: false, cells: {}, object: { id: C } },
  // Its parent is not copied: it stands at the left.
  { id: D, parent: "01900000-0000-7000-8000-0000000000ff", title: "Pack, \"light\"\nreally", closed: false, cells: { tags: "@home @errand" }, object: { id: D } },
];

// --- IDs ---
same("ids are the full UUIDs separated by a space", ids(rows), `${A} ${B} ${C} ${D}`);
same("one id alone", ids(rows.slice(0, 1)), A);
same("no rows, no text", ids([]), "");

// --- Markdown ---
same(
  "text indents a child below its copied parent, [x] when closed, one line per task",
  text(rows),
  ["- [ ] Plan the trip", "  - [x] Book a train", "    - [ ] Seat with a table", "- [ ] Pack, \"light\" really"].join("\n"),
);
same("a child copied alone stands at the left", text([rows[1]]), "- [x] Book a train");

// --- The cases shared with the CLI (testdata/export, ADR 0145 §8) ---
const { readFileSync } = await import("node:fs");
const shared = (name) => readFileSync(new URL(`testdata/export/${name}`, root), "utf8");
const cases = JSON.parse(shared("tasks.json"));
same("the shared cases are read in UTC", cases.zone, "UTC");
const titles = new Map(cases.tags.map((t) => [t.id, t.title]));
const sharedRows = taskRows(cases.tasks, (id) => titles.get(id));
same("there are shared cases", sharedRows.length, 4);
same("markdown as samu list --format md writes it", `${text(sharedRows)}\n`, shared("tasks.md"));
{
  const t = cases.tasks;
  const flat = [t[2], t[3], t[1], t[0]];
  same("outline puts children after their parent", outline(flat).map((x) => x.id).join(" "), [t[3], t[0], t[1], t[2]].map((x) => x.id).join(" "));
}
same("org as samu list --format org writes it", `${org(sharedRows)}\n`, shared("tasks.org"));

same("an org date with a time", orgActive({ at: "2026-11-20T09:00:00+01:00" }), "<2026-11-20 Fri 09:00>");
same("no org date", orgActive(null), null);
same("an org tag keeps letters, digits and _@#%", orgTag("a-b c@d#e%f_g"), "a_b_c@d#e%f_g");

// --- TSV ---
const lines = tsv(rows, ["priority", "due", "tags"]).split("\n");
same("tsv has a header and one line per task", lines.length, 5);
same("the header names id, title and the columns", lines[0], "id\ttitle\tpriority\tdue\ttags");
same("a row in the column order", lines[1], `${A}\tPlan the trip\tA\t2026-03-02\t`);
same("a time stays with its day", lines[2], `${B}\tBook a train\t\t2026-03-01 14:30\t`);
same("a tab in a title becomes a space", lines[3], `${C}\tSeat with a table\t\t\t`);
same("a line break becomes a space, quotes and commas stay", lines[4], `${D}\tPack, "light" really\t\t\t@home @errand`);
same("every line has as many fields as the header", lines.every((l) => l.split("\t").length === 5), true);

// --- JSON ---
const parsed = JSON.parse(json(rows));
same("json has one element per task", parsed.length, 4);
same("shaped like samu show: the keys", Object.keys(parsed[0]).sort().join(","), "id,kind,object,short,title");
same("kind is task", parsed[0].kind, "task");
same("short is the last eight characters", parsed[1].short, "0000000b");
same("object is passed on as it came", JSON.stringify(parsed[0].object), JSON.stringify(rows[0].object));
same("a line break survives in JSON", parsed[3].title, "Pack, \"light\"\nreally");

// --- render ---
same("render picks the form", render("ids", rows, []), ids(rows));
same("an unknown form gives nothing", render("xml", rows, []), "");

// --- dates ---
same("a whole day", dateText({ date: "2026-03-02" }), "2026-03-02");
same("an instant shows its day and time", dateText({ at: "2026-03-02T09:05:00+01:00", zone: "Europe/Berlin" }), "2026-03-02 09:05");
same("no date", dateText(null), "");

// --- file names ---
same("the view in the name", fileName("Heute fällig", "2026-03-02", "tsv"), "samudo-heute-fallig-2026-03-02.tsv");
same("text is markdown", fileName("Inbox", "2026-03-02", "text"), "samudo-inbox-2026-03-02.md");
same("ids are plain text", fileName("Inbox", "2026-03-02", "ids"), "samudo-inbox-2026-03-02.txt");
same("no view, tasks", fileName("  ", "2026-03-02", "json"), "samudo-tasks-2026-03-02.json");

if (failed) {
  console.error(`exporttext: ${failed} of ${checks} checks failed`);
  process.exit(1);
}
console.log(`exporttext: ${checks} checks`);
