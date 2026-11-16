// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Moving a task within its project's tree (ADR 0062 §7–§10): Shift+↑/↓
// one visible place on, into an open group and out of its own, and
// dropping a row before, after or into another. Every move writes
// `parent` and `position` of the task, and of its new siblings what
// shifts, in one write; its descendants go along by their parents.

import { api, change } from "./api.js";
import { draggedTasks, forgetDrag } from "./bulk.js";
import { order, ownOrder } from "./list.js";
import { ancestors, descendants } from "./palette.js";
import { byId, isOpen, state, statusOf, update } from "./state.js";
import { tr } from "./i18n.js";

const NOT_OWN = tr("Verschieben geht nur in der eigenen Ordnung: sortiert nach Ordnung, gruppiert nach Projekt, als Liste.");

const sameSpot = (a, project, parent) => (a.project ?? null) === project && (a.parent ?? null) === parent;
const byPosition = (a, b) => a.position - b.position || a.id.localeCompare(b.id);

// Places `task` under `project` and `parent`, right before `before` (a
// task id among those siblings) or last with null. The siblings, closed
// and hidden ones too, are numbered anew from 0 where they shift.
export function placeAt(task, project, parent, before) {
  return placeMany([task], project, parent, before);
}

// The same for several tasks, in the order given (ADR 0062 §8); a task
// whose ancestor moves too stays under it (ADR 0052 §3). One write.
export async function placeMany(tasks, project, parent, before) {
  const ids = new Set(tasks.map((t) => t.id));
  const roots = tasks.filter((t) => !ancestors(t.id).some((a) => ids.has(a)));
  if (parent && roots.some((t) => parent === t.id || ancestors(parent).includes(t.id))) {
    update({ message: tr("Eine Aufgabe kann nicht unter sich selbst oder darunter.") });
    return;
  }
  const moving = new Set(roots.map((t) => t.id));
  const siblings = state.tasks.filter((t) => !moving.has(t.id) && sameSpot(t, project, parent)).sort(byPosition);
  const at = before ? siblings.findIndex((t) => t.id === before) : -1;
  const placed = [...siblings];
  placed.splice(at < 0 ? placed.length : at, 0, ...roots);
  const changes = [];
  const moved = new Set();
  for (const task of roots) {
    if ((task.project ?? null) !== project) {
      changes.push(change.set(task.id, "project", project));
      for (const t of descendants(task.id)) {
        if ((t.project ?? null) !== project) changes.push(change.set(t.id, "project", project));
      }
      moved.add(task.id);
    }
    if ((task.parent ?? null) !== parent) {
      changes.push(change.set(task.id, "parent", parent));
      moved.add(task.id);
    }
  }
  placed.forEach((t, position) => {
    if (t.position !== position || moved.has(t.id)) changes.push(change.set(t.id, "position", position));
  });
  if (!changes.length) return;
  try {
    await api.write(changes);
    update({ message: null });
  } catch (error) {
    update({ message: error.message });
  }
}

// The drawn siblings of a task: the rows of its project and parent, in the
// order drawn.
function drawnSiblings(task) {
  const project = task.project ?? null;
  const parent = task.parent ?? null;
  return order.map((id) => byId(state.tasks, id)).filter((t) => t && sameSpot(t, project, parent));
}

// An open group: a drawn task with children that is not folded.
const openGroup = (task) => Boolean(statusOf(task.id)?.group) && !state.folded.has(task.id);

// The first of the task's children, hidden ones too; null without any.
function firstChild(id) {
  return state.tasks.filter((t) => t.parent === id).sort(byPosition)[0] ?? null;
}

// The task after (`step` 1) or before (-1) the one it should stand by, to
// say where Shift+↓/↑ puts it (§7).
export function shiftTarget(task, step) {
  const siblings = drawnSiblings(task);
  const at = siblings.findIndex((t) => t.id === task.id);
  const next = at < 0 ? null : siblings[at + step] ?? null;
  const project = task.project ?? null;
  if (next && openGroup(next)) {
    // Into the group: first child going down, last going up.
    return { project, parent: next.id, before: step > 0 ? firstChild(next.id)?.id ?? null : null };
  }
  if (next) {
    // Past the neighbour: before it going up, before the one after it
    // going down (hidden siblings keep their places among themselves).
    if (step < 0) return { project, parent: task.parent ?? null, before: next.id };
    const all = state.tasks.filter((t) => t.id !== task.id && sameSpot(t, project, task.parent ?? null)).sort(byPosition);
    const after = all[all.findIndex((t) => t.id === next.id) + 1] ?? null;
    return { project, parent: task.parent ?? null, before: after?.id ?? null };
  }
  if (task.parent) {
    // Out of its group: before or after the old parent.
    const parent = byId(state.tasks, task.parent);
    if (!parent) return null;
    const grand = parent.parent ?? null;
    if (step < 0) return { project, parent: grand, before: parent.id };
    const all = state.tasks.filter((t) => t.id !== task.id && sameSpot(t, project, grand)).sort(byPosition);
    const after = all[all.findIndex((t) => t.id === parent.id) + 1] ?? null;
    return { project, parent: grand, before: after?.id ?? null };
  }
  return null;
}

// Shift+↑ (-1) and Shift+↓ (1) on a task.
export function shiftTask(status, step) {
  if (!ownOrder()) {
    update({ message: NOT_OWN });
    return null;
  }
  const target = shiftTarget(status.task, step);
  if (!target) {
    update({ message: step < 0 ? tr("Sie steht schon ganz oben.") : tr("Sie steht schon ganz unten.") });
    return null;
  }
  return placeAt(status.task, target.project, target.parent, target.before);
}

// --- Dropping onto rows (§8) -------------------------------------------

const ZONES = ["drop-before", "drop-into", "drop-after"];

export function clearZones() {
  for (const node of document.querySelectorAll(".drop-before, .drop-into, .drop-after")) node.classList.remove(...ZONES);
}

// Where on the row the pointer is: the upper third before, the lower
// after, the middle into; into a closed task never.
function zoneOf(event, task) {
  const box = event.currentTarget.getBoundingClientRect();
  const y = (event.clientY - box.top) / box.height;
  if (y < 1 / 3) return "drop-before";
  if (y > 2 / 3 || !isOpen(task)) return "drop-after";
  return "drop-into";
}

// The dragged tasks, when they may land on `target`: in the own order,
// not on or under themselves.
function landing(target) {
  const ids = draggedTasks();
  if (!ids || !ownOrder() || !target) return null;
  if (ids.includes(target.id) || ancestors(target.id).some((a) => ids.includes(a))) return null;
  return ids.map((id) => byId(state.tasks, id)).filter(Boolean);
}

// Where a drop in `zone` of the row of `target` puts the tasks.
function dropTarget(target, zone, tasks) {
  const project = target.project ?? null;
  if (zone === "drop-into") return { project, parent: target.id, before: null };
  const parent = target.parent ?? null;
  if (zone === "drop-before") return { project, parent, before: target.id };
  const moving = new Set(tasks.map((t) => t.id));
  const all = state.tasks.filter((t) => !moving.has(t.id) && sameSpot(t, project, parent)).sort(byPosition);
  return { project, parent, before: all[all.findIndex((t) => t.id === target.id) + 1]?.id ?? null };
}

// The handlers a row of the list gets; on the board and in other orders
// they let the event pass (the board's cells take it there).
export function rowDrop(id) {
  return {
    ondragover: (event) => {
      const target = byId(state.tasks, id);
      if (!landing(target)) return;
      event.preventDefault();
      event.stopPropagation();
      event.dataTransfer.dropEffect = "move";
      const zone = zoneOf(event, target);
      if (!event.currentTarget.classList.contains(zone)) {
        clearZones();
        event.currentTarget.classList.add(zone);
      }
    },
    ondragleave: (event) => {
      if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.classList.remove(...ZONES);
    },
    ondrop: (event) => {
      const target = byId(state.tasks, id);
      const tasks = landing(target);
      clearZones();
      if (!tasks) return;
      event.preventDefault();
      event.stopPropagation();
      forgetDrag();
      const to = dropTarget(target, zoneOf(event, target), tasks);
      placeMany(tasks, to.project, to.parent, to.before);
    },
  };
}
