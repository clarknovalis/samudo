// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// All state lives here; a view reads it and draws (catastr's one rule).
// Nothing else remembers anything of its own.

import { tr } from "./i18n.js";

const stored = (key, fallback) => {
  try {
    return JSON.parse(localStorage.getItem(key)) ?? fallback;
  } catch {
    return fallback;
  }
};

export const state = {
  session: null,
  // From /changes?after=0, by kind.
  folders: [],
  projects: [],
  tags: [],
  links: [],
  // Clocks (ADR 0046): the running one and the time on each task.
  clocks: [],
  // Reminders on tasks (ADR 0030); when they go off is the phone's business.
  reminders: [],
  // Comments and attachments on tasks and projects (ADR 0039).
  comments: [],
  attachments: [],
  // Every task from /changes, whatever the scope shows: a move takes
  // descendants along that the scope hides.
  tasks: [],
  // From /available?all=true: one entry per task.
  statuses: [],
  // From /available/projects: one entry per open project the scope shows.
  projectStatuses: [],
  // From /available/folders: the ids of the folders the scope shows.
  shownFolders: [],
  // Scopes (ADR 0035): the definitions from /changes, and which one this
  // browser looks through. The choice is per device and never synced.
  scopes: [],
  // Capture templates (ADR 0073), from /changes.
  templates: [],
  // Perspectives (ADR 0090), from /changes.
  perspectives: [],
  // From /habits (ADR 0091): the open habits the scope shows, computed.
  habits: [],
  // Forecast's sections from /forecast (ADR 0045), and the account's
  // settings object they follow.
  forecast: [],
  accountSettings: null,
  // Calendars and their appointments (ADR 0047), for ist:termin and the
  // colours; Forecast brings its own occurrences.
  calendars: [],
  appointments: [],
  // The occurrence chosen in the calendar view, shown in the inspector
  // while no task is (ADR 0047 §11).
  appointment: null,
  scope: stored("samu.scope", null),
  // Following the schedule (ADR 0043 §7): what the server says it picks
  // ({ scope, since, until }), and a choice by hand since ({ id, at }).
  // `scope` above is then what follows from the two.
  scopeAuto: stored("samu.scopeAuto", false),
  scopeHand: stored("samu.scopeHand", null),
  scheduled: null,
  // Open tasks outside the scope that are due: counted, never shown (§6).
  outside: [],
  // Tasks and projects another own scope alone shows (ADR 0124): for the
  // search's hint and the project choice, never listed.
  elsewhere: [],
  elsewhereProjects: [],
  // The scope menu in the header is open.
  scopeMenu: false,
  // The settings window (ADR 0035 §8): { scope } is the one being edited.
  settings: null,
  // What the middle shows: inbox | everything | today | forecast |
  // calendar | time | project | perspective ({ perspective: id }).
  view: { kind: "inbox", project: null },
  // The search (ADR 0042): the free text in the field and the chips before
  // it ({ kind, value, not }); while there is either, the middle shows the
  // hits instead of the view. In a perspective they are its search
  // (ADR 0090 §6), and the view stays.
  search: "",
  searchChips: [],
  // Grouping and sorting per view, this browser only (ADR 0042 §7):
  // { "inbox": { group, sort, desc }, "project:<id>": …, … }.
  listOrder: stored("samu.order", {}),
  // Available | remaining | all (ADR 0020 §1).
  filter: "available",
  selected: null,
  // The tasks picked for one action on many (ADR 0052 §4): ids. A new view
  // or search empties it.
  picked: new Set(),
  // The logbook of the selected task, once fetched.
  // `id` is the task it was asked for, null when it must be asked again;
  // `of` the task the entries belong to.
  logbook: { id: null, of: null, entries: [] },
  // Folded action groups: this browser only (ADR 0021 §4).
  folded: new Set(stored("samu.folded", [])),
  // Groups opened where the list holds them without their children, as a
  // search hit or in a flat grouping (ba0590e4): their children are drawn
  // from the mirror. Not kept; such a group starts closed.
  opened: new Set(),
  // On a phone: the list or the task.
  pane: "list",
  // The open task over the middle and the right (ADR 0055 §6); not kept.
  full: false,
  help: false,
  // The capture window is open (ADR 0026 §5).
  capture: false,
  // The id of the task in the edit dialog (ADR 0078), or null.
  edit: null,
  // The refile palette (ADR 0020 §4): { task, query, index } while open.
  palette: null,
  message: null,
};

let draw = () => {};

export function onChange(fn) {
  draw = fn;
}

export function update(patch) {
  if (!("picked" in patch) && ["view", "search", "searchChips"].some((k) => k in patch && patch[k] !== state[k])) {
    state.picked = new Set();
  }
  Object.assign(state, patch);
  // The full view belongs to the open task: another view, a search or no
  // task ends it (ADR 0055 §6).
  if (
    state.full &&
    (["view", "search", "searchChips"].some((k) => k in patch) || !state.selected || state.pane === "list")
  ) {
    state.full = false;
  }
  draw();
}

const keep = (key, value) => {
  try {
    if (value === null || value === undefined || value === false) localStorage.removeItem(key);
    else localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Private mode: the choice lasts as long as the page.
  }
};

// The last event wins (ADR 0043 §7): a choice by hand at or after the
// schedule's last edge, else the schedule's scope. The schedule itself is
// computed by samu-core on the server.
export function followed(scheduled, hand) {
  const since = scheduled?.since ? Date.parse(scheduled.since) : null;
  if (hand && (since === null || Date.parse(hand.at) >= since)) return hand.id ?? null;
  return scheduled?.scope ?? null;
}

export function chooseScope(id) {
  if (state.scopeAuto) {
    const scopeHand = { id, at: new Date().toISOString() };
    keep("samu.scopeHand", scopeHand);
    update({ scopeHand, scope: followed(state.scheduled, scopeHand), scopeMenu: false, selected: null });
    return;
  }
  keep("samu.scope", id);
  update({ scope: id, scopeMenu: false, selected: null });
}

// Follow the schedule or stop; a choice by hand is forgotten either way.
// Stopping keeps what shows now as the choice.
export function setScopeAuto(on, scheduled = state.scheduled) {
  keep("samu.scopeAuto", on);
  keep("samu.scopeHand", null);
  const scope = on ? followed(scheduled, null) : state.scope;
  if (!on) keep("samu.scope", scope);
  update({ scopeAuto: on, scopeHand: null, scheduled, scope, scopeMenu: false, selected: null });
}

// What the server says the schedule picks now; the shown scope follows.
export function takeSchedule(scheduled) {
  if (!state.scopeAuto) {
    update({ scheduled });
    return;
  }
  const scope = followed(scheduled, state.scopeHand);
  update(scope === state.scope ? { scheduled } : { scheduled, scope, selected: null });
}

function keepFolds() {
  try {
    localStorage.setItem("samu.folded", JSON.stringify([...state.folded]));
  } catch {
    // Private mode: folding then lasts as long as the page.
  }
  draw();
}

export function toggleFold(id) {
  if (state.folded.has(id)) state.folded.delete(id);
  else state.folded.add(id);
  keepFolds();
}

// The fold of a heading of the list and of a lane of the board (8b9e1584):
// one key for both, so a group folded on the board is folded in the list.
export const laneFold = (key) => `lane:${key ?? "inbox"}`;

// The headings drawn: all closed when one is open, else all open.
export function foldAll(ids) {
  const close = ids.some((id) => !state.folded.has(id));
  for (const id of ids) {
    if (close) state.folded.add(id);
    else state.folded.delete(id);
  }
  keepFolds();
}

// Alt-click on a heading's arrow: that one open, the others closed.
export function foldOthers(id, ids) {
  for (const other of ids) state.folded.add(other);
  state.folded.delete(id);
  keepFolds();
}

export function toggleOpened(id) {
  if (state.opened.has(id)) state.opened.delete(id);
  else state.opened.add(id);
  draw();
}

// In a perspective the arrangement is the perspective's (ADR 0090 §6):
// perspectives.js takes it, routed here by app.js.
let perspectiveOrder = null;
export function routePerspectiveOrder(fn) {
  perspectiveOrder = fn;
}

export function chooseOrder(key, patch) {
  if (key.startsWith("perspective:")) {
    perspectiveOrder?.(key.slice("perspective:".length), patch);
    return;
  }
  state.listOrder = { ...state.listOrder, [key]: { ...state.listOrder[key], ...patch } };
  try {
    localStorage.setItem("samu.order", JSON.stringify(state.listOrder));
  } catch {
    // Private mode: the choice lasts as long as the page.
  }
  draw();
}

// Lookups the views share.
export const byId = (list, id) => list.find((o) => o.id === id);
export const statusOf = (id) => state.statuses.find((s) => s.task.id === id);
export const tagTitle = (id) => byId(state.tags, id)?.title ?? "?";
// ADR 0094: the mark of a folder, project or tag with an own scope —
// „nur in AOD“, or „Scope fehlt“ when it does not exist (§5); null for
// one shown everywhere.
export const onlyIn = (object) => {
  if (!object?.own_scope) return null;
  const scope = byId(state.scopes, object.own_scope);
  return scope ? tr("nur in {scope}", { scope: scope.title }) : tr("Scope fehlt");
};
// Whether the search has text or chips.
// In a perspective the field is its search, not one of its own (ADR 0090 §6).
// A block perspective has no search of its own: there the field searches
// everything, as in a fixed view (ADR 0116 §6).
const blockPerspective = () =>
  state.view.kind === "perspective" && Boolean(byId(state.perspectives, state.view.perspective)?.blocks?.length);
export const searching = () =>
  (state.view.kind !== "perspective" || blockPerspective()) && Boolean(state.search.trim() || state.searchChips.length);
// What the middle shows: the search while there is one, else the view.
export const shown = () => (searching() ? { kind: "search", project: null } : state.view);
export const isOpen = (task) => !task.completed_at && !task.dropped_at;

// The open tasks that wait for `id` (53a0d8e9, „Hält auf“), each with its
// `blocked_by` link, by title.
export function heldUpBy(id) {
  return state.links
    .filter((l) => l.kind === "blocked_by" && l.uri === `samu:${id}`)
    .map((link) => ({ link, task: byId(state.tasks, link.owner) }))
    .filter(({ task }) => task && isOpen(task))
    .sort((a, b) => a.task.title.localeCompare(b.task.title));
}

// Whether `id` is a blocker (🚧, `ist:blocker`), as samu-core's
// `blockers::holds_up`: open, and an open task waits for it.
export function holdsUp(id) {
  const task = byId(state.tasks, id);
  return Boolean(task) && isOpen(task) && heldUpBy(id).length > 0;
}

// The own scope that alone shows this project, when it is not the one
// looked through (ADR 0124 §4); null otherwise.
export function elsewhereOf(project) {
  return project ? (state.elsewhereProjects.find((p) => p.project.id === project)?.elsewhere ?? null) : null;
}
