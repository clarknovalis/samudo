// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The targets of jumping (web/lib/jump.js, db3fe005) on synthetic places:
// order, paths, what the scope leaves out, and the fuzzy match over them.
// Run by `make check` with node, no packages.

const root = new URL("../../", import.meta.url);
const { jumpTargets, rank } = await import(new URL("web/lib/jump.js", root));

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

const views = [
  { kind: "inbox", title: "Inbox" },
  { kind: "forecast", title: "Forecast" },
  { kind: "flagged", title: "Flagged" },
];
const folders = [
  { id: "f1", title: "Work", position: 0 },
  { id: "f2", title: "Garden", parent: "f1", position: 0 },
  { id: "f3", title: "Hidden", position: 1 },
];
const projects = [
  { id: "p2", title: "Hedge", folder: "f2", position: 1 },
  { id: "p1", title: "Errands", position: 0 },
];
const tags = [
  { id: "t1", title: "home", position: 0 },
  { id: "t2", title: "phone", parent: "t1", position: 0 },
];
const perspectives = [
  { id: "x2", title: "Later", position: 1 },
  { id: "x1", title: "Now", position: 0 },
];

const all = jumpTargets({
  views,
  perspectives,
  projects,
  folders,
  shownFolders: ["f1", "f2"],
  tags,
  settings: "Settings",
  note: (title, p) => (p.id === "p1" ? `${title} · elsewhere` : title),
});

same("order and titles", all.map((t) => `${t.kind}:${t.title}`), [
  "view:Inbox",
  "view:Forecast",
  "view:Flagged",
  "perspective:Now",
  "perspective:Later",
  "project:Errands · elsewhere",
  "project:Work/Garden/Hedge",
  "folder:Work",
  "folder:Work/Garden",
  "tag:@home",
  "tag:@home/phone",
  "settings:Settings",
]);
same("a view carries its kind", all[1], { kind: "view", view: "forecast", title: "Forecast" });
same("a project carries its id", all.find((t) => t.kind === "project" && t.id === "p2")?.id, "p2");
same("a folder the scope hides is left out", all.some((t) => t.title.includes("Hidden")), false);

const first = (q) => rank(q, all)[0]?.title ?? null;
same("empty query keeps all in order", rank("  ", all).length, all.length);
same("fc finds Forecast", first("fc"), "Forecast");
same("a path finds the project", first("gard/hed"), "Work/Garden/Hedge");
same("@ph finds the tag", first("@ph"), "@home/phone");
same("no match", rank("zzz", all), []);

if (checks < 8) {
  console.error("jump: too few checks");
  process.exit(1);
}
if (failed) process.exit(1);
console.log(`jump: ${checks} checks ok`);
