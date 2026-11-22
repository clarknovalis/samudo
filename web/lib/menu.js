// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The context menu of a task (ADR 0062 §1, §4): a right click on a row of
// the list or a card of the board, Shift+F10 or the menu key on the
// current row, a long press on touch. Every entry goes the way its key or
// button goes already; the key stands beside it, so one learns it. On a
// picked row the menu acts on the picks, with the actions of ADR 0052 §2.

import { api, change } from "./api.js";
import {
  askDateAll,
  askDelete,
  chooseStateAll,
  completeAll,
  dateAllTo,
  dropAll,
  flagAll,
  moveAll,
  pickedStateTitles,
  pickedStatuses,
  reopenAll,
} from "./bulk.js";
import { openCapture } from "./capture.js";
import { closeTasks } from "./closeask.js";
import { openEdit } from "./edit.js";
import { running, toggleClock } from "./clock.js";
import { $, el, replace } from "./dom.js";
import { enterState, setDone, statesOf, toggleFlag, toggleSequential } from "./detail.js";
import { habitMenu } from "./habits.js";
import { ownOrder } from "./list.js";
import { indent, openBlockPalette, openPalette, outdent, siblingAbove } from "./palette.js";
import { shiftTarget, shiftTask } from "./reorder.js";
import { inEffect, normalize, pretty } from "./keymap.js";
import { info } from "./message.js";
import { isOpen, state, statusOf, update } from "./state.js";
import { addDays, today } from "./words.js";
import { tr } from "./i18n.js";

// The open menu: its entries, the current one, the open submenu (with its
// current entry, -1 for none) and where it stands. Null when closed.
let menu = null;
// When a key opened it, so the browser's own menu after it stays away.
let keyed = 0;

// The binding of an action as the account has it (ADR 0079 §1).
const k = (id) => inEffect(state.accountSettings?.keys).byId.get(id) ?? null;

export const menuOpen = () => menu !== null;

async function write(changes, message = null) {
  try {
    await api.write(changes);
    update({ message });
  } catch (error) {
    update({ message: error.message });
  }
}

const DATES = [
  ["due", tr("Fällig")],
  ["planned", tr("Geplant")],
  ["defer", tr("Start")],
];

// The entries of the submenu Datum: per field today, tomorrow, a day of
// one's choice and, when set, none.
function dateEntries(set, pick, has) {
  return DATES.flatMap(([field, label]) => [
    { label, header: true },
    { label: tr("heute"), run: () => set(field, today()) },
    { label: tr("morgen"), run: () => set(field, addDays(today(), 1)) },
    { label: tr("am …"), run: () => pick(field) },
    has(field) ? { label: tr("leeren"), run: () => set(field, null) } : null,
  ]);
}

// The date field of the inspector, opened on this task (no picks).
function pickDay(id, field) {
  update({ selected: id, pane: "detail" });
  setTimeout(() => {
    const input = document.querySelector(`#detail [data-date-field="${field}"]`);
    input?.focus();
    input?.showPicker?.();
  });
}

function single(status) {
  const task = status.task;
  const open = isOpen(task);
  const states = statesOf(status);
  const clock = running();
  const ticking = clock?.owner === task.id;
  const reopen = () =>
    write([change.set(task.id, task.completed_at ? "completed_at" : "dropped_at", null)]);
  return [
    { label: tr("Öffnen"), key: k("open"), run: () => update({ selected: task.id, pane: "detail" }) },
    {
      label: tr("Bearbeiten …"),
      key: k("edit"),
      run: () => {
        update({ selected: task.id });
        openEdit(task.id);
      },
    },
    open
      ? {
          label: tr("Unteraufgabe anlegen"),
          key: k("capture.child"),
          run: () => {
            update({ selected: task.id });
            openCapture(task.id);
          },
        }
      : null,
    "-",
    ...(open && task.habit ? habitMenu(task) : []),
    open
      ? { label: tr("Erledigen"), key: k("complete"), run: () => setDone(status, true) }
      : { label: tr("Wieder öffnen"), key: task.completed_at ? k("complete") : null, run: reopen },
    open
      ? { label: tr("Verwerfen"), key: k("drop"), run: () => closeTasks([task.id], "dropped") }
      : null,
    { label: task.flagged ? tr("Markierung weg") : tr("Markieren"), key: k("flag"), run: () => toggleFlag(status) },
    status.group
      ? {
          label: task.sequential ? tr("Auf parallel stellen") : tr("Auf sequenziell stellen"),
          key: k("sequential"),
          run: () => toggleSequential(status),
        }
      : null,
    "-",
    open && states.length
      ? {
          label: tr("Zustand"),
          sub: [
            ...states.map((tag) => ({
              label: tag.title,
              key: tag.key || null,
              checked: status.state === tag.id,
              run: () => enterState(status, tag.id),
            })),
            { label: tr("keiner"), checked: !status.state, run: () => enterState(status, null) },
          ],
        }
      : null,
    open
      ? {
          label: tr("Datum"),
          sub: dateEntries(
            (field, day) => write([change.set(task.id, field, day ? { date: day } : null)]),
            (field) => pickDay(task.id, field),
            (field) => Boolean(task[field]),
          ),
        }
      : null,
    "-",
    { label: tr("Verschieben …"), key: k("move"), run: () => openPalette(status) },
    open ? { label: tr("Blockiert durch …"), key: k("block"), run: () => openBlockPalette(status) } : null,
    ownOrder() && shiftTarget(task, -1) ? { label: tr("Nach oben"), key: k("shift.up"), run: () => shiftTask(status, -1) } : null,
    ownOrder() && shiftTarget(task, 1) ? { label: tr("Nach unten"), key: k("shift.down"), run: () => shiftTask(status, 1) } : null,
    open && siblingAbove(status) ? { label: tr("Einrücken"), key: k("indent"), run: () => indent(status) } : null,
    task.parent ? { label: tr("Ausrücken"), key: k("outdent"), run: () => outdent(status) } : null,
    "-",
    open || ticking
      ? { label: ticking ? tr("Uhr stoppen") : tr("Uhr starten"), key: k("clock"), run: () => toggleClock(status) }
      : null,
    { label: tr("Link kopieren"), key: k("copy-link"), run: () => copyLink(task.id) },
    "-",
    { label: tr("Löschen …"), key: k("delete"), danger: true, run: () => askDelete([task.id]) },
  ];
}

function many() {
  const list = pickedStatuses().map((s) => s.task);
  const titles = pickedStateTitles();
  return [
    { label: tr("{n} ausgewählt", { n: list.length }), header: true },
    { label: tr("Bearbeiten …"), key: k("edit"), run: () => openEdit(list.map((t) => t.id)) },
    "-",
    list.some(isOpen) ? { label: tr("Erledigen"), key: k("complete"), run: completeAll } : null,
    list.some((t) => !isOpen(t)) ? { label: tr("Wieder öffnen"), run: reopenAll } : null,
    list.some((t) => !t.dropped_at) ? { label: tr("Verwerfen"), key: k("drop"), run: dropAll } : null,
    { label: list.some((t) => !t.flagged) ? tr("Markieren") : tr("Markierung weg"), key: k("flag"), run: flagAll },
    "-",
    titles.length
      ? {
          label: tr("Zustand"),
          sub: [
            ...titles.map((title) => ({ label: title, run: () => chooseStateAll(title) })),
            { label: tr("keiner"), run: () => chooseStateAll(null) },
          ],
        }
      : null,
    {
      label: tr("Datum"),
      sub: dateEntries(dateAllTo, askDateAll, (field) => list.some((t) => t[field])),
    },
    "-",
    { label: tr("Verschieben …"), key: k("move"), run: moveAll },
    "-",
    { label: tr("Löschen …"), key: k("delete"), danger: true, run: () => askDelete() },
  ];
}

export async function copyLink(id) {
  try {
    await navigator.clipboard.writeText(`samu:${id}`);
    update({ message: info(tr("Link kopiert.")) });
  } catch {
    update({ message: info(`samu:${id}`) });
  }
}

// Separators only between entries, never twice or at an end; no empty
// entries, in submenus neither.
function tidy(entries) {
  const out = [];
  for (const entry of entries.filter(Boolean)) {
    if (entry === "-" && (!out.length || out.at(-1) === "-")) continue;
    // Not `entry.sub` alone: a string has a method of that name.
    out.push(entry !== "-" && entry.sub ? { ...entry, sub: tidy(entry.sub) } : entry);
  }
  while (out.at(-1) === "-") out.pop();
  return out;
}

const choosable = (entry) => Boolean(entry) && entry !== "-" && !entry.header;

// Opens the menu of task `id` at x, y (client coordinates).
export function openMenu(id, x, y) {
  const status = statusOf(id);
  if (!status) return;
  const entries = state.picked.has(id) ? many() : single(status);
  if (!state.picked.has(id)) update({ selected: id });
  menu = { entries: tidy(entries), index: -1, sub: null, x, y };
  render();
}

// The menu of the current row, below its start: Shift+F10, the menu key.
export function openMenuHere() {
  const id = state.selected;
  const row = id && document.querySelector(`#list [data-id="${id}"]`);
  if (!row) return;
  keyed = Date.now();
  const box = row.getBoundingClientRect();
  openMenu(id, box.left + 24, box.bottom);
  if (menu) {
    menu.index = menu.entries.findIndex(choosable);
    render();
  }
}

export function closeMenu() {
  if (!menu) return;
  menu = null;
  render();
}

function run(entry) {
  closeMenu();
  entry.run?.();
}

// The level keys act on: the submenu once one of its entries is current.
function level() {
  if (menu.sub && menu.sub.index >= 0) {
    return { entries: menu.entries[menu.index].sub, at: menu.sub, sub: true };
  }
  return { entries: menu.entries, at: menu, sub: false };
}

function step(entries, from, by) {
  const n = entries.length;
  for (let i = 1; i <= n; i += 1) {
    const at = (((from + by * i) % n) + n) % n;
    if (choosable(entries[at])) return at;
  }
  return from;
}

// Keys while the menu is open; it takes every key.
export function menuKey(event) {
  event.preventDefault();
  event.stopPropagation();
  const here = level();
  const current = here.entries[here.at.index];
  switch (event.key) {
    case "Escape":
    case "ArrowLeft":
    case "h":
      if (here.sub || menu.sub) menu.sub = null;
      else if (event.key === "Escape") return closeMenu();
      break;
    case "ArrowDown":
    case "j":
      here.at.index = step(here.entries, here.at.index, 1);
      if (!here.sub) menu.sub = null;
      break;
    case "ArrowUp":
    case "k":
      here.at.index = step(here.entries, here.at.index < 0 ? 0 : here.at.index, -1);
      if (!here.sub) menu.sub = null;
      break;
    case "ArrowRight":
    case "l":
    case "Enter":
    case " ":
      if (current?.sub) menu.sub = { index: step(current.sub, -1, 1) };
      // → and l only open a submenu; ↵ and the space run an entry.
      else if (choosable(current) && (event.key === "Enter" || event.key === " ")) return run(current);
      break;
    default: {
      const pressed = normalize(event);
      const hit = here.entries.find((e) => choosable(e) && !e.sub && e.key && (e.key === pressed || e.key === event.key));
      if (hit) return run(hit);
      return null;
    }
  }
  render();
  return null;
}

function item(entry, index, at, isSub) {
  if (entry === "-") return el("li", { class: "sep", role: "separator" });
  if (entry.header) return el("li", { class: "head", text: entry.label });
  const current = index === at.index;
  const classes = [current ? "current" : null, entry.danger ? "danger" : null, entry.sub ? "has-sub" : null];
  const children = [
    el("span", { class: "check", text: entry.checked ? "✓" : "" }),
    el("span", { class: "label", text: entry.label }),
    el("span", { class: "key", text: entry.sub ? "▸" : entry.hint ?? pretty(entry.key) }),
  ];
  if (!isSub && current && entry.sub && menu.sub) children.push(list(entry.sub, menu.sub, true));
  return el(
    "li",
    {
      role: "menuitem",
      class: classes.filter(Boolean).join(" ") || null,
      "aria-haspopup": entry.sub ? "menu" : null,
      onmouseenter: () => {
        if (at.index === index && (isSub || !entry.sub || menu.sub)) return;
        at.index = index;
        if (!isSub) menu.sub = entry.sub ? { index: -1 } : null;
        render();
      },
      onclick: (event) => {
        event.stopPropagation();
        if (entry.sub) {
          menu.index = index;
          menu.sub = { index: -1 };
          render();
        } else run(entry);
      },
    },
    children,
  );
}

function list(entries, at, isSub) {
  return el(
    "ul",
    { class: isSub ? "sub" : null, role: "menu" },
    entries.map((entry, index) => item(entry, index, at, isSub)),
  );
}

function render() {
  const box = $("menu");
  box.hidden = !menu;
  if (!menu) {
    replace(box);
    return;
  }
  replace(box, list(menu.entries, menu, false));
  // Within the window, and a submenu to the left when the right is full.
  const width = box.offsetWidth;
  const height = box.offsetHeight;
  const x = Math.max(4, Math.min(menu.x, window.innerWidth - width - 4));
  const y = Math.max(4, Math.min(menu.y, window.innerHeight - height - 4));
  box.style.left = `${x}px`;
  box.style.top = `${y}px`;
  const sub = box.querySelector("ul.sub");
  if (sub) {
    const r = sub.getBoundingClientRect();
    if (r.right > window.innerWidth - 4) sub.classList.add("left");
    if (r.bottom > window.innerHeight - 4) sub.style.top = `${window.innerHeight - 4 - r.bottom}px`;
  }
}

export function wireMenu() {
  document.addEventListener("contextmenu", (event) => {
    // After Shift+F10 or the menu key, the browser's own menu stays away.
    if (Date.now() - keyed < 500) {
      event.preventDefault();
      return;
    }
    if (event.target.closest("input, textarea, select, [contenteditable]")) return;
    const row = event.target.closest("#list [data-id]");
    if (!row) return;
    event.preventDefault();
    openMenu(row.dataset.id, event.clientX, event.clientY);
  });
  document.addEventListener("mousedown", (event) => {
    if (menu && !event.target.closest("#menu")) closeMenu();
  });
  window.addEventListener("blur", closeMenu);
  window.addEventListener("resize", closeMenu);
  // Scrolling by hand closes it; a list drawn anew under it does not.
  const away = (event) => {
    if (menu && !event.target.closest?.("#menu")) closeMenu();
  };
  document.addEventListener("wheel", away, { passive: true });
  document.addEventListener("touchmove", away, { passive: true });
}
