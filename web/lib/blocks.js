// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Block perspectives (ADR 0116): several perspectives and fixed views one
// below the other, like org's block agenda. What a single perspective
// shows is perspectives.js's; here the blocks are put together, as
// samu_core::blocks does it for the other clients: headings, blocks that
// show nothing because what they point at is gone, and a task several
// blocks match. testdata/blocks/sections.json holds the two to the same
// answers. No DOM here, so node can test it.

// The fixed views a block may show (§2).
export const VIEWS = ["inbox", "today", "flagged"];
export const knownView = (view) => VIEWS.includes(view);

const isOpen = (task) => !task.completed_at && !task.dropped_at;

// Whether a fixed view shows a task, as its list does with the default
// filter: the inbox and Markiert what is open and available, Heute, a
// view of time, what is open. Null for a view this samu does not know.
export function inView(view, status) {
  const open = isOpen(status.task);
  switch (view) {
    case "inbox":
      return open && Boolean(status.available) && Boolean(status.inbox);
    case "today":
      return open && Boolean(status.today);
    case "flagged":
      return open && Boolean(status.available) && Boolean(status.task.flagged);
    default:
      return null;
  }
}

export const isBlocks = (p) => Boolean(p?.blocks?.length);

// Whether a task passes a filter: available, remaining or all.
export function passes(filter, status) {
  if (filter === "available") return isOpen(status.task) && Boolean(status.available);
  if (filter === "remaining") return isOpen(status.task);
  return true;
}

// A block's name, icon and hits; hits null when it shows nothing this
// samu can show.
function hitsOf(block, perspectives, statuses, shows) {
  if (block.perspective && !block.view) {
    const p = perspectives.find((o) => o.id === block.perspective);
    if (!p) return { name: block.perspective, icon: null, hits: null, target: null };
    if (isBlocks(p)) return { name: p.title, icon: p.icon ?? null, hits: null, target: p };
    return { name: p.title, icon: p.icon ?? null, hits: statuses.filter((s) => shows(p, s)), target: p };
  }
  if (block.view && !block.perspective) {
    const known = knownView(block.view);
    return { name: block.view, icon: null, hits: known ? statuses.filter((s) => inView(block.view, s) === true) : null, target: null };
  }
  return { name: "", icon: null, hits: null, target: null };
}

// The sections of `p` from `statuses`, every task the scope shows in
// order; `shows(perspective, status)` says whether a plain perspective
// shows a task, its filter included. Each section: { title, icon, block,
// missing, tasks, target } (target: the block's perspective, if any).
export function assemble(p, perspectives, statuses, shows) {
  if (!isBlocks(p)) {
    return [{ title: p.title, icon: p.icon ?? null, block: null, missing: false, tasks: statuses.filter((s) => shows(p, s)), target: p }];
  }
  const dedupe = p.dedupe !== false;
  const seen = new Set();
  return p.blocks.map((block) => {
    const { name, icon, hits, target } = hitsOf(block, perspectives, statuses, shows);
    const tasks = (hits ?? []).filter((s) => {
      if (!dedupe) return true;
      if (seen.has(s.task.id)) return false;
      seen.add(s.task.id);
      return true;
    });
    return { title: block.title ?? name, icon, block, missing: hits === null, tasks, target };
  });
}

// Every task of `sections` once, in their order (§4).
export function union(sections) {
  const seen = new Set();
  return sections.flatMap((s) => s.tasks).filter((s) => !seen.has(s.task.id) && seen.add(s.task.id));
}
