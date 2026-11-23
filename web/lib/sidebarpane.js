// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The settings page "Sidebar" (ADR 0142 §3), as in the app (ADR 0102 §3):
// the entries of the upper list with a handle and a switch, below them the
// other perspectives with a switch, and "Show all again". Kept in this
// browser (arrange.js).

import {
  INBOX,
  VIEWS,
  loadArrangement,
  moveBefore,
  parseKey,
  perspectiveKey,
  saveArrangement,
  setHidden,
  showAll,
  toPerspectives,
} from "./arrange.js";
import { el } from "./dom.js";
import { perspectiveOf, perspectives } from "./perspectives.js";
import { update } from "./state.js";
import { tr } from "./i18n.js";

// The key being dragged by its handle.
let dragged = null;

const save = (next) => {
  saveArrangement(next);
  update({});
};

function titleOf(key) {
  const what = parseKey(key);
  if (what?.kind === "perspective") {
    const p = perspectiveOf(what.id);
    return p ? `${p.icon ? `${p.icon} ` : ""}${p.title}` : null;
  }
  return VIEWS.find((v) => v.kind === what?.view)?.title ?? null;
}

function clearMarks(list) {
  for (const node of list.querySelectorAll(".drop-before, .drop-after")) node.classList.remove("drop-before", "drop-after");
}

// One row: handle, title, switch. `upper` rows take drops; the arrow keys
// on the handle move an upper row by one.
function row(a, key, upper) {
  const title = titleOf(key);
  if (!title) return null;
  const shown = !a.hidden.includes(key);
  const after = (event) => {
    const box = event.currentTarget.getBoundingClientRect();
    return event.clientY > box.top + box.height / 2;
  };
  const i = a.order.indexOf(key);
  return el(
    "li",
    {
      class: shown ? null : "off",
      draggable: "true",
      ondragstart: (event) => {
        dragged = key;
        event.dataTransfer.effectAllowed = "move";
        event.dataTransfer.setData("text/plain", key);
      },
      ondragend: () => {
        dragged = null;
      },
      ...(upper
        ? {
            ondragover: (event) => {
              if (!dragged || dragged === key) return;
              event.preventDefault();
              const list = event.currentTarget.parentNode;
              clearMarks(list);
              event.currentTarget.classList.add(after(event) ? "drop-after" : "drop-before");
            },
            ondrop: (event) => {
              if (!dragged) return;
              event.preventDefault();
              event.stopPropagation();
              clearMarks(event.currentTarget.parentNode);
              const moved = dragged;
              dragged = null;
              if (moved === key) return;
              const order = a.order.filter((k) => k !== moved);
              save(moveBefore(a, moved, after(event) ? (order[order.indexOf(key) + 1] ?? null) : key));
            },
          }
        : {}),
    },
    [
      el("button", {
        type: "button",
        class: "handle",
        text: "⠿",
        title: tr("Ziehen zum Anordnen"),
        "aria-label": tr("{title} verschieben", { title }),
        onkeydown: upper
          ? (event) => {
              if (event.key === "ArrowUp" && i > 0) save(moveBefore(a, key, a.order[i - 1]));
              else if (event.key === "ArrowDown" && i < a.order.length - 1) save(moveBefore(a, key, a.order[i + 2] ?? null));
              else return;
              event.preventDefault();
            }
          : null,
      }),
      el("span", { class: "label", text: title }),
      el("label", { class: "toggle" }, [
        el("input", {
          type: "checkbox",
          checked: shown,
          disabled: key === INBOX,
          "aria-label": tr("{title} in der Leiste zeigen", { title }),
          onchange: (event) => save(setHidden(a, key, !event.target.checked)),
        }),
      ]),
    ],
  );
}

export function sidebarPane() {
  const a = loadArrangement(perspectives().map((p) => p.id));
  const lower = perspectives()
    .map((p) => perspectiveKey(p.id))
    .filter((key) => !a.order.includes(key));
  // Dropped on the list below, a perspective goes back to the section.
  const back = {
    ondragover: (event) => {
      if (parseKey(dragged)?.kind === "perspective" && a.order.includes(dragged)) event.preventDefault();
    },
    ondrop: (event) => {
      if (!dragged) return;
      event.preventDefault();
      const moved = dragged;
      dragged = null;
      save(toPerspectives(a, moved));
    },
  };
  return el("div", { class: "sidebar-settings" }, [
    el("p", {
      class: "quiet",
      text: tr("Nur in diesem Browser. Ausgeblendetes öffnet weiter über die Palette und seine Taste; eine ausgeblendete Perspektive bleibt Block ihrer Block-Perspektive."),
    }),
    el("h4", { text: tr("Oben in der Leiste") }),
    el("ul", { class: "sidebar-entries" }, a.order.map((key) => row(a, key, true))),
    el("h4", { text: tr("Perspektiven") }),
    lower.length
      ? el("ul", { class: "sidebar-entries", ...back }, lower.map((key) => row(a, key, false)))
      : el("p", { class: "empty", ...back, text: tr("Alle Perspektiven stehen oben.") }),
    el("p", { class: "quiet", text: tr("Eine Perspektive kommt nach oben, wenn man sie auf einen Eintrag dort zieht.") }),
    el("button", {
      type: "button",
      disabled: a.hidden.length === 0,
      text: tr("Alle wieder zeigen"),
      onclick: () => save(showAll(a)),
    }),
  ]);
}
