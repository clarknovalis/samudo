// Making and moving projects, folders and tags (ADR 0037 §1, ADR 0041).
// The settings and the sidebar both go through here, so there is one way
// to the server: ordinary changes over /changes, in one write each.

import { api, change } from "./api.js";
import { el, uuidv7 } from "./dom.js";
import { state, update } from "./state.js";
import { tr } from "./i18n.js";

// Where each kind hangs: projects in a folder, folders and tags under a
// parent of their own kind.
const PLACE = {
  project: { list: () => state.projects, field: "folder" },
  folder: { list: () => state.folders, field: "parent" },
  tag: { list: () => state.tags, field: "parent" },
};

export const byPosition = (a, b) => a.position - b.position || a.title.localeCompare(b.title);
export const nextPosition = (list) => list.reduce((max, o) => Math.max(max, o.position + 1), 0);
export const placeOf = (object, kind) => object[PLACE[kind].field] ?? null;
const siblings = (kind, at) => PLACE[kind].list().filter((o) => placeOf(o, kind) === at);
const now = () => new Date().toISOString();

// `id` and everything below it in a tree of `list`.
export function subtree(list, id) {
  const out = new Set([id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const o of list) {
      if (o.parent && out.has(o.parent) && !out.has(o.id)) {
        out.add(o.id);
        grew = true;
      }
    }
  }
  return out;
}

async function write(changes) {
  try {
    await api.write(changes);
    return true;
  } catch (error) {
    update({ message: error.message });
    return false;
  }
}

// A new `kind` called `title` at `at`, at `position`, as created on the wire.
function placeValue(kind, title, at, position) {
  const created = now();
  const value = { id: uuidv7(), created, modified: created, title, status: "active", position };
  if (kind === "project") Object.assign(value, { kind: "parallel", flagged: false });
  if (kind === "tag") Object.assign(value, { exclusive: false, ask_note: false, default_workflow: false });
  if (at) value[PLACE[kind].field] = at;
  return value;
}

// Makes a `kind` called `title` at `at` (a folder or tag id, or null for
// the top), last there. The id, or null when the server said no.
export async function createPlace(kind, title, at) {
  const value = placeValue(kind, title, at, nextPosition(siblings(kind, at)));
  return (await write([change.create(kind, value)])) ? value.id : null;
}

// The tag a tag picker means by `path` (`Büro` or `Ort/Büro`), with the
// changes that make whatever step is missing (92af4fff); nothing is
// written here. The same rule as samu-core's `tag_at_path`: steps match
// by whole title ignoring case, a single name at every level, a workflow
// or its state is refused. Throws an Error with a message to show.
// A slash in parentheses would read as a server call to the route test.
const SLASH = "/";

export function tagChanges(path) {
  const steps = path.split(SLASH).map((s) => s.trim()).filter(Boolean);
  if (!steps.length) throw new Error(tr("Ein Tag braucht einen Namen."));
  const pool = state.tags.filter((t) => t.status !== "dropped");
  const changes = [];
  let at = null;
  for (const [i, step] of steps.entries()) {
    const hits = pool.filter(
      (t) => (steps.length === 1 || (t.parent ?? null) === at) && t.title.toLowerCase() === step.toLowerCase(),
    );
    if (hits.length > 1) throw new Error(tr("„{step}“ meint mehr als einen Tag; als Pfad schreiben, etwa Ort/{step}.", { step }));
    if (hits.length === 1) {
      at = hits[0].id;
      continue;
    }
    if (pool.find((t) => t.id === at)?.exclusive) throw new Error(tr("Unter „{path}“ liegen nur Zustände eines Ablaufs.", { path: steps.slice(0, i).join(SLASH) }));
    const value = placeValue("tag", step, at, nextPosition(pool.filter((t) => (t.parent ?? null) === at)));
    pool.push(value);
    changes.push(change.create("tag", value));
    at = value.id;
  }
  const tag = pool.find((t) => t.id === at);
  if (tag.exclusive || pool.find((t) => t.id === tag.parent)?.exclusive) {
    throw new Error(tr("„{path}“ ist ein Ablauf oder sein Zustand, kein Tag zum Setzen.", { path: steps.join(SLASH) }));
  }
  return { id: at, changes };
}

// The field a tag picker shows after „Neuer Tag …“: Enter calls
// `make(id, changes)` with what `tagChanges` found, Escape calls
// `cancel()`. `id` names it; a redraw while typing keeps the text and the
// focus (live updates draw the panes anew).
export function newTagField(id, make, cancel) {
  const kept = typed.get(id);
  const input = el("input", {
    id,
    type: "text",
    class: "new-tag",
    placeholder: tr("Name oder Ort/Name, Enter"),
    "aria-label": tr("Neuer Tag"),
    autocomplete: "off",
    value: kept?.value ?? "",
    oninput: (event) => typed.set(id, { value: event.target.value, focused: true }),
    onfocus: (event) => typed.set(id, { value: event.target.value, focused: true }),
    // Taken out by a redraw, the field loses focus to nothing; the next one
    // takes it back. Leaving it for another field does not.
    onblur: (event) => typed.set(id, { value: event.target.value, focused: !event.relatedTarget }),
    onkeydown: (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        let found;
        try {
          found = tagChanges(event.target.value);
        } catch (error) {
          update({ message: error.message });
          return;
        }
        typed.delete(id);
        make(found.id, found.changes);
      } else if (event.key === "Escape") {
        event.stopPropagation();
        typed.delete(id);
        cancel();
      }
    },
  });
  if (!kept || kept.focused) {
    setTimeout(() => {
      if (!input.isConnected) return;
      input.focus();
      input.setSelectionRange(input.value.length, input.value.length);
    });
  }
  return input;
}
const typed = new Map();

// Moves `object` to `target` (null for the top). Without `before` it goes
// last there; with it, it goes in front of that sibling, and the siblings
// are counted anew, the ones a scope hides among them (ADR 0041 §6).
export async function movePlace(object, kind, target, before = null) {
  const to = target || null;
  if (kind !== "project" && to && subtree(PLACE[kind].list(), object.id).has(to)) {
    update({ message: tr("Ein Ordner kann nicht in sich selbst oder darunter.") });
    return false;
  }
  const field = PLACE[kind].field;
  const moved = placeOf(object, kind) !== to;
  const others = siblings(kind, to)
    .filter((o) => o.id !== object.id)
    .sort(byPosition);
  const at = before ? others.findIndex((o) => o.id === before) : -1;
  if (at < 0) {
    if (!moved) return true;
    return write([change.set(object.id, field, to), change.set(object.id, "position", nextPosition(others))]);
  }
  const order = [...others.slice(0, at), object, ...others.slice(at)];
  const changes = moved ? [change.set(object.id, field, to)] : [];
  order.forEach((o, i) => {
    if (o.position !== i || o.id === object.id) changes.push(change.set(o.id, "position", i));
  });
  return write(changes);
}

export async function renamePlace(object, title) {
  const clean = title.trim();
  if (!clean || clean === object.title) return true;
  return write([change.set(object.id, "title", clean)]);
}
