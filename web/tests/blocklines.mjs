// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The blocks as lines of web/lib/blocklines.js (ADR 0143 §3, §4): what a
// line becomes, that the lines read back to the same blocks, and that a
// search kept as it was keeps its filter and view. Run by `make check`
// with node, no packages. Synthetic data.

const root = new URL("../../", import.meta.url);
const { fromLines, lineOf, parseLine, detached } = await import(new URL("web/lib/blocklines.js", root));

let failed = 0;
let checked = 0;
const check = (what, got, want) => {
  checked += 1;
  const g = JSON.stringify(got);
  const w = JSON.stringify(want);
  if (g !== w) {
    failed += 1;
    console.error(`${what}\n  got  ${g}\n  want ${w}`);
  }
};

const office = { id: "p-office", title: "Büro", chips: [{ kind: "is", value: "markiert" }], filter: "remaining", view: "table" };
const morning = { id: "p-morning", title: "Morgen", blocks: [{ view: "today" }] };
const perspectives = [office, morning];
const perspectiveOf = (id) => perspectives.find((p) => p.id === id);

const kinds = (text) => fromLines(text, [], perspectives, morning.id).rows.map((r) => r.kind);
check("a view, a name in any case and quoted, a search", kinds(':today\nbüro\n"Büro"\nanruf ist:markiert'), ["view", "perspective", "perspective", "query"]);
check("blank lines do not count", kinds("\n  \n:inbox\n\n"), ["view"]);
check("the heading after |", parseLine("anruf | Anrufe", perspectives).block, { query: { text: "anruf" }, title: "Anrufe" });
check("a chip and free text", parseLine("ist:markiert anruf", perspectives).block, { query: { chips: [{ kind: "is", value: "markiert" }], text: "anruf" } });
check("an empty search is all", parseLine(" | Alles", perspectives).block, { query: {}, title: "Alles" });
for (const bad of [":forecast", "anruf | ", "anruf |", "Morgen"]) {
  check(`refused: ${bad}`, parseLine(bad, perspectives).kind, "error");
}
check("itself is refused", parseLine("Büro", perspectives, office.id).kind, "error");
check("an error leaves no blocks", fromLines(":today\n:forecast", [], perspectives).blocks, null);

// Back and forth: the lines of blocks read back to the same blocks.
const blocks = [
  { view: "today", title: "Heute früh" },
  { perspective: office.id },
  { query: { chips: [{ kind: "is", value: "markiert" }], text: "anruf" }, title: "Anrufe" },
  { query: { text: "toner" } },
];
const text = blocks.map((b) => lineOf(b, perspectiveOf, "de")).join("\n");
check("the lines", text.split("\n"), [":today | Heute früh", "Büro", "ist:markiert anruf | Anrufe", "toner"]);
check("read back", fromLines(text, blocks, perspectives).blocks, blocks);

// A search as it was keeps the rest; a changed one starts afresh.
const kept = [{ query: { text: "toner", filter: "all", view: "table" } }];
check("kept", fromLines("toner | Toner", kept, perspectives).blocks, [{ query: { text: "toner", filter: "all", view: "table" }, title: "Toner" }]);
check("changed", fromLines("tinte", kept, perspectives).blocks, [{ query: { text: "tinte" } }]);

// Detach: a copy of the perspective's search under its heading.
check("detached", detached({ perspective: office.id }, office), {
  query: { chips: office.chips, filter: "remaining", view: "table" },
  title: "Büro",
});
check("detached keeps a heading", detached({ perspective: office.id, title: "Kurz" }, office).title, "Kurz");

if (failed) {
  console.error(`${failed} of ${checked} failed`);
  process.exit(1);
}
console.log(`blocklines: ${checked} checks ok`);
