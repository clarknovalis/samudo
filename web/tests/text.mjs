// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The free text of web/lib/query.js against the cases samu-core meets as
// well: testdata/query/text.json. Run by `make check` with node, no
// packages.

import { readFileSync } from "node:fs";

const root = new URL("../../", import.meta.url);
const { matcher, namesTask, parseWord } = await import(new URL("web/lib/query.js", root));
const doc = JSON.parse(readFileSync(new URL("testdata/query/text.json", root), "utf8"));

let failed = 0;
for (const { text, matches, names } of doc.cases) {
  const got = matcher([], text)({ task: doc.task });
  if (got !== matches) {
    failed += 1;
    console.error(`not ok  ${JSON.stringify(text)}: ${got}, want ${matches}`);
  }
  if (names !== undefined && namesTask(text)({ task: doc.task }) !== names) {
    failed += 1;
    console.error(`not ok  names ${JSON.stringify(text)}: want ${names}`);
  }
}
// `id:` takes every form of an id that names a task, as samu-core
// (6aec8bd3); a copied link on its own stays free text.
const whole = doc.task.id;
for (const [word, want] of [
  ["id:00000000ABCD", { kind: "id", value: "00000000abcd", not: false }],
  [`id:samu:${whole}`, { kind: "id", value: whole, not: false }],
  [`-id:https://example.org/o/${whole}`, { kind: "id", value: whole, not: true }],
  ["id:abcd", null],
  [`samu:${whole}`, null],
]) {
  const got = parseWord(word);
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    failed += 1;
    console.error(`not ok  parse ${word}: ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
  }
}
if (doc.cases.length < 10) {
  console.error("text: too few cases, the file was not read right");
  process.exit(1);
}
if (failed) process.exit(1);
console.log(`text: ${doc.cases.length} cases ok`);
