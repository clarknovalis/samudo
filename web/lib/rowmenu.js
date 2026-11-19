// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The menu „Zeile …“ beside the filter (ADR 0130 §6, ADR 0131 §1): three
// rows, line 1 with the title as a fixed piece, line 2 and off; a field
// is dragged to its place, or moved with the arrow keys. One layout for
// every list, Forecast and perspective, kept in this browser only; it
// replaces the switch „Details“ (ADR 0053).

import { $, el, replace } from "./dom.js";
import { tr } from "./i18n.js";
import { DEFAULT_LAYOUT, FIELDS, TITLE, layoutOf, lineOf, move } from "./rowfields.js";
import { update } from "./state.js";

const KEY = "samu.rowfields";
// The switch of ADR 0053, read once into the layout.
const LEGACY = "samu.details";

let layout = (() => {
  try {
    return layoutOf(localStorage.getItem(KEY), localStorage.getItem(LEGACY));
  } catch {
    return layoutOf(null);
  }
})();

export const rowLayout = () => layout;

function save(next) {
  layout = next;
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // Private mode: the choice lasts as long as the page.
  }
  update({});
}

let menuOpen = false;

export function toggleRowMenu() {
  menuOpen = !menuOpen;
  update({});
}

export function closeRowMenu() {
  if (menuOpen) toggleRowMenu();
}

export const rowMenuOpen = () => menuOpen;

const ROWS = [
  [1, tr("Zeile 1")],
  [2, tr("Zeile 2")],
  [0, tr("Aus")],
];

const wordOf = (key) => (key === TITLE ? tr("Titel") : (FIELDS.find(([k]) => k === key)?.[1] ?? key));

// The keys of a row of the menu; off in the order of FIELDS.
const keysOf = (line) => (line === 1 ? layout.line1 : line === 2 ? layout.line2 : FIELDS.map(([key]) => key).filter((key) => !lineOf(layout, key)));

// `key` into row `line` before position `index` of what the row shows now.
function drop(key, line, index = Infinity) {
  if (key === TITLE && line !== 1) return;
  const from = line ? keysOf(line).indexOf(key) : -1;
  save(move(layout, key, line, from >= 0 && from < index ? index - 1 : index));
}

// The arrow keys of a chip (ADR 0131 §4): ← → within its row, ↑ ↓ to the
// end of the row above or below; the title only ← →.
function arrow(event, key, line, index) {
  const up = { 2: 1, 0: 2 }[line];
  const down = { 1: 2, 2: 0 }[line];
  if (event.key === "ArrowLeft" && line && index > 0) save(move(layout, key, line, index - 1));
  else if (event.key === "ArrowRight" && line && index < keysOf(line).length - 1) save(move(layout, key, line, index + 1));
  else if (event.key === "ArrowUp" && key !== TITLE && up !== undefined) save(move(layout, key, up));
  else if (event.key === "ArrowDown" && key !== TITLE && down !== undefined) save(move(layout, key, down));
  else return;
  event.preventDefault();
}

let dragging = null;
const accepts = (line) => dragging && (dragging !== TITLE || line === 1);

// `shown`: whether the view has rows to lay out.
export function renderRowMenu(shown) {
  const box = $("row-menu");
  const button = $("row-fields");
  button.hidden = !shown;
  button.setAttribute("aria-expanded", String(shown && menuOpen));
  box.hidden = !shown || !menuOpen;
  if (box.hidden) {
    replace(box);
    return;
  }
  const focused = document.activeElement?.closest?.("#row-menu") ? document.activeElement.dataset.k : null;
  const rows = ROWS.map(([line, name]) =>
    el(
      "div",
      {
        class: "row-line",
        "data-line": String(line),
        ondragover: (event) => accepts(line) && event.preventDefault(),
        ondrop: (event) => {
          event.preventDefault();
          if (accepts(line)) drop(dragging, line);
        },
      },
      [
        el("span", { class: "row-line-name", text: name }),
        ...keysOf(line).map((key, index) =>
          el("button", {
            type: "button",
            class: key === TITLE ? "row-chip row-title" : "row-chip",
            draggable: "true",
            "data-k": key,
            "aria-label": `${wordOf(key)}, ${name}`,
            title: tr("Ziehen oder mit den Pfeiltasten verschieben"),
            text: wordOf(key),
            ondragstart: (event) => {
              dragging = key;
              event.dataTransfer.setData("text/plain", key);
              event.dataTransfer.effectAllowed = "move";
            },
            ondragend: () => (dragging = null),
            ondragover: (event) => accepts(line) && event.preventDefault(),
            ondrop: (event) => {
              event.preventDefault();
              event.stopPropagation();
              if (accepts(line)) drop(dragging, line, line ? index : Infinity);
            },
            onkeydown: (event) => arrow(event, key, line, index),
          }),
        ),
      ],
    ),
  );
  replace(box, [
    ...rows,
    el("label", { class: "check" }, [
      el("input", { type: "checkbox", "data-k": "short", checked: layout.short, onchange: (event) => save({ ...layout, short: event.target.checked }) }),
      tr("Kurzform mit Symbolen"),
    ]),
    el("div", { class: "order-actions" }, [
      el("button", { type: "button", "data-k": "reset", text: tr("Vorgabe"), onclick: () => save(layoutOf(JSON.stringify(DEFAULT_LAYOUT))) }),
      el("button", { type: "button", "data-k": "done", text: tr("Fertig"), onclick: toggleRowMenu }),
    ]),
  ]);
  if (focused) box.querySelector(`[data-k="${CSS.escape(focused)}"]`)?.focus();
}
