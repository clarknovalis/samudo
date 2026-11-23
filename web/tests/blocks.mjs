// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The blocks of web/lib/blocks.js against the cases samu-core meets as
// well (ADR 0116 §3): testdata/blocks/sections.json. Run by `make check`
// with node, no packages.

import { readFileSync } from "node:fs";

const root = new URL("../../", import.meta.url);
const { assemble, union, inView, knownView, VIEWS } = await import(new URL("web/lib/blocks.js", root));
const doc = JSON.parse(readFileSync(new URL("testdata/blocks/sections.json", root), "utf8"));

let failed = 0;
const check = (what, got, want) => {
  const g = JSON.stringify(got);
  const w = JSON.stringify(want);
  if (g !== w) {
    failed += 1;
    console.error(`${what}\n  got  ${g}\n  want ${w}`);
  }
};

if (doc.cases.length < 4) {
  console.error(`only ${doc.cases.length} cases: the file is not what it should be`);
  process.exit(1);
}
// A block's own search stands for its hits by its text (ADR 0143).
const shows = (p, s) => (doc.shows[p.id] ?? doc.queries[p.text] ?? []).includes(s.task.id);
let found = 0;
for (const c of doc.cases) {
  const p = { ...doc.perspectives.find((o) => o.title === c.perspective) };
  if (typeof c.dedupe === "boolean") p.dedupe = c.dedupe;
  const got = assemble(p, doc.perspectives, doc.statuses, shows);
  found += got.reduce((n, s) => n + s.tasks.length, 0);
  check(
    c.about,
    got.map((s) => ({ title: s.title, missing: s.missing, tasks: s.tasks.map((t) => t.task.id) })),
    c.sections,
  );
  check(`${c.about}: union`, union(got).map((s) => s.task.id), c.union);
}
if (!found) {
  console.error("no case finds a task: the file is not what it should be");
  process.exit(1);
}
check("known views", VIEWS.every(knownView) && !knownView("forecast"), true);

// A block perspective opens with an empty search field, even when it was
// made from a search and still carries its chips: the search would hide
// the blocks (ADR 0116 §6).
const { searchOf } = await import(new URL("web/lib/perspectives.js", root));
const chip = { kind: "tag", value: "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6d01" };
check("a block perspective's field", searchOf({ title: "@today", chips: [chip], text: "x", blocks: [{ view: "today" }] }), { search: "", searchChips: [] });
check("a plain perspective's field", searchOf({ title: "@today", chips: [chip], text: "x" }).searchChips.length, 1);
check("an unknown view", inView("forecast", doc.statuses[0]), null);

if (failed) {
  console.error(`${failed} failed`);
  process.exit(1);
}
console.log(`blocks: ${doc.cases.length} cases ok`);
