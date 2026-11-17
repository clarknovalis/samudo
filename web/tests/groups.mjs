// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The OR groups of web/lib/query.js (ADR 0121) against the cases
// samu-core meets as well: testdata/query/groups.json. The words go
// through what the search field does with them: each word by
// parseWord, kept in a list by addTo, written as the server keeps it by
// withGroups. Run by `make check` with node, no packages.

import { readFileSync } from "node:fs";

const root = new URL("../../", import.meta.url);
const { addTo, freeTerms, matcher, parseWord, withGroups, withOrs } = await import(new URL("web/lib/query.js", root));
const doc = JSON.parse(readFileSync(new URL("testdata/query/groups.json", root), "utf8"));

let failed = 0;
const fail = (what) => {
  failed += 1;
  console.error(`not ok  ${what}`);
};
const last = (task) => parseInt(task.id.slice(-2), 16);

for (const c of doc.cases) {
  const test = matcher(c.chips, c.text);
  const got = doc.tasks.filter((s) => test(s)).map((s) => last(s.task));
  if (JSON.stringify(got) !== JSON.stringify(c.ids)) fail(`${c.about}: ${JSON.stringify(got)}, want ${JSON.stringify(c.ids)}`);
  // The same chips through the field's list come back unchanged.
  const back = withGroups(withOrs(c.chips));
  const want = withGroups(c.chips);
  if (JSON.stringify(back) !== JSON.stringify(want)) fail(`${c.about}: round trip ${JSON.stringify(back)}`);
}

// The words as the field takes them: a word in quotes stays free text.
const WORDS = /"[^"]*"|\S+/g;
for (const c of doc.words) {
  let list = [];
  const free = [];
  for (const [word] of c.text.matchAll(WORDS)) {
    const chip = word.startsWith('"') ? null : parseWord(word);
    if (chip) list = addTo(list, chip, (a, b) => a.kind === b.kind && a.value === b.value && a.not === b.not);
    else free.push(word);
  }
  const chips = withGroups(list);
  if (JSON.stringify(chips) !== JSON.stringify(c.chips)) fail(`${c.text}: ${JSON.stringify(chips)}`);
  if (free.join(" ") !== c.free) fail(`${c.text}: free ${JSON.stringify(free.join(" "))}`);
  if (freeTerms(c.free).length !== freeTerms(free.join(" ")).length) fail(`${c.text}: free terms`);
}

if (doc.cases.length < 5 || doc.words.length < 5) {
  console.error("groups: too few cases, the file was not read right");
  process.exit(1);
}
if (failed) process.exit(1);
console.log(`groups: ${doc.cases.length} cases, ${doc.words.length} words ok`);
