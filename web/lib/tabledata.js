// The table's numbers (ADR 0115 §2, §4): the names of its columns, the
// time worked and the estimate per task and subtree, and the sums of a set
// of rows. Pure: no DOM, no state of the page, so web/tests/table.mjs can
// hold it to the cases samu-core meets as well (testdata/table/worked.json).

// The columns after the title, the same names as samu_core::table::COLUMNS,
// the CLI and a perspective; besides `prop:<key>`.
export const COLUMNS = [
  "state",
  "priority",
  "project",
  "tags",
  "defer",
  "due",
  "planned",
  "created",
  "modified",
  "completed",
  "estimate",
  "worked",
  "remaining",
  "blocked_by",
];

export const DEFAULT_COLUMNS = ["state", "priority", "due", "estimate", "worked"];

export const PROPERTY = "prop:";

export const knownColumn = (name) =>
  COLUMNS.includes(name) || (typeof name === "string" && name.startsWith(PROPERTY) && name.slice(PROPERTY.length).trim() !== "");

// The columns to show of a stored choice: known ones, each once; none
// stored is the default (a perspective's `table.columns` may be empty).
export function columnsOf(table) {
  const stored = table?.columns;
  if (!Array.isArray(stored)) return [...DEFAULT_COLUMNS];
  return [...new Set(stored.filter(knownColumn))];
}

// Seconds of an ISO 8601 duration; days count 24 hours, weeks seven days,
// months and years from 2026-01-01 as samu-core reckons them. Null when it
// is none or not a duration.
export function estimateSeconds(period) {
  const m = /^P(?:(\d+)Y)?(?:(\d+)M)?(?:(\d+)W)?(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+(?:\.\d+)?)S)?)?$/.exec(period ?? "");
  if (!m || period === "P" || period.endsWith("T")) return null;
  const n = (i) => Number(m[i] ?? 0);
  const anchor = Date.UTC(2026, 0, 1);
  const moved = Date.UTC(2026 + n(1), n(2), 1);
  const calendar = (moved - anchor) / 1000;
  return Math.trunc(calendar + n(3) * 7 * 86400 + n(4) * 86400 + n(5) * 3600 + n(6) * 60 + n(7));
}

const instant = (at) => Date.parse(String(at).replace(/\[[^\]]*\]$/, ""));

// Children per parent among `ids`, from the tasks `keep` lets pass.
function childrenOf(tasks, ids, keep = () => true) {
  const children = new Map();
  for (const task of tasks) {
    if (!keep(task) || !task.parent || !ids.has(task.parent)) continue;
    if (!children.has(task.parent)) children.set(task.parent, []);
    children.get(task.parent).push(task.id);
  }
  return children;
}

// The sum of `own` over a task and everything below it, each task once
// (a loop in `parent`, which the server never lets happen, ends there).
function subtreeOf(id, own, children, add, empty) {
  let sum = empty;
  const seen = new Set();
  const stack = [id];
  while (stack.length) {
    const at = stack.pop();
    if (seen.has(at)) continue;
    seen.add(at);
    sum = add(sum, own.get(at));
    stack.push(...(children.get(at) ?? []));
  }
  return sum;
}

// Seconds worked per task (samu_core::table::worked): the sum of its
// clocks, a running one up to `now` (ms), booked ones included; the
// subtree counts every task below, completed and dropped ones too. Clocks
// on projects or on tasks not in `tasks` add to nothing.
export function worked(tasks, clocks, now) {
  const own = new Map(tasks.map((t) => [t.id, 0]));
  for (const clock of clocks) {
    if (!own.has(clock.owner)) continue;
    const start = Math.floor(instant(clock.start) / 1000);
    const end = Math.floor((clock.end ? instant(clock.end) : now) / 1000);
    if (Number.isNaN(start) || Number.isNaN(end)) continue;
    own.set(clock.owner, own.get(clock.owner) + Math.max(0, end - start));
  }
  const ids = new Set(own.keys());
  const children = childrenOf(tasks, ids);
  const result = new Map();
  for (const task of tasks) {
    result.set(task.id, { own: own.get(task.id), subtree: subtreeOf(task.id, own, children, (a, b) => a + (b ?? 0), 0) });
  }
  return result;
}

// The estimate per task in seconds (samu_core::table::estimated): its own,
// and its subtree's, null where nothing below has one. A dropped task keeps
// its own but adds nothing above it; a completed one adds.
export function estimated(tasks) {
  const own = new Map(tasks.map((t) => [t.id, estimateSeconds(t.estimate)]));
  const ids = new Set(own.keys());
  const children = childrenOf(tasks, ids, (t) => !t.dropped_at);
  const add = (a, b) => (b === null || b === undefined ? a : (a ?? 0) + b);
  const result = new Map();
  for (const task of tasks) {
    result.set(task.id, { own: own.get(task.id), subtree: subtreeOf(task.id, own, children, add, null) });
  }
  return result;
}

// The sums of a set of rows (ADR 0115 §4): estimate, worked and remaining
// of their subtrees, each task once. A row whose ancestor is a row too is
// in that ancestor's subtree already; that matters where a grouping draws
// an action group and its children side by side. `parentOf(id)` gives a
// task's parent id or null. Remaining is null without an estimate.
export function sums(ids, parentOf, workedById, estimatedById) {
  const rows = new Set(ids);
  const counted = [...rows].filter((id) => {
    const seen = new Set([id]);
    for (let up = parentOf(id); up && !seen.has(up); up = parentOf(up)) {
      if (rows.has(up)) return false;
      seen.add(up);
    }
    return true;
  });
  let estimate = null;
  let spent = 0;
  for (const id of counted) {
    spent += workedById.get(id)?.subtree ?? 0;
    const e = estimatedById.get(id)?.subtree;
    if (e !== null && e !== undefined) estimate = (estimate ?? 0) + e;
  }
  return { estimate, worked: spent, remaining: estimate === null ? null : estimate - spent };
}

// "1:05" for seconds; "−0:30" below zero; "" for null.
export function hoursText(seconds) {
  if (seconds === null || seconds === undefined) return "";
  const sign = seconds < 0 ? "−" : "";
  const minutes = Math.round(Math.abs(seconds) / 60);
  return `${sign}${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, "0")}`;
}

// --- Editing in a cell (ADR 0115 §6) ---------------------------------------

// What a cell can change; worked, created, modified, project, completed
// and the blockers are only read (the project moves by refiling).
const EDITABLE = ["state", "priority", "defer", "due", "planned", "estimate", "tags"];
const DATES = ["defer", "due", "planned"];

export const editable = (column) => EDITABLE.includes(column) || (typeof column === "string" && knownColumn(column) && column.startsWith(PROPERTY));

// The column one step left or right of `current` among the title (null)
// and `columns`; at either end it stays.
export function stepColumn(columns, current, step) {
  const all = [null, ...columns];
  const at = Math.max(0, all.indexOf(current));
  return all[Math.min(all.length - 1, Math.max(0, at + step))];
}

// What a typed cell writes: `{ field, value }`, `{ error }`, or null when
// the text is what the cell held (`before`). Empty takes the value away.
// `parse` reads an estimate (words.js `parseEstimate`: null for empty,
// undefined for what is not a length).
export function cellWrite(column, text, before, parse) {
  const typed = (text ?? "").trim();
  if (typed === (before ?? "").trim()) return null;
  if (DATES.includes(column)) {
    if (!typed) return { field: column, value: null };
    return /^\d{4}-\d{2}-\d{2}$/.test(typed) ? { field: column, value: { date: typed } } : { error: "date" };
  }
  if (column === "estimate") {
    const value = parse(typed);
    return value === undefined ? { error: "estimate" } : { field: "estimate", value };
  }
  if (column.startsWith(PROPERTY) && knownColumn(column)) {
    return { field: `properties.${column.slice(PROPERTY.length)}`, value: typed || null };
  }
  return { error: "column" };
}
