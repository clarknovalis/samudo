// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The fields of a list row (ADR 0130) in web/lib/rowfields.js: the layout
// this browser keeps, the order, the places, long and short. Run by
// `make check` with node, no packages; synthetic data only.

const root = new URL("../../", import.meta.url);
const { state } = await import(new URL("web/lib/state.js", root));
const { tr } = await import(new URL("web/lib/i18n.js", root));
const { DEFAULT_LAYOUT, FIELDS, fieldParts, layoutOf, withPlace } = await import(new URL("web/lib/rowfields.js", root));

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

Object.assign(state, {
  folders: [{ id: "f1", title: "Haus" }],
  projects: [{ id: "p1", title: "Garten", folder: "f1" }],
  tags: [
    { id: "s1", title: "In Arbeit" },
    { id: "t0", title: "Ort" },
    { id: "t1", title: "Büro", parent: "t0" },
    { id: "t2", title: "Telefon" },
  ],
});

const status = {
  available: false,
  due: "overdue",
  state: "s1",
  reasons: [{ reason: "has_open_children" }],
  task: {
    id: "x1",
    title: "Zaun streichen",
    project: "p1",
    tags: [
      { tag: "s1", position: 0 },
      { tag: "t1", position: 1 },
      { tag: "t2", position: 2 },
    ],
    due: { date: "2026-11-13" },
    defer: { date: "2026-11-10" },
    planned: { at: "2026-11-11T09:00:00+01:00" },
    estimate: "PT1H30M",
    repeat: { rule: "FREQ=WEEKLY;INTERVAL=2", mode: "fixed" },
    note: "\n  Farbe kaufen  \nzweite Zeile",
    created: "2026-11-02T10:00:00Z",
    modified: "2026-11-03T10:00:00Z",
  },
};

const keys = (parts) => parts.map((p) => p.key);
const all = { places: Object.fromEntries(FIELDS.map(([key]) => [key, 2])), short: false };

// Every field says something for this task, in the fixed order.
check("all on line 2, in order", keys(fieldParts(status, 2, all)), FIELDS.map(([key]) => key));
check("nothing on line 1", fieldParts(status, 1, all), []);

// The default: due beside the title, state, tags, place, reason below.
const plain = layoutOf(null);
check("default", plain, DEFAULT_LAYOUT);
check("default line 1", keys(fieldParts(status, 1, plain)), ["due"]);
check("default line 2", keys(fieldParts(status, 2, plain)), ["state", "tags", "place", "reason"]);

// The old switch Details on adds Start, Geplant and the estimate once.
const legacy = layoutOf(null, "on");
check("legacy details on", keys(fieldParts(status, 2, legacy)), ["state", "tags", "place", "reason", "defer", "planned", "estimate"]);
check("legacy details off", layoutOf(null, "off"), DEFAULT_LAYOUT);
// A kept layout wins over the old switch.
const tagsFirst = withPlace(withPlace(plain, "tags", 1), "place", 0);
const kept = layoutOf(JSON.stringify(tagsFirst), "on");
check("kept wins", kept, tagsFirst);
check("tags on line 1", keys(fieldParts(status, 1, kept)), ["due", "tags"]);
check("place off", keys(fieldParts(status, 2, kept)), ["state", "reason"]);
// Broken or partial JSON: defaults for what it does not say.
check("broken json", layoutOf("{nope"), DEFAULT_LAYOUT);
check("unknown field ignored, missing default", layoutOf(JSON.stringify({ places: { note: 1, bogus: 2 }, short: true })), {
  places: { ...DEFAULT_LAYOUT.places, note: 1 },
  short: true,
});

// Texts, long and short.
const text = (layout, key) => fieldParts(status, 2, withPlace(layout, key, 2)).find((p) => p.key === key)?.text;
const short = { ...all, short: true };
check("tags long", text(all, "tags"), "@Ort/Büro @Telefon");
check("tags short", text(short, "tags"), "@Büro @Telefon");
check("place full", text(all, "place"), "Garten · Haus");
check("place short", text(short, "place"), "Garten");
check("place none where the heading says it", keys(fieldParts(status, 2, all, "none")).includes("place"), false);
check("estimate long", text(all, "estimate"), "~1 h 30 min");
check("estimate short", text(short, "estimate"), "~1h30");
check("repeat short", text(short, "repeat"), "↻2w");
check("note long", text(all, "note"), "Farbe kaufen");
check("note short", text(short, "note"), "¶ Farbe kaufen");
check("reason short", text(short, "reason"), `⊘ ${tr("Gruppe: erst die Kinder")}`);
check("state", text(short, "state"), "In Arbeit");
check("due kind", fieldParts(status, 2, all)[0].kind, "due-overdue");
check("short keeps the long form as label", fieldParts(status, 2, short).find((p) => p.key === "estimate").label, "~1 h 30 min");

// What a done task without values leaves out.
const bare = { available: true, due: "none", state: null, reasons: [], task: { id: "x2", title: "leer", tags: [], completed_at: "2026-11-04T10:00:00Z" } };
check("bare task: only its place", keys(fieldParts(bare, 2, all)), ["place"]);
check("inbox", fieldParts(bare, 2, all)[0].text, "Inbox");
// A done task shows no due and no reason, even with values.
const done = { ...status, task: { ...status.task, completed_at: "2026-11-04T10:00:00Z" } };
check("done: no due, no reason", keys(fieldParts(done, 2, all)).filter((k) => k === "due" || k === "reason"), []);

if (checks < 20) {
  console.error(`only ${checks} checks: the test is not what it should be`);
  process.exit(1);
}
if (failed) {
  console.error(`rowfields: ${failed} of ${checks} failed`);
  process.exit(1);
}
console.log(`rowfields: ${checks} checks`);
