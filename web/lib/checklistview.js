// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The checklist of a task in the inspector (ADR 0141): a line to tick off
// each, typed fast. Enter opens the next line, which becomes an item once
// it has a text, Backspace on an empty one removes it, a pasted list
// splits into items (the rules of checklist.js), a handle or Alt+↑/↓
// moves one, and one becomes a subtask.

import { api, change } from "./api.js";
import { $, el, uuidv7 } from "./dom.js";
import {
  allFromSubtasks,
  allToSubtasks,
  fromSubtask,
  itemsOf,
  lines,
  pasted,
  placing as places,
  progress,
  spliced,
  toSubtask,
} from "./checklist.js";
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
const placing = (order) =>
  places(order).map((step) =>
    step.create ? change.create("checklist_item", step.create) : change.set(step.set, "position", step.position),
  );

// The line Enter opens under an item: { task, after, text, typing }. It is
// only here, in the page, until it has a text; an item without one never
// reaches the server, which would refuse it. `text` and `typing` carry what
// is typed across a redraw of the page.
let draft = null;

// New items after `at` (an item id, or null for the end), from text, with
// `first` (changes to the item pasted into) in the same write. `then`: where
// the cursor goes instead of the last new item, as an id of `cl-…`; the
// adder and the draft keep it to type the next one, `false` leaves it.
async function insertAfter(task, at, texts, then = null, first = []) {
  const list = itemsOf(state.checklist, task);
  const fresh = texts.map((l) => {
    const value = newItem(task, l.text, l.checked, 0);
    return { id: value.id, isNew: true, value };
  });
  const last = fresh.at(-1)?.id ?? at;
  // The draft goes on under what it just made.
  if (then === "draft") draft = { task, after: last, text: "", typing: true };
  const ok = await write([...first, ...placing(spliced(list, at, fresh))]);
  if (!ok && then === "draft") draft = { task, after: at, text: texts.map((l) => l.text).join(" "), typing: true };
  if (ok && then !== false) focus(then ?? last);
}

function closeDraft(back) {
  draft = null;
  update({});
  if (back) focus(back);
}

// The draft under the item `after` of `task`.
function draftRow(task, after) {
  const commit = (text, then) => insertAfter(task, after, lines(text), then);
  const input = el("input", {
    type: "text",
    class: "cl-text",
    id: "cl-draft",
    value: draft.text,
    placeholder: tr("Punkt hinzufügen …"),
    "aria-label": tr("Punkt hinzufügen …"),
    oninput: (event) => {
      if (draft) draft.text = event.target.value;
    },
    onfocus: () => {
      if (draft) draft.typing = true;
    },
    onblur: (event) => {
      const left = event.target;
      // A redraw takes the field out of the page; that is not leaving it.
      setTimeout(() => {
        if (!left.isConnected || !draft || document.activeElement === left) return;
        const text = left.value.trim();
        draft = null;
        left.closest("li")?.remove();
        if (text) commit(text, false);
      }, 0);
    },
    onkeydown: (event) => {
      const text = event.target.value.trim();
      if (event.key === "Enter" && !event.isComposing) {
        event.preventDefault();
        if (text) commit(text, "draft");
        else closeDraft(after);
      } else if (event.key === "Escape" || (event.key === "Backspace" && event.target.value === "")) {
        event.preventDefault();
        event.stopPropagation();
        closeDraft(after);
      }
    },
    onpaste: (event) => {
      const got = lines(event.clipboardData?.getData("text") ?? "");
      if (got.length < 2) return;
      event.preventDefault();
      insertAfter(task, after, got, "draft");
    },
  });
  if (draft.typing) focus("draft");
  return el("li", { class: "cl-item cl-draft" }, [input]);
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
        // The next line opens here and is written once it has a text.
        draft = { task, after: item.id, text: "", typing: true };
        if (text && text !== item.text) write([change.set(item.id, "text", text)]);
        else update({});
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
      const clip = event.clipboardData?.getData("text") ?? "";
      const got = lines(clip);
      if (!/\r?\n/.test(clip.trim()) || got.length < 2) return;
      event.preventDefault();
      // Into an empty item the first line goes itself, in the same write.
      const { own, rest } = pasted(event.target.value, got);
      const first = own
        ? [change.set(item.id, "text", own.text), ...(own.checked ? [change.set(item.id, "checked", true)] : [])]
        : [];
      insertAfter(task, item.id, rest, null, first);
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
    list.length
      ? el(
          "ul",
          { class: "cl-items" },
          list.flatMap((item) => [
            itemRow(task, item, list),
            ...(draft?.task === task && draft.after === item.id ? [draftRow(task, item.id)] : []),
          ]),
        )
      : null,
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
              got = allFromSubtasks({ tasks: state.tasks, items: state.checklist, around: around(), task, now: now(), hlc, newId: uuidv7 });
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

// What hangs on tasks and would go with a subtask that becomes an item.
const around = () => ({
  comments: state.comments,
  attachments: state.attachments,
  clocks: state.clocks,
  reminders: state.reminders,
  links: state.links,
});

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
        changes = fromSubtask({ tasks: state.tasks, items: state.checklist, around: around(), task: task.id, id: uuidv7(), now: now(), hlc });
      } catch (error) {
        update({ message: error.message });
        return;
      }
      if (await write(changes)) update({ selected: task.parent });
    },
  });
}
