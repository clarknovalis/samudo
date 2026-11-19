// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The rules for changing a tag's workflow fields (web/lib/tagfields.js)
// against the cases shared with samu-core and the CLI. Run by
// `make check` with node, no packages.

import { readFileSync } from "node:fs";

const root = new URL("../../", import.meta.url);
const { tagChanges, tagRole } = await import(new URL("web/lib/tagfields.js", root));
const data = JSON.parse(readFileSync(new URL("testdata/tagfields/cases.json", root), "utf8"));

let failed = 0;
let checks = 0;
const same = (what, got, want) => {
  checks += 1;
  const [g, w] = [JSON.stringify(got), JSON.stringify(want)];
  if (g !== w) {
    failed += 1;
    console.error(`not ok  ${what}: ${g}, want ${w}`);
  }
};

for (const c of data.cases) {
  const got = tagChanges(data.tags, data.projects, c.tag, c.fields);
  same(c.what, got, c.error ? { error: c.error } : { changes: c.changes });
}
const role = (id) => tagRole(data.tags, data.tags.find((t) => t.id === id));
same("roles", ["status", "waiting", "plain"].map(role), ["workflow", "state", "tag"]);

if (failed) {
  console.error(`${failed} of ${checks} tag field checks failed`);
  process.exit(1);
}
console.log(`ok   ${checks} tag field checks`);
