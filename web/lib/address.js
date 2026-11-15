// One object's address, `/o/<uuid>` (ADR 0075 §1): read when the page
// starts, resolved once the objects are loaded, and kept in the address bar
// while a task is open, so it can be copied, shared or handed over by
// `samu open-uri samu:<uuid>`.

import { searchOf } from "./perspectives.js";
import { chooseScope, state, statusOf, update } from "./state.js";
import { tr } from "./i18n.js";

const PATH = /^\/o\/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\/?$/i;

// The id the page was opened on, until it is shown or found missing.
let pending = PATH.exec(location.pathname)?.[1]?.toLowerCase() ?? null;
// Whether the scope was already left once for it.
let leftScope = false;

const find = (list, id) => list.find((o) => o.id === id);

// After each load: shows what the address names, once.
export function resolveAddress() {
  if (!pending) return;
  const id = pending;
  if (find(state.tasks, id)) {
    if (statusOf(id)) {
      pending = null;
      update({ selected: id, pane: "detail", search: "", searchChips: [] });
    } else if (state.scope && !leftScope) {
      // Not in this scope: without one it is (ADR 0069, by id).
      leftScope = true;
      update({ message: tr("Scope verlassen, um die Aufgabe zu zeigen.") });
      chooseScope(null);
    } else {
      pending = null;
      update({ message: tr("Diese Aufgabe lässt sich hier nicht zeigen.") });
    }
    return;
  }
  pending = null;
  if (find(state.projects, id)) {
    update({ view: { kind: "project", project: id }, search: "", searchChips: [], selected: null, pane: "list" });
  } else if (find(state.folders, id)) {
    update({ search: "", searchChips: [{ kind: "folder", value: id, not: false }], selected: null, pane: "list" });
  } else if (find(state.tags, id)) {
    update({ search: "", searchChips: [{ kind: "tag", value: id, not: false }], selected: null, pane: "list" });
  } else if (find(state.perspectives, id)) {
    // A perspective opens as a view with its search (ADR 0090 §7).
    const p = find(state.perspectives, id);
    const input = document.getElementById("search-input");
    if (input) input.value = p.text ?? "";
    update({ view: { kind: "perspective", perspective: id }, ...searchOf(p), selected: null, pane: "list" });
  } else {
    update({ message: tr("Kein Objekt {id} in diesem Konto.", { id }) });
  }
}

// Opens an object the way its address does: a link in a note to a task,
// project, folder or tag (ADR 0077 §3), or a perspective (ADR 0090 §7).
export function openObject(id) {
  pending = id;
  leftScope = false;
  resolveAddress();
}

// After each drawing: the address bar shows the open task, or nothing.
export function keepAddress() {
  if (pending) return;
  const want = state.selected ? `/o/${state.selected}` : "/";
  if (location.pathname !== want) history.replaceState(null, "", want + location.search + location.hash);
}
