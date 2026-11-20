// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The address scheme of the page (ADR 0075 §1, §7), without the state:
// what the address bar says for a view, and what an address asks for.
// The path is the view, `/v/<word>` in English whatever the language of
// the page, under /v/ so that no API route can ever take its place;
// the query adds the search, the scope, the filter, the calendar and the
// open task. The server serves the page under exactly these paths
// (`VIEW_PATHS` in crates/samu-server/src/web.rs).

// The fixed views by kind, as their word under /v/. `/` is the inbox too.
export const VIEW_PATHS = {
  inbox: "inbox",
  everything: "projects",
  today: "today",
  forecast: "forecast",
  calendar: "calendar",
  flagged: "flagged",
  stuck: "stuck",
  review: "review",
  // ADR 0081 §1: the kind stays `time`, the view is the report.
  time: "report",
};

const ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const OBJECT = /^\/o\/([^/]+)\/?$/;
const VIEW = /^\/v\/([a-z]+)\/?$/;
const SHOWS = ["available", "remaining", "all"];
const MODES = ["day", "week", "month", "list"];
const DAY = /^\d{4}-\d{2}-\d{2}$/;

// The address of what the page shows. `view` is the state's view; `q` the
// search as text (empty or null: none); `scope` an id or null; `filter`
// available | remaining | all, null where the view has its own; `calendar`
// { mode, day } in the calendar, `day` null for today; `open` the open
// task, `full` whether it fills the window.
export function addressOf({ view, q = null, scope = null, filter = null, calendar = null, open = null, full = false }) {
  let path = "/";
  if (view.kind === "project" && view.project) path = `/o/${view.project}`;
  else if (view.kind === "perspective" && view.perspective) path = `/o/${view.perspective}`;
  else if (view.kind !== "inbox" && VIEW_PATHS[view.kind]) path = `/v/${VIEW_PATHS[view.kind]}`;
  const query = new URLSearchParams();
  if (q?.trim()) query.set("q", q.trim());
  if (scope) query.set("scope", scope);
  if (filter && filter !== "available") query.set("show", filter);
  if (calendar?.mode) query.set("mode", calendar.mode);
  if (calendar?.day) query.set("day", calendar.day);
  if (open) {
    query.set("open", open);
    if (full) query.set("full", "1");
  }
  const rest = query.toString();
  return rest ? `${path}?${rest}` : path;
}

// What an address asks for, or null for a path that is not a page:
// { view, object, q, scope, show, mode, day, open, full }. `view` is a
// kind, `object` an id under /o/ (task, project, folder, tag or
// perspective: only the loaded objects tell which); `scope` is an id or
// "none". What does not parse is left out.
export function parseAddress(pathname, search = "") {
  const out = { view: null, object: null, q: null, scope: null, show: null, mode: null, day: null, open: null, full: false };
  const object = OBJECT.exec(pathname);
  if (object) {
    if (!ID.test(object[1])) return null;
    out.object = object[1].toLowerCase();
  } else {
    const word = pathname === "/" ? "" : VIEW.exec(pathname)?.[1];
    const kind = word === "" ? "inbox" : Object.keys(VIEW_PATHS).find((k) => VIEW_PATHS[k] === word);
    if (!kind) return null;
    out.view = kind;
  }
  const query = new URLSearchParams(search);
  const q = query.get("q");
  if (q?.trim()) out.q = q.trim();
  const scope = query.get("scope");
  if (scope === "none") out.scope = "none";
  else if (scope && ID.test(scope)) out.scope = scope.toLowerCase();
  if (SHOWS.includes(query.get("show"))) out.show = query.get("show");
  if (MODES.includes(query.get("mode"))) out.mode = query.get("mode");
  if (DAY.test(query.get("day") ?? "")) out.day = query.get("day");
  const open = query.get("open");
  if (open && ID.test(open)) {
    out.open = open.toLowerCase();
    out.full = query.has("full") && query.get("full") !== "0";
  }
  return out;
}

// What changing between two addresses is: a new view (a new entry in
// the history) or the same view (the entry is replaced).
export const viewKey = (view) =>
  view.kind === "project" ? `project:${view.project}` : view.kind === "perspective" ? `perspective:${view.perspective}` : view.kind;
