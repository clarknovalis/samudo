// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// What a list row shows besides its title (ADR 0130): every field has a
// place, line 1, line 2 or off, kept in this browser; the fields stand in
// one fixed order, which is also the order they are cut in, and each has
// a long and a short form. Nothing here touches the page.

import { byId, state, tagTitle } from "./state.js";
import { tr } from "./i18n.js";
import { parseRule, repeatText } from "./repeat.js";
import { dueText, estimateText, localDay, placeOf, reasonText, shortDay, tagPath } from "./words.js";

// In order (ADR 0130 §2): key, the word in the menu.
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

// A new browser (ADR 0130 §8): the due mark beside the title, where it
// stood; state, tags, place and reason below it.
export const DEFAULT_LAYOUT = {
  places: { due: 1, state: 2, tags: 2, place: 2, reason: 2, defer: 0, planned: 0, estimate: 0, repeat: 0, note: 0, created: 0, modified: 0 },
  short: false,
};

// The layout from what this browser kept: `stored` is the JSON of
// `samu.rowfields`, `legacy` the old switch `samu.details` (ADR 0053),
// which adds Start, Geplant and the estimate to line 2 once. A field the
// stored layout does not know yet gets its default.
export function layoutOf(stored, legacy = null) {
  let kept = null;
  try {
    kept = stored ? JSON.parse(stored) : null;
  } catch {
    kept = null;
  }
  const places = { ...DEFAULT_LAYOUT.places };
  if (kept && typeof kept.places === "object" && kept.places) {
    for (const key of KEYS) {
      const place = kept.places[key];
      if (place === 0 || place === 1 || place === 2) places[key] = place;
    }
    return { places, short: Boolean(kept.short) };
  }
  if (legacy === "on") {
    places.defer = 2;
    places.planned = 2;
    places.estimate = 2;
  }
  return { places, short: DEFAULT_LAYOUT.short };
}

export const withPlace = (layout, key, place) => ({ ...layout, places: { ...layout.places, [key]: place } });

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

// The fields of `line` (1 or 2) for one task, in order, without the empty
// ones. `where` is what the row says of its place (21df91a1): "none"
// where the heading says it already, "project" or "full".
export function fieldParts(status, line, layout, where = "full") {
  return KEYS.filter((key) => layout.places[key] === line)
    .map((key) => part(key, status, layout.short, where))
    .filter(Boolean);
}
