// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The columns of a board and where each task stands in them (ADR 0061).
// Pure: no DOM, no state of the page; board.js hands in what it needs, so
// the rules can be checked on their own.

import { tr } from "./i18n.js";

// How far back the column tr("Erledigt") reaches (§6).
export const DONE_DAYS = 7;

const byPosition = (a, b) => a.position - b.position || a.id.localeCompare(b.id);

// The columns in order: tr("Ohne") first when wanted, then a workflow's states
// (§2) or the chosen tags (§3), then tr("Erledigt") when wanted (§6). Each is
// { key, title, tag, paused }; `key` is the tag id, "none" or "done".
export function boardColumns(config, tags, workflow, nameOf = (t) => t.title) {
  const byId = new Map(tags.map((t) => [t.id, t]));
  const columns = [];
  const tagged = config.source === "tags";
  if (config.none !== false) columns.push({ key: "none", title: tagged ? tr("Ohne") : tr("Ohne Zustand"), tag: null, paused: false });
  const chosen = tagged
    ? [...new Set(config.tags ?? [])].map((id) => byId.get(id)).filter((t) => t && t.status !== "dropped")
    : tags.filter((t) => workflow && t.parent === workflow && t.status !== "dropped").sort(byPosition);
  for (const t of chosen) columns.push({ key: t.id, title: tagged ? nameOf(t) : t.title, tag: t.id, paused: t.status === "on_hold" });
  if (config.done) columns.push({ key: "done", title: tr("Erledigt"), tag: null, paused: false });
  return columns;
}

export const recentlyDone = (task, now) =>
  Boolean(task.completed_at) && !task.dropped_at && Date.parse(task.completed_at) >= now - DONE_DAYS * 86400000;

// The keys of the columns a task stands in: one for a state, every column
// tag it carries (§3), "foreign" for a state no column has by title (§2),
// none at all for what the board does not show.
export function placeTask(status, columns, config, tagsById, now) {
  const task = status.task;
  const has = (key) => columns.some((c) => c.key === key);
  if (task.dropped_at) return [];
  if (task.completed_at) return has("done") && recentlyDone(task, now) ? ["done"] : [];
  const tagged = columns.filter((c) => c.tag);
  if (config.source === "tags") {
    const carried = new Set((task.tags ?? []).map((t) => t.tag));
    const hits = tagged.filter((c) => carried.has(c.tag)).map((c) => c.key);
    if (hits.length) return hits;
  } else if (status.state) {
    const own = tagged.find((c) => c.tag === status.state);
    if (own) return [own.key];
    const title = tagsById.get(status.state)?.title;
    const same = tagged.find((c) => c.title === title);
    return [same ? same.key : "foreign"];
  }
  return has("none") ? ["none"] : [];
}

// What moving tasks from column `from` to column `to` writes (§4, §6):
// { changes, missed, asks }. A change is { op: "add" | "remove", id, tag }
// or { op: "set", id, field, value }; `missed` counts tasks whose workflow
// has no state of that title, `asks` the state that wants a note, if any.
// `statesFor(status)` gives the states of the task's own workflow.
export function moveChanges(statuses, from, to, config, columns, statesFor, now) {
  const result = { changes: [], missed: 0, asks: null };
  if (from === to) return result;
  const target = columns.find((c) => c.key === to);
  if (!target) return result;
  const tagged = columns.filter((c) => c.tag);
  const at = new Date(now).toISOString();
  for (const status of statuses) {
    const task = status.task;
    if (task.dropped_at) continue;
    const id = task.id;
    if (to === "done") {
      if (!task.completed_at) result.changes.push({ op: "set", id, field: "completed_at", value: at });
      continue;
    }
    const reopen = task.completed_at ? [{ op: "set", id, field: "completed_at", value: null }] : [];
    if (config.source === "tags") {
      const carried = new Set((task.tags ?? []).map((t) => t.tag));
      const out =
        to === "none"
          ? tagged.filter((c) => carried.has(c.tag)).map((c) => c.tag)
          : from !== "none" && from !== "done" && from !== to && carried.has(from)
            ? [from]
            : [];
      const into = target.tag && !carried.has(target.tag) ? [{ op: "add", id, tag: target.tag }] : [];
      result.changes.push(...reopen, ...out.map((tag) => ({ op: "remove", id, tag })), ...into);
      continue;
    }
    if (to === "none") {
      result.changes.push(...reopen);
      if (status.state) result.changes.push({ op: "remove", id, tag: status.state });
      continue;
    }
    // A state of the board's workflow, or the one of the same title in the
    // task's own (§2).
    const state = statesFor(status).find((t) => t.id === target.tag) ?? statesFor(status).find((t) => t.title === target.title);
    if (!state) {
      result.missed += 1;
      continue;
    }
    result.changes.push(...reopen);
    if (state.id !== status.state) {
      result.changes.push({ op: "add", id, tag: state.id });
      if (state.ask_note) result.asks = state.title;
    }
  }
  return result;
}

// The column after or before `key`, for Alt+←/→ (§4); null at the edge.
export function neighbour(columns, key, step) {
  const at = columns.findIndex((c) => c.key === key);
  if (at < 0) return null;
  return columns[at + step]?.key ?? null;
}
