// Capture as a window (ADR 0026 §5) with the quick-entry syntax of
// ADR 0011 over POST /capture (ADR 0027): while one types, the server's
// preview shows as chips; in a `#…` or `@…` the known projects or tags are
// offered. The server parses and resolves; this page only shows.

import { api } from "./api.js";
import { $, el, replace } from "./dom.js";
import { byId, searching, state, tagTitle, update } from "./state.js";
import { acceptOffer, offerFor } from "./offer.js";
import { reminderText, templateButtons } from "./templates.js";
import { goalText } from "./habits.js";
import { captureWarningText, estimateText, shortDay } from "./words.js";
import { bookedText, completedText } from "./clock.js";
import { tr } from "./i18n.js";

// Where a new task goes when the text names no project: the project of the
// view, else the inbox.
const target = () => (state.view.kind === "project" && !searching() ? state.view.project : null);

// The task a new one goes under, from „Unteraufgabe anlegen“ (ADR 0062 §2);
// null for an ordinary capture.
let under = null;

// The preview of the text in the field, and the offer at the cursor.
let preview = null;
let offer = null;
let asked = 0;
let timer = null;

// `parent`: the id of an open task the new ones go under, as its last
// children; the window then stays open for the next one.
// `text`: what the field starts with, the cursor after it (`z:` from the
// report, ADR 0081 §3).
// `cursor` places the caret, at the end without it: a template's call
// ` /name` keeps it in front, where the title goes (ADR 0084 §2).
export function openCapture(parent = null, text = null, cursor = null) {
  under = parent;
  preview = null;
  update({ capture: true });
  const input = $("capture-input");
  input.focus();
  if (text === null) input.select();
  else {
    input.value = text;
    const at = cursor ?? text.length;
    input.setSelectionRange(at, at);
  }
  refresh();
}

function close() {
  offer = null;
  under = null;
  update({ capture: false });
  $("capture-input").blur();
}

// Ask the server what it would make of the text, a moment after typing;
// only the latest answer counts.
function refresh() {
  const input = $("capture-input");
  offer = offerFor(input, offer);
  clearTimeout(timer);
  const text = input.value;
  if (!text.trim()) {
    preview = null;
    renderCapture();
    return;
  }
  renderCapture();
  timer = setTimeout(async () => {
    const mine = ++asked;
    try {
      const answer = await api.capture(text, target(), true, state.scope, under);
      if (mine === asked) {
        preview = answer;
        renderCapture();
      }
    } catch {
      // A failed preview changes nothing; creating will say what is wrong.
    }
  }, 150);
}

function chips(answer) {
  const task = answer.task;
  const out = [];
  if (task.project) out.push(["project", `${tr("Projekt:")} ${byId(state.projects, task.project)?.title ?? "?"}`]);
  for (const tagging of task.tags ?? []) {
    if (byId(state.tags, tagging.tag)) out.push(["tag", `Tag: ${tagTitle(tagging.tag)}`]);
  }
  for (const name of answer.new_tags ?? []) out.push(["tag new", `${tr("Tag neu:")} ${name}`]);
  if (task.flagged) out.push(["flag", "markiert"]);
  if (task.priority) out.push(["plain", `${tr("Priorität")} ${task.priority}`]);
  if (task.defer) out.push(["date", `Start: ${shortDay(task.defer)}`]);
  if (task.due) out.push(["due", `${tr("Fällig:")} ${shortDay(task.due)}`]);
  if (task.planned) out.push(["date", `Geplant: ${shortDay(task.planned)}`]);
  if (task.estimate) out.push(["plain", `${tr("Schätzung:")} ${estimateText(task.estimate)}`]);
  if (task.repeat) out.push(["plain", `Wiederholung: ${task.repeat.rule}${task.repeat.catch_up ? " (aufholen)" : ""}`]);
  if (task.habit) out.push(["plain", `Habit: ${goalText(task.habit)}`]);
  if (task.note) out.push(["plain", tr("Notiz")]);
  // ADR 0073: the template called, and what reminds.
  const called = (answer.marks ?? []).find((m) => m.meaning === "template");
  if (called) {
    const bytes = new TextEncoder().encode($("capture-input").value);
    out.unshift(["template", `Vorlage: ${new TextDecoder().decode(bytes.slice(called.start, called.end))}`]);
  }
  for (const r of answer.reminders ?? []) out.push(["plain", `${tr("Erinnerung")} ${reminderText(r)}`]);
  // ADR 0081 §3: the clocks `z:` books, and `x:`.
  for (const c of answer.clocks ?? []) out.push(["plain", bookedText(c)]);
  if (task.completed_at) out.push(["plain", completedText(task.completed_at)]);
  return out;
}

export function renderCapture() {
  $("capture").hidden = !state.capture;
  if (!state.capture) return;
  const answer = preview && $("capture-input").value.trim() ? preview : null;
  const parent = under ? byId(state.tasks, under) : null;
  const title = answer?.task.title;
  // Under a task unless the text names a project, which wins (ADR 0062 §2).
  if (parent && (!answer || answer.task.parent === under)) {
    replace($("capture-where"), title ? tr("„{title}“ unter „{parent}“", { title, parent: parent.title }) : tr("Neue Unteraufgabe unter „{parent}“", { parent: parent.title }));
  } else {
    const project = answer ? answer.task.project : parent ? parent.project : target();
    const where = project ? byId(state.projects, project)?.title ?? "?" : "Inbox";
    replace($("capture-where"), title ? tr("„{title}“ in {where}", { title, where }) : tr("Neue Aufgabe in {where}", { where }));
  }
  replace(
    $("capture-chips"),
    answer ? chips(answer).map(([kind, text]) => el("span", { class: `chip ${kind}`, text })) : [],
  );
  replace($("capture-templates"), templateButtons($("capture-input"), refresh));
  replace(
    $("capture-warnings"),
    answer ? (answer.warnings ?? []).map((w) => el("li", { text: captureWarningText(w) })) : [],
  );
  $("capture-suggest").hidden = !offer;
  replace(
    $("capture-suggest"),
    offer
      ? offer.items.map((name, i) =>
          el("li", {
            class: i === offer.index ? "current" : null,
            text: `${offer.sign}${name}`,
            onmousedown: (event) => {
              event.preventDefault();
              offer.index = i;
              acceptOffer($("capture-input"), offer);
              offer = null;
              refresh();
            },
          }),
        )
      : [],
  );
  $("capture-hint").textContent = offer
    ? tr("↑↓ wählen · ⇥ oder ↵ übernehmen · Esc")
    : under
      ? tr("↵ anlegen, nächste · ⇥ anlegen und öffnen · Esc schließen")
      : tr("↵ anlegen · ⇥ anlegen und öffnen · Esc abbrechen");
}

async function create(open) {
  const input = $("capture-input");
  const text = input.value.trim();
  if (!text) return;
  try {
    const answer = await api.capture(text, target(), false, state.scope, under);
    input.value = "";
    preview = null;
    // Under a task, the window stays for the next child (ADR 0062 §2).
    if (under && !open) {
      update({ selected: answer.task.id, message: null });
      renderCapture();
      input.focus();
      return;
    }
    close();
    const patch = { selected: answer.task.id, message: null };
    if (open) patch.pane = "detail";
    update(patch);
  } catch (error) {
    update({ message: error.message });
  }
}

export function wireCapture() {
  const input = $("capture-input");
  $("capture-open").addEventListener("click", () => openCapture());
  $("capture").addEventListener("click", (event) => {
    if (event.target.id === "capture") close();
  });
  input.addEventListener("input", refresh);
  input.addEventListener("click", refresh);
  input.addEventListener("keydown", (event) => {
    const key = event.ctrlKey ? `C-${event.key}` : event.key;
    if (offer) {
      const step = (n) => {
        offer.index = (offer.index + n + offer.items.length) % offer.items.length;
        renderCapture();
      };
      if (key === "ArrowDown" || key === "C-n") step(1);
      else if (key === "ArrowUp" || key === "C-p") step(-1);
      else if (key === "Tab" || key === "Enter") {
        acceptOffer(input, offer);
        offer = null;
        refresh();
      }
      else if (key === "Escape") {
        offer = null;
        renderCapture();
      } else return;
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    if (key === "Enter" || key === "Tab") create(key === "Tab");
    else if (key === "Escape") close();
    else if (key === "ArrowLeft" || key === "ArrowRight" || key === "Home" || key === "End") {
      setTimeout(refresh, 0);
      return;
    } else return;
    event.preventDefault();
    event.stopPropagation();
  });
  $("capture-form").addEventListener("submit", (event) => event.preventDefault());
}
