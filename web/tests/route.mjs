// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The address scheme (web/lib/route.js, ADR 0075 §7) and the search as a
// line of text (searchText, searchFrom in web/lib/query.js) on synthetic
// objects. Run by `make check` with node, no packages.

const root = new URL("../../", import.meta.url);
const { addressOf, parseAddress, viewKey, VIEW_PATHS } = await import(new URL("web/lib/route.js", root));
const { state } = await import(new URL("web/lib/state.js", root));
const { searchFrom, searchText } = await import(new URL("web/lib/query.js", root));

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

const P = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c01";
const T = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c02";
const S = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c03";
const view = (kind, extra = {}) => ({ kind, project: null, ...extra });

// --- Writing ----------------------------------------------------------------

same("inbox is /", addressOf({ view: view("inbox") }), "/");
same("projects", addressOf({ view: view("everything") }), "/v/projects");
same("report", addressOf({ view: view("time") }), "/v/report");
same("project", addressOf({ view: view("project", { project: P }) }), `/o/${P}`);
same("perspective", addressOf({ view: { kind: "perspective", perspective: P } }), `/o/${P}`);
same("defaults left out", addressOf({ view: view("today"), q: "  ", filter: "available", scope: null }), "/v/today");
same(
  "everything",
  addressOf({ view: view("calendar"), q: "@home tea", scope: S, filter: "all", calendar: { mode: "month", day: "2026-12-01" }, open: T, full: true }),
  `/v/calendar?q=%40home+tea&scope=${S}&show=all&mode=month&day=2026-12-01&open=${T}&full=1`,
);
same("full only with a task", addressOf({ view: view("today"), full: true }), "/v/today");

// --- Reading ----------------------------------------------------------------

const read = (path, search = "") => parseAddress(path, search);
same("root", read("/").view, "inbox");
same("each view back", Object.entries(VIEW_PATHS).map(([kind, word]) => read(`/v/${word}`).view === kind).every(Boolean), true);
same("trailing slash", read("/v/forecast/").view, "forecast");
same("unknown path", read("/nope"), null);
same("unknown view", read("/v/nope"), null);
same("a view at the root is not one", read("/forecast"), null);
same("object not an id", read("/o/nope"), null);
same("deeper object path", read(`/o/${P}/x`), null);
same("object, lower case", read(`/o/${P.toUpperCase()}`).object, P);
same(
  "round trip",
  read(...(() => {
    const a = addressOf({ view: view("calendar"), q: '#"Big one" -is:done', scope: S, filter: "remaining", calendar: { mode: "week", day: "2026-11-30" }, open: T, full: true });
    const at = a.indexOf("?");
    return [a.slice(0, at), a.slice(at)];
  })()),
  { view: "calendar", object: null, q: '#"Big one" -is:done', scope: S, show: "remaining", mode: "week", day: "2026-11-30", open: T, full: true },
);
same("scope none", read("/v/today", "?scope=none").scope, "none");
same("bad values dropped", read("/v/today", "?scope=x&show=some&mode=year&day=tomorrow&open=y&full=1"), {
  view: "today", object: null, q: null, scope: null, show: null, mode: null, day: null, open: null, full: false,
});

same("a new view", viewKey(view("today")) === viewKey(view("forecast")), false);
same("another project", viewKey(view("project", { project: P })) === viewKey(view("project", { project: T })), false);

// --- The search as text -----------------------------------------------------

Object.assign(state, {
  projects: [{ id: "p1", title: "Big one", position: 0 }],
  tags: [{ id: "t1", title: "home", position: 0 }],
  folders: [{ id: "f1", title: "Work", position: 0 }],
});
const chips = [
  { kind: "project", value: "p1", not: false },
  { kind: "tag", value: "t1", not: true },
  { kind: "or" },
  { kind: "is", value: "erledigt", not: false },
  { kind: "folder", value: "f1", not: false },
];
const line = searchText(chips, " tea time ");
same("chips in English, then text", line, '#"Big one" -@home or is:done folder:Work tea time');
const back = searchFrom(line);
same("text back", back.search, "tea time");
same(
  "chips back",
  back.searchChips.map((c) => (c.kind === "or" ? "or" : `${c.not ? "-" : ""}${c.kind}:${c.value}`)),
  ["project:p1", "-tag:t1", "or", "is:erledigt", "folder:f1"],
);
same("German words read too", searchFrom("@home ist:markiert").searchChips.length, 2);
same("unknown name stays text", searchFrom("@nowhere x").search, "@nowhere x");

if (checks < 20) {
  console.error("route: too few checks");
  process.exit(1);
}
if (failed) process.exit(1);
console.log(`route: ${checks} checks ok`);
