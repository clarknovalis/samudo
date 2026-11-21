// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Checklists (ADR 0141), the logic without a page: reading pasted lines
// into items and the two plans that turn an item into a subtask and back.
// A small copy of samu-core's `checklist` module, held to the same cases
// by web/tests/checklist.mjs. No imports, so node can run it.

// Bullets and numbers, once: "- ", "* ", "+ ", "• ", "1. ", "1) ".
function bullet(text) {
  for (const mark of ["- ", "* ", "+ ", "• "]) {
    if (text.startsWith(mark)) return text.slice(mark.length);
  }
  const match = /^[0-9]+(\. |\) )/.exec(text);
  return match ? text.slice(match[0].length) : null;
}

const BOXES = [
  ["[ ]", false],
  ["[x]", true],
  ["[X]", true],
  ["[-]", false],
];

function line(raw) {
  let rest = raw.trim();
  const dropped = bullet(rest);
  if (dropped !== null) rest = dropped;
  let checked = false;
  for (const [mark, ticked] of BOXES) {
    if (rest.startsWith(mark) && (rest.length === mark.length || /^\s/.test(rest.slice(mark.length)))) {
      rest = rest.slice(mark.length);
      checked = ticked;
      break;
    }
  }
  const text = rest.trim();
  return text ? { text, checked } : null;
}

// Pasted or typed text as items, one per non-empty line.
export const lines = (text) => text.split(/\r?\n/).map(line).filter(Boolean);

// The items of a task, in their order.
export const itemsOf = (items, task) =>
  items
    .filter((i) => i.owner === task)
    .sort((a, b) => a.position - b.position || a.id.localeCompare(b.id));

// [done, all]
export function progress(items, task) {
  const mine = items.filter((i) => i.owner === task);
  return [mine.filter((i) => i.checked).length, mine.length];
}

// One past the last position, 0 for none.
export const after = (positions) => (positions.length ? Math.max(...positions) + 1 : 0);

// What an item cannot keep, named; null for a plain subtask. `around` is
// what hangs on tasks as objects of their own and would go with the task:
// { items, comments, attachments, clocks, reminders, links }.
export function carries(task, around = {}) {
  if (task.note?.trim()) return "a note";
  if (["defer", "due", "planned", "due_warning", "planned_duration", "repeat"].some((k) => task[k])) return "dates";
  if ((task.tags ?? []).length) return "tags";
  if (task.estimate) return "an estimate";
  if (task.flagged) return "a flag";
  if (task.priority) return "a priority";
  if (Object.keys(task.properties ?? {}).length) return "properties";
  const mine = (list) => (list ?? []).some((o) => o.owner === task.id);
  if (mine(around.items)) return "a checklist of its own";
  if (mine(around.comments)) return "comments";
  if (mine(around.attachments)) return "attachments";
  if (mine(around.clocks)) return "clocked time";
  if (mine(around.reminders)) return "reminders";
  if (mine(around.links)) return "links";
  // Deleting it would silently unblock the task that waits.
  if ((around.links ?? []).some((l) => l.kind === "blocked_by" && l.uri === `samu:${task.id}`)) return "a task that waits for it";
  return null;
}

// The changes that make `item` a subtask `id` of its task: its text as the
// title, last among the subtasks, completed when ticked; the item goes.
// `hlc` stamps a change; the server stamps again (?stamp=server).
export function toSubtask({ tasks, items, item, id, now, hlc }) {
  const it = items.find((i) => i.id === item);
  if (!it) throw new Error("No such checklist item.");
  const owner = tasks.find((t) => t.id === it.owner);
  if (!owner) throw new Error("No such task.");
  const value = {
    id,
    created: now,
    modified: now,
    title: it.text,
    parent: owner.id,
    position: after(tasks.filter((t) => t.parent === owner.id).map((t) => t.position)),
  };
  if (owner.project) value.project = owner.project;
  if (it.checked) value.completed_at = now;
  return [
    { op: "create", object: id, kind: "task", value, hlc: hlc() },
    { op: "delete", object: item, hlc: hlc() },
  ];
}

// All items of `task` as subtasks, in their order: each is placed after the
// ones made before it, which `toSubtask` alone would not see.
export function allToSubtasks({ tasks, items, task, now, hlc, newId }) {
  const known = [...tasks];
  const changes = [];
  for (const it of itemsOf(items, task)) {
    const made = toSubtask({ tasks: known, items, item: it.id, id: newId(), now, hlc });
    known.push(made[0].value);
    changes.push(...made);
  }
  return changes;
}

// The changes that make the plain subtask `task` an item `id` of its
// parent, last, ticked when completed; the subtask goes. Throws an Error
// with the reason when it is no plain subtask. `around` as in `carries`,
// without the items.
export function fromSubtask({ tasks, items, around = {}, task, id, now, hlc }) {
  const t = tasks.find((x) => x.id === task);
  if (!t) throw new Error("No such task.");
  if (!t.parent) throw new Error("The task is not a subtask.");
  if (tasks.some((x) => x.parent === t.id)) throw new Error("The task has subtasks of its own.");
  if (t.habit) throw new Error("The task is a habit.");
  if (t.repeat) throw new Error("The task repeats.");
  if (t.dropped_at) throw new Error("The task was dropped.");
  const what = carries(t, { ...around, items });
  if (what) throw new Error(`The task carries ${what}, which an item cannot keep.`);
  const value = {
    id,
    created: now,
    modified: now,
    owner: t.parent,
    text: t.title,
    position: after(items.filter((i) => i.owner === t.parent).map((i) => i.position)),
  };
  if (t.completed_at) value.checked = true;
  return [
    { op: "create", object: id, kind: "checklist_item", value, hlc: hlc() },
    { op: "delete", object: task, hlc: hlc() },
  ];
}

// Every plain subtask of `task` as an item, in the order of the subtasks,
// each after the ones made before it. Those that carry something an item
// cannot keep stay and come back in `skipped` with the reason; throws the
// first reason when none can be converted, or when there are no subtasks.
export function allFromSubtasks({ tasks, items, around = {}, task, now, hlc, newId }) {
  const subs = tasks.filter((t) => t.parent === task).sort((a, b) => a.position - b.position || (a.id < b.id ? -1 : 1));
  const known = [...items];
  const changes = [];
  const skipped = [];
  for (const sub of subs) {
    try {
      const made = fromSubtask({ tasks, items: known, around, task: sub.id, id: newId(), now, hlc });
      known.push(made[0].value);
      changes.push(...made);
    } catch (error) {
      skipped.push({ id: sub.id, title: sub.title, reason: error.message });
    }
  }
  if (!changes.length) throw new Error(skipped[0]?.reason ?? "The task has no subtask that an item could take over.");
  return { changes, skipped };
}
