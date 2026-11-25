// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// What a write changes, drawn before the server answers: the objects as
// `/changes` gave them, with the changes applied on copies, the way
// `samu_core::overlay` lays the phone's outbox over its mirror (ADR 0028
// §4). Nothing here is computed: availability, forecast and the project
// status stay what the server last said until the reload brings them
// anew, for availability is computed only in samu-core. What this does not
// know it leaves to the reload, which brings the truth either way.

// Applies `changes` to `objects`, a Map of id → { kind, object }, and
// answers a function that puts back exactly what was there before.
// `changes` are those of `change.*` in api.js, and two more the routes
// stand for: `{ op: "delete", object, below: true }` for
// POST /objects/{id}/delete (a task takes the tasks below it along, ADR
// 0036 §5), and `{ op: "close", object, field, children }` for
// POST /objects/{id}/close (ADR 0128).
export function applyLocal(objects, changes, now = new Date().toISOString()) {
  const before = new Map();
  const keep = (id) => {
    if (!before.has(id)) before.set(id, objects.get(id));
  };
  // Each entry is copied once, the first time a change touches it, so what
  // the views drew before stays as it was.
  const copied = new Set();
  const writable = (id) => {
    const entry = objects.get(id);
    if (!entry) return null;
    if (!copied.has(id)) {
      keep(id);
      objects.set(id, { ...entry, object: structuredClone(entry.object) });
      copied.add(id);
    }
    return objects.get(id).object;
  };
  for (const c of changes) {
    switch (c.op) {
      case "set": {
        const object = writable(c.object);
        if (object) setField(object, c.field, c.value);
        break;
      }
      case "add":
        add(objects, writable, c);
        break;
      case "remove": {
        const entry = objects.get(c.object);
        if (entry?.kind === "scope") {
          const scope = writable(c.object);
          scope.members = (scope.members ?? []).filter((m) => m !== c.element);
        } else if (entry?.kind === "task" || entry?.kind === "project") {
          const object = writable(c.object);
          object.tags = (object.tags ?? []).filter((t) => t.tag !== c.element);
        }
        break;
      }
      case "delete":
        for (const id of c.below ? withBelow(objects, c.object) : [c.object]) {
          keep(id);
          objects.delete(id);
        }
        break;
      case "create":
        keep(c.object);
        objects.set(c.object, { kind: c.kind, object: structuredClone(c.value) });
        copied.add(c.object);
        break;
      case "close":
        close(objects, writable, c, now);
        break;
      default:
        // Not known here: the reload shows it.
        break;
    }
  }
  // What this made, so taking it back leaves alone what a reload brought
  // in the meantime: that is the server's and newer.
  const made = new Map([...before.keys()].map((id) => [id, objects.get(id)]));
  return () => {
    for (const [id, entry] of before) {
      if (objects.get(id) !== made.get(id)) continue;
      if (entry) objects.set(id, entry);
      else objects.delete(id);
    }
  };
}

// Whether `changes` can be drawn before the server answers. Something new
// needs what only the server computes before a list or the sidebar shows
// it, and the callers pick it right after; a deleted project, tag or
// folder leaves its tasks to where the server puts them (ADR 0036 §2).
export function drawsAtOnce(objects, changes) {
  return changes.every((c) => c.op !== "create" && !(c.below && objects.get(c.object)?.kind !== "task"));
}

// A map written per key (ADR 0023, ADR 0054 §3); null takes a field away,
// as on the server.
function setField(object, field, value) {
  const keyed = ["properties", "property_links"].find((name) => field.startsWith(`${name}.`));
  if (keyed) {
    const key = field.slice(keyed.length + 1);
    const inner = { ...(object[keyed] ?? {}) };
    if (value === null) delete inner[key];
    else inner[key] = value;
    object[keyed] = inner;
  } else if (value === null) delete object[field];
  else object[field] = value;
}

// A member of a scope, or a tag of a task or project; a state of a
// workflow replaces its siblings (ADR 0024 §1).
function add(objects, writable, c) {
  const entry = objects.get(c.object);
  if (entry?.kind === "scope") {
    const scope = writable(c.object);
    const members = scope.members ?? [];
    if (!members.includes(c.element)) scope.members = [...members, c.element];
    return;
  }
  if (entry?.kind !== "task" && entry?.kind !== "project") return;
  const parent = objects.get(c.element)?.object.parent;
  const siblings = new Set();
  if (parent && objects.get(parent)?.object.exclusive) {
    for (const [id, other] of objects) {
      if (other.kind === "tag" && other.object.parent === parent && id !== c.element) siblings.add(id);
    }
  }
  const object = writable(c.object);
  const tags = (object.tags ?? []).filter((t) => !siblings.has(t.tag));
  if (!tags.some((t) => t.tag === c.element)) tags.push({ tag: c.element, position: c.position ?? 0 });
  object.tags = tags;
}

// Completed or dropped now; what is open below it closed along or lifted
// to the task's own place (ADR 0128 §3). The positions the server gives
// the lifted ones come with the reload.
function close(objects, writable, c, now) {
  const task = objects.get(c.object);
  if (task?.kind !== "task") return;
  const field = c.field === "dropped" ? "dropped_at" : "completed_at";
  const open = (id) => {
    const o = objects.get(id)?.object;
    return o && !o.completed_at && !o.dropped_at;
  };
  if (c.children === "close") {
    for (const id of withBelow(objects, c.object)) if (open(id)) writable(id)[field] = now;
    return;
  }
  // `ask` with open ones below writes nothing (ADR 0128 §1).
  if (c.children !== "lift" && withBelow(objects, c.object).slice(1).some(open)) return;
  if (c.children === "lift") {
    const parent = task.object.parent ?? null;
    for (const [id, entry] of objects) {
      if (entry.kind === "task" && entry.object.parent === c.object && open(id)) {
        setField(writable(id), "parent", parent);
      }
    }
  }
  writable(c.object)[field] = now;
}

// A task with the tasks below it; anything else alone.
function withBelow(objects, id) {
  const out = [id];
  if (objects.get(id)?.kind !== "task") return out;
  const ids = new Set(out);
  let grew = true;
  while (grew) {
    grew = false;
    for (const [other, entry] of objects) {
      if (entry.kind === "task" && entry.object.parent && ids.has(entry.object.parent) && !ids.has(other)) {
        ids.add(other);
        out.push(other);
        grew = true;
      }
    }
  }
  return out;
}
