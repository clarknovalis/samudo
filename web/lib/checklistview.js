// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The checklist of a task in the inspector (ADR 0141): a line to tick off
// each, typed fast. Enter makes the next item, Backspace on an empty one
// removes it, a pasted list splits into items (the rules of
// checklist.js), a handle or Alt+↑/↓ moves one, and one becomes a subtask.

import { api, change } from "./api.js";
import { $, el, uuidv7 } from "./dom.js";
import { allFromSubtasks, allToSubtasks, fromSubtask, itemsOf, lines, progress, toSubtask } from "./checklist.js";
import { state, update } from "./state.js";
import { tr } from "./i18n.js";

const hlc = () => ({ wall: 0, counter: 0, device: "web" });
const now = () => new Date().toISOString();

async function write(changes) {
  try {
    await api.write(changes);
    update({ message: null });
    return true;
  } catch (error) {
    update({ message: error.message });
    return false;
  }
}

// The page is drawn again after a write, later than the write returns: the
// cursor goes to the item once that draw has it.
let wanted = null;

function focus(id) {
  wanted = id;
  let tries = 0;
  const go = () => {
    const input = $(`cl-${id}`);
    if (wanted !== id) return;
    if (input?.isConnected) {
      input.focus();
      // Drawn again once more? Then the next try takes it from there.
      if (document.activeElement === input && ++tries > 2) wanted = null;
    }
    if (wanted === id && tries < 10) setTimeout(go, 50);
  };
  go();
}

const newItem = (owner, text, checked, position) => {
  const created = now();
  const value = { id: uuidv7(), created, modified: created, owner, text, position };
  if (checked) value.checked = true;
  return value;
};

// The changes that give `order` (items, some of them new values to create)
// their places 0, 1, 2 …; only what is not there yet.
function placing(order) {
  const changes = [];
  order.forEach((item, position) => {
    if (item.isNew) changes.push(change.create("checklist_item", { ...item.value, position }));
    else if (item.position !== position) changes.push(change.set(item.id, "position", position));
  });
  return changes;
}

// New items after `at` (an item id, or null for the end), from text.
async function insertAfter(task, at, texts, then = null) {
  const list = itemsOf(state.checklist, task);
  const index = at ? list.findIndex((i) => i.id === at) + 1 : list.length;
  const fresh = texts.map((l) => {
    const value = newItem(task, l.text, l.checked, 0);
    return { id: value.id, isNew: true, value };
  });
  const order = [...list.slice(0, index), ...fresh, ...list.slice(index)];
  // `then`: where the cursor goes instead of the last new item, as an id of
  // `cl-…`; the adder keeps it to type the next one.
  if (await write(placing(order))) focus(then ?? fresh.at(-1).id);
}

async function move(task, id, to) {
  const list = itemsOf(state.checklist, task);
  const from = list.findIndex((i) => i.id === id);
  if (from < 0 || to < 0 || to >= list.length || to === from) return;
  const [item] = list.splice(from, 1);
  list.splice(to, 0, item);
  if (await write(placing(list))) focus(id);
}

let dragging = null;

function itemRow(task, item, list) {
  const index = list.indexOf(item);
  const input = el("input", {
    type: "text",
    class: "cl-text",
    id: `cl-${item.id}`,
    value: item.text,
    "aria-label": tr("Checkpunkt"),
    onchange: (event) => {
      const text = event.target.value.trim();
      if (text && text !== item.text) write([change.set(item.id, "text", text)]);
    },
    onkeydown: (event) => {
      const text = event.target.value.trim();
      if (event.key === "Enter" && !event.isComposing) {
        event.preventDefault();
        const rest = text !== item.text && text ? [change.set(item.id, "text", text)] : [];
        // A change of the text goes with the new item, in one write.
        const fresh = newItem(task, "", false, 0);
        const order = [...list.slice(0, index + 1), { id: fresh.id, isNew: true, value: fresh }, ...list.slice(index + 1)];
        write([...rest, ...placing(order)]).then((ok) => ok && focus(fresh.id));
      } else if (event.key === "Backspace" && event.target.value === "") {
        event.preventDefault();
        const before = list[index - 1] ?? list[index + 1];
        write([change.delete(item.id)]).then((ok) => ok && before && focus(before.id));
      } else if (event.altKey && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
        event.preventDefault();
        move(task, item.id, index + (event.key === "ArrowUp" ? -1 : 1));
      }
    },
    onpaste: (event) => {
      const pasted = event.clipboardData?.getData("text") ?? "";
      const got = lines(pasted);
      if (!/\r?\n/.test(pasted.trim()) || got.length < 2) return;
      event.preventDefault();
      // Into an empty item the first line goes itself.
      const own = event.target.value.trim() === "" ? got.shift() : null;
      if (own) write([change.set(item.id, "text", own.text), ...(own.checked ? [change.set(item.id, "checked", true)] : [])]);
      insertAfter(task, item.id, got);
    },
  });
  return el(
    "li",
    {
      class: item.checked ? "cl-item done" : "cl-item",
      ondragover: (event) => dragging && event.preventDefault(),
      ondrop: (event) => {
        event.preventDefault();
        if (dragging && dragging !== item.id) move(task, dragging, index);
        dragging = null;
      },
    },
    [
      el("span", {
        class: "cl-handle",
        draggable: "true",
        title: tr("Ziehen zum Verschieben (Alt+↑/↓)"),
        text: "⋮⋮",
        ondragstart: (event) => {
          dragging = item.id;
          event.dataTransfer.effectAllowed = "move";
          event.dataTransfer.setData("text/plain", item.id);
        },
        ondragend: () => {
          dragging = null;
        },
      }),
      el("input", {
        type: "checkbox",
        class: "tick",
        checked: item.checked,
        "aria-label": tr("erledigt"),
        onchange: (event) => write([change.set(item.id, "checked", event.target.checked)]),
      }),
      input,
      el("button", {
        type: "button",
        class: "cl-tool",
        title: tr("In Unteraufgabe umwandeln"),
        "aria-label": tr("In Unteraufgabe umwandeln"),
        text: "↳",
        onclick: () =>
          write(toSubtask({ tasks: state.tasks, items: state.checklist, item: item.id, id: uuidv7(), now: now(), hlc })),
      }),
      el("button", {
        type: "button",
        class: "cl-tool",
        title: tr("Löschen"),
        "aria-label": tr("Löschen"),
        text: "×",
        onclick: () => write([change.delete(item.id)]),
      }),
    ],
  );
}

// The section's title carries the count: „Checkliste 2/5“.
export const checklistTitle = (task) => {
  const [done, all] = progress(state.checklist, task);
  return all ? `${tr("Checkliste")} ${done}/${all}` : tr("Checkliste");
};

// The body of the section for `task`: the items and the line that adds.
export function checklistBody(task) {
  const list = itemsOf(state.checklist, task);
  const adder = el("input", {
    type: "text",
    class: "cl-add",
    id: `cl-add-${task}`,
    placeholder: tr("Punkt hinzufügen …"),
    "aria-label": tr("Punkt hinzufügen …"),
    onkeydown: (event) => {
      const text = event.target.value.trim();
      if (event.key === "Enter" && !event.isComposing && text) {
        event.preventDefault();
        event.target.value = "";
        insertAfter(task, null, lines(text), `add-${task}`);
      }
    },
    onpaste: (event) => {
      const pasted = event.clipboardData?.getData("text") ?? "";
      const got = lines(pasted);
      if (!/\r?\n/.test(pasted.trim()) || got.length < 2) return;
      event.preventDefault();
      insertAfter(task, null, got, `add-${task}`);
    },
  });
  return el("div", { class: "checklist" }, [
    list.length ? el("ul", { class: "cl-items" }, list.map((item) => itemRow(task, item, list))) : null,
    adder,
    list.length > 1
      ? el("button", {
          type: "button",
          class: "cl-all",
          title: tr("Alle in Unteraufgaben umwandeln"),
          text: tr("↳ Alle in Unteraufgaben"),
          onclick: () => write(allToSubtasks({ tasks: state.tasks, items: state.checklist, task, now: now(), hlc, newId: uuidv7 })),
        })
      : null,
    state.tasks.some((t) => t.parent === task)
      ? el("button", {
          type: "button",
          class: "cl-all",
          title: tr("Alle Unteraufgaben in Checkpunkte umwandeln"),
          text: tr("↴ Alle Unteraufgaben in Punkte"),
          onclick: async () => {
            let got;
            try {
              got = allFromSubtasks({ tasks: state.tasks, items: state.checklist, task, now: now(), hlc, newId: uuidv7 });
            } catch (error) {
              update({ message: error.message });
              return;
            }
            if (await write(got.changes) && got.skipped.length) {
              update({ message: got.skipped.map((s) => `${s.title}: ${s.reason}`).join("\n") });
            }
          },
        })
      : null,
  ]);
}

// The button that makes a plain subtask an item of its parent; null for a
// task without a parent. What an item cannot keep is named in the message.
export function toItemButton(task) {
  if (!task.parent) return null;
  return el("button", {
    type: "button",
    text: tr("In Checkpunkt umwandeln"),
    onclick: async () => {
      let changes;
      try {
        changes = fromSubtask({ tasks: state.tasks, items: state.checklist, task: task.id, id: uuidv7(), now: now(), hlc });
      } catch (error) {
        update({ message: error.message });
        return;
      }
      if (await write(changes)) update({ selected: task.parent });
    },
  });
}
