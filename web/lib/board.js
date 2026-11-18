// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The board (ADR 0061): the tasks of a view as cards in columns, from the
// states of a workflow or from chosen tags, with lanes from the menu
// Gruppieren. Dragging a card writes over /changes; what the board shows
// is what the list shows, only laid out across. Where a task stands is
// columns.js; this module draws and writes.

import { api, change } from "./api.js";
import { draggedTasks, forgetDrag, pickedStatuses } from "./bulk.js";
import { boardColumns, moveChanges, neighbour, placeTask } from "./columns.js";
import { closeTasks } from "./closeask.js";
import { statesOf } from "./detail.js";
import { $, el, replace } from "./dom.js";
import { inEffect, pretty } from "./keymap.js";
import { byId, chooseOrder, isOpen, laneFold, state, statusOf, update } from "./state.js";
import { tagPath } from "./words.js";
import { tr } from "./i18n.js";

export const BOARD_DEFAULT = { source: "workflow", workflow: null, tags: [], none: true, done: false, wip: {} };

export const boardConfig = (chosen) => ({ ...BOARD_DEFAULT, ...chosen.board });

const byPosition = (a, b) => a.position - b.position || a.id.localeCompare(b.id);

const workflows = () => state.tags.filter((t) => t.exclusive && t.status !== "dropped").sort(byPosition);

// The workflow the columns come from (§2): the one chosen, else in a
// project its own, else the account's default.
export function workflowOf(config, view) {
  if (config.source !== "workflow") return null;
  const all = workflows();
  if (config.workflow && all.some((t) => t.id === config.workflow)) return config.workflow;
  const project = view.kind === "project" ? byId(state.projects, view.project) : null;
  if (project?.workflow && all.some((t) => t.id === project.workflow)) return project.workflow;
  return (all.find((t) => t.default_workflow) ?? all[0])?.id ?? null;
}

// The board as last drawn, for dropping and for Alt+←/→; null while the
// middle shows a list.
let layout = null;
// The column a drag started in.
let origin = null;
// A move waiting for the note its state asks for: { ids, from, to, title }.
let pending = null;

export const clearBoard = () => {
  layout = null;
};

const toChange = (c, note) =>
  c.op === "set"
    ? change.set(c.id, c.field, c.value)
    : c.op === "add"
      ? change.addTag(c.id, c.tag, note || undefined)
      : change.removeTag(c.id, c.tag, note || undefined);

// Moves tasks from one column into another in one write (§4, §6). A state
// that asks for a note waits for one in the bar above the board.
async function moveTo(ids, from, to, note = null) {
  if (!layout) return;
  const statuses = ids.map(statusOf).filter(Boolean);
  // Into „Erledigt“ the same way as every other completing, with the
  // question about what is open below (ADR 0128 F3).
  if (to === "done" && from !== "done") {
    pending = null;
    await closeTasks(statuses.map((s) => s.task.id), "completed");
    return;
  }
  const { changes, missed, asks } = moveChanges(statuses, from, to, layout.config, layout.columns, statesOf, Date.now());
  if (asks && !note) {
    pending = { ids, from, to, title: asks };
    update({ message: tr("„{title}“ verlangt eine Notiz.", { title: asks }) });
    setTimeout(() => $("board-note")?.focus());
    return;
  }
  pending = null;
  const title = layout.columns.find((c) => c.key === to)?.title;
  const said = missed ? tr("{n} ohne „{title}“ im Ablauf, unverändert.", { n: missed, title }) : null;
  if (!changes.length) {
    update({ message: said });
    return;
  }
  try {
    await api.write(changes.map((c) => toChange(c, note)));
    update({ message: said });
  } catch (error) {
    update({ message: error.message });
  }
}

// Alt+←/→: the chosen card (or the picked ones with it) one column on.
// False when no board is drawn, so the key does nothing else.
export function shiftSelected(step) {
  if (!layout || !state.selected) return false;
  const from = layout.placed.get(state.selected)?.[0];
  if (!from) return true;
  const to = neighbour(layout.columns, from, step);
  if (!to) return true;
  const ids = state.picked.has(state.selected) ? pickedStatuses().map((s) => s.task.id) : [state.selected];
  moveTo(ids, from, to);
  return true;
}

function noteBar() {
  if (!pending) return null;
  const send = () => {
    const note = $("board-note")?.value.trim();
    if (note) moveTo(pending.ids, pending.from, pending.to, note);
  };
  const cancel = () => {
    pending = null;
    update({ message: null });
  };
  return el("div", { class: "board-note" }, [
    el("label", { for: "board-note", text: tr("„{title}“ verlangt eine Notiz:", { title: pending.title }) }),
    el("input", {
      id: "board-note",
      type: "text",
      onkeydown: (event) => {
        if (event.key === "Enter") send();
        else if (event.key === "Escape") cancel();
        else return;
        event.preventDefault();
        event.stopPropagation();
      },
    }),
    el("button", { type: "button", class: "primary", text: tr("Setzen"), onclick: send }),
    el("button", { type: "button", text: tr("Abbrechen"), onclick: cancel }),
  ]);
}

function cell(column, cards) {
  return el(
    "div",
    {
      class: `cell${column.key === "done" ? " done-column" : ""}`,
      "data-column": column.key,
      ondragstart: () => {
        origin = column.key;
      },
      ondragend: () => {
        origin = null;
      },
      ondragover: (event) => {
        if (!draggedTasks()) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        event.currentTarget.classList.add("drop-into");
      },
      ondragleave: (event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.classList.remove("drop-into");
      },
      ondrop: (event) => {
        event.currentTarget.classList.remove("drop-into");
        const ids = draggedTasks();
        if (!ids) return;
        event.preventDefault();
        const from = origin;
        forgetDrag();
        origin = null;
        if (from) moveTo(ids, from, column.key);
      },
    },
    el("ul", { class: "rows" }, cards),
  );
}

// The board for the statuses the view shows. `ctx` brings what the list
// knows: `chosen` (the view's choices), `view`, `filter` (as it acts),
// `open(s)` and `done(s)` (shown by the view), `lanes(statuses)` (the
// groups as [key, heading, statuses]), `compare`, `card(status)` and
// `visit(id)` for the keys.
export function boardView(ctx) {
  const config = boardConfig(ctx.chosen);
  const workflow = workflowOf(config, ctx.view);
  const tagsById = new Map(state.tags.map((t) => [t.id, t]));
  const columns = boardColumns(config, state.tags, workflow, tagPath);
  const now = Date.now();
  const placed = new Map();
  let foreign = 0;
  for (const s of state.statuses) {
    if (isOpen(s.task) ? !ctx.open(s) : !ctx.done(s)) continue;
    const keys = placeTask(s, columns, config, tagsById, now);
    if (keys[0] === "foreign") foreign += 1;
    else if (keys.length) placed.set(s.task.id, keys);
  }
  layout = { config, columns, placed };
  if (!columns.some((c) => c.tag)) {
    return el("p", {
      class: "empty",
      text:
        config.source === "tags"
          ? tr("Noch keine Spalten: im Menü „Spalten“ Tags wählen.")
          : tr("Kein Ablauf: im Menü „Spalten“ einen wählen oder Tags nehmen."),
    });
  }
  const counts = new Map(columns.map((c) => [c.key, 0]));
  for (const keys of placed.values()) for (const k of keys) counts.set(k, counts.get(k) + 1);
  const cards = state.statuses.filter((s) => placed.has(s.task.id));
  const newest = (a, b) => (b.task.completed_at ?? "").localeCompare(a.task.completed_at ?? "");

  const heads = columns.map((c) => {
    const n = counts.get(c.key);
    const limit = Number(config.wip?.[c.key]) || null;
    const over = limit !== null && n > limit;
    const quiet = c.paused && ctx.filter === "available";
    return el("div", { class: `column-head${over ? " wip-over" : ""}`, "data-column": c.key }, [
      el("h3", { text: c.title }),
      el("span", { class: "count", title: limit ? `WIP-Grenze ${limit}` : null, text: limit ? `${n}/${limit}` : String(n) }),
      quiet ? el("span", { class: "hint", text: tr("pausiert — unter Verbleibend sichtbar") }) : null,
    ]);
  });

  const body = [];
  const lanes = ctx.lanes(cards);
  for (const [key, heading, list] of lanes) {
    const fold = laneFold(key);
    const folded = heading && state.folded.has(fold);
    if (heading) {
      body.push(
        el("div", { class: "lane-head" }, [
          el("button", {
            type: "button",
            class: "fold",
            "aria-label": folded ? tr("aufklappen") : tr("zuklappen"),
            text: folded ? "▸" : "▾",
            title: tr("Alt-Klick: nur diese Bahn offen"),
            onclick: (event) => ctx.foldClick(fold, event),
          }),
          el("h3", { text: heading }),
          el("span", { text: String(list.length) }),
        ]),
      );
    }
    if (folded) continue;
    for (const c of columns) {
      const inColumn = list.filter((s) => placed.get(s.task.id)?.includes(c.key)).sort(c.key === "done" ? newest : ctx.compare);
      for (const s of inColumn) ctx.visit(s.task.id);
      body.push(cell(c, inColumn.map((s) => ctx.card(s))));
    }
  }

  return [
    noteBar(),
    foreign
      ? el("p", { class: "board-foreign", text: tr("{n} in anderen Abläufen ohne passende Spalte", { n: foreign }) })
      : null,
    el("div", { class: "board-scroll" }, [
      el("div", { class: `board${config.source === "workflow" ? " by-state" : ""}`, style: `--columns: ${columns.length}` }, [...heads, ...body]),
    ]),
  ];
}

// --- Full screen (ADR 0063 §4, §5) -------------------------------------

// The page clears away all but the board and asks the browser for its
// full screen; not remembered.
let full = false;

export const boardFull = () => full;

export function setBoardFull(on) {
  if (on === full) return;
  full = on;
  document.body.dataset.boardFull = on ? "on" : "off";
  const button = $("board-full");
  button.classList.toggle("current", on);
  button.setAttribute("aria-pressed", String(on));
  const key = pretty(inEffect(state.accountSettings?.keys).byId.get("board.full"));
  button.title = on ? tr("Vollbild verlassen ({keys})", { keys: key ? `${key}, Esc` : "Esc" }) : key ? tr("Board im Vollbild ({key})", { key }) : tr("Board im Vollbild");
  if (on && !document.fullscreenElement) {
    // A browser that says no leaves the page cleared away all the same.
    document.documentElement.requestFullscreen?.().catch(() => {});
  } else if (!on && document.fullscreenElement) {
    document.exitFullscreen?.().catch(() => {});
  }
}

// Leaving the browser's full screen (its own Esc) ends ours too.
document.addEventListener("fullscreenchange", () => {
  if (!document.fullscreenElement) setBoardFull(false);
});

// --- The menu "Spalten" (§9) --------------------------------------------

let menuOpen = false;

export function toggleBoardMenu() {
  menuOpen = !menuOpen;
  update({});
}

// One menu at a time under the list head (ADR 0064 §2).
export function closeBoardMenu() {
  if (menuOpen) toggleBoardMenu();
}

const option = (value, text, selected) => el("option", { value, text, selected: selected || null });

export function renderBoardMenu(chosen, key, view) {
  const box = $("board-menu");
  const button = $("board-columns");
  const board = chosen.view === "board";
  button.hidden = !board;
  $("board-full").hidden = !board;
  // The list, a view without a board: no full screen (§5).
  if (!board) setBoardFull(false);
  button.setAttribute("aria-expanded", String(board && menuOpen));
  box.hidden = !board || !menuOpen;
  if (box.hidden) {
    replace(box);
    return;
  }
  const config = boardConfig(chosen);
  const set = (patch) => chooseOrder(key, { board: { ...config, ...patch } });
  const fallback = byId(state.tags, workflowOf({ ...config, workflow: null }, view));
  const columns = boardColumns(config, state.tags, workflowOf(config, view), tagPath);

  const source = el("label", {}, [
    `${tr("Spalten aus")} `,
    el("select", { onchange: (event) => set({ source: event.target.value }) }, [
      option("workflow", tr("Ablauf"), config.source === "workflow"),
      option("tags", "Tags", config.source === "tags"),
    ]),
  ]);

  let choice;
  if (config.source === "workflow") {
    choice = el("label", {}, [
      `${tr("Ablauf")} `,
      el("select", { onchange: (event) => set({ workflow: event.target.value || null }) }, [
        option("", fallback ? tr("Vorgabe ({days})", { days: fallback.title }) : tr("Vorgabe"), !config.workflow),
        ...workflows().map((t) => option(t.id, t.title, t.id === config.workflow)),
      ]),
    ]);
  } else {
    const tags = config.tags.filter((id) => byId(state.tags, id));
    const swap = (i, j) => {
      const next = [...tags];
      [next[i], next[j]] = [next[j], next[i]];
      set({ tags: next });
    };
    const others = state.tags
      .filter((t) => t.status !== "dropped" && !t.exclusive && !tags.includes(t.id))
      .map((t) => [t.id, tagPath(t)])
      .sort((a, b) => a[1].localeCompare(b[1], "de"));
    choice = el("div", { class: "board-tags" }, [
      el(
        "ol",
        {},
        tags.map((id, i) =>
          el("li", {}, [
            el("span", { text: tagPath(byId(state.tags, id)) }),
            el("button", { type: "button", "aria-label": tr("nach vorn"), text: "↑", disabled: i === 0, onclick: () => swap(i, i - 1) }),
            el("button", { type: "button", "aria-label": tr("nach hinten"), text: "↓", disabled: i === tags.length - 1, onclick: () => swap(i, i + 1) }),
            el("button", { type: "button", "aria-label": tr("entfernen"), text: "×", onclick: () => set({ tags: tags.filter((t) => t !== id) }) }),
          ]),
        ),
      ),
      el(
        "select",
        {
          "aria-label": tr("Tag hinzufügen"),
          onchange: (event) => event.target.value && set({ tags: [...tags, event.target.value] }),
        },
        [option("", tr("Tag hinzufügen …"), true), ...others.map(([id, name]) => option(id, name, false))],
      ),
    ]);
  }

  const check = (label, field) =>
    el("label", { class: "check" }, [
      el("input", { type: "checkbox", checked: config[field] !== false, onchange: (event) => set({ [field]: event.target.checked }) }),
      label,
    ]);

  const wip = el(
    "fieldset",
    { class: "board-wip" },
    [
      el("legend", { text: tr("WIP-Grenze je Spalte") }),
      ...columns.map((c) =>
        el("label", {}, [
          el("span", { text: c.title }),
          el("input", {
            type: "number",
            min: "1",
            placeholder: "–",
            value: config.wip?.[c.key] ?? "",
            onchange: (event) => {
              const n = Number.parseInt(event.target.value, 10);
              const next = { ...config.wip };
              if (n > 0) next[c.key] = n;
              else delete next[c.key];
              set({ wip: next });
            },
          }),
        ]),
      ),
    ],
  );

  replace(box, [
    source,
    choice,
    check(config.source === "tags" ? tr("Spalte „Ohne“") : tr("Spalte „Ohne Zustand“"), "none"),
    el("label", { class: "check" }, [
      el("input", { type: "checkbox", checked: Boolean(config.done), onchange: (event) => set({ done: event.target.checked }) }),
      tr("Spalte „Erledigt“ (7 Tage)"),
    ]),
    wip,
    el("button", { type: "button", text: tr("Fertig"), onclick: toggleBoardMenu }),
  ]);
}
