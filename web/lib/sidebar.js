// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Left: the built-in views and the tree of folders and projects, each with
// a count of what is available there (ADR 0020 §1). The counts add up what
// the server computed; they decide nothing. In the tree one makes, renames,
// moves and folds projects and folders (ADR 0041).

import { api, change } from "./api.js";
import { draggedTasks, dropTasks, forgetDrag } from "./bulk.js";
import { $, el, icon, replace } from "./dom.js";
import { openPlacePalette } from "./palette.js";
import { byPosition, createPlace, movePlace, placeOf, renamePlace, subtree } from "./placeops.js";
import {
  createPerspective,
  deletePerspective,
  duplicate,
  filterOf,
  holds,
  movePerspective,
  perspectiveOf,
  perspectives,
  renamePerspective,
  searchOf,
  setIcon,
} from "./perspectives.js";
import { assemble, isBlocks, passes, union } from "./blocks.js";
import { isBlockOfAnother, openBlocks } from "./blockdialog.js";
import { openSettings } from "./settings.js";
import { matcher } from "./query.js";
import { byId, isOpen, onlyIn, searching, shown, state, toggleFold, update } from "./state.js";
import { addDays, dayOf, today } from "./words.js";
import { tr } from "./i18n.js";
import {
  INBOX,
  VIEWS,
  loadArrangement,
  lowerPerspectives,
  moveBefore,
  parseKey,
  perspectiveKey,
  saveArrangement,
  setHidden,
  toPerspectives,
  viewKey,
} from "./arrange.js";

// The fixed views and the arranged upper list live in arrange.js (ADR 0142).
export { VIEWS };

// The arrangement as drawn last; the menus and drops act on it.
let arranged = loadArrangement([]);
const arrange = (next) => {
  saveArrangement(next);
  update({});
};
export const hideInSidebar = (key) => arrange(setHidden(arranged, key, true));

// --- Three steps: full, narrow, hidden (ADR 0055) -----------------------

const SIDEBAR_KEY = "samu.sidebar";
const STEPS = ["full", "narrow", "hidden"];
// Up to 800 px the sidebar lies above the list: no narrow step there (§5).
const small = window.matchMedia("(max-width: 800px)");
let chosen = (() => {
  try {
    const step = localStorage.getItem(SIDEBAR_KEY);
    return STEPS.includes(step) ? step : "full";
  } catch {
    return "full";
  }
})();

export const sidebarStep = () => (small.matches && chosen === "narrow" ? "full" : chosen);

function setStep(step) {
  chosen = step;
  flyout = null;
  menu = null;
  try {
    localStorage.setItem(SIDEBAR_KEY, step);
  } catch {
    // Private mode: the step lasts as long as the page.
  }
  update({});
}

// `[` and the button by the logo: full → narrow → hidden → full.
export function cycleSidebar() {
  const steps = small.matches ? ["full", "hidden"] : STEPS;
  setStep(steps[(steps.indexOf(sidebarStep()) + 1) % steps.length]);
}

// The folder whose flyout is open beside its tile in the narrow sidebar.
let flyout = null;

// Projects the server found stuck (ADR 0006): active, no next action.
export const stuckProjects = () => state.projectStatuses.filter((p) => p.stuck).map((p) => p.project);
// The projects due for review, oldest date first (ADR 0088 §5).
export const reviewProjects = () =>
  state.projectStatuses
    .filter((p) => p.review_due)
    .sort((a, b) => (a.review < b.review ? -1 : a.review > b.review ? 1 : a.project.position - b.project.position));
const reviewDue = (id) => state.projectStatuses.some((p) => p.project.id === id && p.review_due);

export function forecastDay(status) {
  return dayOf(status.task.due) ?? dayOf(status.task.planned);
}

// What each built-in view holds, before the filter (ADR 0025 §6).
// The search as one test, made again only when the search or the trees of
// tags and folders it reads change.
let search = { key: [], test: () => false };
function found(status) {
  const key = [state.searchChips, state.search, state.tags, state.folders];
  if (key.some((k, i) => k !== search.key[i])) search = { key, test: matcher(state.searchChips, state.search) };
  return search.test(status);
}

// The tasks of a block perspective's blocks (ADR 0116 §5), made again
// only when what they read changes.
let unions = { key: [], sets: new Map() };
function blockSet(p) {
  const key = [state.statuses, state.perspectives, state.tags, state.folders];
  if (key.some((k, i) => k !== unions.key[i])) unions = { key, sets: new Map() };
  const own = JSON.stringify([p.blocks, p.dedupe]);
  let made = unions.sets.get(p.id);
  if (!made || made.own !== own) {
    const statuses = state.statuses.filter((s) => !s.outside);
    const shows = (q, s) => holds(q, s) && passes(filterOf(q), s);
    const ids = union(assemble(p, perspectives(), statuses, shows)).map((s) => s.task.id);
    made = { own, set: new Set(ids) };
    unions.sets.set(p.id, made);
  }
  return made.set;
}

export function inView(status, view = shown()) {
  const task = status.task;
  switch (view.kind) {
    // The server says what is in the inbox, orphans among it (ADR 0036 §6).
    case "inbox":
      return status.inbox;
    case "everything":
      return !status.inbox;
    case "project":
      return !status.inbox && task.project === view.project;
    case "today":
      return isOpen(task) && status.today;
    case "flagged":
      return isOpen(task) && task.flagged;
    case "search":
      return found(status);
    // A perspective's chips and free text (ADR 0090 §2); the filter comes
    // after, as for every view.
    case "perspective": {
      const p = perspectiveOf(view.perspective);
      if (isBlocks(p)) return blockSet(p).has(task.id);
      return Boolean(p) && holds(p, status);
    }
    case "forecast": {
      const day = forecastDay(status);
      return isOpen(task) && Boolean(day) && day <= addDays(today(), 7);
    }
    default:
      return false;
  }
}

// What Forecast counts: the open rows of Überfällig and today (ADR 0045).
const forecastNow = () =>
  state.forecast
    .filter((s) => s.section === "overdue" || (s.section === "day" && s.date === today()))
    .flatMap((s) => s.rows)
    .filter((r) => isOpen(r.status.task));

function count(view) {
  if (view.kind === "stuck") return stuckProjects().length;
  if (view.kind === "review") return reviewProjects().length;
  if (view.kind === "forecast") return new Set(forecastNow().map((r) => r.status.task.id)).size;
  return state.statuses.filter((s) => inView(s, view) && isOpen(s.task) && s.available).length;
}

// Forecast turns red when something in it is overdue, Markiert orange
// (ADR 0026 §6); the number says it either way.
function countClass(view) {
  if (view.kind === "forecast" && state.forecast.some((s) => s.section === "overdue" && s.rows.length)) return "count overdue";
  if (view.kind === "flagged") return "count flag";
  return "count";
}

// A fixed view in the upper list (ADR 0142 §1): it drags, and all but
// Inbox have a menu to hide it.
function item(view, title, glyph = null) {
  const key = viewKey(view.kind);
  const current = !searching() && state.view.kind === view.kind;
  const n = count(view);
  return el(
    "li",
    {
      class: ["place-row", current ? "current" : null, menu?.id === key ? "menu-open" : null].filter(Boolean).join(" "),
      ...entryTarget(key, view.kind === "inbox" ? taskTarget(null) : null),
    },
    [
      viewButton(view, title, n, glyph),
      key === INBOX ? null : menuButton(key, "view", tr("Menü für {title}", { title })),
      menu?.id === key
        ? el("div", { class: "place-menu", role: "menu" }, [menuItem(tr("In der Leiste ausblenden"), () => hideInSidebar(key))])
        : null,
    ],
  );
}

// --- Dragging in the upper list (ADR 0142 §3) ---------------------------

// The key of the entry dragged from the upper list.
let draggedEntry = null;

// What a row of the upper list does when dragged and dropped on: an entry
// from the list or a perspective from below goes before it, or after it
// in its lower half. `tasks` are the handlers for tasks from the list,
// which come first.
function entryTarget(key, tasks = null) {
  const moving = () => draggedEntry ?? (draggedPerspective ? perspectiveKey(draggedPerspective) : null);
  const after = (event) => {
    const box = event.currentTarget.getBoundingClientRect();
    return event.clientY > box.top + box.height / 2;
  };
  return {
    draggable: "true",
    ondragstart: (event) => {
      if (event.target.closest?.(".place-menu")) return;
      draggedEntry = key;
      menu = null;
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", key);
    },
    ondragend: () => {
      draggedEntry = null;
      clearDrop();
    },
    ondragover: (event) => {
      if (tasks && draggedTasks()) return tasks.ondragover(event);
      const moved = moving();
      if (!moved || moved === key) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      const mark = after(event) ? "drop-after" : "drop-before";
      if (!event.currentTarget.classList.contains(mark)) {
        clearDrop();
        event.currentTarget.classList.add(mark);
      }
    },
    ondragleave: (event) => {
      if (!event.currentTarget.contains(event.relatedTarget)) {
        event.currentTarget.classList.remove("drop-before", "drop-after", "drop-into");
      }
    },
    ondrop: (event) => {
      if (tasks && draggedTasks()) return tasks.ondrop(event);
      const moved = moving();
      clearDrop();
      if (!moved) return;
      event.preventDefault();
      event.stopPropagation();
      draggedEntry = null;
      draggedPerspective = null;
      if (moved === key) return;
      const order = arranged.order.filter((k) => k !== moved);
      const before = after(event) ? (order[order.indexOf(key) + 1] ?? null) : key;
      arrange(moveBefore(arranged, moved, before));
    },
  };
}

export function chooseView(view) {
  flyout = null;
  // A perspective brings its search into the field (ADR 0090 §6).
  const p = view.kind === "perspective" ? perspectiveOf(view.perspective) : null;
  const search = p ? searchOf(p) : { search: "", searchChips: [] };
  $("search-input").value = search.search;
  update({ view, ...search, selected: null, pane: "list" });
}

function viewButton(view, title, n, glyph = null) {
  return el(
    "button",
    {
      type: "button",
      class: "view",
      "data-view": view.kind,
      // The narrow sidebar shows only the icon: the title on hover.
      title: glyph && sidebarStep() === "narrow" ? title : null,
      onclick: () => chooseView(view),
    },
    [
      glyph ? icon(glyph) : null,
      el("span", { class: "label", text: title }),
      n ? el("span", { class: countClass(view), text: String(n) }) : null,
    ],
  );
}

// --- The tree (ADR 0041) ---------------------------------------------

// The row being typed into: { mode: "new", kind, at } or
// { mode: "rename", id, kind }. One field, kept across redraws, so a
// reload from /events does not take the text from under the cursor.
let draft = null;
let drawing = false;
let committing = false;
const draftInput = el("input", {
  class: "draft",
  autocomplete: "off",
  "aria-label": "Name",
  onkeydown: (event) => {
    if (event.key === "Enter") commitDraft();
    else if (event.key === "Escape") cancelDraft();
    else return;
    event.preventDefault();
    event.stopPropagation();
  },
  onblur: () => {
    if (drawing || committing) return;
    if (draftInput.value.trim()) commitDraft();
    else cancelDraft();
  },
});

// The open row menu: { id, kind }, or { id: "top" } for the "+" above.
let menu = null;
// What is being dragged: { id, kind }.
let drag = null;
let unfoldTimer = null;

const KIND_LIST = { project: () => state.projects, folder: () => state.folders, perspective: () => state.perspectives };
const objectOf = (id, kind) => byId(KIND_LIST[kind](), id);
const NEW_PLACEHOLDER = { project: tr("Neues Projekt"), folder: tr("Neuer Ordner"), perspective: tr("Neue Perspektive") };

function startDraft(next) {
  menu = null;
  draft = next;
  if (next.mode === "new" && next.at && state.folded.has(next.at)) toggleFold(next.at);
  const object = next.mode === "rename" ? objectOf(next.id, next.kind) : null;
  draftInput.value = object?.title ?? "";
  draftInput.placeholder = next.mode === "new" ? NEW_PLACEHOLDER[next.kind] : "";
  // `update` draws at once, so the field is in place now.
  update({});
  draftInput.focus();
  draftInput.select();
}

function cancelDraft() {
  draft = null;
  update({});
}

async function commitDraft() {
  const d = draft;
  const title = draftInput.value.trim();
  if (!d || committing) return;
  committing = true;
  try {
    if (!title) {
      draft = null;
    } else if (d.kind === "perspective" && d.mode === "new") {
      const { blocks: asBlocks, ...from } = d.from ?? {};
      const id = await createPerspective(title, from);
      if (id) {
        draft = null;
        chooseView({ kind: "perspective", perspective: id });
        // A block perspective is made with its blocks next (ADR 0116 §6).
        if (asBlocks === true) {
          const made = perspectiveOf(id);
          if (made) openBlocks(made);
        }
      }
    } else if (d.kind === "perspective") {
      const object = objectOf(d.id, d.kind);
      if (!object || (await renamePerspective(object, title))) draft = null;
    } else if (d.mode === "new") {
      if (await createPlace(d.kind, title, d.at)) draft = null;
    } else {
      const object = objectOf(d.id, d.kind);
      if (!object || (await renamePlace(object, title))) draft = null;
    }
  } finally {
    committing = false;
  }
  update({});
}

function draftRow(depth) {
  return el("li", { class: "place-row draft-row", style: `padding-left: ${depth * 14}px` }, draftInput);
}

function closeMenu() {
  if (!menu) return;
  menu = null;
  update({});
}

function menuButton(id, kind, label) {
  return el("button", {
    type: "button",
    class: "more",
    "aria-label": label,
    "aria-haspopup": "menu",
    "aria-expanded": menu?.id === id ? "true" : "false",
    text: id === "top" ? "+" : "⋯",
    onclick: (event) => {
      event.stopPropagation();
      menu = menu?.id === id ? null : { id, kind };
      update({});
    },
  });
}

function menuItem(text, action) {
  return el("button", {
    type: "button",
    role: "menuitem",
    text,
    onclick: (event) => {
      event.stopPropagation();
      menu = null;
      action();
    },
  });
}

function placeMenu(id, kind) {
  const items =
    id === "top"
      ? [
          menuItem(tr("Neues Projekt"), () => startDraft({ mode: "new", kind: "project", at: null })),
          menuItem(tr("Neuer Ordner"), () => startDraft({ mode: "new", kind: "folder", at: null })),
        ]
      : [
          ...(kind === "folder"
            ? [
                menuItem(tr("Neues Projekt hier"), () => startDraft({ mode: "new", kind: "project", at: id })),
                menuItem(tr("Neuer Ordner hier"), () => startDraft({ mode: "new", kind: "folder", at: id })),
              ]
            : []),
          ...(kind === "project" && reviewDue(id)
            ? [menuItem(tr("Geprüft"), () => api.write(change.reviewed(id)).catch((error) => update({ message: error.message })))]
            : []),
          menuItem(tr("Verschieben nach …"), () => openPlacePalette(id, kind)),
          menuItem(tr("Umbenennen"), () => startDraft({ mode: "rename", id, kind })),
          menuItem(tr("In den Einstellungen"), () => {
            openSettings(null, "places");
            update({ settings: { ...state.settings, place: id } });
          }),
        ];
  return el("div", { class: "place-menu", role: "menu" }, items);
}

// --- Perspectives (ADR 0090 §4) -----------------------------------------

// A new perspective from what is on screen (§5), or an empty one: a row to
// name it in, under the others. The sidebar opens fully for it.
export function startPerspectiveDraft(from = {}) {
  if (sidebarStep() !== "full") setStep("full");
  startDraft({ mode: "new", kind: "perspective", at: null, from });
}

// A few emojis to pick for a perspective, as for a place (ADR 0059 §5).
const PERSPECTIVE_EMOJIS = ["⭐", "🔥", "⏱️", "📅", "🏢", "🏠", "💼", "📞", "🛒", "📚", "🧾", "🌱", "🎯", "🧘", "🚗", "💡"];

function perspectiveMenu(p) {
  const fail = (error) => update({ message: error.message });
  if (menu?.confirm) {
    return el("div", { class: "place-menu", role: "menu" }, [
      menuItem(tr("„{title}“ löschen", { title: p.title }), () => {
        deletePerspective(p).then((ok) => {
          if (ok && state.view.kind === "perspective" && state.view.perspective === p.id) chooseView({ kind: "inbox", project: null });
        });
      }),
      menuItem(tr("Abbrechen"), () => {}),
    ]);
  }
  return el("div", { class: "place-menu", role: "menu" }, [
    menuItem(tr("Umbenennen"), () => startDraft({ mode: "rename", id: p.id, kind: "perspective" })),
    // ADR 0116 §6: a block perspective's blocks; a plain one that is no
    // block of another may become one.
    isBlocks(p) || !isBlockOfAnother(p) ? menuItem(isBlocks(p) ? tr("Blöcke …") : tr("Zu Block-Perspektive …"), () => openBlocks(p)) : null,
    el(
      "div",
      { class: "emoji-row", role: "group", "aria-label": "Icon" },
      [
        ...PERSPECTIVE_EMOJIS.map((emoji) =>
          el("button", {
            type: "button",
            class: emoji === p.icon ? "on" : null,
            "aria-pressed": String(emoji === p.icon),
            text: emoji,
            onclick: (event) => {
              event.stopPropagation();
              menu = null;
              setIcon(p, emoji === p.icon ? null : emoji).catch(fail);
            },
          }),
        ),
        el("button", {
          type: "button",
          title: tr("Kein Icon"),
          "aria-label": tr("Kein Icon"),
          text: "–",
          onclick: (event) => {
            event.stopPropagation();
            menu = null;
            setIcon(p, null).catch(fail);
          },
        }),
        // Any emoji besides the choices, as for folders and projects.
        el("input", {
          class: "emoji-field",
          autocomplete: "off",
          placeholder: tr("Anderes Emoji"),
          "aria-label": tr("Anderes Emoji"),
          value: PERSPECTIVE_EMOJIS.includes(p.icon) ? "" : (p.icon ?? ""),
          onclick: (event) => event.stopPropagation(),
          onkeydown: (event) => {
            if (event.key === "Enter") event.target.blur();
          },
          onchange: (event) => {
            const value = event.target.value.trim() || null;
            if (value && new TextEncoder().encode(value).length > 32) {
              update({ message: tr("Ein Icon ist ein Emoji, höchstens 32 Bytes.") });
              return;
            }
            menu = null;
            if (value !== (p.icon ?? null)) setIcon(p, value).catch(fail);
            else update({});
          },
        }),
      ],
    ),
    arranged.order.includes(perspectiveKey(p.id))
      ? menuItem(tr("Zurück zu den Perspektiven"), () => arrange(toPerspectives(arranged, perspectiveKey(p.id))))
      : menuItem(tr("Zu den Ansichten"), () => arrange(moveBefore(arranged, perspectiveKey(p.id), null))),
    menuItem(tr("In der Leiste ausblenden"), () => hideInSidebar(perspectiveKey(p.id))),
    menuItem(tr("Duplizieren"), () => {
      duplicate(p).then((id) => id && chooseView({ kind: "perspective", perspective: id }));
    }),
    el("button", {
      type: "button",
      role: "menuitem",
      text: tr("Löschen …"),
      onclick: (event) => {
        event.stopPropagation();
        menu = { id: p.id, kind: "perspective", confirm: true };
        update({});
      },
    }),
  ]);
}

// The funnel for a perspective without an emoji.
const FUNNEL = ["M3 5h18l-7 8.5V19l-4 2v-7.5z"];

// Dragging one perspective onto another puts it before that one.
let draggedPerspective = null;

// `upper`: the perspective stands in the upper list (ADR 0142 §1) and
// drags as its entry.
function perspectiveRow(p, upper = false) {
  const view = { kind: "perspective", perspective: p.id };
  const current = state.view.kind === "perspective" && state.view.perspective === p.id;
  const n = count(view);
  const rowClass = ["place-row", current ? "current" : null, menu?.id === p.id ? "menu-open" : null].filter(Boolean).join(" ");
  return el(
    "li",
    upper ? { class: rowClass, ...entryTarget(perspectiveKey(p.id)) } : {
      class: rowClass,
      draggable: "true",
      ondragstart: (event) => {
        draggedPerspective = p.id;
        menu = null;
        event.dataTransfer.effectAllowed = "move";
        event.dataTransfer.setData("text/plain", p.id);
      },
      ondragend: () => {
        draggedPerspective = null;
        clearDrop();
      },
      ondragover: (event) => {
        if (!draggedPerspective || draggedPerspective === p.id) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        if (!event.currentTarget.classList.contains("drop-before")) {
          clearDrop();
          event.currentTarget.classList.add("drop-before");
        }
      },
      ondragleave: (event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.classList.remove("drop-before");
      },
      ondrop: (event) => {
        if (!draggedPerspective) return;
        event.preventDefault();
        clearDrop();
        const moved = draggedPerspective;
        draggedPerspective = null;
        if (moved !== p.id) movePerspective(moved, p.id);
      },
    },
    [
      el(
        "button",
        {
          type: "button",
          class: "view",
          "data-view": "perspective",
          title: sidebarStep() === "narrow" ? p.title : null,
          onclick: () => chooseView(view),
        },
        [
          p.icon ? el("span", { class: "emoji-icon", text: p.icon }) : icon(FUNNEL),
          el("span", { class: "label", text: p.title }),
          n ? el("span", { class: "count", text: String(n) }) : null,
        ],
      ),
      menuButton(p.id, "perspective", tr("Menü für {title}", { title: p.title })),
      menu?.id === p.id ? perspectiveMenu(p) : null,
    ],
  );
}

function drawPerspectives(step) {
  const renaming = (id) => draft?.mode === "rename" && draft.kind === "perspective" && draft.id === id;
  // Only those neither in the upper list nor hidden (ADR 0142 §1).
  const below = new Set(lowerPerspectives(arranged, perspectives().map((p) => p.id)));
  const rows = perspectives()
    .filter((p) => below.has(p.id))
    .map((p) => (renaming(p.id) && step === "full" ? draftRow(0) : perspectiveRow(p)));
  if (step === "full" && draft?.mode === "new" && draft.kind === "perspective") rows.push(draftRow(0));
  const head = $("persp-head");
  // Without one, only the "+" in the head (§4).
  replace(head, [
    el("h2", { text: tr("Perspektiven") }),
    el("button", {
      type: "button",
      class: "more",
      "aria-label": tr("Perspektive anlegen"),
      title: tr("Perspektive anlegen"),
      text: "+",
      onclick: (event) => {
        event.stopPropagation();
        // Plain or of blocks (ADR 0116 §6).
        menu = menu?.id === "persp-new" ? null : { id: "persp-new", kind: "perspective" };
        update({});
      },
    }),
    menu?.id === "persp-new"
      ? el("div", { class: "place-menu", role: "menu" }, [
          menuItem(tr("Perspektive"), () => startPerspectiveDraft({})),
          menuItem(tr("Block-Perspektive"), () => startPerspectiveDraft({ blocks: true })),
        ])
      : null,
  ]);
  replace($("perspectives"), rows);
}

// --- Dragging (ADR 0041 §6) --------------------------------------------

// Tasks from the list drop on a project or the inbox (ADR 0052 §5).
function taskTarget(project) {
  return {
    ondragover: (event) => {
      if (!draggedTasks()) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      if (!event.currentTarget.classList.contains("drop-into")) {
        clearDrop();
        event.currentTarget.classList.add("drop-into");
      }
    },
    ondragleave: (event) => {
      if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.classList.remove("drop-into");
    },
    ondrop: (event) => {
      clearDrop();
      if (!draggedTasks()) return;
      event.preventDefault();
      dropTasks(project);
    },
  };
}

function clearDrop() {
  for (const node of document.querySelectorAll("#sidebar .drop-before, #sidebar .drop-after, #sidebar .drop-into")) {
    node.classList.remove("drop-before", "drop-after", "drop-into");
  }
}

// Where a drop over `row` (a project or folder) at `event` would go, or
// null when it may not.
function dropZone(event, row) {
  if (!drag || drag.id === row.id) return null;
  const box = event.currentTarget.getBoundingClientRect();
  const y = (event.clientY - box.top) / box.height;
  if (row.kind === "folder" && y > 1 / 3 && y < 2 / 3) {
    if (drag.kind === "folder" && subtree(state.folders, drag.id).has(row.id)) return null;
    return "into";
  }
  const zone = y < 0.5 ? "before" : "after";
  const parent = placeOf(objectOf(row.id, row.kind), row.kind);
  if (drag.kind === "folder" && parent && subtree(state.folders, drag.id).has(parent)) return null;
  return zone;
}

// The id of the sibling that follows `id` among its kind at its place,
// leaving out what is being dragged.
function nextSibling(id, kind) {
  const object = objectOf(id, kind);
  const at = placeOf(object, kind);
  const list = KIND_LIST[kind]()
    .filter((o) => placeOf(o, kind) === at && o.id !== drag.id)
    .sort(byPosition);
  return list[list.findIndex((o) => o.id === id) + 1]?.id ?? null;
}

function dropOn(zone, row) {
  const object = objectOf(drag.id, drag.kind);
  if (!object) return;
  if (zone === "into") {
    movePlace(object, drag.kind, row.id);
    return;
  }
  const parent = placeOf(objectOf(row.id, row.kind), row.kind);
  if (row.kind !== drag.kind) {
    movePlace(object, drag.kind, parent);
    return;
  }
  movePlace(object, drag.kind, parent, zone === "before" ? row.id : nextSibling(row.id, row.kind));
}

function draggable(row) {
  return {
    draggable: "true",
    "data-id": row.id,
    ondragstart: (event) => {
      drag = row;
      menu = null;
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", row.id);
    },
    ondragend: () => {
      drag = null;
      clearTimeout(unfoldTimer);
      clearDrop();
    },
    ondragover: (event) => {
      if (draggedTasks()) {
        taskOver(event, row);
        return;
      }
      const zone = dropZone(event, row);
      if (!zone) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      const node = event.currentTarget;
      if (!node.classList.contains(`drop-${zone}`)) {
        clearDrop();
        node.classList.add(`drop-${zone}`);
        clearTimeout(unfoldTimer);
        if (zone === "into" && state.folded.has(row.id)) {
          unfoldTimer = setTimeout(() => state.folded.has(row.id) && drag && toggleFold(row.id), 700);
        }
      }
    },
    ondragleave: (event) => {
      if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.classList.remove("drop-before", "drop-after", "drop-into");
    },
    ondrop: (event) => {
      if (draggedTasks()) {
        clearDrop();
        if (row.kind !== "project") return;
        event.preventDefault();
        dropTasks(row.id);
        return;
      }
      const zone = dropZone(event, row);
      clearDrop();
      if (!zone) return;
      event.preventDefault();
      dropOn(zone, row);
    },
  };
}

// A project takes the tasks; a folder takes none but opens when one holds
// over it a moment.
function taskOver(event, row) {
  const node = event.currentTarget;
  if (row.kind === "folder") {
    if (state.folded.has(row.id) && !node.dataset.unfolding) {
      clearTimeout(unfoldTimer);
      node.dataset.unfolding = "1";
      unfoldTimer = setTimeout(() => state.folded.has(row.id) && draggedTasks() && toggleFold(row.id), 700);
    }
    return;
  }
  event.preventDefault();
  event.dataTransfer.dropEffect = "move";
  if (!node.classList.contains("drop-into")) {
    clearDrop();
    node.classList.add("drop-into");
  }
}

function treeHead() {
  return [
    el("h2", { text: tr("Projekte") }),
    menuButton("top", null, tr("Projekt oder Ordner anlegen")),
    menu?.id === "top" ? placeMenu("top", null) : null,
  ];
}

function wireTreeHead() {
  const head = $("tree-head");
  head.addEventListener("dragover", (event) => {
    if (!drag) return;
    event.preventDefault();
    clearDrop();
    head.classList.add("drop-into");
  });
  head.addEventListener("dragleave", (event) => {
    if (!head.contains(event.relatedTarget)) head.classList.remove("drop-into");
  });
  head.addEventListener("drop", (event) => {
    clearDrop();
    if (!drag) return;
    event.preventDefault();
    const object = objectOf(drag.id, drag.kind);
    if (object) movePlace(object, drag.kind, null);
  });
}

// `plain`: a row in the flyout of the narrow sidebar, which neither drags
// places nor has their menu (ADR 0055 §3).
function folderRow(folder, depth, total, plain = false) {
  const folded = state.folded.has(folder.id);
  const row = { id: folder.id, kind: "folder" };
  return el(
    "li",
    {
      // ADR 0094 §8: a quiet mark on what only its own scope shows.
      class: ["place-row", "folder", folder.own_scope ? "scope-only" : null, menu?.id === folder.id ? "menu-open" : null]
        .filter(Boolean)
        .join(" "),
      title: onlyIn(folder),
      style: depth ? `padding-left: ${depth * 14}px` : null,
      ...(plain ? {} : draggable(row)),
    },
    [
      el("button", {
        type: "button",
        class: "fold",
        "aria-expanded": folded ? "false" : "true",
        "aria-label": folded ? tr("{title} aufklappen", { title: folder.title }) : tr("{title} zuklappen", { title: folder.title }),
        text: folded ? "▸" : "▾",
        onclick: () => toggleFold(folder.id),
      }),
      el("button", { type: "button", class: "folder-title", onclick: () => toggleFold(folder.id) }, [
        el("span", { class: "label", text: folder.title }),
        folded && total ? el("span", { class: "count", text: String(total) }) : null,
      ]),
      plain ? null : menuButton(folder.id, "folder", tr("Menü für {title}", { title: folder.title })),
      !plain && menu?.id === folder.id ? placeMenu(folder.id, "folder") : null,
    ],
  );
}

function projectRow(project, depth, plain = false) {
  const held = project.status === "on_hold";
  const title = held ? `${project.title} · ruht` : project.title;
  const view = { kind: "project", project: project.id };
  const current = !searching() && state.view.kind === "project" && state.view.project === project.id;
  return el(
    "li",
    {
      class: [
        "place-row",
        current ? "current" : null,
        held ? "held" : null,
        reviewDue(project.id) ? "review-due" : null,
        project.own_scope ? "scope-only" : null,
        menu?.id === project.id ? "menu-open" : null,
      ]
        .filter(Boolean)
        .join(" "),
      title: onlyIn(project),
      style: depth ? `padding-left: ${depth * 14}px` : null,
      ...(plain ? taskTarget(project.id) : draggable({ id: project.id, kind: "project" })),
    },
    [
      viewButton(view, title, count(view)),
      plain ? null : menuButton(project.id, "project", tr("Menü für {title}", { title: project.title })),
      !plain && menu?.id === project.id ? placeMenu(project.id, "project") : null,
    ],
  );
}

// Building the rows moves the draft field into a new row, which takes its
// focus: while drawing, its blur is not a leaving.
export function renderSidebar() {
  const focused = document.activeElement === draftInput;
  drawing = true;
  try {
    drawSidebar();
  } finally {
    drawing = false;
  }
  if (focused && draftInput.isConnected && document.activeElement !== draftInput) draftInput.focus();
}

function drawSidebar() {
  const step = sidebarStep();
  document.body.dataset.sidebar = step;
  // The upper list (ADR 0142 §1): what is hidden stands nowhere.
  arranged = loadArrangement(perspectives().map((p) => p.id));
  const renamingPerspective = (id) => step === "full" && draft?.mode === "rename" && draft.kind === "perspective" && draft.id === id;
  replace(
    $("views"),
    arranged.order
      .filter((key) => !arranged.hidden.includes(key))
      .map((key) => {
        const what = parseKey(key);
        if (what.kind === "perspective") {
          const p = perspectiveOf(what.id);
          if (!p) return null;
          return renamingPerspective(p.id) ? draftRow(0) : perspectiveRow(p, true);
        }
        const v = VIEWS.find((one) => one.kind === what.view);
        return v ? item({ kind: v.kind, project: null }, v.title, v.icon) : null;
      }),
  );
  drawPerspectives(step);

  const rows = [];
  // The open projects and the folders the server says the scope shows
  // (ADR 0035 §5, ADR 0037 §4); an empty folder among them.
  const shown = state.projectStatuses.map((s) => s.project);
  const projectsIn = (folder) => shown.filter((p) => (p.folder ?? null) === folder).sort(byPosition);
  const visible = new Set(state.shownFolders);
  const renaming = (id) => draft?.mode === "rename" && draft.id === id;
  const drafting = (kind, at) => draft?.mode === "new" && draft.kind === kind && draft.at === at;
  // What a folded folder sums up: the counts of the projects below it.
  const total = (folder) => {
    const below = subtree(state.folders, folder);
    return shown
      .filter((p) => p.folder && below.has(p.folder))
      .reduce((sum, p) => sum + count({ kind: "project", project: p.id }), 0);
  };
  const walk = (folder, depth, seen, into = rows, plain = false) => {
    for (const child of state.folders.filter((f) => (f.parent ?? null) === folder).sort(byPosition)) {
      if (seen.has(child.id) || !visible.has(child.id)) continue;
      seen.add(child.id);
      into.push(renaming(child.id) && !plain ? draftRow(depth) : folderRow(child, depth, total(child.id), plain));
      if (!state.folded.has(child.id)) walk(child.id, depth + 1, seen, into, plain);
    }
    if (!plain && drafting("folder", folder)) into.push(draftRow(depth));
    for (const project of projectsIn(folder)) {
      into.push(renaming(project.id) && !plain ? draftRow(depth) : projectRow(project, depth, plain));
    }
    if (!plain && drafting("project", folder)) into.push(draftRow(depth));
  };
  if (step === "narrow") {
    if (flyout && !visible.has(flyout)) flyout = null;
    const folders = state.folders.filter((f) => !f.parent && visible.has(f.id)).sort(byPosition);
    const loose = projectsIn(null);
    const marks = initials([...folders, ...loose].map((o) => o.title));
    replace($("tree-head"));
    replace($("tree"), [
      // An emoji in place of the initials (ADR 0059 §4).
      ...folders.map((f, i) => folderTile(f, f.icon || marks[i], total(f.id))),
      ...loose.map((p, i) => projectTile(p, p.icon || marks[folders.length + i])),
    ]);
    const inside = [];
    if (flyout) walk(flyout, 0, new Set([flyout]), inside, true);
    drawFlyout(inside);
    return;
  }
  flyout = null;
  drawFlyout([]);
  walk(null, 0, new Set());
  replace($("tree-head"), treeHead());
  replace($("tree"), rows);
}

// --- The narrow sidebar: tiles and the flyout (ADR 0055 §3) -------------

// Two first letters of each title, one more for all that would share theirs.
export function initials(titles) {
  const letters = titles.map((t) => [...t.trim()].filter((c) => /[\p{L}\p{N}]/u.test(c)));
  const length = letters.map(() => 2);
  for (let round = 0; round < 4; round += 1) {
    const marks = letters.map((l, i) => l.slice(0, length[i]).join("").toLowerCase());
    let grew = false;
    marks.forEach((mark, i) => {
      if (marks.some((other, j) => j !== i && other === mark) && length[i] < letters[i].length) {
        length[i] += 1;
        grew = true;
      }
    });
    if (!grew) break;
  }
  return letters.map((l, i) => {
    const mark = l.slice(0, length[i]).join("") || "·";
    return mark[0].toUpperCase() + mark.slice(1);
  });
}

function tileBadge(n, view = null) {
  return n ? el("span", { class: view ? countClass(view) : "count", text: String(n) }) : null;
}

function projectTile(project, mark) {
  const view = { kind: "project", project: project.id };
  const current = !searching() && state.view.kind === "project" && state.view.project === project.id;
  const held = project.status === "on_hold";
  return el("li", { class: ["tile-row", current ? "current" : null, held ? "held" : null].filter(Boolean).join(" "), ...taskTarget(project.id) }, [
    el("button", {
      type: "button",
      class: "tile project",
      title: held ? `${project.title} · ruht` : project.title,
      "aria-label": project.title,
      onclick: () => chooseView(view),
    }, [el("span", { class: project.icon ? "mark emoji" : "mark", text: mark }), tileBadge(count(view), view)]),
  ]);
}

function folderTile(folder, mark, n) {
  const open = flyout === folder.id;
  const current =
    !searching() &&
    state.view.kind === "project" &&
    subtree(state.folders, folder.id).has(byId(state.projects, state.view.project)?.folder);
  return el(
    "li",
    {
      class: ["tile-row", current ? "current" : null].filter(Boolean).join(" "),
      // A task held over the tile opens the flyout, to drop it on a project there.
      ondragover: () => {
        if (!draggedTasks() || flyout === folder.id) return;
        clearTimeout(unfoldTimer);
        unfoldTimer = setTimeout(() => {
          if (!draggedTasks()) return;
          flyout = folder.id;
          update({});
        }, 500);
      },
      ondragleave: () => clearTimeout(unfoldTimer),
    },
    [
      el("button", {
        type: "button",
        class: "tile folder",
        "data-tile": folder.id,
        title: folder.title,
        "aria-label": folder.title,
        "aria-haspopup": "true",
        "aria-expanded": open ? "true" : "false",
        onclick: (event) => {
          event.stopPropagation();
          flyout = open ? null : folder.id;
          update({});
        },
      }, [el("span", { class: folder.icon ? "mark emoji" : "mark", text: mark }), tileBadge(n)]),
    ],
  );
}

function drawFlyout(rows) {
  const box = $("flyout");
  const folder = flyout ? byId(state.folders, flyout) : null;
  box.hidden = !folder;
  if (!folder) {
    replace(box);
    return;
  }
  replace(box, [el("h2", { text: folder.title }), el("ul", {}, rows.length ? rows : el("li", { class: "empty", text: tr("leer") }))]);
  // Beside its tile; the tile list scrolls, the flyout must not be cut by it.
  const tile = document.querySelector(`#tree [data-tile="${flyout}"]`);
  if (!tile) return;
  const at = tile.getBoundingClientRect();
  const side = $("sidebar").getBoundingClientRect();
  box.style.left = `${side.right + 6}px`;
  box.style.top = `${Math.max(8, Math.min(at.top, window.innerHeight - box.offsetHeight - 8))}px`;
}

function closeFlyout() {
  if (!flyout) return;
  flyout = null;
  update({});
}

// Once at the start: the menu closes on Esc and on a click beside it, and
// the heading takes drops for the top level.
export function wireSidebar() {
  wireTreeHead();
  // A redraw during a drag can take the dragged row away, and its dragend
  // with it: the end of any drop forgets the drag as well.
  const forget = () => setTimeout(() => {
    drag = null;
    forgetDrag();
    clearTimeout(unfoldTimer);
    clearDrop();
  });
  document.addEventListener("drop", forget, true);
  document.addEventListener("dragend", forget, true);
  document.addEventListener("click", (event) => {
    if (menu && !event.target.closest(".place-menu, .more")) closeMenu();
    if (flyout && !event.target.closest("#flyout .fold, #flyout .folder-title, .tile.folder")) closeFlyout();
  });
  document.addEventListener("keydown", (event) => {
    if (menu && event.key === "Escape") {
      event.stopPropagation();
      closeMenu();
    } else if (flyout && event.key === "Escape") {
      event.stopPropagation();
      closeFlyout();
    }
  }, true);
  $("sidebar").querySelector(".side-scroll").addEventListener("scroll", closeFlyout);
  $("sidebar-toggle").addEventListener("click", cycleSidebar);
  $("sidebar-show").addEventListener("click", () => setStep("full"));
  small.addEventListener("change", () => update({}));
}
