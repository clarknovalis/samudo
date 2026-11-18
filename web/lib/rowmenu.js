// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The menu „Zeile …“ beside the filter (ADR 0130 §6): for every field of
// a row its place, line 1, line 2 or off, and the short form. One layout
// for every list, Forecast and perspective, kept in this browser only
// (§7); it replaces the switch „Details“ (ADR 0053).

import { $, el, replace } from "./dom.js";
import { tr } from "./i18n.js";
import { DEFAULT_LAYOUT, FIELDS, layoutOf, withPlace } from "./rowfields.js";
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

const PLACES = [
  [1, "1", tr("in Zeile 1")],
  [2, "2", tr("in Zeile 2")],
  [0, tr("aus"), tr("nicht zeigen")],
];

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
  const rows = FIELDS.map(([key, word]) =>
    el("li", {}, [
      el("span", { class: "row-field", text: word }),
      el(
        "span",
        { class: "row-places", role: "group", "aria-label": word },
        PLACES.map(([place, text, label]) =>
          el("button", {
            type: "button",
            "data-k": `${key}:${place}`,
            class: layout.places[key] === place ? "current" : null,
            "aria-pressed": String(layout.places[key] === place),
            "aria-label": `${word} ${label}`,
            text,
            onclick: () => save(withPlace(layout, key, place)),
          }),
        ),
      ),
    ]),
  );
  replace(box, [
    el("ul", { class: "row-fields" }, rows),
    el("label", { class: "check" }, [
      el("input", { type: "checkbox", "data-k": "short", checked: layout.short, onchange: (event) => save({ ...layout, short: event.target.checked }) }),
      tr("Kurzform mit Symbolen"),
    ]),
    el("div", { class: "order-actions" }, [
      el("button", { type: "button", "data-k": "reset", text: tr("Vorgabe"), onclick: () => save({ places: { ...DEFAULT_LAYOUT.places }, short: DEFAULT_LAYOUT.short }) }),
      el("button", { type: "button", "data-k": "done", text: tr("Fertig"), onclick: toggleRowMenu }),
    ]),
  ]);
  if (focused) box.querySelector(`[data-k="${CSS.escape(focused)}"]`)?.focus();
}
