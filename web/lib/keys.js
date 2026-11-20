// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Keyboard first (ADR 0020 §4), bound as the account says (ADR 0079). The
// table of actions and the bindings live in keymap.js; here each action id
// gets what it does, and a key press is looked up in the bindings in effect.
// The help (`?`) is drawn from the same table.

import { api, change } from "./api.js";
import { reportTab } from "./report.js";
import { boardFull, setBoardFull, shiftSelected } from "./board.js";
import { openCapture } from "./capture.js";
import { closeTasks } from "./closeask.js";
import { openEdit } from "./edit.js";
import {
  askDelete,
  clearPicks,
  completeAll,
  cycleAll,
  dateAllTo,
  dropAll,
  flagAll,
  moveAll,
  pickStateAll,
  pickedStatuses,
  picking,
  togglePick,
} from "./bulk.js";
import { toggleClock } from "./clock.js";
import { $ } from "./dom.js";
import { cycleState, pickState, toggleFlag } from "./detail.js";
import { ACTIONS, action, conflicts, effective, inEffect, normalize, pretty } from "./keymap.js";
import { foldHeadings, order, setDone } from "./list.js";
import { copyLink, menuKey, menuOpen, openMenuHere } from "./menu.js";
import { shiftTask } from "./reorder.js";
import { indent, openBlockPalette, openGoPalette, openPalette, outdent } from "./palette.js";
import { tableKey } from "./table.js";
import { closeSettings, openSettings } from "./settings.js";
import { chooseView, cycleSidebar } from "./sidebar.js";
import { info } from "./message.js";
import { isOpen, shown, state, statusOf, update } from "./state.js";
import { keyedTemplates } from "./templates.js";
import { cancelVoice, toggleVoice } from "./voice.js";
import { addDays, today } from "./words.js";
import { tr } from "./i18n.js";

// The bindings in effect, from the account's settings.
export const bindings = () => inEffect(state.accountSettings?.keys);

// The binding of an action, null for none.
export const keyOf = (id) => bindings().byId.get(id) ?? null;

// Rows of the help that are not actions.
const NOTES = [
  [tr("x f m s t … mit Auswahl"), tr("wirken auf alle ausgewählten; Esc hebt die Auswahl auf")],
  [tr("⌫ im leeren Suchfeld"), tr("letzten Chip entfernen")],
  [tr("← → Enter in der Tabelle"), tr("Zelle wählen und bearbeiten; Esc bricht ab")],
  ["Esc", tr("zurück, Feld verlassen")],
];

// The help: every action with a key, then the notes.
export function helpRows() {
  const rows = ACTIONS.filter((a) => keyOf(a.id)).map((a) => [
    [pretty(keyOf(a.id)), ...(a.fixed ?? []).map(pretty)].join(" / "),
    a.hint ? `${a.title}: ${a.hint}` : a.title,
  ]);
  return [...rows, ...NOTES];
}

function move(step) {
  if (!order.length) return;
  const at = order.indexOf(state.selected);
  const next = at < 0 ? 0 : Math.min(order.length - 1, Math.max(0, at + step));
  update({ selected: order[next] });
  document.querySelector(`#list [data-id="${order[next]}"]`)?.scrollIntoView({ block: "nearest" });
}

async function write(changes) {
  try {
    await api.write(changes);
  } catch (error) {
    update({ message: error.message });
  }
}

const go = (kind) => () => chooseView({ kind, project: null });
const onBoard = () => !$("board-full").hidden;

let pickingState = false;

// What each action does: (status, event) → false when it did not apply,
// so the key goes on to the browser.
const RUN = {
  next: () => move(1),
  prev: () => move(-1),
  open: (s) => s && update({ pane: "detail" }),
  pick: (s, event) => {
    // On a focused button the space presses it.
    if (event.target.tagName === "BUTTON") return false;
    if (s) togglePick(s.task.id);
  },
  complete: (s) => (picking() ? completeAll() : s && setDone(s, isOpen(s.task))),
  "state.cycle": (s) => (picking() ? cycleAll() : s && cycleState(s)),
  "state.pick": (s) => (picking() || s) && (pickingState = true),
  flag: (s) => (picking() ? flagAll() : s && toggleFlag(s)),
  clock: (s) => s && toggleClock(s),
  move: (s) => (picking() ? moveAll() : s && openPalette(s)),
  block: (s) => s && isOpen(s.task) && openBlockPalette(s),
  indent: (s) => s && indent(s),
  outdent: (s) => s && outdent(s),
  "shift.up": (s) => s && shiftTask(s, -1),
  "shift.down": (s) => s && shiftTask(s, 1),
  "board.left": () => shiftSelected(-1),
  "board.right": () => shiftSelected(1),
  "fold.all": () => foldHeadings(),
  "board.full": () => onBoard() && setBoardFull(!boardFull()),
  "capture.child": (s) => {
    if (s && isOpen(s.task)) openCapture(s.task.id);
    else if (s) update({ message: tr("Unter Erledigtes kommt keine Unteraufgabe.") });
  },
  edit: (s) => (picking() ? openEdit(pickedStatuses().map((p) => p.task.id)) : s && openEdit(s.task.id)),
  menu: (s) => s && openMenuHere(),
  delete: (s) => (picking() ? askDelete() : s && askDelete([s.task.id])),
  drop: (s) =>
    picking() ? dropAll() : s && isOpen(s.task) && closeTasks([s.task.id], "dropped"),
  "review.done": (s) => {
    const project = s && !s.inbox ? s.task.project : shown().kind === "project" ? shown().project : null;
    if (!project) return false;
    write(change.reviewed(project));
  },
  "copy-link": (s) => s && copyLink(s.task.id),
  "date.today": (s) => due(s, today()),
  "date.tomorrow": (s) => due(s, addDays(today(), 1)),
  capture: () => openCapture(),
  "capture.lines": () => openCapture(null, null, null, true),
  "capture.template": () => startTemplates(),
  voice: () => toggleVoice(),
  search: () => $("search-input").focus(),
  sidebar: () => cycleSidebar(),
  full: (s) => s && update(state.full ? { full: false } : { full: true, pane: "detail" }),
  help: () => update({ help: !state.help }),
  "go.open": () => openGoPalette(),
  "go.inbox": go("inbox"),
  "go.forecast": go("forecast"),
  "go.flagged": go("flagged"),
  "go.projects": go("everything"),
  "go.review": go("review"),
  // ADR 0081 §1: the report, at its tab „Zeit“.
  "go.time": () => {
    reportTab("time");
    go("time")();
  },
  "go.calendar": go("calendar"),
  "go.settings": () => openSettings(),
};

function due(s, day) {
  if (picking()) return dateAllTo("due", day);
  if (s) return write([change.set(s.task.id, "due", { date: day })]);
}

// Whether an action works where the page stands now.
function here(a) {
  if (a.place === "board") return onBoard();
  if (a.place === "list") return !onBoard();
  return true;
}

// The action bound to `binding` here (fixed keys included), or null.
function bound(binding) {
  const { byId } = bindings();
  return (
    ACTIONS.find((a) => here(a) && (byId.get(a.id) === binding || (a.fixed ?? []).includes(binding))) ?? null
  );
}

// The second key of a sequence is awaited after its first (ADR 0079 §6).
let prefix = null;
let prefixTimer = null;

function endPrefix() {
  clearTimeout(prefixTimer);
  prefix = null;
  update({ message: null });
}

function startPrefix(first) {
  const { byId } = bindings();
  const next = ACTIONS.filter((a) => here(a) && byId.get(a.id)?.startsWith(`${first} `)).map(
    (a) => `${pretty(byId.get(a.id).slice(first.length + 1))} ${a.title}`,
  );
  if (!next.length) return false;
  prefix = first;
  clearTimeout(prefixTimer);
  prefixTimer = setTimeout(endPrefix, 2000);
  update({ message: info(`${pretty(first)} … ${next.join(" · ")}`) });
  return true;
}

// After the capture prefix the next key is a template's (ADR 0084 §2):
// the continuations come from the templates, not from the table, like the
// keys of the states after `t`.
const TEMPLATES = Symbol("templates");

function startTemplates() {
  const list = keyedTemplates();
  if (!list.length) {
    update({ message: tr("Keine Vorlage mit Taste – Einstellungen > Vorlagen") });
    return;
  }
  prefix = TEMPLATES;
  clearTimeout(prefixTimer);
  prefixTimer = setTimeout(endPrefix, 2000);
  const first = pretty(keyOf("capture.template") ?? "");
  update({ message: info(`${first} … ${list.map((t) => `${t.key} ${t.title}`).join(" · ")}`) });
}

// The template whose key was pressed: `h` and `H` are two keys, so the
// character as typed, not the normalized binding.
function captureWith(event) {
  if (event.ctrlKey || event.altKey || event.metaKey) return;
  const t = keyedTemplates().find((t) => t.key === event.key);
  if (t) openCapture(null, ` /${t.title}`, 0);
}

function run(a, event) {
  const done = RUN[a.id]?.(statusOf(state.selected), event);
  if (done === false) return;
  event.preventDefault();
}

export function wireKeys() {
  // The defaults must not clash among themselves (ADR 0079, Konsequenzen).
  const clashing = conflicts(effective({}).byId);
  if (clashing.length) {
    update({
      message: `${tr("Tastenvorgaben kollidieren:")} ${clashing.map((p) => p.map((id) => action(id).title).join(" / ")).join(", ")}`,
    });
  }
  document.addEventListener("keydown", (event) => {
    // An open menu takes every key (ADR 0062 §4).
    if (menuOpen()) {
      menuKey(event);
      return;
    }
    const typing = ["INPUT", "TEXTAREA", "SELECT"].includes(event.target.tagName);
    // The settings window keeps its keys to itself.
    if (state.settings) {
      if (event.key === "Escape") {
        if (typing) event.target.blur();
        else closeSettings();
      }
      return;
    }
    if (event.key === "Escape") {
      if (prefix) return endPrefix();
      if (cancelVoice()) return;
      if (typing) event.target.blur();
      else if (boardFull()) setBoardFull(false);
      else if (state.help) update({ help: false });
      else if (picking() && !state.palette) clearPicks();
      else if (state.full) update({ full: false });
      else update({ pane: "list", message: null });
      return;
    }
    if (typing || !$("gate").hidden) return;
    const key = normalize(event);
    if (!key) return;
    if (pickingState) {
      pickingState = false;
      // The key of a state comes from its tag, not from the table.
      const status = statusOf(state.selected);
      if (picking()) pickStateAll(event.key);
      else if (status) pickState(status, event.key);
      event.preventDefault();
      return;
    }
    if (prefix === TEMPLATES) {
      endPrefix();
      captureWith(event);
      event.preventDefault();
      return;
    }
    if (prefix) {
      const first = prefix;
      endPrefix();
      const a = bound(`${first} ${key}`);
      if (a) run(a, event);
      else event.preventDefault();
      return;
    }
    // In the table ← → choose a cell and Enter opens it (ADR 0115 §6).
    if (tableKey(key)) {
      event.preventDefault();
      return;
    }
    const a = bound(key);
    if (a) return run(a, event);
    if (startPrefix(key)) event.preventDefault();
  });
}
