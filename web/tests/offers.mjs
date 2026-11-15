// The search offers of web/lib/query.js against the cases samu-core meets
// as well (ADR 0103 §2): testdata/query/offers.json. Run by `make check`
// with node, no packages. The web gets `inherited` from the server; here
// it is a task's project's properties it lacks itself, which is all the
// cases need.

import { readFileSync } from "node:fs";

const root = new URL("../../", import.meta.url);
const { state } = await import(new URL("web/lib/state.js", root));
const { offers } = await import(new URL("web/lib/query.js", root));
const doc = JSON.parse(readFileSync(new URL("testdata/query/offers.json", root), "utf8"));

const inherited = (task) => {
  const project = doc.projects.find((p) => p.id === task.project);
  const own = task.properties ?? {};
  return Object.fromEntries(
    Object.entries(project?.properties ?? {})
      .filter(([key]) => !(key in own))
      .map(([key, value]) => [key, { value, from: project.id }]),
  );
};
Object.assign(state, {
  folders: doc.folders,
  projects: doc.projects,
  tags: doc.tags,
  statuses: doc.tasks.map((task) => ({ task, inherited: inherited(task) })),
});

const shown = (o) =>
  o.chip
    ? { label: o.label, chip: { kind: o.chip.kind, value: o.chip.value, ...(o.chip.not ? { not: true } : {}) } }
    : { label: o.label, insert: o.insert };

let failed = 0;
for (const c of doc.cases) {
  const got = JSON.stringify(offers(c.word, { inScope: !c.appointments, lang: c.lang ?? "de" }).map(shown));
  const want = JSON.stringify(c.offers);
  if (got !== want) {
    failed += 1;
    console.error(`offers for ${JSON.stringify(c.word)}\n  got  ${got}\n  want ${want}`);
  }
}
if (doc.cases.length < 10) {
  console.error(`only ${doc.cases.length} cases: the file is not what it should be`);
  process.exit(1);
}
console.log(`web offers: ${doc.cases.length} cases, ${failed} failed`);
process.exit(failed ? 1 : 0);
