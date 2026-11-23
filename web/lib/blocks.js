// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Block perspectives (ADR 0116, ADR 0143): perspectives, fixed views and
// searches of a block's own one below the other, like org's block agenda.
// What a single perspective shows is perspectives.js's; here the blocks
// are put together, as samu_core::blocks does it for the other clients:
// headings, blocks that show nothing because what they point at is gone,
// and a task several blocks match. testdata/blocks/sections.json holds the two to the same
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

// The plain perspective a block's own search shows as (ADR 0143 §2): an
// id of its own, so that a test made for it is kept apart from the block
// perspective's and from every other block's.
export const queryOf = (p, i) => ({ id: `${p.id}:block:${i}`, title: "", ...p.blocks[i].query });

// A block's name, icon and hits; hits null when it shows nothing this
// samu can show.
function hitsOf(p, i, perspectives, statuses, shows, name) {
  const block = p.blocks[i];
  const forms = ["perspective", "view", "query"].filter((k) => block[k]);
  if (forms.length !== 1) return { name: "", icon: null, hits: null, target: null };
  if (block.perspective) {
    const target = perspectives.find((o) => o.id === block.perspective);
    if (!target) return { name: block.perspective, icon: null, hits: null, target: null };
    if (isBlocks(target)) return { name: target.title, icon: target.icon ?? null, hits: null, target };
    return { name: target.title, icon: target.icon ?? null, hits: statuses.filter((s) => shows(target, s)), target };
  }
  if (block.view) {
    const known = knownView(block.view);
    return { name: block.view, icon: null, hits: known ? statuses.filter((s) => inView(block.view, s) === true) : null, target: null };
  }
  const query = queryOf(p, i);
  return { name: name(block.query), icon: null, hits: statuses.filter((s) => shows(query, s)), target: null, query };
}

// The sections of `p` from `statuses`, every task the scope shows in
// order; `shows(perspective, status)` says whether a plain perspective
// (or a block's own search, as `queryOf` makes it) shows a task, its
// filter included, and `name(query)` what a search without a heading is
// called. Each section: { title, icon, block, missing, tasks, target,
// query } (target: the block's perspective, query: its own search).
export function assemble(p, perspectives, statuses, shows, name = (q) => q.text ?? "") {
  if (!isBlocks(p)) {
    return [{ title: p.title, icon: p.icon ?? null, block: null, missing: false, tasks: statuses.filter((s) => shows(p, s)), target: p, query: null }];
  }
  const dedupe = p.dedupe !== false;
  const seen = new Set();
  return p.blocks.map((block, i) => {
    const { name: called, icon, hits, target, query = null } = hitsOf(p, i, perspectives, statuses, shows, name);
    const tasks = (hits ?? []).filter((s) => {
      if (!dedupe) return true;
      if (seen.has(s.task.id)) return false;
      seen.add(s.task.id);
      return true;
    });
    return { title: block.title ?? called, icon, block, missing: hits === null, tasks, target, query };
  });
}

// Every task of `sections` once, in their order (§4).
export function union(sections) {
  const seen = new Set();
  return sections.flatMap((s) => s.tasks).filter((s) => !seen.has(s.task.id) && seen.add(s.task.id));
}
