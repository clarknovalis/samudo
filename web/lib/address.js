// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The address of what the page shows (ADR 0075 §1, §7; the scheme is
// route.js): read when the page starts and on Back, applied once the
// objects are loaded, and kept in the address bar after each drawing, so
// every state can be bookmarked, shared or handed over by
// `samu open-uri samu:<uuid>`.

import { calendarChoice, setCalendarChoice } from "./calendar.js";
import { edited, searchOf } from "./perspectives.js";
import { searchFrom, searchText } from "./query.js";
import { addressOf, parseAddress, viewKey } from "./route.js";
import { chooseScope, state, statusOf, update } from "./state.js";
import { tr } from "./i18n.js";

// What the address asks for, until it is applied; null for a path that is
// no page of ours (then the page starts as it would at `/`).
let pending = parseAddress(location.pathname, location.search);
// Whether the scope was already left once for the open task, and set
// once from the address.
let leftScope = false;
let scopeTaken = false;
// The view the address bar was last written for: another one is a new
// entry in the history (§7). After applying an address, the next writing
// replaces, so Back does not land on the same view twice.
let shownView = null;
let replaceNext = true;

const find = (list, id) => list.find((o) => o.id === id);

function setField(text) {
  const input = document.getElementById("search-input");
  if (input) input.value = text;
}

// After each load: applies what the address asks for, once.
export function resolveAddress() {
  if (!pending) return;
  const a = pending;
  // The scope first: its load comes back here with the statuses for it.
  if (a.scope && !scopeTaken) {
    scopeTaken = true;
    const want = a.scope === "none" ? null : a.scope;
    if (want !== state.scope && (want === null || find(state.scopes, want))) {
      chooseScope(want);
      return;
    }
  }
  const patch = { selected: null, pane: "list" };
  // A task: under /o/ (ADR 0075 §1) or open on a view.
  const taskId = a.open ?? (a.object && find(state.tasks, a.object) ? a.object : null);
  if (taskId && find(state.tasks, taskId)) {
    if (statusOf(taskId)) Object.assign(patch, { selected: taskId, pane: "detail" });
    else if (state.scope && !leftScope) {
      // Not in this scope: without one it is (ADR 0069, by id).
      leftScope = true;
      update({ message: tr("Scope verlassen, um die Aufgabe zu zeigen.") });
      chooseScope(null);
      return;
    } else patch.message = tr("Diese Aufgabe lässt sich hier nicht zeigen.");
  } else if (taskId) patch.message = tr("Kein Objekt {id} in diesem Konto.", { id: taskId });
  // The view, and the search that goes with it.
  let search = { search: "", searchChips: [] };
  const object = a.object && a.object !== taskId ? a.object : null;
  if (object && find(state.projects, object)) {
    patch.view = { kind: "project", project: object };
  } else if (object && find(state.folders, object)) {
    search = { search: "", searchChips: [{ kind: "folder", value: object, not: false }] };
  } else if (object && find(state.tags, object)) {
    search = { search: "", searchChips: [{ kind: "tag", value: object, not: false }] };
  } else if (object && find(state.perspectives, object)) {
    // A perspective opens as a view with its search (ADR 0090 §7).
    patch.view = { kind: "perspective", perspective: object };
    search = searchOf(find(state.perspectives, object));
  } else if (object) {
    patch.message = tr("Kein Objekt {id} in diesem Konto.", { id: object });
  } else if (a.view) {
    patch.view = { kind: a.view, project: null };
  }
  if (a.q) search = searchFrom(a.q);
  const perspective = (patch.view ?? state.view).kind === "perspective";
  if (!perspective) patch.filter = a.show ?? "available";
  if ((patch.view ?? state.view).kind === "calendar") setCalendarChoice({ mode: a.mode, day: a.day });
  pending = null;
  // Applying an address is no step of its own in the history, unless a
  // link asked for a new view.
  replaceNext = !a.push;
  setField(search.search);
  update({ ...patch, ...search });
  // A new view or search ends the full view (ADR 0055 §6): asked for, it
  // comes after them.
  if (a.full && state.selected) {
    replaceNext = true;
    update({ full: true });
  }
}

// Opens an object the way its address does: a link in a note to a task,
// project, folder or tag (ADR 0077 §3), or a perspective (ADR 0090 §7).
// A task opens on the view on screen.
export function openObject(id) {
  const task = find(state.tasks, id);
  // Anything but a task is a new view: an entry of its own.
  pending = task ? { ...here(), open: id } : { ...blank(), object: id, push: true };
  leftScope = false;
  scopeTaken = true;
  resolveAddress();
}

// After each drawing: the address bar shows the state.
export function keepAddress() {
  if (pending) return;
  const view = state.view;
  const perspective = view.kind === "perspective";
  const q = perspective && !edited() ? null : searchText(state.searchChips, state.search);
  const want = addressOf({
    view,
    q,
    scope: state.scope,
    filter: perspective ? null : state.filter,
    calendar: view.kind === "calendar" ? calendarChoice() : null,
    open: state.selected,
    full: state.full,
  });
  const key = viewKey(view);
  const push = !replaceNext && shownView !== null && key !== shownView;
  replaceNext = false;
  shownView = key;
  if (location.pathname + location.search === want) return;
  if (push) history.pushState(null, "", want + location.hash);
  else history.replaceState(null, "", want + location.hash);
}

// Back and Forward: the address the browser went to is applied as it is.
if (typeof window !== "undefined") {
  window.addEventListener("popstate", () => {
    pending = parseAddress(location.pathname, location.search) ?? { ...blank(), view: "inbox" };
    leftScope = false;
    scopeTaken = false;
    resolveAddress();
  });
}

// An address that asks for nothing, and one for the view on screen.
function blank() {
  return { view: null, object: null, q: null, scope: null, show: null, mode: null, day: null, open: null, full: false };
}

function here() {
  const v = state.view;
  const object = v.kind === "project" ? v.project : v.kind === "perspective" ? v.perspective : null;
  return {
    ...blank(),
    view: object ? null : v.kind,
    object,
    q: searchText(state.searchChips, state.search) || null,
    show: state.filter,
  };
}
