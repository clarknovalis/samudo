// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// What the server computed, in words (ADR 0020 §2, §5). Nothing here
// decides anything; it only says it.

import { byId, state, tagTitle } from "./state.js";
import { lang, locale, tr } from "./i18n.js";

const weekday = new Intl.DateTimeFormat(locale(), { weekday: "short" });
// In English "Wed 4 Nov" as the browser's English writes it (ADR 0105 §2).
const englishDay = new Intl.DateTimeFormat(locale(), { weekday: "short", day: "numeric", month: "short" });

// The local date of a `when` ({date} or {at}) as YYYY-MM-DD.
export function dayOf(when) {
  if (!when) return null;
  if (when.date) return when.date;
  if (when.at) return when.at.slice(0, 10);
  return null;
}

export function today() {
  const now = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

// The local date of an ISO timestamp like `created`, as YYYY-MM-DD.
export function localDay(stamp) {
  if (!stamp) return null;
  const date = new Date(stamp);
  if (Number.isNaN(date.getTime())) return null;
  const pad = (n) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

// A tag's or folder's path, as typed after `@`: `ort/büro`.
const STEP = "/";

export function pathOf(list, object) {
  const parts = [object.title];
  let parent = object.parent ? byId(list, object.parent) : null;
  for (let depth = 0; parent && depth < 10; depth += 1) {
    parts.unshift(parent.title);
    parent = parent.parent ? byId(list, parent.parent) : null;
  }
  return parts.join(STEP);
}

export const tagPath = (tag) => pathOf(state.tags, tag);

// Where a task lives (ADR 0026 §7): "Projekt · Ordner/Unterordner", the
// folder with its whole path; "Inbox" without a project, or with one that
// is gone (ADR 0036 §6). Without the folder when `folder` is false.
export function placeOf(task, folder = true) {
  const project = task.project ? byId(state.projects, task.project) : null;
  if (!project) return "Inbox";
  const where = folder && project.folder ? byId(state.folders, project.folder) : null;
  return [project.title, where ? pathOf(state.folders, where) : null].filter(Boolean).join(" · ");
}

export function addDays(day, days) {
  const date = new Date(`${day}T12:00:00`);
  date.setDate(date.getDate() + days);
  const pad = (n) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

// "Mi 04.11." or "Mi 04.11. 19:30".
export function shortDay(when) {
  const day = dayOf(when);
  if (!day) return "";
  const date = new Date(`${day}T12:00:00`);
  const text = lang() === "en" ? englishDay.format(date) : `${weekday.format(date)} ${day.slice(8, 10)}.${day.slice(5, 7)}.`;
  return when.at ? `${text} ${when.at.slice(11, 16)}` : text;
}

export function dueText(status) {
  const due = status.task.due;
  if (!due) return null;
  if (status.due === "overdue") return { text: `${tr("überfällig")} · ${shortDay(due)}`, kind: "overdue" };
  if (status.due === "soon") return { text: `${tr("bald fällig")} · ${shortDay(due)}`, kind: "soon" };
  return { text: tr("fällig {day}", { day: shortDay(due) }), kind: "due" };
}

const titleOfTask = (id) => state.statuses.find((s) => s.task.id === id)?.task.title ?? "?";

export function reasonText(reason) {
  switch (reason.reason) {
    case "completed":
      return tr("erledigt");
    case "dropped":
      return tr("verworfen");
    case "deferred":
      return tr("zurückgestellt bis {day}", { day: shortDay(reason.until) });
    case "project_not_active":
      return tr("Projekt ruht: {name}", { name: byId(state.projects, reason.project)?.title ?? "?" });
    case "folder_dropped":
      return tr("Ordner verworfen: {name}", { name: byId(state.folders, reason.folder)?.title ?? "?" });
    case "tag_not_active":
      return tagTitle(reason.tag);
    case "waiting_in_sequence":
      return tr("wartet: erst „{title}“", { title: titleOfTask(reason.first) });
    case "has_open_children":
      return tr("Gruppe: erst die Kinder");
    case "container_unavailable":
      return tr("Gruppe darüber nicht verfügbar");
    case "blocked_by":
      return tr("blockiert von „{title}“", { title: titleOfTask(reason.task) });
    case "cycle":
      return tr("Zyklus in den Daten");
    default:
      return tr("unbekannter Grund");
  }
}

export const logWords = {
  state: tr("Zustand"),
  completed: tr("erledigt"),
  dropped: tr("verworfen"),
  reopened: tr("wieder offen"),
};

export function stamp(text) {
  const date = new Date(text);
  return Number.isNaN(date.getTime())
    ? text
    : date.toLocaleString(locale(), { dateStyle: "medium", timeStyle: "short" });
}

// The second line of a row (ADR 0053 §3): what is set of Start, Fällig,
// Geplant and the estimate; empty when none is.
export function detailsText(task) {
  return [
    task.priority ? `${tr("Priorität")} ${task.priority}` : null,
    task.defer ? `${tr("Start")} ${shortDay(task.defer)}` : null,
    task.due ? `${tr("Fällig")} ${shortDay(task.due)}` : null,
    task.planned ? `${tr("Geplant")} ${shortDay(task.planned)}` : null,
    task.estimate ? `~${estimateText(task.estimate)}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

// What is typed as an estimate, as the wire wants it: "90", "90m", "1h",
// "1h30", "1h 30m", "1:30", "1,5h" and "PT1H30M" → "PT1H30M"; "" → null
// (none); undefined when it is not a length.
export function parseEstimate(text) {
  const t = (text ?? "").trim().toLowerCase().replace(/\s+/g, "");
  if (!t) return null;
  let minutes;
  let m;
  if ((m = /^pt(?:(\d+)h)?(?:(\d+)m)?$/.exec(t)) && (m[1] || m[2])) minutes = Number(m[1] ?? 0) * 60 + Number(m[2] ?? 0);
  else if ((m = /^(\d+):(\d{1,2})$/.exec(t))) minutes = Number(m[1]) * 60 + Number(m[2]);
  else if ((m = /^(\d+(?:[.,]\d+)?)(?:h|std)$/.exec(t))) minutes = Math.round(Number(m[1].replace(",", ".")) * 60);
  else if ((m = /^(\d+)h(\d+)(?:m|min)?$/.exec(t))) minutes = Number(m[1]) * 60 + Number(m[2]);
  else if ((m = /^(\d+)(?:m|min)?$/.exec(t))) minutes = Number(m[1]);
  else return undefined;
  if (!(minutes > 0)) return undefined;
  const h = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return `PT${h ? `${h}H` : ""}${rest ? `${rest}M` : ""}`;
}

// "PT1H30M" → "1 h 30 min".
export function estimateText(period) {
  const m = /^PT(?:(\d+)H)?(?:(\d+)M)?$/.exec(period ?? "");
  if (!m) return period ?? "";
  return [m[1] ? `${m[1]} h` : null, m[2] ? `${m[2]} min` : null].filter(Boolean).join(" ");
}

// A length that may run to days, for a reminder's `before` (ADR 0030 §1):
// "1d", "2 Tage", "1d2h", "1 Woche", "90min", "P1D", and everything
// parseEstimate takes → "P1D", "P1DT2H", "PT1H30M"; "" → null (at the
// date itself); undefined when it is not a length.
const UNITS = [
  [/^(w|wo|wochen?|weeks?)$/, "W"],
  [/^(d|t|tage?n?|days?)$/, "D"],
  [/^(h|std|stunden?|hours?|hrs?)$/, "H"],
  [/^(m|min|minuten?|mins?|minutes?)$/, "M"],
];

export function parsePeriod(text) {
  const t = (text ?? "").trim().toLowerCase();
  if (!t) return null;
  if (/^p(?=\d|t\d)(\d+w)?(\d+d)?(t(?=\d)(\d+h)?(\d+m)?)?$/.test(t)) return t.toUpperCase();
  const amounts = {};
  const tokens = [...t.matchAll(/(\d+)\s*([a-zäöü]+)\s*/g)];
  if (tokens.length && tokens.map((m) => m[0]).join("") === t.replace(/^\s+/, "")) {
    for (const [, n, word] of tokens) {
      const unit = UNITS.find(([re]) => re.test(word))?.[1];
      if (!unit || unit in amounts) return undefined;
      amounts[unit] = Number(n);
    }
  } else {
    const short = parseEstimate(t);
    if (!short) return short;
    const m = /^PT(?:(\d+)H)?(?:(\d+)M)?$/.exec(short);
    amounts.H = Number(m[1] ?? 0);
    amounts.M = Number(m[2] ?? 0);
  }
  const part = (unit) => (amounts[unit] ? `${amounts[unit]}${unit}` : "");
  const date = part("W") + part("D");
  const time = part("H") + part("M");
  if (!date && !time) return undefined;
  return `P${date}${time ? `T${time}` : ""}`;
}

// "P1DT2H" → "1 Tag 2 h"; what it cannot read, as it is.
export function periodText(period) {
  const m = /^P(?:(\d+)W)?(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?)?$/.exec(period ?? "");
  if (!m) return period ?? "";
  const n = (i) => Number(m[i] ?? 0);
  return [
    n(1) ? (n(1) === 1 ? tr("1 Woche") : tr("{n} Wochen", { n: n(1) })) : null,
    n(2) ? (n(2) === 1 ? tr("1 Tag") : tr("{n} Tage", { n: n(2) })) : null,
    n(3) ? `${n(3)} h` : null,
    n(4) ? `${n(4)} min` : null,
    n(5) ? `${n(5)} s` : null,
  ]
    .filter(Boolean)
    .join(" ");
}

const MEANINGS = {
  project: tr("Projekt"),
  tag: "Tag",
  flag: tr("Markierung"),
  priority: tr("Priorität"),
  defer: tr("Start"),
  due: tr("Fällig"),
  planned: tr("Geplant"),
  estimate: tr("Schätzung"),
  repeat: tr("Wiederholung"),
  note: tr("Notiz"),
  template: tr("Vorlage"),
  clock: tr("Zeit"),
  completed: tr("Erledigt"),
  habit: "Habit",
};

// A capture warning in words (ADR 0027 §4: the server sends codes).
export function captureWarningText(w) {
  const meaning = MEANINGS[w.meaning] ?? tr("Angabe");
  switch (w.warning) {
    case "unknown_date":
      return tr("„{text}“: kein Datum, das ich kenne ({meaning}); bleibt im Titel", { text: w.text, meaning });
    case "unknown_estimate":
      return tr("„{text}“: keine Dauer; bleibt im Titel", { text: w.text });
    case "unknown_repeat":
      return tr("„{text}“: keine Wiederholung, die ich kenne; bleibt im Titel", { text: w.text });
    case "twice":
      return tr("„{text}“: {meaning} doppelt; bleibt im Titel", { text: w.text, meaning });
    case "unknown_project":
      return tr("#{name}: kein oder kein eindeutiges Projekt; die Aufgabe geht in die Inbox", { name: w.name });
    case "unknown_tag":
      return tr("@{name}: mehrdeutig oder neu unter einem Ablauf; bleibt im Titel", { name: w.name });
    case "unknown_template":
      return tr("/{name}: keine Vorlage; bleibt im Titel", { name: w.name });
    case "free_text":
      return tr("„{text}“: kein Kürzel; der Titel hat sein eigenes Feld", { text: w.text });
    case "clock_in_future":
      return tr("„{text}“: endet in der Zukunft; nicht gebucht", { text: w.text });
    case "unknown_clock":
      return tr("„{text}“: keine Zeit, die ich kenne; bleibt im Titel", { text: w.text });
    case "unknown_habit":
      return tr("„{text}“: kein Habit, das ich kenne (habit:, 3/w, 8/d:Glas, avoid); bleibt im Titel", { text: w.text });
    default:
      return tr("Hinweis von einem neueren Server");
  }
}
