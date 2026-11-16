// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The table (ADR 0115): a third way to show a view beside list and board.
// The rows are the list's — the same tree, filter, grouping and sorting —
// drawn as a table with chosen columns, the sums of an action group's
// subtree, a group's rows and the whole, and sorting at the head. A click
// on a cell that can change opens its editor there (§6): the state and
// the priority as buttons, the dates, the estimate and a property as a
// field, the tags as chips with a field to add one. ← → choose a cell of
// the selected row, Enter opens it, Esc closes it without writing.
//
// list.js hands in what it has (`tableView(ctx)`); the numbers come from
// tabledata.js, which node checks against samu-core's cases.

import { api, change } from "./api.js";
import { enterState, statesOf } from "./detail.js";
import { $, el, replace } from "./dom.js";
import { tr } from "./i18n.js";
import { isKey, knownKeys, targetOf } from "./props.js";
import { byId, state, statusOf, tagTitle, update } from "./state.js";
import { COLUMNS, DEFAULT_COLUMNS, PROPERTY, cellWrite, editable, estimated, hoursText, stepColumn, sums, worked } from "./tabledata.js";
import { dayOf, estimateText, localDay, parseEstimate, placeOf, shortDay, tagPath } from "./words.js";

// What each column is called, and the sort of "Anordnen" its head sets
// (§5); none for the columns that do not sort in this round.
const NAMES = {
  state: tr("Zustand"),
  priority: tr("Priorität"),
  project: tr("Projekt"),
  tags: "Tags",
  defer: tr("Start"),
  due: tr("Fällig"),
  planned: tr("Geplant"),
  created: tr("Erstellt"),
  modified: tr("Geändert"),
  completed: tr("Erledigt"),
  estimate: tr("Schätzung"),
  worked: tr("Ist"),
  remaining: tr("Rest"),
  blocked_by: "Blocker",
};
const SORTED = { priority: "priority", defer: "defer", due: "due", planned: "planned", created: "created", modified: "modified", estimate: "estimate" };
const NUMBERS = ["estimate", "worked", "remaining"];

export const columnName = (column) => (column.startsWith(PROPERTY) ? column.slice(PROPERTY.length) : NAMES[column] ?? column);

// --- The numbers, once per drawing --------------------------------------

let cached = { tasks: null, estimated: new Map() };

// Worked counts a running clock up to now, so it is asked anew each time
// the table is drawn; the estimates only when the tasks changed.
function numbersNow() {
  if (cached.tasks !== state.tasks) cached = { tasks: state.tasks, estimated: estimated(state.tasks) };
  return { worked: worked(state.tasks, state.clocks, Date.now()), estimated: cached.estimated };
}

const parentOf = (id) => byId(state.tasks, id)?.parent ?? null;

// --- Cells --------------------------------------------------------------

const stampDay = (stamp) => (stamp ? shortDay({ date: localDay(stamp) }) : "");

function blockers(task) {
  return state.links
    .filter((l) => l.kind === "blocked_by" && l.owner === task.id)
    .map((l) => {
      const id = l.uri.startsWith("samu:") ? l.uri.slice(5) : null;
      return (id && byId(state.tasks, id)?.title) || l.uri;
    })
    .join(", ");
}

function property(status, key) {
  const own = status.task.properties?.[key];
  const value = own ?? status.inherited?.[key]?.value ?? null;
  if (value === null) return null;
  // A value that is a link stays one (ADR 0054).
  const href = targetOf(own !== undefined ? status.task.id : status.inherited[key].from, key, value);
  return href
    ? el("a", { href, target: "_blank", rel: "noopener noreferrer", text: value, onclick: (event) => event.stopPropagation() })
    : value;
}

// What a cell holds: text or an element; for the numbers, an action
// group's subtree as the CLI shows it.
function cell(column, status, n) {
  const task = status.task;
  if (column.startsWith(PROPERTY)) return property(status, column.slice(PROPERTY.length));
  switch (column) {
    case "state":
      return status.state ? tagTitle(status.state) : "";
    case "priority":
      return task.priority ? el("span", { class: `prio prio-${task.priority}`, text: task.priority }) : "";
    case "project":
      return placeOf(task);
    case "tags":
      return (task.tags ?? [])
        .filter((t) => t.tag !== status.state)
        .map((t) => byId(state.tags, t.tag))
        .filter(Boolean)
        .map((t) => `@${tagPath(t)}`)
        .join(" ");
    case "defer":
    case "due":
    case "planned":
      return task[column] ? shortDay(task[column]) : "";
    case "created":
    case "modified":
      return stampDay(task[column]);
    case "completed":
      return stampDay(task.completed_at);
    case "estimate":
      return hoursText(n.estimated.get(task.id)?.subtree ?? null);
    case "worked":
      return hoursText(n.worked.get(task.id)?.subtree ?? 0);
    case "remaining": {
      const e = n.estimated.get(task.id)?.subtree ?? null;
      return e === null ? "" : remaining(e - (n.worked.get(task.id)?.subtree ?? 0));
    }
    case "blocked_by":
      return blockers(task);
    default:
      // A column from a newer samu: nothing to show (ADR 0015).
      return "";
  }
}

const remaining = (seconds) => (seconds < 0 ? el("span", { class: "over", text: hoursText(seconds) }) : hoursText(seconds));

// The sums of rows in the number columns, nothing in the others.
function sumCells(columns, ids, n) {
  const s = sums(ids, parentOf, n.worked, n.estimated);
  return columns.map((column) => {
    if (!NUMBERS.includes(column)) return el("td");
    const value = column === "remaining" ? (s.remaining === null ? "" : remaining(s.remaining)) : hoursText(column === "worked" ? s.worked : s.estimate);
    return el("td", { class: "num" }, value);
  });
}

// --- The head: sorting and moving columns (§2, §5) ----------------------

let dragged = null;

function head(ctx, columns) {
  const { chosen } = ctx;
  const th = (column, key) => {
    const sort = column === null ? "title" : SORTED[column];
    const on = sort && chosen.sort === sort;
    const text = column === null ? tr("Titel") : columnName(column);
    const attrs = {
      scope: "col",
      class: [column && NUMBERS.includes(column) ? "num" : null, sort ? "sorts" : null, on ? "sorted" : null].filter(Boolean).join(" ") || null,
      "aria-sort": on ? (chosen.desc ? "descending" : "ascending") : null,
      title: sort ? tr("Danach sortieren; noch einmal: umgekehrt") : null,
    };
    if (column !== null) {
      Object.assign(attrs, {
        draggable: "true",
        "data-column": column,
        ondragstart: (event) => {
          dragged = column;
          event.dataTransfer.effectAllowed = "move";
          event.dataTransfer.setData("text/plain", column);
        },
        ondragend: () => {
          dragged = null;
        },
        ondragover: (event) => {
          if (dragged && dragged !== column) event.preventDefault();
        },
        ondrop: (event) => {
          event.preventDefault();
          if (!dragged || dragged === column) return;
          const next = columns.filter((c) => c !== dragged);
          next.splice(next.indexOf(column) + (columns.indexOf(dragged) < columns.indexOf(column) ? 1 : 0), 0, dragged);
          dragged = null;
          ctx.setColumns(next);
        },
      });
    }
    const label = sort
      ? el("button", { type: "button", class: "sort", onclick: () => ctx.sortBy(sort) }, [text, on ? ` ${chosen.desc ? "↓" : "↑"}` : ""])
      : text;
    return el("th", attrs, label);
  };
  return el("thead", {}, el("tr", {}, [th(null), ...columns.map((c) => th(c))]));
}

// --- The table ------------------------------------------------------------

// ctx: { chosen, columns, sections: [{ fold, heading, subtitle, folded,
// entries: [{ status, depth, context, bare }] }], lead(status, bare),
// lines(task), rowClick(event, id), foldClick(fold, event),
// sortBy(sort), setColumns(columns) }
export function tableView(ctx) {
  const n = numbersNow();
  const { columns } = ctx;
  const width = columns.length + 1;
  const bodies = [];
  const all = new Set();
  for (const section of ctx.sections) {
    const ids = section.entries.map((e) => e.status.task.id);
    for (const id of ids) all.add(id);
    const rows = [];
    if (section.heading) {
      const count = new Set(ids).size;
      rows.push(
        el("tr", { class: "table-group" }, [
          el("th", { scope: "rowgroup" }, el("span", { class: "group-head" }, [
            el("button", {
              type: "button",
              class: "fold",
              "aria-label": section.folded ? tr("aufklappen") : tr("zuklappen"),
              "aria-expanded": String(!section.folded),
              title: tr("Alt-Klick: nur diese Gruppe offen"),
              text: section.folded ? "▸" : "▾",
              onclick: (event) => ctx.foldClick(section.fold, event),
            }),
            el("span", { class: "heading", text: section.heading, onclick: (event) => ctx.foldClick(section.fold, event) }),
            el("span", { class: "count", text: String(count) }),
            section.subtitle ? el("span", { class: "subtitle", text: section.subtitle }) : null,
          ])),
          ...sumCells(columns, ids, n),
        ]),
      );
    }
    if (!section.folded) {
      for (const { status, depth, context, bare } of section.entries) rows.push(row(ctx, columns, status, depth, context, bare, n));
    }
    bodies.push(el("tbody", {}, rows));
  }
  if (!bodies.length) return el("p", { class: "empty", text: tr("Nichts hier.") });
  const shows = columns.some((c) => NUMBERS.includes(c));
  const foot = shows
    ? el("tfoot", {}, el("tr", { class: "table-sum" }, [el("th", { scope: "row", text: tr("Summe") }), ...sumCells(columns, [...all], n)]))
    : null;
  return el("div", { class: "table-wrap" }, el("table", { class: "task-table", style: `--columns: ${width}` }, [head(ctx, columns), ...bodies, foot]));
}

function row(ctx, columns, status, depth, context, bare, n) {
  const task = status.task;
  const open = !task.completed_at && !task.dropped_at;
  const classes = [
    "trow",
    state.selected === task.id ? "selected" : null,
    state.picked.has(task.id) ? "picked" : null,
    open && !status.available ? "dim" : null,
    context ? "context" : null,
    open ? null : "done",
    status.group ? "group" : null,
  ];
  return el(
    "tr",
    {
      class: classes.filter(Boolean).join(" "),
      "data-id": task.id,
      onclick: (event) => ctx.rowClick(event, task.id),
    },
    [
      el("td", { class: "title-cell", style: depth ? `--depth: ${depth}` : null }, el("span", { class: "lead" }, [...ctx.lead(status, bare), ctx.lines(task)])),
      ...columns.map((column) => {
        const open = editing?.id === task.id && editing.column === column;
        const classes = [
          NUMBERS.includes(column) ? "num" : null,
          editable(column) ? "editable" : null,
          state.selected === task.id && active === column ? "cell-active" : null,
          open ? "editing" : null,
        ];
        return el(
          "td",
          {
            class: classes.filter(Boolean).join(" ") || null,
            "data-column": column,
            onclick: (event) => cellClick(event, task.id, column),
          },
          open ? editor(status) : cell(column, status, n),
        );
      }),
    ],
  );
}

// --- Editing in a cell (§6) -------------------------------------------------

// The column under the keyboard in the selected row (null: the title), and
// the cell being edited: `field` is what takes the focus, kept across
// redraws so a reload from /events does not take the text from under the
// cursor; `before` what a typed cell held when it opened.
let active = null;
let editing = null;
// While the table is drawn, a field moved into its new cell loses the
// focus: that blur is not a leaving.
let drawing = false;
// What the last drawing showed, for the keys: each table with its
// columns (a block perspective draws one per block, ADR 0116 §5).
let drawn = [];

async function write(changes) {
  try {
    await api.write(changes);
    update({ message: null });
  } catch (error) {
    update({ message: error.message });
  }
}

function openEditor(id, column) {
  active = column;
  if (!editable(column) || !byId(state.tasks, id)) return false;
  editing = { id, column, field: null, before: "" };
  update({ selected: id });
  return true;
}

function closeEditor() {
  editing = null;
  update({});
}

// A plain click on a cell chooses it; on one that can change, it opens the
// editor. With Ctrl, ⌘ or Shift the row's click picks, as in the list.
function cellClick(event, id, column) {
  if (event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
  if (editing?.id === id && editing.column === column) {
    event.stopPropagation();
    return;
  }
  active = column;
  if (editable(column) && !event.target.closest("a, button, input")) {
    event.stopPropagation();
    openEditor(id, column);
  }
}

// Leaving the editor's cell closes it; within the cell (a chip's ✕) not.
function leaves(event, cellNode) {
  if (drawing || editing?.field !== cellNode.field) return false;
  return !event.relatedTarget || !event.relatedTarget.closest?.("td.editing");
}

function editor(status) {
  const column = editing.column;
  if (column === "state" || column === "priority") return choices(status, column);
  if (column === "tags") return tagEditor(status);
  return typed(status, column);
}

// The text a typed cell starts with.
function startText(status, column) {
  const task = status.task;
  if (column === "estimate") return task.estimate ? estimateText(task.estimate) : "";
  if (column.startsWith(PROPERTY)) return task.properties?.[column.slice(PROPERTY.length)] ?? "";
  return dayOf(task[column]) ?? "";
}

// A date, the estimate or a property: Enter or leaving writes, a value
// that is not one says so and stays.
function typed(status, column) {
  if (editing.field) return editing.field;
  const id = status.task.id;
  const before = startText(status, column);
  const key = column.startsWith(PROPERTY) ? column.slice(PROPERTY.length) : null;
  const inherited = key && status.task.properties?.[key] === undefined ? status.inherited?.[key]?.value : null;
  const commit = () => {
    const result = cellWrite(column, field.value, before, parseEstimate);
    if (result?.error) {
      const text = result.error === "estimate" ? tr("Keine Dauer: z. B. 30m, 1h30, 1:30 oder 1,5h") : tr("Kein Datum");
      field.classList.add("invalid");
      field.title = text;
      update({ message: text });
      return;
    }
    editing = null;
    if (result) write([change.set(id, result.field, result.value)]);
    update({});
  };
  const field = el("input", {
    type: column === "estimate" || key ? "text" : "date",
    class: "cell-input",
    value: before,
    list: column === "estimate" ? "estimate-choices" : null,
    placeholder: column === "estimate" ? tr("z. B. 30m, 1h30, 1:30") : inherited,
    "aria-label": columnName(column),
    onkeydown: (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        commit();
      } else if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        closeEditor();
      }
    },
    onblur: (event) => {
      if (leaves(event, { field })) commit();
    },
  });
  editing.field = field;
  editing.before = before;
  return field;
}

// The state or the priority: one button each, the current one pressed.
// ← → walk the buttons, Enter or Space presses one, its key (a state's,
// or a b c - for the priority) picks it at once.
function choices(status, column) {
  if (editing.field) return editing.field;
  const task = status.task;
  const options =
    column === "priority"
      ? [null, "A", "B", "C"].map((v) => ({ value: v, text: v ?? "–", key: v ? v.toLowerCase() : "-" }))
      : [{ value: null, text: tr("kein"), key: null }, ...statesOf(status).map((t) => ({ value: t.id, text: t.title, key: t.key }))];
  const now = column === "priority" ? (task.priority ?? null) : (status.state ?? null);
  const pick = (value) => {
    editing = null;
    update({});
    if (value === now) return;
    if (column === "priority") write([change.set(task.id, "priority", value)]);
    else enterState(status, value);
  };
  const buttons = options.map((o) =>
    el("button", {
      type: "button",
      class: o.value === now ? "current" : null,
      "aria-pressed": String(o.value === now),
      title: o.key ? `${o.text} (${o.key})` : o.text,
      text: o.text,
      onclick: (event) => {
        event.stopPropagation();
        pick(o.value);
      },
    }),
  );
  const group = el(
    "span",
    {
      class: "cell-choices",
      role: "group",
      "aria-label": columnName(column),
      onkeydown: (event) => {
        if (event.ctrlKey || event.metaKey || event.altKey) return;
        event.stopPropagation();
        const at = buttons.indexOf(event.target);
        if (event.key === "Escape") closeEditor();
        else if (event.key === "ArrowLeft" || event.key === "ArrowRight") buttons[Math.min(buttons.length - 1, Math.max(0, at + (event.key === "ArrowLeft" ? -1 : 1)))]?.focus();
        else if (event.key === "Enter" || event.key === " ") pick(options[Math.max(0, at)].value);
        else if (event.key === "Tab") return;
        else {
          const hit = options.find((o) => o.key && o.key === event.key);
          if (hit) pick(hit.value);
          else return;
        }
        event.preventDefault();
      },
      onfocusout: (event) => {
        if (leaves(event, { field: group })) closeEditor();
      },
    },
    buttons,
  );
  editing.field = group;
  return group;
}

// The tags without the states: chips with ✕, and a field whose Enter adds
// the tag of that name (offered as you type) and stays open; ⌫ in the
// empty field takes the last one off. New tags come from the inspector.
function tagEditor(status) {
  const task = status.task;
  const stateIds = new Set(statesOf(status).map((t) => t.id));
  const own = (task.tags ?? []).filter((t) => !stateIds.has(t.tag) && t.tag !== status.state);
  const have = new Set((task.tags ?? []).map((t) => t.tag));
  const offer = state.tags.filter((t) => !have.has(t.id) && !stateIds.has(t.id) && !t.exclusive && t.status !== "dropped");
  if (!editing.field) {
    const field = el("input", {
      type: "text",
      class: "cell-input",
      list: "table-tag-choices",
      placeholder: "+ Tag",
      "aria-label": tr("Tag hinzufügen"),
      onkeydown: (event) => {
        const now = statusOf(task.id);
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          closeEditor();
        } else if (event.key === "Enter") {
          event.preventDefault();
          const name = field.value.trim().replace(/^@/, "").toLowerCase();
          if (!name) return closeEditor();
          const tag = state.tags.find((t) => !t.exclusive && t.status !== "dropped" && (tagPath(t).toLowerCase() === name || t.title.toLowerCase() === name));
          if (!tag) {
            update({ message: tr("Kein Tag „{name}“.", { name: field.value.trim() }) });
            return;
          }
          field.value = "";
          if (!(now?.task.tags ?? []).some((t) => t.tag === tag.id)) write([change.addTag(task.id, tag.id)]);
        } else if (event.key === "Backspace" && !field.value) {
          const states = new Set(statesOf(now).map((t) => t.id));
          const last = (now?.task.tags ?? []).filter((t) => !states.has(t.tag) && t.tag !== now.state).at(-1);
          if (last) {
            event.preventDefault();
            write([change.removeTag(task.id, last.tag)]);
          }
        }
      },
      onblur: (event) => {
        if (leaves(event, { field })) closeEditor();
      },
    });
    editing.field = field;
  }
  return el("span", { class: "cell-tags" }, [
    ...own.map((t) =>
      el("span", { class: "chip" }, [
        tagTitle(t.tag),
        el("button", {
          type: "button",
          "aria-label": tr("Tag entfernen"),
          text: "×",
          onclick: (event) => {
            event.stopPropagation();
            write([change.removeTag(task.id, t.tag)]);
          },
        }),
      ]),
    ),
    editing.field,
    el("datalist", { id: "table-tag-choices" }, offer.map((t) => el("option", { value: tagPath(t) }))),
  ]);
}

// Draws the table into `node`, the field being edited keeping its focus.
// An editor whose row is not drawn any more is closed.
export function drawTable(node, ctx) {
  drawTables(node, (table) => table(ctx));
}

// The same for several tables: `build` gets a function that makes one
// from its context and returns what goes into `node`.
export function drawTables(node, build) {
  const focused = editing?.field && editing.field.contains(document.activeElement) ? document.activeElement : null;
  const tables = [];
  drawing = true;
  try {
    replace(
      node,
      build((ctx) => {
        const view = tableView(ctx);
        tables.push({ node: view, columns: ctx.columns });
        return view;
      }),
    );
  } finally {
    drawing = false;
  }
  drawn = tables;
  if (!drawn.some((d) => d.columns.includes(active))) active = null;
  if (!editing) return;
  if (!editing.field?.isConnected) {
    editing = null;
    return;
  }
  if (focused?.isConnected) focused.focus();
  else if (!editing.field.contains(document.activeElement)) {
    const first = editing.field.matches("input") ? editing.field : (editing.field.querySelector(".current") ?? editing.field.querySelector("button"));
    first?.focus();
    if (first?.select && !focused) first.select();
  }
}

// ← → choose a cell in the selected row, Enter opens it (keys.js asks this
// first; `key` as keymap.js spells it). False lets the key go on.
export function tableKey(key) {
  if (editing || !state.selected) return false;
  const selector = `tr[data-id="${CSS.escape(state.selected)}"]`;
  const table = drawn.find((d) => d.node.isConnected && d.node.querySelector(selector));
  if (!table) return false;
  if (key === "ArrowLeft" || key === "ArrowRight") {
    active = stepColumn(table.columns, active, key === "ArrowLeft" ? -1 : 1);
    update({});
    table.node.querySelector(`tr[data-id="${CSS.escape(state.selected)}"] td[data-column="${CSS.escape(active ?? "")}"]`)?.scrollIntoView({ block: "nearest", inline: "nearest" });
    return true;
  }
  if (key === "Enter" && active && editable(active)) return openEditor(state.selected, active);
  return false;
}

// --- The menu "Spalten" (§2) ----------------------------------------------

let menuOpen = false;

export function toggleTableMenu() {
  menuOpen = !menuOpen;
  update({});
}

// One menu at a time under the list head (ADR 0064 §2).
export function closeTableMenu() {
  if (menuOpen) toggleTableMenu();
}

export const tableMenuOpen = () => menuOpen;

// `shown`: whether the view is a table now; `columns` its columns.
export function renderTableMenu(shown, columns, setColumns) {
  const box = $("table-menu");
  const button = $("table-columns");
  button.hidden = !shown;
  button.setAttribute("aria-expanded", String(shown && menuOpen));
  box.hidden = !shown || !menuOpen;
  if (box.hidden) {
    replace(box);
    return;
  }
  const focused = document.activeElement?.closest?.("#table-menu") ? document.activeElement.dataset.k : null;
  const move = (i, j) => {
    const next = [...columns];
    [next[i], next[j]] = [next[j], next[i]];
    setColumns(next);
  };
  const chosen = el(
    "ol",
    { class: "table-chosen" },
    columns.map((column, i) =>
      el("li", {}, [
        el("label", { class: "check" }, [
          el("input", { type: "checkbox", checked: true, "data-k": `on:${column}`, onchange: () => setColumns(columns.filter((c) => c !== column)) }),
          columnName(column),
        ]),
        el("button", { type: "button", "data-k": `up:${column}`, "aria-label": tr("nach vorn"), text: "↑", disabled: i === 0, onclick: () => move(i, i - 1) }),
        el("button", { type: "button", "data-k": `down:${column}`, "aria-label": tr("nach hinten"), text: "↓", disabled: i === columns.length - 1, onclick: () => move(i, i + 1) }),
      ]),
    ),
  );
  const others = [...COLUMNS, ...knownKeys().map((k) => `${PROPERTY}${k}`)].filter((c) => !columns.includes(c));
  const rest = el(
    "ul",
    { class: "table-others" },
    others.map((column) =>
      el(
        "li",
        {},
        el("label", { class: "check" }, [
          el("input", { type: "checkbox", "data-k": `on:${column}`, onchange: () => setColumns([...columns, column]) }),
          column.startsWith(PROPERTY) ? `${tr("Property")} ${columnName(column)}` : columnName(column),
        ]),
      ),
    ),
  );
  const key = el("input", {
    type: "text",
    "data-k": "key",
    placeholder: tr("weitere Property …"),
    "aria-label": tr("weitere Property"),
    onkeydown: (event) => {
      if (event.key !== "Enter") return;
      const k = event.target.value.trim().toLowerCase();
      if (!isKey(k) || columns.includes(`${PROPERTY}${k}`)) return;
      setColumns([...columns, `${PROPERTY}${k}`]);
    },
  });
  replace(box, [
    chosen,
    rest,
    key,
    el("div", { class: "order-actions" }, [
      el("button", { type: "button", "data-k": "reset", text: tr("Vorgabe"), onclick: () => setColumns([...DEFAULT_COLUMNS]) }),
      el("button", { type: "button", "data-k": "done", text: tr("Fertig"), onclick: toggleTableMenu }),
    ]),
  ]);
  if (focused) box.querySelector(`[data-k="${CSS.escape(focused)}"]`)?.focus();
}
