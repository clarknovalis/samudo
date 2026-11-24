// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The forms tasks leave the web UI in (ADR 0145 §2): IDs, a Markdown task
// list, Org, TSV and JSON. Pure: no DOM, no state of the page, so
// web/tests/exporttext.mjs checks it with node. export.js hands in rows
// of plain values: `{ id, parent, title, closed, tags, cells, object }`,
// `tags` the titles of the task's tags.

export const FORMS = ["ids", "text", "org", "tsv", "json"];

// The file ending of each form when it is saved (§3).
export const ENDING = { ids: "txt", text: "md", org: "org", tsv: "tsv", json: "json" };

// A date of the wire as TSV writes it: `YYYY-MM-DD`, with ` HH:MM` when
// it carries a time (ADR 0144). The time is the one written with the
// instant, in the account's zone, as the rest of the web UI shows it.
export function dateText(when) {
  if (!when) return "";
  if (when.date) return when.date;
  if (when.at) return `${when.at.slice(0, 10)} ${when.at.slice(11, 16)}`;
  return "";
}

// The depth of each row: below its parent when that is copied with it.
// Rows come in the order drawn, where a parent stands before its children.
function depths(rows) {
  const depth = new Map();
  return rows.map((r) => {
    const d = r.parent && depth.has(r.parent) ? depth.get(r.parent) + 1 : 0;
    depth.set(r.id, d);
    return d;
  });
}

export const ids = (rows) => rows.map((r) => r.id).join(" ");

// `- [ ] Title`, `- [x]` for a closed task; a row whose parent is copied
// with it goes below it, two spaces deeper.
export function text(rows) {
  const d = depths(rows);
  return rows.map((r, i) => `${"  ".repeat(d[i])}- [${r.closed ? "x" : " "}] ${oneLine(r.title)}`).join("\n");
}

// --- Org, as `samu export-org` writes a task (ADR 0074 §8) ---------------

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const weekday = (day) => WEEKDAYS[new Date(`${day}T12:00:00Z`).getUTCDay()];

// `<2026-11-20 Fri>` or `<2026-11-20 Fri 09:00>`, in the time's own zone.
export function orgActive(when) {
  if (!when) return null;
  if (when.date) return `<${when.date} ${weekday(when.date)}>`;
  if (when.at) return `<${when.at.slice(0, 10)} ${weekday(when.at.slice(0, 10))} ${when.at.slice(11, 16)}>`;
  return null;
}

// `[2026-11-11 Wed 12:00]` for a stamp, in this browser's zone.
export function orgInactive(stamp) {
  const t = new Date(stamp);
  if (!stamp || Number.isNaN(t.getTime())) return null;
  const pad = (n) => String(n).padStart(2, "0");
  return `[${t.getFullYear()}-${pad(t.getMonth() + 1)}-${pad(t.getDate())} ${WEEKDAYS[t.getDay()]} ${pad(t.getHours())}:${pad(t.getMinutes())}]`;
}

// An Org tag takes letters, digits, `_`, `@`, `#` and `%`; the rest `_`.
export const orgTag = (title) => String(title).replace(/[^\p{L}\p{N}_@#%]/gu, "_");

function orgTask(r, level) {
  const t = r.object ?? {};
  const keyword = t.completed_at ? "DONE" : t.dropped_at ? "CANCELLED" : "TODO";
  const priority = t.priority ? `[#${t.priority}] ` : "";
  const tags = (r.tags ?? []).map(orgTag).filter(Boolean);
  const lines = [`${"*".repeat(level)} ${keyword} ${priority}${oneLine(r.title)}${tags.length ? ` :${tags.join(":")}:` : ""}`];
  const planning = [
    t.completed_at || t.dropped_at ? `CLOSED: ${orgInactive(t.completed_at ?? t.dropped_at)}` : null,
    orgActive(t.due) ? `DEADLINE: ${orgActive(t.due)}` : null,
    orgActive(t.planned) ? `SCHEDULED: ${orgActive(t.planned)}` : null,
  ].filter(Boolean);
  if (planning.length) lines.push(planning.join(" "));
  const props = new Map([["ID", r.id]]);
  if (orgInactive(t.created)) props.set("CREATED", orgInactive(t.created));
  if (orgActive(t.defer)) props.set("DEFER", orgActive(t.defer));
  for (const [key, value] of Object.entries(t.properties ?? {})) {
    props.set(props.has(key.toUpperCase()) ? `samu_${key}` : key, value);
  }
  lines.push(":PROPERTIES:", ...[...props].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)).map(([k, v]) => `:${k}: ${oneLine(v)}`), ":END:");
  // Body text cannot open a heading or end a drawer (as export_org.rs).
  for (const line of t.note ? String(t.note).split("\n") : []) {
    lines.push(line.startsWith("*") || line.trim().toLowerCase() === ":end:" ? ` ${line}` : line);
  }
  return lines.join("\n");
}

// One heading per task, a level deeper below a copied parent.
export function org(rows) {
  const d = depths(rows);
  return rows.map((r, i) => orgTask(r, d[i] + 1)).join("\n");
}

// A header row, then one row per task: id, title, then `columns` from
// each row's `cells`. A tab or a line break in a value becomes a space.
export function tsv(rows, columns) {
  const line = (values) => values.map(cell).join("\t");
  return [line(["id", "title", ...columns]), ...rows.map((r) => line([r.id, r.title, ...columns.map((c) => r.cells?.[c] ?? "")]))].join("\n");
}

// Shaped like `samu show --format json`: `{id, kind, short, title, object}`.
export function json(rows) {
  const out = rows.map((r) => ({ id: r.id, kind: "task", object: r.object, short: r.id.slice(-8), title: r.title }));
  return JSON.stringify(out, null, 2);
}

export function render(form, rows, columns) {
  switch (form) {
    case "ids":
      return ids(rows);
    case "text":
      return text(rows);
    case "org":
      return org(rows);
    case "tsv":
      return tsv(rows, columns);
    case "json":
      return json(rows);
    default:
      return "";
  }
}

// `samudo-<view>-<day>.<ending>`, the view reduced to letters, digits and
// dashes.
export function fileName(view, day, form) {
  const slug = String(view ?? "")
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return `samudo-${slug || "tasks"}-${day}.${ENDING[form] ?? "txt"}`;
}

const oneLine = (value) => String(value ?? "").replace(/\s*[\r\n]+\s*/g, " ");
const cell = (value) => String(value ?? "").replace(/[\t\r\n]+/g, " ");
