// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The free text of web/lib/query.js against the cases samu-core meets as
// well: testdata/query/text.json. Run by `make check` with node, no
// packages.

import { readFileSync } from "node:fs";

const root = new URL("../../", import.meta.url);
const { matcher } = await import(new URL("web/lib/query.js", root));
const doc = JSON.parse(readFileSync(new URL("testdata/query/text.json", root), "utf8"));

let failed = 0;
for (const { text, matches } of doc.cases) {
  const got = matcher([], text)({ task: doc.task });
  if (got !== matches) {
    failed += 1;
    console.error(`not ok  ${JSON.stringify(text)}: ${got}, want ${matches}`);
  }
}
if (doc.cases.length < 10) {
  console.error("text: too few cases, the file was not read right");
  process.exit(1);
}
if (failed) process.exit(1);
console.log(`text: ${doc.cases.length} cases ok`);
