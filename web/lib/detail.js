// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Right: the inspector of the selected task (ADR 0020 §1): edit what it
// is, see every reason, its properties (ADR 0054), links, reminders
// (ADR 0030) and logbook (ADR 0024 §7).
// Every edit is one `set` or one tag change through api.js.

import { api, change } from "./api.js";
import { appointmentDetail } from "./appointment.js";
import { closeTasks } from "./closeask.js";
import { clockSection } from "./clock.js";
import { checklistBody, checklistTitle, toItemButton } from "./checklistview.js";
import { commentSections } from "./comments.js";
import { $, el, icon, replace } from "./dom.js";
import { blockersOf, moveTo, openBlockPalette, openPalette, withScope } from "./palette.js";
import { openObject } from "./address.js";
import { newTagField } from "./placeops.js";
import { seriesSection } from "./habits.js";
import { foldLine, noteMode, shownParts } from "./layout.js";
import { propertyBlock } from "./props.js";
import { reminderSection } from "./remind.js";
import { DAYS, FREQS, MODES, buildRule, formFits, nextText, parseRule, repeatText } from "./repeat.js";
import { richText } from "./rich.js";
import { byId, heldUpBy, isOpen, shown, state, statusOf, tagTitle, update } from "./state.js";
import { dayOf, estimateText, logWords, parseEstimate, periodText, placeOf, reasonText, stamp } from "./words.js";
import { tr } from "./i18n.js";

// Writes of the inspector still on the way, so completing can wait for
// them (e72d21c3).
const inFlight = new Set();

function write(changes) {
  const sent = (async () => {
    try {
      await api.write(changes);
      update({ message: null });
    } catch (error) {
      update({ message: error.message });
    }
  })();
  inFlight.add(sent);
  sent.finally(() => inFlight.delete(sent));
  return sent;
}

// Sends what is typed in the inspector and not yet on the server, and waits
// for it (e72d21c3): a date field fires `change` in some browsers only
// when it is left, which came after the click on the checkbox, so the
// next repetition counted from the old date and the new one landed on
// the completed task. A half-typed date (`badInput`) is left alone.
export async function flushDetail() {
  const active = document.activeElement;
  if (active && $("detail")?.contains(active)) active.blur();
  for (const input of $("detail")?.querySelectorAll("input[data-date-field]") ?? []) {
    if (input.validity.badInput || input.value === input.dataset.sent) continue;
    const task = byId(state.tasks, input.dataset.task);
    const name = input.dataset.dateField;
    if (!task || input.value === (dayOf(task[name]) ?? "")) continue;
    sendDate(input, task.id, name);
  }
  await Promise.all([...inFlight]);
}

function sendDate(input, id, name) {
  input.dataset.sent = input.value;
  return write([change.set(id, name, input.value ? { date: input.value } : null)]);
}

// Completes a task, asking about open tasks below (ADR 0128), or opens it
// again, after what the inspector holds is sent. When the selected task
// repeats, its next one is selected then, so what is changed next lands
// there and its new date shows.
export async function setDone(status, done) {
  const id = status.task.id;
  if (done) {
    if (!(await closeTasks([id], "completed"))) return update({});
  } else {
    await flushDetail();
    try {
      await api.write([change.set(id, "completed_at", null)]);
    } catch (error) {
      update({ message: error.message });
      return;
    }
  }
  if (done && state.selected === id) {
    const next = state.tasks.find((t) => t.repeat_of === id && isOpen(t));
    if (next) update({ selected: next.id });
  }
}

// The states of the workflow the server named for this task, in order.
export function statesOf(status) {
  if (!status?.workflow) return [];
  return state.tags
    .filter((t) => t.parent === status.workflow && t.status !== "dropped")
    .sort((a, b) => a.position - b.position || a.id.localeCompare(b.id));
}

// Puts a task into a state, or out of every state with `null`. A state
// that asks for a note gets none without one (ADR 0024 §6).
export async function enterState(status, target, note) {
  const tag = target ? byId(state.tags, target) : null;
  if (tag?.ask_note && !note) {
    update({ selected: status.task.id, pane: "detail", message: tr("„{title}“ verlangt eine Notiz.", { title: tag.title }) });
    const field = $("detail-note-for-state");
    if (field) {
      field.dataset.pending = target;
      field.focus();
    }
    return;
  }
  if (target === status.state) return;
  if (target) await write([change.addTag(status.task.id, target, note || undefined)]);
  else if (status.state) await write([change.removeTag(status.task.id, status.state, note || undefined)]);
}

// `s`: the next state, as the server computed it (ADR 0024 §4).
export function cycleState(status) {
  return enterState(status, status.next_state ?? null, $("detail-note-for-state")?.value);
}

// `t` and a key: the state with that key.
export function pickState(status, key) {
  const target = statesOf(status).find((t) => t.key === key);
  if (target) return enterState(status, target.id, $("detail-note-for-state")?.value);
  update({ message: tr("Kein Zustand mit Kürzel „{key}“.", { key }) });
  return null;
}

export function toggleFlag(status) {
  return write([change.set(status.task.id, "flagged", !status.task.flagged)]);
}

const DATE_LABELS = { defer: tr("Start"), due: tr("Fällig"), planned: tr("Geplant") };

function dateInput(task, name) {
  const value = dayOf(task[name]) ?? "";
  return el("input", {
    type: "date",
    "aria-label": DATE_LABELS[name],
    value,
    "data-task": task.id,
    "data-date-field": name,
    onchange: (event) => {
      if (event.target.value !== event.target.dataset.sent) sendDate(event.target, task.id, name);
    },
  });
}

// How long before „Fällig“ the task counts as due soon, its own
// (cbda7d51); „Vorgabe“ follows the account's `due_soon` (ADR 0017 §2).
// Only with a due date: without one it warns of nothing.
const WARNINGS = [0, 1, 2, 3, 7, 14];

function warningDays(period) {
  if (period === "PT0S" || period === "P0D") return 0;
  const m = /^P(?:(\d+)W)?(?:(\d+)D)?$/.exec(period ?? "");
  if (!m || (!m[1] && !m[2])) return null;
  return Number(m[1] ?? 0) * 7 + Number(m[2] ?? 0);
}

function warningWords(days) {
  if (days === 0) return tr("am Tag");
  if (days % 7 === 0) return days === 7 ? tr("1 Woche vorher") : tr("{n} Wochen vorher", { n: days / 7 });
  return days === 1 ? tr("1 Tag vorher") : tr("{n} Tage vorher", { n: days });
}

function dueWarningSelect(task) {
  const own = task.due_warning ?? null;
  const days = warningDays(own);
  const fallback = warningDays(state.accountSettings?.due_soon);
  const options = [["", fallback === null ? tr("Vorgabe") : tr("Vorgabe ({days})", { days: warningWords(fallback) })]];
  for (const d of WARNINGS) options.push([`P${d}D`, warningWords(d)]);
  let current = "";
  if (own !== null) {
    current = days !== null && WARNINGS.includes(days) ? `P${days}D` : own;
    if (!options.some(([v]) => v === current)) {
      options.push([own, days !== null ? warningWords(days) : `${periodText(own)} vorher`]);
    }
  }
  return el(
    "select",
    {
      class: "due-warning",
      id: "detail-due-warning",
      "aria-label": tr("Vorwarnung"),
      title: tr("Ab wann die Aufgabe als bald fällig gilt"),
      onchange: (event) => {
        const value = event.target.value || null;
        if (value !== current) write([change.set(task.id, "due_warning", value)]);
      },
    },
    options.map(([value, text]) => el("option", { value, text, selected: value === current })),
  );
}

function dueControl(task) {
  const date = dateInput(task, "due");
  if (!task.due) return date;
  return el("span", { class: "due-row" }, [date, dueWarningSelect(task)]);
}

// The estimate (ba4ab964): typed freely, a few lengths offered; empty
// takes it away. What is not a length is not written and says so.
function estimateInput(task) {
  return el("input", {
    type: "text",
    id: "detail-estimate",
    list: "estimate-choices",
    "aria-label": tr("Schätzung"),
    placeholder: tr("z. B. 30m, 1h30, 1:30"),
    value: task.estimate ? estimateText(task.estimate) : "",
    onchange: (event) => {
      const input = event.target;
      const value = parseEstimate(input.value);
      input.classList.toggle("invalid", value === undefined);
      input.title = value === undefined ? tr("Keine Dauer: z. B. 30m, 1h30, 1:30 oder 1,5h") : "";
      if (value !== undefined && value !== (task.estimate ?? null)) write([change.set(task.id, "estimate", value)]);
    },
  });
}

// The id under the title, to copy (01a1c271): the whole of it is what
// the CLI, the search and `samu:` take. After it, when the task was
// captured (a1f43aed), the day `erstellt:` in the search filters by.
function idLine(task) {
  const copy = el("button", {
    type: "button",
    class: "copy",
    "aria-label": tr("ID kopieren"),
    title: tr("ID kopieren"),
    text: "⧉",
    onclick: async () => {
      try {
        await navigator.clipboard.writeText(task.id);
        copy.textContent = "✓";
      } catch {
        // No clipboard (no secure context): select it to copy by hand.
        const range = document.createRange();
        range.selectNodeContents(code);
        getSelection()?.removeAllRanges();
        getSelection()?.addRange(range);
      }
    },
  });
  const code = el("code", { text: task.id });
  const by = task.created_by;
  const created = task.created
    ? el("span", {
        class: "created",
        text: tr("erfasst {when}", { when: stamp(task.created) }),
        title: by ? by.kind === "agent" ? tr("erfasst auf {device} (Agent)", { device: by.device }) : tr("erfasst auf {device}", { device: by.device }) : tr("erfasst"),
      })
    : null;
  return el("p", { class: "object-id" }, [code, copy, created]);
}

// The priority (ADR 0056 §6): none, A, B, C.
function priorityControls(task) {
  const now = task.priority ?? null;
  return el(
    "div",
    { class: "priority", role: "group", "aria-label": tr("Priorität") },
    [null, "A", "B", "C"].map((value) =>
      el("button", {
        type: "button",
        class: value === now ? "current" : null,
        "aria-pressed": value === now ? "true" : "false",
        title: value ? tr("Priorität {value}", { value }) : tr("Keine Priorität"),
        text: value ?? "–",
        onclick: () => value !== now && write([change.set(task.id, "priority", value)]),
      }),
    ),
  );
}

// The repetition (ADR 0007 §3, 85b3799a): in words with the next date
// the server computed, and a form behind tr("ändern"). Every field writes the
// whole `repeat` at once; the RRULE field takes what the form cannot.
let repeatEditing = null;

function repeatControl(status) {
  const task = status.task;
  const repeat = task.repeat ?? null;
  const editing = repeatEditing === task.id;
  const summary = el("div", { class: "repeat-summary" }, [
    el("span", { text: repeat ? repeatText(repeat) : tr("keine") }),
    repeat && status.next_repeat
      ? el("span", { class: "repeat-next", text: nextText(status.next_repeat) })
      : null,
    el("button", {
      type: "button",
      class: "repeat-toggle",
      "aria-expanded": editing ? "true" : "false",
      text: editing ? tr("fertig") : repeat ? tr("ändern") : tr("einrichten"),
      onclick: () => {
        repeatEditing = editing ? null : task.id;
        renderDetail();
      },
    }),
  ]);
  return summary;
}

function repeatForm(task) {
  const repeat = task.repeat ?? null;
  const form = parseRule(repeat?.rule);
  const mode = repeat?.mode ?? "fixed";
  const catchUp = repeat?.catch_up ?? false;
  const send = (value) => write([change.set(task.id, "repeat", value)]);
  const sendRule = (rule) => send({ rule, mode, catch_up: catchUp });
  const sendForm = (patch) => sendRule(buildRule({ ...form, ...patch }));
  const rows = [];

  if (!repeat || formFits(form)) {
    const unit = FREQS.find(([code]) => code === form.freq)?.[2];
    rows.push(
      el("div", { class: "repeat-row" }, [
        el(
          "select",
          {
            "aria-label": tr("Häufigkeit"),
            onchange: (event) =>
              event.target.value ? sendForm({ freq: event.target.value }) : send(null),
          },
          [
            el("option", { value: "", text: tr("keine"), selected: !repeat }),
            ...FREQS.map(([code, label]) => el("option", { value: code, text: label, selected: form.freq === code })),
          ],
        ),
        repeat ? tr("alle") : null,
        repeat
          ? el("input", {
              type: "number",
              min: "1",
              class: "repeat-interval",
              "aria-label": tr("Abstand"),
              value: String(form.interval),
              onchange: (event) => {
                const n = Number(event.target.value);
                if (Number.isInteger(n) && n >= 1 && n !== form.interval) sendForm({ interval: n });
              },
            })
          : null,
        repeat && unit ? (form.interval > 1 ? unit[1] : unit[0]) : null,
      ]),
    );
    if (repeat && form.freq === "WEEKLY") {
      rows.push(
        el(
          "div",
          { class: "repeat-row repeat-days", role: "group", "aria-label": tr("Wochentage") },
          DAYS.map(([code, label]) => {
            const on = form.byday.includes(code);
            return el("button", {
              type: "button",
              class: on ? "current" : null,
              "aria-pressed": on ? "true" : "false",
              text: label,
              onclick: () =>
                sendForm({ byday: on ? form.byday.filter((d) => d !== code) : [...form.byday, code] }),
            });
          }),
        ),
      );
    }
  }

  if (repeat) {
    rows.push(
      el("div", { class: "repeat-row" }, [
        el(
          "select",
          {
            "aria-label": tr("Modus"),
            onchange: (event) =>
              send({ rule: repeat.rule, mode: event.target.value, catch_up: event.target.value === "fixed" && catchUp }),
          },
          MODES.map(([code, label]) => el("option", { value: code, text: label, selected: mode === code })),
        ),
        mode === "fixed"
          ? el("label", { class: "repeat-check" }, [
              el("input", {
                type: "checkbox",
                checked: catchUp,
                onchange: (event) => send({ rule: repeat.rule, mode, catch_up: event.target.checked }),
              }),
              tr("aufholen"),
            ])
          : null,
      ]),
      el("div", { class: "repeat-row" }, [
        tr("bis"),
        el("input", {
          type: "date",
          "aria-label": tr("Ende am"),
          value: form.until ?? "",
          onchange: (event) => sendForm({ until: event.target.value || null, count: null }),
        }),
        tr("oder nach"),
        el("input", {
          type: "number",
          min: "1",
          class: "repeat-interval",
          "aria-label": tr("Ende nach so vielen"),
          value: form.count ? String(form.count) : "",
          onchange: (event) => {
            const n = Number(event.target.value);
            if (event.target.value === "") sendForm({ count: null });
            else if (Number.isInteger(n) && n >= 1) sendForm({ count: n, until: null });
          },
        }),
        tr("Mal"),
      ]),
    );
  }

  rows.push(
    el("div", { class: "repeat-row" }, [
      el("input", {
        type: "text",
        class: "repeat-rule",
        "aria-label": "RRULE",
        placeholder: tr("RRULE, z. B. FREQ=MONTHLY;BYMONTHDAY=-1"),
        spellcheck: "false",
        value: repeat?.rule ?? "",
        onchange: (event) => {
          const rule = event.target.value.trim().replace(/^RRULE:/i, "");
          if (!rule) send(null);
          else if (rule !== repeat?.rule) sendRule(rule);
        },
      }),
      repeat ? el("button", { type: "button", text: tr("Entfernen"), onclick: () => send(null) }) : null,
    ]),
  );
  return el("section", { class: "repeat-form", "aria-label": tr("Wiederholung einstellen") }, [
    el("h4", { text: tr("Wiederholung") }),
    ...rows,
  ]);
}

function stateControls(status) {
  const states = statesOf(status);
  if (!states.length) return null;
  const note = el("input", {
    id: "detail-note-for-state",
    type: "text",
    placeholder: tr("Notiz zum Wechsel (optional)"),
    onkeydown: (event) => {
      if (event.key === "Enter" && event.target.dataset.pending) {
        enterState(status, event.target.dataset.pending, event.target.value);
      }
    },
  });
  const button = (title, target, key) =>
    el("button", {
      type: "button",
      class: target === (status.state ?? null) ? "current" : null,
      text: key ? `${title} (${key})` : title,
      onclick: () => enterState(status, target, note.value),
    });
  return el("div", { class: "states" }, [
    el("div", { class: "buttons" }, [
      button(tr("kein"), null),
      ...states.map((t) => button(t.title, t.id, t.key)),
    ]),
    note,
  ]);
}

function tagControls(status) {
  const task = status.task;
  const stateIds = new Set(statesOf(status).map((t) => t.id));
  const own = (task.tags ?? []).filter((t) => !stateIds.has(t.tag));
  const chips = own.map((t) =>
    el("span", { class: "chip" }, [
      tagTitle(t.tag),
      el("button", {
        type: "button",
        "aria-label": tr("Tag entfernen"),
        text: "×",
        onclick: () => write([change.removeTag(task.id, t.tag)]),
      }),
    ]),
  );
  const have = new Set((task.tags ?? []).map((t) => t.tag));
  const offer = state.tags
    .filter((t) => !have.has(t.id) && !stateIds.has(t.id) && !t.exclusive && t.status !== "dropped")
    .sort((a, b) => a.title.localeCompare(b.title));
  if (newTagFor === task.id) {
    const field = newTagField(
      "detail-new-tag",
      (tag, made) => {
        newTagFor = null;
        write([...made, ...(have.has(tag) ? [] : [change.addTag(task.id, tag)])]);
      },
      () => {
        newTagFor = null;
        update({});
      },
    );
    return el("div", { class: "chips" }, [...chips, field]);
  }
  const add = el(
    "select",
    {
      "aria-label": tr("Tag hinzufügen"),
      onchange: (event) => {
        const value = event.target.value;
        if (value === NEW_TAG) {
          newTagFor = task.id;
          update({});
        } else if (value) write([change.addTag(task.id, value)]);
      },
    },
    [
      el("option", { value: "", text: "+ Tag" }),
      ...offer.map((t) => el("option", { value: t.id, text: t.title })),
      el("option", { value: NEW_TAG, text: tr("Neuer Tag …") }),
    ],
  );
  return el("div", { class: "chips" }, [...chips, add]);
}

// The option of a tag select that asks for a new tag (92af4fff), and the
// task whose inspector shows that field instead of the select.
const NEW_TAG = "new";
let newTagFor = null;

function projectSelect(task) {
  const projects = state.projects
    .filter((p) => p.status !== "dropped" && p.status !== "completed")
    .sort((a, b) => a.position - b.position);
  return el(
    "select",
    {
      "aria-label": tr("Projekt"),
      onchange: (event) => moveTo(task, event.target.value || null, null),
    },
    [
      el("option", { value: "", text: "Inbox", selected: !task.project }),
      // With the folder path, so two projects of one title are told apart.
      // One another own scope alone shows says so (ADR 0124 §4).
      ...projects.map((p) => el("option", { value: p.id, text: withScope(placeOf({ project: p.id }), p.id), selected: p.id === task.project })),
    ],
  );
}

function logbook() {
  // Those of another task, until this one's arrive, are not shown.
  const entries = state.logbook.of === state.selected ? state.logbook.entries : [];
  if (!entries.length) return el("p", { class: "quiet", text: tr("Noch nichts.") });
  return el(
    "ol",
    { class: "logbook" },
    [...entries].reverse().map((e) => {
      const what =
        e.kind === "state"
          ? `${e.old ? tagTitle(e.old) : tr("kein")} → ${e.new ? tagTitle(e.new) : tr("kein")}`
          : logWords[e.kind] ?? e.kind;
      return el("li", {}, [
        el("span", { class: "when", text: stamp(e.at) }),
        el("span", { text: what }),
        e.note ? el("span", { class: "note", text: e.note }) : null,
        el("span", { class: "device", text: e.author_kind === "agent" ? `${e.device} · Agent` : e.device }),
      ]);
    }),
  );
}

// "Projekt · Ordner" over the title (ADR 0026 §7), the folder with its
// whole path.
function where(task) {
  const parent = task.parent ? statusOf(task.parent)?.task.title : null;
  return [placeOf(task), parent ? tr("in „{parent}“", { parent }) : null].filter(Boolean).join(" · ");
}

function availability(status) {
  const task = status.task;
  if (!isOpen(task)) {
    return el("div", { class: "availability closed" }, el("strong", { text: task.completed_at ? tr("erledigt") : tr("verworfen") }));
  }
  if (status.available) {
    return el("div", { class: "availability available" }, el("strong", { text: tr("✓ verfügbar") }));
  }
  return el("div", { class: "availability unavailable" }, [
    el("strong", { text: tr("nicht verfügbar") }),
    el("ul", { class: "reasons" }, status.reasons.map((r) => el("li", { text: reasonText(r) }))),
  ]);
}

function facts(rows) {
  return el(
    "dl",
    { class: "facts" },
    rows.flatMap(([label, control]) => [el("dt", { text: label }), el("dd", {}, control)]),
  );
}

// The note (ADR 0058 §6): rendered, and the field to type in after the
// pencil or a double click; an empty note is the field at once. Leaving
// the field saves as before and shows it rendered again.
let noteEditing = null;
let noteSaving = null;
const PENCIL = ["M4 20h4L19 9l-4-4L4 16v4z", "M13.5 6.5l4 4"];

function editNote(id) {
  noteEditing = id;
  update({});
  requestAnimationFrame(() => {
    const field = $("detail-note");
    if (!field) return;
    field.focus();
    field.setSelectionRange(field.value.length, field.value.length);
  });
}

function noteView(task) {
  const text = task.note ?? "";
  // ADR 0072 §2: in a scope that wants the note rendered, an empty one is
  // a quiet line, not a field.
  if (!text.trim() && noteEditing !== task.id && noteMode() === "rendered") {
    return el("button", { type: "button", class: "link note-start", text: tr("Notiz schreiben"), onclick: () => editNote(task.id) });
  }
  if (!text.trim() || noteEditing === task.id) {
    return el("textarea", {
      id: "detail-note",
      rows: Math.max(6, Math.min(24, text.split("\n").length + 1)),
      "aria-label": tr("Notiz"),
      text,
      onchange: (event) => {
        noteSaving = write([change.set(task.id, "note", event.target.value || null)]);
      },
      onblur: async () => {
        if (noteEditing !== task.id) return;
        await noteSaving;
        noteSaving = null;
        noteEditing = null;
        update({});
      },
    });
  }
  return el(
    "div",
    {
      class: "note-view",
      ondblclick: (event) => {
        if (event.target.closest("a, input, summary, button")) return;
        editNote(task.id);
      },
    },
    [
      el(
        "button",
        {
          type: "button",
          class: "edit-toggle",
          title: tr("Bearbeiten (Doppelklick)"),
          "aria-label": tr("Notiz bearbeiten"),
          onclick: () => editNote(task.id),
        },
        icon(PENCIL),
      ),
      richText(text, { onText: (next) => write([change.set(task.id, "note", next)]) }),
    ],
  );
}

// The tasks this one waits for (ae5a3206): each opens on a click, ✕ takes
// it away, „+ Blocker“ picks another in the palette.
function blockerSection(status) {
  const task = status.task;
  const found = blockersOf(task.id);
  if (!found.length && !isOpen(task)) return null;
  return section(
    tr("Blockiert durch"),
    el("div", {}, [
      found.length
        ? el(
            "ul",
            { class: "links blockers" },
            found.map(({ link, target }) => {
              const t = byId(state.tasks, target);
              return el("li", {}, [
                el("button", {
                  type: "button",
                  class: t && !isOpen(t) ? "link done" : "link",
                  text: t ? t.title : link.uri,
                  title: t ? (isOpen(t) ? tr("offen") : tr("erledigt")) : tr("nicht in diesem Konto"),
                  onclick: () => openObject(target),
                }),
                " ",
                el("button", {
                  type: "button",
                  class: "link",
                  text: "✕",
                  "aria-label": tr("Blocker entfernen"),
                  title: tr("Blocker entfernen"),
                  onclick: () => write([change.delete(link.id)]),
                }),
              ]);
            }),
          )
        : null,
      isOpen(task)
        ? el("button", { type: "button", class: "link", text: "+ Blocker", onclick: () => openBlockPalette(status) })
        : null,
    ]),
  );
}

// The open tasks that wait for this one (53a0d8e9): each opens on a click,
// ✕ takes its blocker away. Only there when some wait.
function heldUpSection(task) {
  const found = heldUpBy(task.id);
  if (!found.length) return null;
  return section(
    tr("Hält auf"),
    el(
      "ul",
      { class: "links blockers" },
      found.map(({ link, task: waiting }) =>
        el("li", {}, [
          el("button", { type: "button", class: "link", text: waiting.title, onclick: () => openObject(waiting.id) }),
          " ",
          el("button", {
            type: "button",
            class: "link",
            text: "✕",
            "aria-label": tr("Blocker entfernen"),
            title: tr("Blocker entfernen"),
            onclick: () => write([change.delete(link.id)]),
          }),
        ]),
      ),
    ),
  );
}

function section(title, body) {
  return el("section", {}, [el("h4", { text: title }), body]);
}

export function renderDetail() {
  const status = statusOf(state.selected);
  const pane = $("detail");
  if (!status && state.appointment && ["calendar", "forecast"].includes(shown().kind)) {
    // An appointment chosen or made in the calendar view, or made from a
    // day of Forecast (ADR 0047 §11, ADR 0068 §11). Drawing again replaces
    // the fields; the one being typed in keeps its focus.
    const focused = document.activeElement;
    const keep = focused && pane.contains(focused) && focused.id ? focused : null;
    const caret = keep && ["text", "textarea", "search"].includes(keep.type) ? [keep.selectionStart, keep.selectionEnd] : null;
    replace(pane, appointmentDetail(state.appointment));
    const again = keep && $(keep.id);
    if (again && again !== keep) {
      again.focus();
      if (caret) again.setSelectionRange(...caret);
    }
    return;
  }
  if (!status) {
    replace(pane, el("p", { class: "empty", text: tr("Keine Aufgabe gewählt.") }));
    return;
  }
  const task = status.task;
  const open = isOpen(task);
  // Blockers have their section; the other links stay under „Links“.
  const links = state.links.filter((l) => l.owner === task.id && l.kind !== "blocked_by");
  // The scope's layout (ADR 0072): null shows every part.
  const parts = shownParts();
  const show = (part) => !parts || parts.has(part);
  const factRows = [
    ["project", tr("Projekt"), projectSelect(task)],
    ["priority", tr("Priorität"), priorityControls(task)],
    ["defer", tr("Start"), dateInput(task, "defer")],
    ["due", tr("Fällig"), dueControl(task)],
    ["planned", tr("Geplant"), dateInput(task, "planned")],
    ["repeat", tr("Wiederholung"), repeatControl(status)],
    ["estimate", tr("Schätzung"), estimateInput(task)],
    ["tags", "Tags", tagControls(status)],
  ]
    .filter(([part]) => show(part))
    .map(([, label, control]) => [label, control]);
  const [attachments, comments] = commentSections(task.id, section);
  // Drawing again replaces the fields; one being typed in keeps its focus.
  const focused = document.activeElement;
  const keep =
    focused && pane.contains(focused) && focused.id && focused.tagName === "TEXTAREA"
      ? { id: focused.id, start: focused.selectionStart, end: focused.selectionEnd }
      : null;
  replace(pane, [
    el("button", {
      type: "button",
      class: "back",
      text: tr("‹ Liste"),
      onclick: () => update({ pane: "list" }),
    }),
    el("button", {
      type: "button",
      class: "widen",
      "aria-pressed": state.full ? "true" : "false",
      "aria-label": state.full ? tr("Zurück zur Liste (])") : tr("In voller Breite (])"),
      title: state.full ? tr("Zurück zur Liste (])") : tr("In voller Breite (])"),
      text: state.full ? "⤡" : "⤢",
      onclick: () => update(state.full ? { full: false } : { full: true, pane: "detail" }),
    }),
    el("p", { class: "where", text: where(task) }),
    el("input", {
      class: "title",
      id: "detail-title",
      value: task.title,
      "aria-label": tr("Titel"),
      onchange: (event) => event.target.value.trim() && write([change.set(task.id, "title", event.target.value.trim())]),
    }),
    idLine(task),
    show("properties") ? propertyBlock(task.id, task.properties ?? {}, status.inherited ?? {}) : null,
    show("availability") ? availability(status) : null,
    show("state") ? stateControls(status) : null,
    factRows.length ? facts(factRows) : null,
    repeatEditing === task.id ? repeatForm(task) : null,
    show("series") ? seriesSection(task, section) : null,
    show("note") ? section(tr("Notiz"), noteView(task)) : null,
    show("checklist") ? section(checklistTitle(task.id), checklistBody(task.id)) : null,
    show("links") ? blockerSection(status) : null,
    show("links") ? heldUpSection(task) : null,
    show("links") && links.length
      ? section(
          "Links",
          el(
            "ul",
            { class: "links" },
            links.map((l) =>
              el("li", {}, [
                /^https?:\/\//.test(l.uri)
                  ? el("a", { href: l.uri, target: "_blank", rel: "noopener noreferrer", text: l.label ?? l.uri })
                  : el("span", { text: l.label ? `${l.label} (${l.uri})` : l.uri }),
              ]),
            ),
          ),
        )
      : null,
    show("reminders") ? reminderSection(task, section) : null,
    show("clock") ? clockSection(task, section) : null,
    show("attachments") ? attachments : null,
    show("comments") ? comments : null,
    foldLine(task, status),
    el("div", { class: "actions" }, [
      open
        ? el("button", {
            type: "button",
            class: "primary",
            text: tr("Erledigen (x)"),
            onclick: () => setDone(status, true),
          })
        : el("button", {
            type: "button",
            text: tr("Wieder öffnen"),
            onclick: () =>
              write([
                change.set(task.id, task.completed_at ? "completed_at" : "dropped_at", null),
              ]),
          }),
      el("button", { type: "button", text: tr("Umhängen (m)"), onclick: () => openPalette(status) }),
      open ? toItemButton(task) : null,
      open
        ? el("button", {
            type: "button",
            text: tr("Verwerfen"),
            onclick: () => closeTasks([task.id], "dropped"),
          })
        : null,
      el("label", { class: "flagbox" }, [
        el("input", { type: "checkbox", checked: task.flagged, onchange: () => toggleFlag(status) }),
        tr("markiert (f)"),
      ]),
    ]),
    show("logbook") ? section("Logbook", logbook()) : null,
  ]);
  const again = keep && $(keep.id);
  if (again) {
    again.focus();
    again.setSelectionRange(keep.start, keep.end);
  }
}
