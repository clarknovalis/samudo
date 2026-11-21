// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The edit dialog (ADR 0078): a task's title in its own field, what is set
// as chips with ✕, and a line in the quick-capture syntax for what comes
// on top. The server applies the line (POST /capture with `task`); the
// title and the removals go first as ordinary changes. Opened with `e` and
// from the task's menu.
//
// On picked tasks (§5) the same dialog without the title: the chips are
// what all of them share, ✕ takes it off each, and the line goes to each
// task in turn. The preview is computed against the first.

import { api, change } from "./api.js";
import { $, el, replace } from "./dom.js";
import { acceptOffer, offerFor } from "./offer.js";
import { moveChanges } from "./palette.js";
import { repeatText } from "./repeat.js";
import { byId, state, update } from "./state.js";
import { reminderText } from "./templates.js";
import { goalText } from "./habits.js";
import { captureWarningText, estimateText, shortDay, tagPath } from "./words.js";
import { bookedText, completedText } from "./clock.js";
import { tr } from "./i18n.js";

// The tasks being edited as they were when the dialog opened (`task` is the
// first), the chips marked for removal (by key), the server's preview of
// the line and the offer at the cursor.
let tasks = [];
let task = null;
let removed = new Set();
let preview = null;
let offer = null;
let asked = 0;
let timer = null;

// `ids`: one id or the picked ones. `text`: what the line starts with
// (`z:` from the report, ADR 0081 §3).
export function openEdit(ids, text = "") {
  const found = [ids].flat().map((id) => byId(state.tasks, id)).filter(Boolean);
  if (!found.length) return;
  tasks = found;
  task = found[0];
  removed = new Set();
  preview = null;
  offer = null;
  update({ edit: task.id });
  const many = tasks.length > 1;
  $("edit-title").hidden = many;
  $("edit-title").value = many ? "" : task.title;
  $("edit-head").hidden = !many;
  $("edit-head").textContent = many ? tr("{count} Aufgaben bearbeiten", { count: tasks.length }) : "";
  const input = $("edit-input");
  input.value = text;
  input.focus();
  input.setSelectionRange(text.length, text.length);
  if (text) refresh();
}

function close() {
  tasks = [];
  task = null;
  offer = null;
  preview = null;
  update({ edit: null });
  $("edit-input").blur();
  $("edit-title").blur();
}

// The chips of a task: a key, a class, the text, and the changes that take
// it off. The same keys for the task and the preview, so what is new shows.
function chipsOf(t) {
  const out = [];
  const unset = (field) => [change.set(t.id, field, null)];
  if (t.project) {
    out.push({
      key: "project",
      kind: "project",
      text: `${tr("Projekt:")} ${byId(state.projects, t.project)?.title ?? "?"}`,
      off: () => moveChanges([t], null, null) ?? [],
    });
  }
  for (const tagging of t.tags ?? []) {
    const tag = byId(state.tags, tagging.tag);
    if (!tag) continue;
    out.push({
      key: `tag:${tag.id}`,
      kind: "tag",
      text: `@${tagPath(tag)}`,
      off: () => [change.removeTag(t.id, tag.id)],
    });
  }
  if (t.flagged) out.push({ key: "flagged", kind: "flag", text: tr("markiert"), off: () => [change.set(t.id, "flagged", false)] });
  if (t.priority) out.push({ key: "priority", kind: "plain", text: `${tr("Priorität")} ${t.priority}`, off: () => unset("priority") });
  if (t.defer) out.push({ key: "defer", kind: "date", text: `${tr("Start:")} ${shortDay(t.defer)}`, off: () => unset("defer") });
  if (t.due) out.push({ key: "due", kind: "due", text: `${tr("Fällig:")} ${shortDay(t.due)}`, off: () => unset("due") });
  if (t.planned) out.push({ key: "planned", kind: "date", text: `${tr("Geplant:")} ${shortDay(t.planned)}`, off: () => unset("planned") });
  if (t.estimate) out.push({ key: "estimate", kind: "plain", text: `${tr("Schätzung:")} ${estimateText(t.estimate)}`, off: () => unset("estimate") });
  if (t.repeat) out.push({ key: "repeat", kind: "plain", text: `${tr("Wiederholung:")} ${repeatText(t.repeat)}`, off: () => unset("repeat") });
  if (t.habit) out.push({ key: "habit", kind: "plain", text: `${tr("Habit:")} ${goalText(t.habit)}`, off: () => unset("habit") });
  return out;
}

// The chips all tasks share: the same key with the same text.
function sharedChips(list) {
  const [first, ...rest] = list;
  const others = rest.map((t) => new Map(chipsOf(t).map((c) => [c.key, c.text])));
  return chipsOf(first).filter((c) => others.every((m) => m.get(c.key) === c.text));
}

// The changes that take the chip `key` off every task; a project as one
// move, so the positions in the inbox follow each other.
function removal(key) {
  if (key === "project") return moveChanges(tasks, null, null) ?? [];
  return tasks.flatMap((t) => chipsOf(t).find((c) => c.key === key)?.off() ?? []);
}

// What the line adds or changes: the preview's chips that differ from the
// task's, the new tags, a note appended, the template's reminders.
function added(answer) {
  const before = new Map(chipsOf(task).map((c) => [c.key, c.text]));
  const out = chipsOf(answer.task)
    .filter((c) => before.get(c.key) !== c.text)
    .map((c) => ({ kind: c.kind, text: c.text }));
  for (const name of answer.new_tags ?? []) out.push({ kind: "tag", text: tr("@{name} (neu)", { name }) });
  if ((answer.task.note ?? "") !== (task.note ?? "")) out.push({ kind: "plain", text: tr("Notiz +") });
  for (const r of answer.reminders ?? []) out.push({ kind: "plain", text: `${tr("Erinnerung")} ${reminderText(r)}` });
  // ADR 0081 §3: the clocks `z:` books, and `x:`.
  for (const c of answer.clocks ?? []) out.push({ kind: "plain", text: bookedText(c) });
  if (answer.task.completed_at && answer.task.completed_at !== task.completed_at) {
    out.push({ kind: "plain", text: completedText(answer.task.completed_at) });
  }
  return out;
}

// Ask the server what the line would do, a moment after typing; only the
// latest answer counts.
function refresh() {
  const input = $("edit-input");
  offer = offerFor(input, offer);
  clearTimeout(timer);
  const text = input.value.trim();
  if (!text) {
    preview = null;
    renderEdit();
    return;
  }
  renderEdit();
  const id = task.id;
  timer = setTimeout(async () => {
    const mine = ++asked;
    try {
      const answer = await api.capture(text, null, true, null, null, id);
      if (mine === asked && task?.id === id) {
        preview = answer;
        renderEdit();
      }
    } catch {
      // A failed preview changes nothing; saving will say what is wrong.
    }
  }, 150);
}

function toggle(key) {
  if (removed.has(key)) removed.delete(key);
  else removed.add(key);
  renderEdit();
  $("edit-input").focus();
}

export function renderEdit() {
  $("edit").hidden = !state.edit;
  if (!state.edit || !task) return;
  const line = $("edit-input").value.trim();
  const answer = preview && line ? preview : null;
  // What the line overwrites or replaces (a state of the same group).
  const after = answer ? new Map(chipsOf(answer.task).map((c) => [c.key, c.text])) : null;
  replace(
    $("edit-chips"),
    [
      ...sharedChips(tasks).map((c) => {
        const off = removed.has(c.key);
        const replaced = !off && after && after.get(c.key) !== c.text;
        return el("span", { class: `chip ${c.kind}${off ? " removed" : ""}${replaced ? " replaced" : ""}` }, [
          el("span", { class: "word", text: c.text }),
          el("button", {
            type: "button",
            class: "remove",
            text: off ? "↶" : "✕",
            title: off ? tr("doch behalten") : tr("entfernen"),
            "aria-label": off ? tr("{name} behalten", { name: c.text }) : tr("{name} entfernen", { name: c.text }),
            onmousedown: (event) => event.preventDefault(),
            onclick: () => toggle(c.key),
          }),
        ]);
      }),
      ...(answer ? added(answer).map((c) => el("span", { class: `chip ${c.kind} new`, text: c.text })) : []),
    ],
  );
  replace(
    $("edit-warnings"),
    answer ? (answer.warnings ?? []).map((w) => el("li", { text: captureWarningText(w) })) : [],
  );
  $("edit-suggest").hidden = !offer;
  replace(
    $("edit-suggest"),
    offer
      ? offer.items.map((name, i) =>
          el("li", {
            class: i === offer.index ? "current" : null,
            text: `${offer.sign}${name}`,
            onmousedown: (event) => {
              event.preventDefault();
              offer.index = i;
              acceptOffer($("edit-input"), offer);
              offer = null;
              refresh();
            },
          }),
        )
      : [],
  );
  $("edit-hint").textContent = offer
    ? tr("↑↓ wählen · ⇥ oder ↵ übernehmen · Esc")
    : tr("↵ speichern · ⌫ im leeren Feld entfernt den letzten Chip · Esc verwerfen");
}

// Title and removals first, as changes in one write; then the line, task
// by task (ADR 0078 §3, §5). Failing in between says how far it came.
async function save() {
  if (!task) return;
  const id = task.id;
  const list = tasks;
  const title = $("edit-title").value.trim();
  const line = $("edit-input").value.trim();
  const changes = [];
  if (list.length === 1 && title && title !== task.title) changes.push(change.set(id, "title", title));
  for (const c of sharedChips(list)) if (removed.has(c.key)) changes.push(...removal(c.key));
  let done = 0;
  try {
    if (changes.length) await api.write(changes);
    if (line) {
      for (const t of list) {
        await api.capture(line, null, false, null, null, t.id);
        done += 1;
      }
    }
    close();
    update({ selected: id, message: null });
  } catch (error) {
    const far = list.length > 1 && line ? tr("{done} von {n} geändert: ", { done, n: list.length }) : "";
    update({ message: far + error.message });
  }
}

function lineKeys(event) {
  const input = $("edit-input");
  const key = event.ctrlKey ? `C-${event.key}` : event.key;
  if (offer) {
    const step = (n) => {
      offer.index = (offer.index + n + offer.items.length) % offer.items.length;
      renderEdit();
    };
    if (key === "ArrowDown" || key === "C-n") step(1);
    else if (key === "ArrowUp" || key === "C-p") step(-1);
    else if (key === "Tab" || key === "Enter") {
      acceptOffer(input, offer);
      offer = null;
      refresh();
    } else if (key === "Escape") {
      offer = null;
      renderEdit();
    } else return;
    event.preventDefault();
    event.stopPropagation();
    return;
  }
  if (key === "Backspace" && !input.value) {
    // The last chip still kept is marked for removal, as in the search.
    const last = sharedChips(tasks)
      .filter((c) => !removed.has(c.key))
      .pop();
    if (last) toggle(last.key);
  } else if (key === "Enter") save();
  else if (key === "Escape") close();
  else if (key === "ArrowLeft" || key === "ArrowRight" || key === "Home" || key === "End") {
    setTimeout(refresh, 0);
    return;
  } else return;
  event.preventDefault();
  event.stopPropagation();
}

export function wireEdit() {
  const input = $("edit-input");
  $("edit").addEventListener("click", (event) => {
    if (event.target.id === "edit") close();
  });
  input.addEventListener("input", refresh);
  input.addEventListener("click", refresh);
  input.addEventListener("keydown", lineKeys);
  $("edit-title").addEventListener("keydown", (event) => {
    if (event.key === "Enter") save();
    else if (event.key === "Escape") close();
    else return;
    event.preventDefault();
    event.stopPropagation();
  });
  $("edit-form").addEventListener("submit", (event) => event.preventDefault());
}
