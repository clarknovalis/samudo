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
// A word that looks like a chip but names nothing is refused (§3,
// Changes); a word whose key is none of the search's stays text.
for (const bad of ["@Nirgends", "anruf #Nirgends", "fällig:quatsch", "-@Nirgends | Weg"]) {
  check(`no chip: ${bad}`, parseLine(bad, perspectives).kind, "error");
}
check("an unknown key is text", parseLine("Uhrzeit:17", perspectives).block, { query: { text: "Uhrzeit:17" } });
check("a quoted word is text", parseLine('"@Nirgends"', perspectives).kind, "query");
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

// A note block (ADR 0146 §4): `:note #Project` as the search writes it,
// or the start or end of a project's or task's id; it reads back.
const { state } = await import(new URL("web/lib/state.js", root));
state.projects = [{ id: "01936f5e-8a00-7c3e-9b1a-3f2d4c5b7c01", title: "Wochen Rückblick", note: "- [ ] Eingang" }];
state.tasks = [
  { id: "01936f5e-8a00-7c3e-9b1a-3f2d4c5b7c02", title: "Tagesfokus" },
  { id: "01936f5e-8a00-7c3e-9b1a-3f2d4c5b7c12", title: "Andere" },
];
const [review, focus] = [state.projects[0].id, state.tasks[0].id];
check("a project's note", parseLine(':note #"Wochen Rückblick" | Checkliste', perspectives).block, { note: review, title: "Checkliste" });
check("a task's note by the end of its id", parseLine(":note 4c5b7c02", perspectives), { kind: "note", block: { note: focus } });
check("by its full id", parseLine(`:note samu:${focus}`, perspectives).block, { note: focus });
for (const bad of [":note", ":note #Nirgends", ":note @Büro", ":note ffffffff", ":note 01936f5e", ":notes"]) {
  check(`refused: ${bad}`, parseLine(bad, perspectives).kind, "error");
}
const notes = [{ note: review, title: "Checkliste" }, { note: focus }, { view: "today" }];
const noteText = notes.map((b) => lineOf(b, perspectiveOf, "de")).join("\n");
check("note lines", noteText.split("\n"), [':note #"Wochen Rückblick" | Checkliste', ":note 4c5b7c02", ":today"]);
check("note lines read back", fromLines(noteText, notes, perspectives).blocks, notes);

if (failed) {
  console.error(`${failed} of ${checked} failed`);
  process.exit(1);
}
console.log(`blocklines: ${checked} checks ok`);
