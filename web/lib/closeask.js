// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The one way the web completes or drops tasks (ADR 0128 §5): through
// POST /objects/{id}/close, and with open tasks below first a question,
// mit erledigen / herausholen / abbrechen, one for all tasks at once.
// Opening again stays a plain field change (§6).

import { api, together } from "./api.js";
import { deepestFirst, waiting } from "./close.js";
import { flushDetail } from "./detail.js";
import { el } from "./dom.js";
import { tr } from "./i18n.js";
import { info } from "./message.js";
import { byId, isOpen, state, update } from "./state.js";

// Completes (`field` "completed") or drops ("dropped") the open ones of
// `ids`. True when something was written.
export async function closeTasks(ids, field) {
  // What the inspector holds goes first (e72d21c3).
  await flushDetail();
  const open = ids.filter((id) => {
    const task = byId(state.tasks, id);
    return task && isOpen(task);
  });
  if (!open.length) return false;
  const ordered = deepestFirst(state.tasks, open);
  let children = "ask";
  const wait = waiting(state.tasks, ordered);
  if (wait.length) {
    children = await ask(wait, ordered.length, field);
    if (!children) return false;
  }
  return send(ordered, field, children);
}

async function send(ids, field, children) {
  let lifted = 0;
  let wrote = false;
  // Open below on the server though not here: what was read is old.
  const left = [];
  try {
    // One reload for all of them, not one after each.
    await together(async () => {
      for (const id of ids) {
        const answer = await api.closeObject(id, { field, children });
        if (answer.seq == null && answer.open > 0) left.push({ id, open: answer.open });
        else {
          lifted += answer.lifted;
          wrote ||= answer.seq != null;
        }
      }
    });
  } catch (error) {
    update({ message: error.message });
    return wrote;
  }
  if (left.length) {
    const again = await ask(left, left.length, field);
    if (again) return (await send(left.map((w) => w.id), field, again)) || wrote;
    return wrote;
  }
  update({ message: lifted ? info(lifted === 1 ? tr("1 Unteraufgabe herausgeholt.") : tr("{n} Unteraufgaben herausgeholt.", { n: lifted })) : null });
  return wrote;
}

// What the question says: about the one task, or how many of the tasks.
function question(wait, total) {
  if (total === 1) {
    const title = byId(state.tasks, wait[0].id)?.title ?? "";
    return wait[0].open === 1
      ? tr("„{title}“ hat 1 offene Unteraufgabe.", { title })
      : tr("„{title}“ hat {n} offene Unteraufgaben.", { title, n: wait[0].open });
  }
  return tr("{k} der {n} Aufgaben haben offene Unteraufgaben.", { k: wait.length, n: total });
}

// The dialog; answers "close", "lift" or null for abbrechen. „Mit
// erledigen“ has the focus (F1), so Enter takes it.
function ask(wait, total, field) {
  return new Promise((resolve) => {
    const done = (choice) => {
      window.removeEventListener("keydown", onKey, true);
      overlay.remove();
      resolve(choice);
    };
    // Every key stays in the dialog; Enter, Space and Tab still work the
    // buttons.
    const onKey = (event) => {
      event.stopPropagation();
      if (event.key === "Escape") {
        event.preventDefault();
        done(null);
      }
    };
    const along = el("button", {
      type: "button",
      class: "primary",
      text: field === "dropped" ? tr("Mit verwerfen") : tr("Mit erledigen"),
      onclick: () => done("close"),
    });
    const box = el("div", { class: "box close-ask", role: "dialog", "aria-modal": "true", "aria-labelledby": "close-ask-text" }, [
      el("p", { id: "close-ask-text", text: question(wait, total) }),
      el("p", {
        class: "quiet",
        text:
          field === "dropped"
            ? tr("Mit verwerfen: alle offenen darunter auch. Herausholen: sie rücken eine Ebene höher.")
            : tr("Mit erledigen: alle offenen darunter auch. Herausholen: sie rücken eine Ebene höher."),
      }),
      el("div", { class: "actions" }, [
        along,
        el("button", { type: "button", text: tr("Herausholen"), onclick: () => done("lift") }),
        el("button", { type: "button", text: tr("Abbrechen"), onclick: () => done(null) }),
      ]),
    ]);
    const overlay = el("div", { class: "overlay" }, [box]);
    overlay.addEventListener("click", (event) => {
      if (event.target === overlay) done(null);
    });
    window.addEventListener("keydown", onKey, true);
    document.body.append(overlay);
    along.focus();
  });
}
