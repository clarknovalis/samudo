// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// What a list row shows besides its title (ADR 0130, 0131): line 1 is a
// list of fields with the title as a fixed piece in it, line 2 another
// list, and what neither names is off; kept in this browser. Order on the
// screen is order in the list, and what stands last is cut first. Each
// field has a long and a short form. Nothing here touches the page.

import { byId, state, tagTitle } from "./state.js";
import { tr } from "./i18n.js";
import { parseRule, repeatText } from "./repeat.js";
import { dueText, estimateText, localDay, placeOf, reasonText, shortDay, tagPath } from "./words.js";

// Every field: key, the word in the menu. The order is ADR 0130's, by
// which a layout of that ADR is read (ADR 0131 §7).
export const FIELDS = [
  ["due", tr("Fällig")],
  ["state", tr("Zustand")],
  ["tags", tr("Tags")],
  ["place", tr("Ort")],
  ["reason", tr("Grund")],
  ["defer", tr("Start")],
  ["planned", tr("Geplant")],
  ["estimate", tr("Schätzung")],
  ["repeat", tr("Wiederholung")],
  ["note", tr("Notiz")],
  ["created", tr("erstellt")],
  ["modified", tr("geändert")],
];

const KEYS = FIELDS.map(([key]) => key);
export const TITLE = "title";

// A new browser (ADR 0131 §6): the state before the title, the due mark
// beside it; tags, place and reason below.
export const DEFAULT_LAYOUT = {
  line1: ["state", TITLE, "due"],
  line2: ["tags", "place", "reason"],
  short: false,
};

const copy = (layout) => ({ line1: [...layout.line1], line2: [...layout.line2], short: layout.short });

// A layout as it counts: known keys only, each once, the title once in
// line 1 (at its start when it was missing).
function settled(line1, line2, short) {
  const seen = new Set();
  const keep = (list, title) =>
    (Array.isArray(list) ? list : []).filter((key) => {
      const ok = (KEYS.includes(key) || (title && key === TITLE)) && !seen.has(key);
      if (ok) seen.add(key);
      return ok;
    });
  const one = keep(line1, true);
  if (!one.includes(TITLE)) one.unshift(TITLE);
  return { line1: one, line2: keep(line2, false), short: Boolean(short) };
}

// The layout from what this browser kept: `stored` is the JSON of
// `samu.rowfields`, `legacy` the old switch `samu.details` (ADR 0053),
// which adds Start, Geplant and the estimate to line 2. A layout of ADR
// 0130 (`places`, 1 or 2 per field) is read once: its fields in its order,
// a state that was on before the title (ADR 0131 §7).
export function layoutOf(stored, legacy = null) {
  let kept = null;
  try {
    kept = stored ? JSON.parse(stored) : null;
  } catch {
    kept = null;
  }
  if (kept && Array.isArray(kept.line1)) return settled(kept.line1, kept.line2, kept.short);
  if (kept && typeof kept.places === "object" && kept.places) {
    const on = (n) => KEYS.filter((key) => kept.places[key] === n && key !== "state");
    const state = kept.places.state === 1 || kept.places.state === 2 ? ["state"] : [];
    return settled([...state, TITLE, ...on(1)], on(2), kept.short);
  }
  const layout = copy(DEFAULT_LAYOUT);
  if (legacy === "on") layout.line2.push("defer", "planned", "estimate");
  return layout;
}

// Where a field stands: 1, 2, or 0 when it is off.
export const lineOf = (layout, key) => (layout.line1.includes(key) ? 1 : layout.line2.includes(key) ? 2 : 0);

// The layout with `key` moved into line `to` (1, 2, or 0 for off) at
// `index` (the end when it is beyond). The title stays in line 1.
export function move(layout, key, to, index = Infinity) {
  if (key === TITLE && to !== 1) return layout;
  const next = copy(layout);
  next.line1 = next.line1.filter((k) => k !== key);
  next.line2 = next.line2.filter((k) => k !== key);
  if (to === 1 || to === 2) {
    const list = to === 1 ? next.line1 : next.line2;
    list.splice(Math.max(0, Math.min(index, list.length)), 0, key);
  }
  return next;
}

// "PT1H30M" → "1h30", "PT45M" → "45m".
function estimateShort(period) {
  const m = /^PT(?:(\d+)H)?(?:(\d+)M)?$/.exec(period ?? "");
  if (!m) return period ?? "";
  if (m[1] && m[2]) return `${m[1]}h${m[2]}`;
  return m[1] ? `${m[1]}h` : `${m[2] ?? 0}m`;
}

const UNIT = { DAILY: "d", WEEKLY: "w", MONTHLY: "M", YEARLY: "y" };

// "FREQ=WEEKLY;INTERVAL=2" → "2w"; a rule without a known frequency only ↻.
function repeatShort(repeat) {
  const form = parseRule(repeat.rule);
  return UNIT[form.freq] ? `${form.interval}${UNIT[form.freq]}` : "";
}

// The first line of the note that says something, cut at 60 characters.
function noteLine(note) {
  const line = (note ?? "")
    .split("\n")
    .map((l) => l.trim())
    .find(Boolean);
  if (!line) return null;
  return line.length > 60 ? `${line.slice(0, 60)}…` : line;
}

// One field of one task, or null when it has nothing to say: `text` as
// shown, `label` its long form for a hover and a screen reader, `kind`
// for the style (`due-overdue`, `tags`, …).
function part(key, status, short, where) {
  const task = status.task;
  const open = !task.completed_at && !task.dropped_at;
  const make = (long, brief, kind = null) => ({ key, text: short ? brief : long, label: long, kind });
  switch (key) {
    case "due": {
      const due = open ? dueText(status) : null;
      // Short, only the day: the colour says overdue or soon.
      return due ? make(due.text, shortDay(task.due), `due-${due.kind}`) : null;
    }
    case "state":
      return status.state ? make(tagTitle(status.state), tagTitle(status.state), "state") : null;
    case "tags": {
      const tags = (task.tags ?? [])
        .filter((t) => t.tag !== status.state)
        .map((t) => byId(state.tags, t.tag))
        .filter(Boolean);
      if (!tags.length) return null;
      // `items`: one chip per tag in the short form.
      return { ...make(tags.map((tag) => `@${tagPath(tag)}`).join(" "), tags.map((tag) => `@${tag.title}`).join(" "), "tags"), items: tags.map((tag) => tag.title) };
    }
    case "place":
      return where === "none" ? null : make(placeOf(task, where === "full"), placeOf(task, false), "place");
    case "reason": {
      const reason = open && !status.available ? status.reasons?.[0] : null;
      return reason ? make(reasonText(reason), `⊘ ${reasonText(reason)}`, "reason") : null;
    }
    case "defer":
      return task.defer ? make(`${tr("Start")} ${shortDay(task.defer)}`, `▶ ${shortDay(task.defer)}`) : null;
    case "planned":
      return task.planned ? make(`${tr("Geplant")} ${shortDay(task.planned)}`, `◷ ${shortDay(task.planned)}`) : null;
    case "estimate":
      return task.estimate ? make(`~${estimateText(task.estimate)}`, `~${estimateShort(task.estimate)}`) : null;
    case "repeat":
      return task.repeat ? make(`↻ ${repeatText(task.repeat)}`, `↻${repeatShort(task.repeat)}`) : null;
    case "note": {
      const line = noteLine(task.note);
      return line ? make(line, `¶ ${line}`, "note") : null;
    }
    case "created":
      return task.created ? make(tr("erstellt {day}", { day: shortDay({ date: localDay(task.created) }) }), `+${shortDay({ date: localDay(task.created) })}`) : null;
    case "modified":
      return task.modified ? make(tr("geändert {day}", { day: shortDay({ date: localDay(task.modified) }) }), `Δ${shortDay({ date: localDay(task.modified) })}`) : null;
    default:
      return null;
  }
}

// The fields of one part of a row, in order, without the empty ones:
// `slot` is "before" or "after" the title in line 1, or 2. `where` is
// what the row says of its place (21df91a1): "none" where the heading
// says it already, "project" or "full".
export function fieldParts(status, slot, layout, where = "full") {
  const at = layout.line1.indexOf(TITLE);
  const keys = slot === 2 ? layout.line2 : slot === "before" ? layout.line1.slice(0, at) : layout.line1.slice(at + 1);
  return keys.map((key) => part(key, status, layout.short, where)).filter(Boolean);
}
