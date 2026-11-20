// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The double click on a row (web/lib/clicks.js), counted by task id and
// time, since Firefox fires no dblclick once the first click redrew the
// row. Run by `make check` with node, no packages.

const root = new URL("../../", import.meta.url);
const { clickCounter, DOUBLE_MS } = await import(new URL("web/lib/clicks.js", root));

let failed = 0;
let checks = 0;
const same = (what, got, want) => {
  checks += 1;
  if (got !== want) {
    failed += 1;
    console.error(`${what}\n  got  ${got}\n  want ${want}`);
  }
};

let second = clickCounter();
same("a first click is single", second("a", 1000), false);
same("the second on the same row within the gap is double", second("a", 1200), true);
same("a third click starts a new pair", second("a", 1300), false);
same("and its fourth closes that pair", second("a", 1400), true);

second = clickCounter();
second("a", 1000);
same("another row is no double click", second("b", 1100), false);
same("but that row's second click is", second("b", 1200), true);

second = clickCounter();
second("a", 1000);
same("at the gap still double", second("a", 1000 + DOUBLE_MS), true);
second("a", 5000);
same("past the gap single", second("a", 5001 + DOUBLE_MS), false);

second = clickCounter();
second("a", 1000);
same("a clock going back is no double click", second("a", 900), false);

if (checks < 9) {
  console.error(`only ${checks} checks ran`);
  process.exit(1);
}
if (failed) process.exit(1);
console.log(`clicks: ${checks} checks`);
