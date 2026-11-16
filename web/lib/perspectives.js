// Perspectives (ADR 0090): saved views of the search, the filter, the
// arrangement and the presentation, kept on the server. In a perspective
// the search field holds its chips and free text; changing them asks to
// save or discard (§6). Anordnen, the board and the filter write to the
// perspective itself. What it shows is computed here, as the search is
// (query.js), and by samu-core for the other clients (§2, §3).

import { api, change } from "./api.js";
import { uuidv7 } from "./dom.js";
import { asksForClosed, asksForWaiting, matcher } from "./query.js";
import { byId, state, update } from "./state.js";

export const byPosition = (a, b) => a.position - b.position || a.title.localeCompare(b.title);
export const perspectives = () => [...state.perspectives].sort(byPosition);
export const perspectiveOf = (id) => byId(state.perspectives, id);

// The perspective the middle shows, or null.
export const current = () => (state.view.kind === "perspective" ? perspectiveOf(state.view.perspective) ?? null : null);

const FILTERS = ["available", "remaining", "all"];
// A filter from a newer samu is read as the default (ADR 0090 §1).
export const baseFilter = (p) => (FILTERS.includes(p.filter) ? p.filter : "available");

// The filter that holds: the perspective's, lifted by what its chips ask
// for (ADR 0042 §4). The open one is judged by the chips in the field.
export function filterOf(p) {
  const chips = current()?.id === p.id ? state.searchChips : p.chips ?? [];
  if (asksForClosed(chips)) return "all";
  const base = baseFilter(p);
  if (base === "available" && asksForWaiting(chips)) return "remaining";
  return base;
}

// One test per perspective, made again only when what it reads changes.
const tests = new Map();
export function holds(p, status) {
  const open = current()?.id === p.id;
  const chips = open ? state.searchChips : p.chips ?? [];
  const text = open ? state.search : p.text ?? "";
  const key = [chips, text, state.tags, state.folders];
  let made = tests.get(p.id);
  if (!made || made.key.some((k, i) => k !== key[i])) {
    made = { key, test: matcher(chips, text) };
    tests.set(p.id, made);
  }
  return made.test(status);
}

// --- Arrangement: the browser's form and the wire's (ADR 0090 §1) ------

const DISPLAYS = ["list", "board", "table"];
const viewOf = (view) => (DISPLAYS.includes(view) ? view : "list");

const DEFAULT_ARRANGE = { group: "project", sort: "position", desc: false, then: null, thenDesc: false };

// What list.js and board.js read as `chosen`.
export function orderOf(p) {
  const a = p.arrange ?? {};
  return {
    view: viewOf(p.view),
    group: a.group ?? DEFAULT_ARRANGE.group,
    sort: a.sort ?? DEFAULT_ARRANGE.sort,
    desc: Boolean(a.desc),
    then: a.then ?? null,
    thenDesc: Boolean(a.then_desc),
    ...(p.board ? { board: p.board } : {}),
    // The table's columns stay when the view goes back to list or board
    // (ADR 0115 §7).
    ...(p.table ? { table: p.table } : {}),
  };
}

// The fields of a perspective for a `chosen` of the browser.
function fieldsOf(chosen) {
  const plain =
    chosen.group === DEFAULT_ARRANGE.group && chosen.sort === DEFAULT_ARRANGE.sort && !chosen.desc && !chosen.then;
  const arrange = plain
    ? null
    : {
        group: chosen.group,
        sort: chosen.sort,
        ...(chosen.desc ? { desc: true } : {}),
        ...(chosen.then ? { then: chosen.then } : {}),
        ...(chosen.then && chosen.thenDesc ? { then_desc: true } : {}),
      };
  return { arrange, view: viewOf(chosen.view), board: chosen.board ?? null, table: chosen.table ?? null };
}

const same = (a, b) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

async function write(changes) {
  if (!changes.length) return true;
  try {
    await api.write(changes);
    return true;
  } catch (error) {
    update({ message: error.message });
    return false;
  }
}

// Anordnen, Liste/Board and the board's columns change the perspective
// (§6); only the fields that changed are written.
export function chooseOrderOf(id, patch) {
  const p = perspectiveOf(id);
  if (!p) return;
  const next = fieldsOf({ ...orderOf(p), ...patch });
  const changes = Object.entries(next)
    .filter(([field, value]) => !same(p[field], value))
    .map(([field, value]) => change.set(p.id, field, value));
  // Drawn at once from the object, before the server answers.
  Object.assign(p, next);
  update({});
  write(changes);
}

export function chooseFilterOf(id, filter) {
  const p = perspectiveOf(id);
  if (!p || p.filter === filter) return;
  p.filter = filter;
  update({});
  write([change.set(p.id, "filter", filter)]);
}

// --- The search of the open perspective (§6) ---------------------------

const text = (t) => (t ?? "").trim().replace(/\s+/g, " ");

// Chips as the server keeps them: `not` only when set.
const plain = (chips) => (chips ?? []).map(({ kind, value, not }) => (not ? { kind, value, not: true } : { kind, value }));
const sameChips = (a, b) => same(plain(a), plain(b));

// Whether the field differs from what the perspective holds.
export function edited() {
  const p = current();
  return Boolean(p) && !p.blocks?.length && (!sameChips(state.searchChips, p.chips) || text(state.search) !== text(p.text));
}

// The field as the open perspective holds it.
export function searchOf(p) {
  return { search: p.text ?? "", searchChips: (p.chips ?? []).map((c) => ({ ...c, not: Boolean(c.not) })) };
}

export async function saveSearch() {
  const p = current();
  if (!p) return;
  const chips = plain(state.searchChips);
  const words = text(state.search);
  const changes = [];
  if (!sameChips(chips, p.chips)) changes.push(change.set(p.id, "chips", chips));
  if (words !== text(p.text)) changes.push(change.set(p.id, "text", words || null));
  if (await write(changes)) {
    Object.assign(p, { chips, text: words });
    update({});
  }
}

export function discardSearch(input) {
  const p = current();
  if (!p) return;
  const back = searchOf(p);
  input.value = back.search;
  update({ ...back, selected: null });
}

// --- Making, renaming, deleting ----------------------------------------

// What a new perspective takes from the view on screen (§5): the search,
// the filter and how it is arranged and shown.
export function captureFrom(chosen) {
  // A view that is a chip brings it along, so the perspective shows what
  // the view showed: Inbox, a project, Markiert.
  const view = state.view;
  const searching = state.search.trim() || state.searchChips.length;
  const own =
    searching || view.kind === "perspective"
      ? []
      : view.kind === "inbox"
        ? [{ kind: "project", value: "inbox", not: false }]
        : view.kind === "project" && view.project
          ? [{ kind: "project", value: view.project, not: false }]
          : view.kind === "flagged"
            ? [{ kind: "is", value: "markiert", not: false }]
            : [];
  return {
    chips: plain([...own, ...state.searchChips]),
    text: text(state.search),
    filter: state.filter,
    ...fieldsOf(chosen),
  };
}

// A perspective called `title` from `from`, last in the sidebar. Its id,
// or null when the server said no.
export async function createPerspective(title, from = {}) {
  const created = new Date().toISOString();
  const value = {
    id: uuidv7(),
    created,
    modified: created,
    title,
    filter: from.filter && FILTERS.includes(from.filter) ? from.filter : "available",
    view: viewOf(from.view),
    position: state.perspectives.reduce((max, p) => Math.max(max, p.position + 1), 0),
  };
  if (from.chips?.length) value.chips = from.chips;
  if (from.text) value.text = from.text;
  if (from.arrange) value.arrange = from.arrange;
  if (from.board) value.board = from.board;
  if (from.table) value.table = from.table;
  // A block perspective (ADR 0116 §1).
  if (from.blocks?.length) value.blocks = from.blocks;
  if (from.dedupe === false) value.dedupe = false;
  if (!(await write([change.create("perspective", value)]))) return null;
  // The reload after the write may have brought it already.
  if (!perspectiveOf(value.id)) state.perspectives = [...state.perspectives, value];
  return value.id;
}

export const renamePerspective = (p, title) => (title === p.title ? Promise.resolve(true) : write([change.set(p.id, "title", title)]));

export const setIcon = (p, icon) => write([change.set(p.id, "icon", icon || null)]);

export async function duplicate(p) {
  const taken = new Set(state.perspectives.map((o) => o.title.toLowerCase()));
  let title = `${p.title} (Kopie)`;
  for (let n = 2; taken.has(title.toLowerCase()); n += 1) title = `${p.title} (Kopie ${n})`;
  const { id, created, modified, position, title: _, ...rest } = p;
  return createPerspective(title, rest);
}

export const deletePerspective = (p) => write([change.delete(p.id)]);

// Moves `id` before `before` (null: to the end); writes the positions
// that change.
export function movePerspective(id, before) {
  const list = perspectives().filter((p) => p.id !== id);
  const moved = perspectiveOf(id);
  if (!moved) return;
  const at = before ? list.findIndex((p) => p.id === before) : list.length;
  list.splice(at < 0 ? list.length : at, 0, moved);
  const changes = list.flatMap((p, i) => (p.position === i ? [] : [change.set(p.id, "position", i)]));
  list.forEach((p, i) => {
    p.position = i;
  });
  update({});
  write(changes);
}
