// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// What closing tasks has to ask about (ADR 0128): the open tasks below
// them. Pure, on the tasks given, so web/tests/close.mjs runs it in Node;
// the question and the writes are in closeask.js.

const isOpen = (task) => !task.completed_at && !task.dropped_at;

// The open tasks below `id` at any depth.
export function openBelow(tasks, id) {
  const out = [];
  const under = new Set([id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const t of tasks) {
      if (t.parent && under.has(t.parent) && !under.has(t.id)) {
        under.add(t.id);
        if (isOpen(t)) out.push(t);
        grew = true;
      }
    }
  }
  return out;
}

// Those of `ids` with open tasks below that are not among `ids`
// themselves, each with how many: [{ id, open }]. As `samu bulk done`
// counts them.
export function waiting(tasks, ids) {
  const picked = new Set(ids);
  const out = [];
  for (const id of ids) {
    const open = openBelow(tasks, id).filter((t) => !picked.has(t.id)).length;
    if (open) out.push({ id, open });
  }
  return out;
}

// How many groups are above `id`; a cycle in broken data stops it.
function depth(tasks, id) {
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const seen = new Set([id]);
  let n = 0;
  let at = byId.get(id)?.parent;
  while (at && !seen.has(at)) {
    seen.add(at);
    n += 1;
    at = byId.get(at)?.parent;
  }
  return n;
}

// `ids` with the deepest first, so a group closes after its picked
// children; the same depth keeps its order.
export function deepestFirst(tasks, ids) {
  const depths = new Map(ids.map((id) => [id, depth(tasks, id)]));
  return [...ids].sort((a, b) => depths.get(b) - depths.get(a));
}
