// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// One action on many tasks (ADR 0052): picking rows in the list, the bar
// above it with the actions, and dragging rows onto the sidebar. Every
// action is one write over /changes; only deleting goes task by task.

import { api, change } from "./api.js";
import { $, el, replace } from "./dom.js";
import { flushDetail, statesOf } from "./detail.js";
import { openEdit } from "./edit.js";
import { order } from "./list.js";
import { descendants, moveMany, openPalette, ancestors } from "./palette.js";
import { newTagField } from "./placeops.js";
import { clearZones } from "./reorder.js";
import { inEffect, pretty } from "./keymap.js";
import { byId, isOpen, state, statusOf, update } from "./state.js";
import { addDays, today } from "./words.js";
import { tr } from "./i18n.js";

// --- Picking (§4) -------------------------------------------------------

// The picked statuses in the order drawn. Picks no longer drawn (a filter
// hid them) fall away.
export function pickedStatuses() {
  return order.filter((id) => state.picked.has(id)).map(statusOf).filter(Boolean);
}

export const picking = () => state.picked.size > 0;

export function togglePick(id) {
  const picked = new Set(state.picked);
  if (picked.has(id)) picked.delete(id);
  else picked.add(id);
  update({ picked, selected: id });
}

// From the current row to `id`, as drawn; the current row counts too.
export function pickRange(id) {
  const from = order.indexOf(state.selected);
  const to = order.indexOf(id);
  if (from < 0 || to < 0) return togglePick(id);
  const picked = new Set(state.picked);
  for (const at of order.slice(Math.min(from, to), Math.max(from, to) + 1)) picked.add(at);
  update({ picked, selected: id });
}

export function clearPicks() {
  bar = {};
  update({ picked: new Set() });
}

// What a click on a row does: Ctrl/⌘ picks, Shift picks a range, else
// it opens the task as before and leaves the picks.
export function rowClick(event, id) {
  if (event.ctrlKey || event.metaKey) togglePick(id);
  else if (event.shiftKey) pickRange(id);
  else update({ selected: id, pane: "detail" });
}

// --- The actions (§2) -----------------------------------------------------

async function write(changes, done = null) {
  if (!changes.length) return true;
  try {
    await api.write(changes);
    update({ message: done });
    return true;
  } catch (error) {
    update({ message: error.message });
    return false;
  }
}

const tasks = () => pickedStatuses().map((s) => s.task);

export async function completeAll() {
  // What the inspector holds goes first (e72d21c3).
  await flushDetail();
  const list = tasks();
  const open = list.filter(isOpen);
  if (open.length) {
    const now = new Date().toISOString();
    return write(open.map((t) => change.set(t.id, "completed_at", now)));
  }
  return reopenAll();
}

export function dropAll() {
  const now = new Date().toISOString();
  return write(tasks().filter((t) => !t.dropped_at).map((t) => change.set(t.id, "dropped_at", now)));
}

export function reopenAll() {
  const changes = [];
  for (const t of tasks()) {
    if (t.completed_at) changes.push(change.set(t.id, "completed_at", null));
    if (t.dropped_at) changes.push(change.set(t.id, "dropped_at", null));
  }
  return write(changes);
}

// Flags all unless all are flagged; then takes the flag off.
export function flagAll() {
  const list = tasks();
  const on = list.some((t) => !t.flagged);
  return write(list.filter((t) => t.flagged !== on).map((t) => change.set(t.id, "flagged", on)));
}

export const moveAll = () => openPalette(null, pickedStatuses());

function tagAll(tag, on) {
  const list = tasks();
  const has = (t) => (t.tags ?? []).some((x) => x.tag === tag);
  return write(list.filter((t) => has(t) !== on).map((t) => (on ? change.addTag(t.id, tag) : change.removeTag(t.id, tag))));
}

// Each task into the state of its own workflow that has `title` (or out of
// every state with null); the others stay and are counted (§2).
async function stateAll(title, note) {
  const changes = [];
  let missed = 0;
  for (const status of pickedStatuses()) {
    if (title === null) {
      if (status.state) changes.push(change.removeTag(status.task.id, status.state, note || undefined));
      continue;
    }
    const target = statesOf(status).find((t) => t.title === title);
    if (!target) missed += 1;
    else if (target.id !== status.state) changes.push(change.addTag(status.task.id, target.id, note || undefined));
  }
  const said = missed ? tr("{n} ohne „{title}“ im Ablauf, unverändert.", { n: missed, title }) : null;
  if (!changes.length) return update({ message: said });
  return write(changes, said);
}

// `s`: each to the next state the server computed for it; `t` and a key:
// the state with that key in each one's workflow.
export function cycleAll() {
  const changes = [];
  for (const status of pickedStatuses()) {
    const next = status.next_state ?? null;
    if (next && byId(state.tags, next)?.ask_note) continue;
    if (next) changes.push(change.addTag(status.task.id, next));
    else if (status.state) changes.push(change.removeTag(status.task.id, status.state));
  }
  return write(changes);
}

export function pickStateAll(key) {
  const titles = new Set(pickedStatuses().flatMap((s) => statesOf(s).filter((t) => t.key === key).map((t) => t.title)));
  if (titles.size !== 1) return update({ message: tr("Kein Zustand mit Kürzel „{key}“.", { key }) });
  return chooseState([...titles][0]);
}

function chooseState(title) {
  const asks = title !== null && pickedStatuses().some((s) => statesOf(s).some((t) => t.title === title && t.ask_note));
  if (asks) {
    bar = { ...bar, note: title };
    update({ message: tr("„{title}“ verlangt eine Notiz.", { title }) });
    setTimeout(() => $("bulk-note")?.focus());
    return null;
  }
  return stateAll(title, null);
}

function dateAll(field, value) {
  return write(tasks().map((t) => change.set(t.id, field, value ? { date: value } : null)));
}

// The picked tasks without those under a picked ancestor, and how many go
// with them, the closed ones and those the scope hides counted.
function roots() {
  const list = tasks();
  const ids = new Set(list.map((t) => t.id));
  const top = list.filter((t) => !ancestors(t.id).some((a) => ids.has(a)));
  return { top, count: top.reduce((n, t) => n + 1 + descendants(t.id).length, 0) };
}

async function deleteAll() {
  const { top } = roots();
  bar = {};
  try {
    for (const t of top) await api.deleteObject(t.id, { preview: false });
    update({ picked: new Set(), message: null });
  } catch (error) {
    update({ message: error.message });
  }
}

// --- For the context menu (ADR 0062 §4) -----------------------------------

// The state titles of the picked tasks' workflows; null takes them out.
export const pickedStateTitles = () => [...new Set(pickedStatuses().flatMap((s) => statesOf(s).map((t) => t.title)))];
export const chooseStateAll = (title) => chooseState(title);
export const dateAllTo = (field, day) => dateAll(field, day);

// The bar asks for the date of `field`, as its menu Termin does.
export function askDateAll(field) {
  bar = { ...bar, date: field };
  update({});
  setTimeout(() => $("bulk-date")?.showPicker?.());
}

// Deleting asks first in the bar; `ids` become the picks when given, so
// one task is deleted the same way as many.
export function askDelete(ids = null) {
  bar = { confirm: true };
  update(ids ? { picked: new Set(ids), selected: ids[0] } : {});
}

// --- Dragging onto the sidebar (§5) ---------------------------------------

// The ids being dragged, while a row of the list is.
let dragged = null;
export const draggedTasks = () => dragged;

export function rowDrag(id) {
  return {
    draggable: "true",
    ondragstart: (event) => {
      dragged = state.picked.has(id) ? pickedStatuses().map((s) => s.task.id) : [id];
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", dragged.join("\n"));
    },
    ondragend: () => {
      dragged = null;
      clearZones();
    },
  };
}

// Dropped on a project (or the inbox with null): last there, without a
// parent.
export function dropTasks(project) {
  const ids = dragged ?? [];
  dragged = null;
  const list = ids.map((id) => statusOf(id)?.task).filter(Boolean);
  if (list.length) moveMany(list, project, null);
}

export function forgetDrag() {
  dragged = null;
}

// --- The bar (§4) -----------------------------------------------------------

// What the bar asks for: { note } a note for a state, { date } a field
// waiting for a date, { confirm } deleting.
let bar = {};

const DATE_FIELDS = [
  ["due", tr("Fällig")],
  ["planned", tr("Geplant")],
  ["defer", tr("Start")],
];

function select(label, options, onchoose) {
  return el(
    "select",
    {
      "aria-label": label,
      onchange: (event) => {
        const value = event.target.value;
        event.target.value = "";
        if (value) onchoose(value);
      },
    },
    [el("option", { value: "", text: label }), ...options],
  );
}

const option = (value, text) => el("option", { value, text });

// The option of „+ Tag“ that asks for a new tag (92af4fff).
const NEW_TAG = "new";

function tagSelects(list) {
  const stateIds = new Set(pickedStatuses().flatMap((s) => statesOf(s).map((t) => t.id)));
  const plain = (t) => !t.exclusive && !stateIds.has(t.id) && !byId(state.tags, t.parent)?.exclusive && t.status !== "dropped";
  const byTitle = (a, b) => a.title.localeCompare(b.title, "de");
  const on = new Set(list.flatMap((t) => (t.tags ?? []).map((x) => x.tag)));
  const add = bar.newTag
    ? newTagField(
        "bulk-new-tag",
        (tag, made) => {
          bar = {};
          const has = (t) => (t.tags ?? []).some((x) => x.tag === tag);
          write([...made, ...tasks().filter((t) => !has(t)).map((t) => change.addTag(t.id, tag))]);
        },
        () => ((bar = {}), update({})),
      )
    : select(
        tr("+ Tag"),
        [...state.tags.filter(plain).sort(byTitle).map((t) => option(t.id, t.title)), option(NEW_TAG, tr("Neuer Tag …"))],
        (id) => {
          if (id !== NEW_TAG) return tagAll(id, true);
          bar = { newTag: true };
          update({});
        },
      );
  return [
    add,
    on.size
      ? select(tr("− Tag"), state.tags.filter((t) => plain(t) && on.has(t.id)).sort(byTitle).map((t) => option(t.id, t.title)), (id) => tagAll(id, false))
      : null,
  ];
}

function stateSelect() {
  const titles = [...new Set(pickedStatuses().flatMap((s) => statesOf(s).map((t) => t.title)))];
  if (!titles.length) return null;
  return select(tr("Zustand"), [option("\u0000", tr("kein")), ...titles.map((t) => option(t, t))], (value) =>
    chooseState(value === "\u0000" ? null : value),
  );
}

function dateSelect() {
  const groups = DATE_FIELDS.map(([field, label]) =>
    el("optgroup", { label }, [
      option(`${field} ${today()}`, `${label} heute`),
      option(`${field} ${addDays(today(), 1)}`, `${label} morgen`),
      option(`${field} pick`, tr("{label} am …", { label })),
      option(`${field} none`, `${label} leeren`),
    ]),
  );
  return select(tr("Termin"), groups, (value) => {
    const [field, day] = value.split(" ");
    if (day === "pick") {
      bar = { ...bar, date: field };
      update({});
      setTimeout(() => $("bulk-date")?.showPicker?.());
    } else dateAll(field, day === "none" ? null : day);
  });
}

function asking() {
  if (bar.note) {
    return el("span", { class: "ask" }, [
      el("input", {
        id: "bulk-note",
        type: "text",
        placeholder: tr("Notiz für „{title}“", { title: bar.note }),
        onkeydown: (event) => {
          if (event.key === "Enter" && event.target.value.trim()) {
            const title = bar.note;
            bar = {};
            stateAll(title, event.target.value.trim());
          } else if (event.key === "Escape") {
            event.stopPropagation();
            bar = {};
            update({ message: null });
          }
        },
      }),
    ]);
  }
  if (bar.date) {
    const label = DATE_FIELDS.find(([f]) => f === bar.date)?.[1];
    return el("span", { class: "ask" }, [
      el("label", { text: `${tr("{label} am", { label })} ` }, [
        el("input", {
          id: "bulk-date",
          type: "date",
          onchange: (event) => {
            const field = bar.date;
            bar = {};
            if (event.target.value) dateAll(field, event.target.value);
            else update({});
          },
        }),
      ]),
    ]);
  }
  if (bar.confirm) {
    const { count } = roots();
    return el("span", { class: "ask" }, [
      el("span", { text: count === 1 ? tr("1 Aufgabe löschen, Unteraufgaben mitgezählt?") : tr("{n} Aufgaben löschen, Unteraufgaben mitgezählt?", { n: count }) }),
      el("button", { type: "button", class: "danger", text: tr("Löschen"), onclick: deleteAll }),
      el("button", { type: "button", text: tr("Abbrechen"), onclick: () => ((bar = {}), update({})) }),
    ]);
  }
  return null;
}

// The key of an action for a button's title (ADR 0079 §1), or none.
const k = (id) => pretty(inEffect(state.accountSettings?.keys).byId.get(id)) || null;

export function renderBulk() {
  const box = $("bulk");
  // Picks the list no longer draws fall away, quietly.
  for (const id of [...state.picked]) if (!order.includes(id)) state.picked.delete(id);
  box.hidden = !picking();
  if (box.hidden) {
    bar = {};
    replace(box);
    return;
  }
  const list = tasks();
  const open = list.some(isOpen);
  const closed = list.some((t) => !isOpen(t));
  const button = (text, onclick, title = null) => el("button", { type: "button", text, title, onclick });
  replace(box, [
    el("strong", { text: tr("{n} ausgewählt", { n: list.length }) }),
    button(tr("Bearbeiten …"), () => openEdit(list.map((t) => t.id)), k("edit")),
    open ? button(tr("Erledigen"), completeAll, k("complete")) : null,
    closed ? button(tr("Wieder öffnen"), reopenAll) : null,
    list.some((t) => !t.dropped_at) ? button(tr("Verwerfen"), dropAll, k("drop")) : null,
    button(list.some((t) => !t.flagged) ? tr("⚑ Markieren") : tr("Markierung weg"), flagAll, k("flag")),
    button(tr("Verschieben …"), moveAll, k("move")),
    ...tagSelects(list),
    stateSelect(),
    dateSelect(),
    button(tr("Löschen …"), () => ((bar = { confirm: true }), update({})), k("delete")),
    asking(),
    el("button", { type: "button", class: "close", text: "✕", title: tr("Auswahl aufheben (Esc)"), "aria-label": tr("Auswahl aufheben"), onclick: clearPicks }),
  ]);
}
