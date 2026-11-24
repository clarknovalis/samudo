// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The middle: what the view holds, through the filter, grouped by project
// (ADR 0020 §1), with action groups as ADR 0021 draws them. Availability,
// reasons, state and group summaries come from the server (ADR 0025 §3).

import { api, change } from "./api.js";
import { startNew, writableCalendars } from "./appointment.js";
import { boardConfig, boardFull, boardView, clearBoard, closeBoardMenu, renderBoardMenu, setBoardFull, toggleBoardMenu } from "./board.js";
import { rowClick, rowDrag } from "./bulk.js";
import { $, el, replace } from "./dom.js";
import { calendarView } from "./calendar.js";
import { setDone, toggleSequential } from "./detail.js";
import { progress } from "./checklist.js";
import { closeTableMenu, drawTable, drawTables, renderTableMenu, tableMenuOpen, toggleTableMenu } from "./table.js";
import { assemble, blockFolded, isBlocks, passes } from "./blocks.js";
import { openBlocks } from "./blockdialog.js";
import { queryWords } from "./blocklines.js";
import { columnsOf } from "./tabledata.js";
import { habitControl, habitOf } from "./habits.js";
import { reportView } from "./report.js";
import { asksForClosed, asksForWaiting, chipText, minutes, rangeOf, wantsAppointments } from "./query.js";
import { rowDrop } from "./reorder.js";
import { baseFilter, captureFrom, current as perspective, filterOf, holds, orderOf, perspectives, searchOf } from "./perspectives.js";
import { forecastDay, inView, reviewProjects, startPerspectiveDraft, stuckProjects, VIEWS } from "./sidebar.js";
import { byId, chooseOrder, chooseScope, foldAll, foldOthers, heldUpBy, holdsUp, isOpen, laneFold, shown, state, statusOf, tagTitle, toggleFold, toggleOpened, update } from "./state.js";
import { addDays, dueText, pathOf, shortDay, tagPath, today } from "./words.js";
import { fieldParts } from "./rowfields.js";
import { closeRowMenu, renderRowMenu, rowLayout, rowMenuOpen, toggleRowMenu } from "./rowmenu.js";
import { locale, tr } from "./i18n.js";

// The rows in the order drawn, for the keys (j/k).
export let order = [];

// A search that asks for what is closed sees everything, one that asks for
// what waits at least what remains (ADR 0042 §4).
function filterNow() {
  // A perspective has its own, lifted the same way (ADR 0090 §2, §6).
  if (shown().kind === "perspective") return perspective() ? filterOf(perspective()) : state.filter;
  if (shown().kind !== "search") return state.filter;
  if (asksForClosed(state.searchChips)) return "all";
  if (state.filter === "available" && asksForWaiting(state.searchChips)) return "remaining";
  return state.filter;
}

const matches = (status) => passes(filterNow(), status);

// The block perspective the middle shows, or null (ADR 0116).
const blocksShown = () => (shown().kind === "perspective" && isBlocks(perspective()) ? perspective() : null);

// The built-in views of time show open tasks whatever the filter; each
// block of a block perspective has its own.
const filtered = () => !["today", "forecast", "calendar", "stuck", "review", "time"].includes(shown().kind) && !blocksShown();

// Completing goes through the inspector, which first sends what it holds
// (e72d21c3).
export { setDone };

// Why a row stands in a section of Forecast (ADR 0045).
const WHY = { due: tr("fällig"), planned: tr("geplant"), defer: tr("verfügbar ab"), tag: tr("Forecast-Tag"), done: tr("erledigt"), flagged: null };

// What a row says of where its task lives (21df91a1): nothing where the
// heading says it already (by project, or one project's view), the project
// where the heading is its folder, else project and folder path.
let place = "none";

// The first lines of a note, for the mark's hover (ADR 0053 §5).
const noteHint = (note) => (note.length > 300 ? `${note.slice(0, 300)} …` : note);

// Whether a group drawn without its children is folded. Where it is a hit
// and its children are none, the account says how it starts (ADR 0021 §6,
// `hit_children`), and `state.opened` holds what was toggled since. In a
// flat list its children stand on their own, so there it starts folded.
const bareFolded = (status) =>
  status.flat || state.accountSettings?.hit_children === "folded" ? !state.opened.has(status.task.id) : state.opened.has(status.task.id);

// What a row starts with, in the list and the table (ADR 0115 §3): the
// arrow of an action group, the priority bar and the tick.
function lead(status, bare = false) {
  const task = status.task;
  const open = isOpen(task);
  const group = status.group;
  const folded = group && (bare ? bareFolded(status) : state.folded.has(task.id));
  return [
    group
      ? el("button", {
          type: "button",
          class: "fold",
          "aria-label": folded ? tr("aufklappen") : tr("zuklappen"),
          text: folded ? "▸" : "▾",
          onclick: (event) => {
            event.stopPropagation();
            (bare ? toggleOpened : toggleFold)(task.id);
          },
        })
      : el("span", { class: "fold" }),
    // The priority as a bar beside the tick (ADR 0089); empty keeps the place.
    el("span", {
      class: `prio-bar${task.priority ? ` prio-${task.priority}` : ""}`,
      title: task.priority ? `${tr("Priorität")} ${task.priority}` : null,
      "aria-label": task.priority ? `${tr("Priorität")} ${task.priority}` : null,
      role: task.priority ? "img" : null,
    }),
    // An open habit is entered day by day instead (ADR 0091 §6).
    open && task.habit && habitOf(task.id)
      ? habitControl(status)
      : el("input", {
          type: "checkbox",
          class: "tick",
          checked: Boolean(task.completed_at),
          disabled: Boolean(task.dropped_at),
          "aria-label": tr("erledigt"),
          onclick: (event) => event.stopPropagation(),
          onchange: (event) => setDone(status, event.target.checked),
        }),
  ];
}

// `bare`: a group drawn without its children, which the arrow fetches
// (ba0590e4); it folds by `bareFolded`, not by `state.folded`.
function row(status, depth, context, why = null, bare = false) {
  const task = status.task;
  const open = isOpen(task);
  const group = status.group;
  const folded = group && (bare ? bareFolded(status) : state.folded.has(task.id));
  const due = open ? dueText(status) : null;
  const summary = group
    ? task.sequential
      ? group.next
        ? tr("nächste: {title}", { title: statusOf(group.next)?.task.title ?? "?" })
        : null
      : tr("{n} verfügbar", { n: group.available })
    : null;
  const [ticked, listed] = progress(state.checklist, task.id);
  const checks = listed ? [ticked, listed] : null;
  const classes = [
    "row",
    state.selected === task.id ? "selected" : null,
    state.picked.has(task.id) ? "picked" : null,
    open && !status.available ? "dim" : null,
    context ? "context" : null,
    open ? null : "done",
    group ? "group" : null,
    depth ? "child" : null,
    due?.kind === "overdue" ? "row-overdue" : null,
  ];
  return el(
    "li",
    {
      class: classes.filter(Boolean).join(" "),
      "data-id": task.id,
      style: depth ? `--depth: ${depth}` : null,
      onclick: (event) => rowClick(event, task.id),
      ...rowDrag(task.id),
      ...rowDrop(task.id),
    },
    [
      ...lead(status, bare),
      // The fields placed before the title (ADR 0131 §2), never cut.
      ...fieldParts(status, "before", rowLayout(), whereOf(depth)).map((part) => fieldSpan(part, true)),
      lines(task, status, depth),
      el("span", { class: "meta" }, [
        why ? el("span", { class: `why why-${why.kind}`, text: why.text }) : null,
        group
          ? el("button", {
              type: "button",
              class: "seq",
              title: task.sequential ? tr("Auf parallel stellen") : tr("Auf sequenziell stellen"),
              text: task.sequential ? tr("sequenziell") : "parallel",
              onclick: (event) => {
                event.stopPropagation();
                toggleSequential(status);
              },
            })
          : null,
        group ? el("span", { class: "badge", text: `${group.done}/${group.done + group.open}` }) : null,
        checks ? el("span", { class: "badge", title: tr("Checkliste"), text: `☑ ${checks[0]}/${checks[1]}` }) : null,
        folded && summary ? el("span", { class: "summary", text: summary }) : null,
        task.flagged ? el("span", { class: "flag", text: tr("⚑ markiert") }) : null,
        // The fields placed on line 1 (ADR 0130 §2).
        ...fieldParts(status, "after", rowLayout(), whereOf(depth)).map((part) => fieldSpan(part)),
      ]),
    ],
  );
}

// The descendants of a task in the mirror, for a group drawn without its
// children (ba0590e4).
let index = { of: null, below: null };
function descendants(id) {
  if (index.of !== state.statuses) {
    const below = new Map();
    for (const s of state.statuses) {
      if (!s.task.parent) continue;
      if (!below.has(s.task.parent)) below.set(s.task.parent, []);
      below.get(s.task.parent).push(s);
    }
    index = { of: state.statuses, below };
  }
  const below = index.below;
  const found = [];
  const seen = new Set([id]);
  const visit = (parent) => {
    for (const s of below.get(parent) ?? []) {
      if (seen.has(s.task.id)) continue;
      seen.add(s.task.id);
      found.push(s);
      visit(s.task.id);
    }
  };
  visit(id);
  return found;
}

// One project's tasks as a tree: a task shows if it passes the filter, or
// as context above a child that does (ADR 0021 §2). A group whose children
// the list does not hold (a search hit, a flat grouping) is bare: its
// arrow draws them from the mirror, with the same filter (ba0590e4), as
// context: `below` says the rows stand under such a group (ADR 0021 §6).
// `make` builds what a row is: an element of the list, or what the table
// draws (ADR 0115 §3).
function tree(statuses, rows, compare, start = 0, make = row, below = false) {
  const ids = new Set(statuses.map((s) => s.task.id));
  const children = new Map();
  for (const s of statuses) {
    const parent = s.task.parent && ids.has(s.task.parent) ? s.task.parent : null;
    if (!children.has(parent)) children.set(parent, []);
    children.get(parent).push(s);
  }
  for (const list of children.values()) list.sort(compare);
  const shows = new Map();
  const visible = (s, seen = new Set()) => {
    if (shows.has(s.task.id)) return shows.get(s.task.id);
    if (seen.has(s.task.id)) return false;
    seen.add(s.task.id);
    const own = !filtered() || matches(s);
    const below = (children.get(s.task.id) ?? []).some((c) => visible(c, seen));
    const result = own || below;
    shows.set(s.task.id, result);
    return result;
  };
  const walk = (parent, depth, seen) => {
    for (const s of children.get(parent) ?? []) {
      if (seen.has(s.task.id) || !visible(s)) continue;
      seen.add(s.task.id);
      const context = below || (filtered() && !matches(s));
      const bare = Boolean(s.group) && !(children.get(s.task.id) ?? []).some((c) => visible(c));
      rows.push(make(s, depth, context, null, bare));
      // Under several tags a task shows more than once; j/k visit it once.
      if (!order.includes(s.task.id)) order.push(s.task.id);
      if (bare) {
        if (!bareFolded(s)) tree(descendants(s.task.id), rows, compare, depth + 1, make, true);
      } else if (!state.folded.has(s.task.id)) walk(s.task.id, depth + 1, seen);
    }
  };
  walk(null, start, new Set());
}

const KINDS = { sequential: tr("sequenziell"), parallel: "parallel", single_actions: tr("Einzelaktionen") };

// "Ordner · Art" under a project's heading (ADR 0026 §6), the folder with
// its whole path.
function projectLine(id) {
  const project = byId(state.projects, id);
  if (!project) return null;
  const at = project.folder ? byId(state.folders, project.folder) : null;
  const folder = at ? pathOf(state.folders, at) : null;
  const held = project.status === "on_hold" ? tr("ruht") : null;
  return [folder, KINDS[project.kind], held].filter(Boolean).join(" · ");
}

// The fold keys of the headings drawn now, in the list or on the board,
// for „alle zu / auf“ and Alt-click (8b9e1584).
let headings = [];

export function foldClick(fold, event) {
  if (event?.altKey) foldOthers(fold, headings);
  else toggleFold(fold);
}

export const foldHeadings = () => headings.length > 1 && foldAll(headings);

function renderFoldAll() {
  const button = $("fold-all");
  button.hidden = headings.length < 2;
  const close = headings.some((id) => !state.folded.has(id));
  button.textContent = close ? tr("Alle zu") : tr("Alle auf");
  button.setAttribute("aria-label", close ? tr("alle Gruppen zuklappen") : tr("alle Gruppen aufklappen"));
}

// A heading with its arrow and count; folded, its rows are left out and
// j/k skip them.
function section(title, statuses, compare, subtitle = null, key = null, prefix = "") {
  const rows = [];
  const visited = order.length;
  tree(statuses, rows, compare);
  if (!rows.length) return null;
  if (!title) return el("section", {}, [el("ul", { class: "rows" }, rows)]);
  const fold = laneFold(`${prefix}${key ?? "inbox"}`);
  headings.push(fold);
  const folded = state.folded.has(fold);
  if (folded) order.length = visited;
  const count = new Set(rows.map((r) => r.dataset.id)).size;
  return el("section", { class: folded ? "folded" : null }, [
    el("div", { class: "head" }, [
      el("button", {
        type: "button",
        class: "fold",
        "aria-label": folded ? tr("aufklappen") : tr("zuklappen"),
        "aria-expanded": String(!folded),
        title: tr("Alt-Klick: nur diese Gruppe offen"),
        text: folded ? "▸" : "▾",
        onclick: (event) => foldClick(fold, event),
      }),
      el("h3", { text: title, onclick: (event) => foldClick(fold, event) }),
      el("span", { class: "count", text: String(count) }),
      subtitle ? el("span", { text: subtitle }) : null,
    ]),
    folded ? null : el("ul", { class: "rows" }, rows),
  ]);
}

// One project of the view Review (ADR 0088 §5): its head with when it is
// due and when it was last reviewed, the button „Geprüft“, then its open
// tasks as in the project's own view.
function reviewSection(entry) {
  const project = entry.project;
  const statuses = state.statuses.filter((s) => !s.inbox && s.task.project === project.id && isOpen(s.task));
  const rows = [];
  tree(statuses, rows, comparer(chosenOrder()));
  const last = project.last_reviewed ? tr("zuletzt geprüft {day}", { day: shortDay({ date: localDay(project.last_reviewed) }) }) : tr("noch nie geprüft");
  const meta = [projectLine(project.id), tr("fällig seit {day}", { day: shortDay({ date: entry.review }) }), last].filter(Boolean).join(" · ");
  const open = () => update({ view: { kind: "project", project: project.id }, selected: null });
  const reviewed = async () => {
    try {
      await api.write(change.reviewed(project.id));
    } catch (error) {
      update({ message: error.message });
    }
  };
  return el("section", { class: "review" }, [
    el("div", { class: "head review-head" }, [
      el("h3", { text: [project.icon, project.title].filter(Boolean).join(" "), title: tr("Projekt öffnen"), onclick: open }),
      el("span", { class: "quiet", text: meta }),
      el("button", { type: "button", class: "reviewed", text: tr("Geprüft"), onclick: reviewed }),
    ]),
    rows.length ? el("ul", { class: "rows" }, rows) : el("p", { class: "empty", text: tr("Keine offene Aufgabe.") }),
  ]);
}

// The local day of a timestamp, in this browser.
function localDay(at) {
  const d = new Date(at);
  const pad = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function title() {
  if (shown().kind === "search") return `${tr("Suche:")} ${[...state.searchChips.map((c) => chipText(c)), state.search.trim()].filter(Boolean).join(" ")}`;
  if (state.view.kind === "perspective") {
    const p = perspective();
    return p ? [p.icon, p.title].filter(Boolean).join(" ") : tr("Perspektive");
  }
  if (state.view.kind === "project") {
    // The project's emoji before its title (ADR 0059 §4).
    const project = byId(state.projects, state.view.project);
    return project ? [project.icon, project.title].filter(Boolean).join(" ") : tr("Projekt");
  }
  return VIEWS.find((v) => v.kind === state.view.kind)?.title ?? "";
}

// --- Grouping and sorting (ADR 0042 §5–7) ------------------------------

const orderKey = () =>
  shown().kind === "project"
    ? `project:${shown().project}`
    : shown().kind === "perspective"
      ? `perspective:${shown().perspective}`
      : shown().kind;

// Whether the list stands in its own order, the one Shift+↑/↓ and
// dropping onto rows change (ADR 0062 §9): a list, by Ordnung upward,
// grouped by project, so each project draws its tree.
export function ownOrder() {
  const chosen = chosenOrder();
  return !asBoard() && chosen.sort === "position" && !chosen.desc && chosen.group === "project";
}

// What a view arranges by when nothing is chosen (ADR 0042 §5, §6).
const arrangedBy = () => ({
  group: shown().kind === "forecast" ? "date" : "project",
  sort: "position",
  desc: false,
  then: null,
  thenDesc: false,
});

export function chosenOrder() {
  // A perspective's arrangement is its own, on the server (ADR 0090 §6).
  const p = shown().kind === "perspective" ? perspective() : null;
  return { view: "list", ...arrangedBy(), ...(p ? orderOf(p) : state.listOrder[orderKey()]) };
}

// The second key, when it counts: Ordnung is unique, and a key twice says
// nothing more (ADR 0064 §2).
const thenKey = ({ sort, then }) => (sort === "position" || then === sort ? null : then ?? null);

// What a task is sorted by; null when it has none.
function sortValue(status, sort) {
  const task = status.task;
  switch (sort) {
    case "due":
    case "planned":
    case "defer":
      return task[sort] ? task[sort].date ?? task[sort].at ?? null : null;
    case "title":
      return task.title;
    case "created":
    case "modified":
      return task[sort] ?? null;
    case "estimate":
      return minutes(task.estimate);
    // A, B, C, then none (ADR 0056 §5).
    case "priority":
      return { A: 1, B: 2, C: 3 }[task.priority] ?? null;
    default:
      return task.position;
  }
}

// One level of the order; what lacks the value goes last either way.
function byKey(key, desc) {
  const sign = desc ? -1 : 1;
  return (a, b) => {
    const va = sortValue(a, key);
    const vb = sortValue(b, key);
    if (va === vb) return 0;
    if (va === null) return 1;
    if (vb === null) return -1;
    const c = typeof va === "number" ? va - vb : String(va).localeCompare(String(vb), "de");
    return sign * c;
  };
}

// Siblings in the chosen order: the first key, on a tie the second
// (ADR 0064 §3), then the order of the project.
function comparer(chosen) {
  const first = byKey(chosen.sort, chosen.desc);
  const then = thenKey(chosen);
  const second = then ? byKey(then, chosen.thenDesc) : () => 0;
  return (a, b) =>
    first(a, b) ||
    second(a, b) ||
    a.task.position - b.task.position ||
    a.task.title.localeCompare(b.task.title, "de");
}

// Groups as [key, heading, statuses, subtitle], in the order drawn.
const flat = (statuses) => statuses.map((s) => ({ ...s, flat: true, task: { ...s.task, parent: null } }));
const LAST = "\uffff";

// What a row says of its place (21df91a1): a child says it never, its
// group above does.
const whereOf = (depth) => (depth || place === "none" ? "none" : place);

// One field of a row (ADR 0130): its short form keeps the long one for a
// hover and a screen reader; tags in the short form are chips.
// `before`: in front of the title, as a small plaque like the state's.
function fieldSpan(part, before = false) {
  const short = part.text !== part.label;
  const kind = part.kind?.startsWith("due-") ? `badge ${part.kind}` : part.kind;
  const attrs = { class: ["field", `field-${part.key}`, kind, before ? "before" : null].filter(Boolean).join(" "), title: short ? part.label : null };
  if (part.items && rowLayout().short) return el("span", attrs, part.items.map((item) => el("span", { class: "chip-tag", text: item })));
  return el("span", { ...attrs, text: part.text });
}

// The title, 🚧 before it when an open task waits for it, the note mark
// when there is a note, and the fields placed on line 2 (ADR 0130). The
// table draws it without `status`: its columns say the rest.
function lines(task, status = null, depth = 0) {
  const note = task.note?.trim();
  const second = status ? fieldParts(status, 2, rowLayout(), whereOf(depth)).map((part) => fieldSpan(part)) : [];
  return el("span", { class: "lines" }, [
    el("span", { class: "title" }, [
      holdsUp(task.id) ? el("span", { class: "blocker-mark", title: `${tr("Hält auf:")} ${heldUpBy(task.id).map((h) => h.task.title).join(", ")}`, "aria-label": "Blocker", text: "🚧 " }) : null,
      task.title,
      note ? el("span", { class: "note-mark", title: noteHint(note), "aria-label": tr("hat Notiz"), text: "✎" }) : null,
    ]),
    second.length ? el("span", { class: "details" }, second) : null,
  ]);
}

function groups(statuses, group) {
  const into = (keyOf) => {
    const map = new Map();
    for (const s of statuses) {
      for (const key of [].concat(keyOf(s))) {
        if (!map.has(key)) map.set(key, []);
        map.get(key).push(s);
      }
    }
    return map;
  };
  switch (group) {
    case "folder": {
      const map = into((s) => {
        if (s.inbox) return "inbox";
        const project = s.task.project ? byId(state.projects, s.task.project) : null;
        return project?.folder ?? "none";
      });
      const name = (key) =>
        key === "inbox" ? "Inbox" : key === "none" ? tr("Ohne Ordner") : pathOf(state.folders, byId(state.folders, key) ?? { title: "?" });
      const rank = (key) => (key === "inbox" ? "" : key === "none" ? LAST : name(key));
      return [...map.keys()]
        .sort((a, b) => rank(a).localeCompare(rank(b), "de"))
        .map((key) => [key, name(key), map.get(key)]);
    }
    case "tag": {
      const map = into((s) => (s.task.tags?.length ? s.task.tags.map((t) => t.tag) : ["none"]));
      const name = (key) => (key === "none" ? tr("Ohne Tag") : byId(state.tags, key) ? tagPath(byId(state.tags, key)) : "?");
      const rank = (key) => (key === "none" ? LAST : name(key));
      return [...map.keys()]
        .sort((a, b) => rank(a).localeCompare(rank(b), "de"))
        .map((key) => [key, name(key), flat(map.get(key))]);
    }
    case "state": {
      const map = into((s) => s.state ?? "none");
      const position = (key) => (key === "none" ? Infinity : byId(state.tags, key)?.position ?? 0);
      return [...map.keys()]
        .sort((a, b) => position(a) - position(b))
        .map((key) => [key, key === "none" ? tr("Ohne Zustand") : tagTitle(key), flat(map.get(key))]);
    }
    // A, B, C, then none (ADR 0064 §4).
    case "priority": {
      const map = into((s) => s.task.priority ?? "none");
      return ["A", "B", "C", "none"]
        .filter((key) => map.has(key))
        .map((key) => [key, key === "none" ? tr("Ohne Priorität") : `${tr("Priorität")} ${key}`, flat(map.get(key))]);
    }
    case "date": {
      const now = today();
      const map = into((s) => {
        const day = forecastDay(s);
        if (!day) return LAST;
        return day < now && isOpen(s.task) ? "" : day;
      });
      const name = (key) => (key === "" ? tr("Überfällig") : key === LAST ? tr("Ohne Datum") : key === now ? tr("Heute") : shortDay({ date: key }));
      return [...map.keys()].sort().map((key) => [key, name(key), flat(map.get(key))]);
    }
    case "none":
      return [["all", null, flat(statuses)]];
    default: {
      const map = into((s) => (s.inbox ? null : s.task.project ?? null));
      const keys = [...map.keys()].sort((a, b) => {
        const pa = byId(state.projects, a);
        const pb = byId(state.projects, b);
        return (pa?.position ?? -1) - (pb?.position ?? -1);
      });
      const many = keys.length > 1 || ["everything", "flagged", "search", "perspective"].includes(shown().kind);
      return keys.map((key) => {
        const project = key ? byId(state.projects, key) : null;
        const heading = many ? (key ? (project ? [project.icon, project.title].filter(Boolean).join(" ") : "?") : "Inbox") : null;
        return [key, heading, map.get(key), heading && key ? projectLine(key) : null];
      });
    }
  }
}

// `09:00` in this browser's zone, from a zoned time of the wire.
const clockTime = (at) =>
  new Date(at.replace(/\[[^\]]*\]$/, "")).toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" });

const whenText = (a) =>
  a.start.date ? tr("ganztägig") : [a.start.at, a.end?.at].filter(Boolean).map(clockTime).join("–");

// One appointment of a calendar (ADR 0047 §10): the colour of its
// calendar, its time, its title, its place; the note on hover. With
// `choose`, as in Forecast, the title opens the appointment on the right
// like a block of the calendar view does, with editing and the way to the
// original there (ADR 0068 §11, ADR 0047 §9); else the title is that way.
function appointmentLine(a, extra = null, choose = false) {
  const calendar = byId(state.calendars, a.calendar);
  const title = choose
    ? el("button", {
        type: "button",
        class: "title",
        text: a.title,
        onclick: () => update({ appointment: a, selected: null, pane: "detail" }),
      })
    : a.link
      ? el("a", { class: "title", href: a.link, target: "_blank", rel: "noopener noreferrer", text: a.title })
      : el("span", { class: "title", text: a.title });
  const chosen = choose && state.appointment === a;
  return el("li", { class: `appointment${a.status === "tentative" ? " tentative" : ""}${chosen ? " selected" : ""}`, title: a.note ?? "" }, [
    el("span", { class: "dot", style: calendar?.color ? `background:${calendar.color}` : "" }),
    el("span", { class: "time", text: extra ?? whenText(a) }),
    title,
    el("span", { class: "meta", text: [a.location, calendar?.title].filter(Boolean).join(" · ") }),
  ]);
}

// Forecast from the server (ADR 0045): a strip of the days with their
// counts, then the sections in order, each row with why it is there; the
// appointments of a day above its tasks (ADR 0047 §10).
function forecastSection(section) {
  const now = today();
  const heading =
    section.section === "overdue"
      ? tr("Überfällig")
      : section.section === "flagged"
        ? tr("Markiert")
        : section.section === "later"
          ? tr("Später")
          : section.date === now
            ? tr("Heute")
            : shortDay({ date: section.date });
  const rows = section.rows.map((r) => {
    const label = WHY[r.why];
    const date = r.date && (section.section === "later" || section.section === "overdue" || r.date !== section.date) ? ` ${shortDay({ date: r.date })}` : "";
    const why = label ? { kind: r.why, text: `${label}${date}` } : null;
    if (!order.includes(r.status.task.id)) order.push(r.status.task.id);
    return row(r.status, 0, false, why);
  });
  const id = section.section === "day" ? `fc-${section.date}` : `fc-${section.section}`;
  const appointments = section.appointments ?? [];
  return el("section", { id, class: `forecast-${section.section}` }, [
    el("div", { class: "head" }, [
      el("h3", { text: heading }),
      el("span", { text: rows.length ? String(rows.length) : "" }),
      // A day's head makes an appointment over that day (ADR 0068 §11).
      section.section === "day" && writableCalendars().length
        ? el("button", {
            type: "button",
            class: "new-appointment",
            title: tr("Ganztägiger Termin an diesem Tag"),
            "aria-label": tr("Ganztägiger Termin an diesem Tag"),
            text: tr("+ Termin"),
            onclick: () => startNew({ day: section.date }),
          })
        : null,
    ]),
    appointments.length ? el("ul", { class: "appointments" }, appointments.map((a) => appointmentLine(a, null, true))) : null,
    rows.length ? el("ul", { class: "rows" }, rows) : appointments.length ? null : el("p", { class: "empty", text: tr("Nichts für heute.") }),
  ]);
}

function forecastStrip() {
  const now = today();
  const days = state.accountSettings?.forecast_days ?? 7;
  const of = (key) => state.forecast.find((s) => (s.section === "day" ? s.date === key : s.section === key));
  const cell = (key, label, extra = "") => {
    const section = of(key);
    const n = section ? section.rows.length : 0;
    const dated = Boolean(section?.appointments?.length);
    return el(
      "button",
      {
        type: "button",
        class: `day${extra}${n || dated ? "" : " empty"}`,
        disabled: !section,
        onclick: () => document.getElementById(key.length === 10 ? `fc-${key}` : `fc-${key}`)?.scrollIntoView({ block: "start", behavior: "smooth" }),
      },
      [
        el("span", { class: "label", text: label }),
        el("span", { class: "n", text: n ? String(n) : "·" }),
        // A dot for a day with appointments, not their number (ADR 0047 §10).
        el("span", { class: dated ? "dated" : "dated none" }),
      ],
    );
  };
  const cells = [];
  if (of("overdue")) cells.push(cell("overdue", tr("Überfällig"), " overdue"));
  for (let i = 0; i <= days; i += 1) {
    const key = addDays(now, i);
    cells.push(cell(key, i === 0 ? tr("Heute") : shortDay({ date: key }), i === 0 ? " today" : ""));
  }
  if (of("later")) cells.push(cell("later", tr("Später")));
  return el("nav", { class: "forecast-strip", "aria-label": tr("Tage") }, cells);
}

// Views that can be a board (ADR 0061 §1): those with the menus, and not
// a search for appointments.
const boardable = () =>
  !["stuck", "review", "forecast", "calendar", "time"].includes(shown().kind) &&
  !(shown().kind === "search" && wantsAppointments(state.searchChips));

const asBoard = () => boardable() && chosenOrder().view === "board";

// The table goes where the board goes (ADR 0115 §1).
const asTable = () => boardable() && chosenOrder().view === "table";

// The columns of the table here: a perspective's on the server, else this
// browser's for the view (§7).
const tableColumns = (chosen) => columnsOf(chosen.table);

// With columns from a workflow, the lanes cannot be the states too (§8).
function laneGroup(chosen) {
  return chosen.group === "state" && boardConfig(chosen).source === "workflow" ? "none" : chosen.group;
}

// --- The button Anordnen and its menu (ADR 0064) -------------------------

export const GROUPS = [
  ["project", tr("Projekt")],
  ["folder", tr("Ordner")],
  ["tag", "Tag"],
  ["state", tr("Zustand")],
  ["priority", tr("Priorität")],
  ["date", tr("Datum")],
  ["none", tr("keine")],
];
export const SORTS = [
  ["position", tr("Ordnung")],
  ["due", tr("Fällig")],
  ["planned", tr("Geplant")],
  ["defer", tr("Start")],
  ["title", tr("Titel")],
  ["created", tr("Erstellt")],
  ["modified", tr("Geändert")],
  ["estimate", tr("Schätzung")],
  ["priority", tr("Priorität")],
];
const label = (list, key) => list.find(([k]) => k === key)?.[1] ?? key;
const arrow = (desc) => (desc ? "↓" : "↑");

// `⇅ Zustand · Priorität ↑, Fällig ↓` (§1).
function orderSummary(chosen) {
  const then = thenKey(chosen);
  const sorts = [`${label(SORTS, chosen.sort)} ${arrow(chosen.desc)}`];
  if (then) sorts.push(`${label(SORTS, then)} ${arrow(chosen.thenDesc)}`);
  const group = chosen.group === "none" ? null : label(GROUPS, chosen.group);
  return `⇅ ${[group, sorts.join(", ")].filter(Boolean).join(" · ")}`;
}

let orderMenuOpen = false;

function setOrderMenu(open) {
  if (orderMenuOpen === open) return;
  orderMenuOpen = open;
  update({});
}

function renderOrderMenu(chosen, board) {
  const button = $("order-button");
  button.textContent = orderSummary(chosen);
  button.setAttribute("aria-expanded", String(orderMenuOpen));
  const box = $("order-menu");
  box.hidden = !orderMenuOpen;
  if (!orderMenuOpen) {
    replace(box);
    return;
  }
  const key = orderKey();
  const set = (patch) => chooseOrder(key, patch);
  // Drawing anew loses the focus; it goes back to the same control.
  const focused = document.activeElement?.closest?.("#order-menu") ? document.activeElement.dataset.k : null;
  const select = (k, list, value, onchange, disabled = () => false) =>
    el(
      "select",
      { "data-k": k, onchange: (event) => onchange(event.target.value) },
      list.map(([v, text]) => el("option", { value: v, text, selected: v === value || null, disabled: disabled(v) || null })),
    );
  const direction = (k, desc, flip, disabled = false) =>
    el("button", {
      type: "button",
      class: "dir",
      "data-k": k,
      text: arrow(desc),
      title: desc ? tr("absteigend") : tr("aufsteigend"),
      "aria-label": desc ? tr("absteigend") : tr("aufsteigend"),
      disabled,
      onclick: flip,
    });
  const noThen = chosen.sort === "position";
  const then = thenKey(chosen);
  const line = (text, control, dir = null) => el("label", { class: "order-line" }, [el("span", { text }), control, dir]);
  replace(box, [
    line(
      tr("Gruppieren"),
      select("group", GROUPS, chosen.group, (group) => set({ group }), (v) => v === "state" && board && boardConfig(chosen).source === "workflow"),
    ),
    line(
      tr("Sortieren"),
      select("sort", SORTS, chosen.sort, (sort) => set({ sort })),
      direction("desc", chosen.desc, () => set({ desc: !chosen.desc })),
    ),
    line(
      tr("dann nach"),
      Object.assign(
        select("then", [["", "—"], ...SORTS.filter(([k]) => k !== "position" && k !== chosen.sort)], then ?? "", (v) => set({ then: v || null })),
        { disabled: noThen, title: noThen ? tr("Ordnung ist eindeutig") : "" },
      ),
      direction("thenDesc", chosen.thenDesc, () => set({ thenDesc: !chosen.thenDesc }), noThen || !then),
    ),
    el("div", { class: "order-actions" }, [
      // What is on screen as a perspective of its own (ADR 0090 §5).
      shown().kind === "perspective"
        ? null
        : el("button", {
            type: "button",
            "data-k": "save",
            text: tr("Als Perspektive sichern …"),
            onclick: () => {
              setOrderMenu(false);
              startPerspectiveDraft(captureFrom(chosenOrder()));
            },
          }),
      el("button", { type: "button", "data-k": "reset", text: tr("Zurücksetzen"), onclick: () => set(arrangedBy()) }),
      el("button", { type: "button", "data-k": "done", text: tr("Fertig"), onclick: () => setOrderMenu(false) }),
    ]),
  ]);
  if (focused) box.querySelector(`[data-k="${focused}"]`)?.focus();
}

function renderOrder() {
  renderRowMenu(!["stuck", "calendar", "time"].includes(shown().kind));
  const box = $("order");
  // A block perspective is arranged block by block (ADR 0116 §5).
  box.hidden = ["stuck", "review", "forecast", "calendar", "time"].includes(shown().kind) || Boolean(blocksShown());
  if (box.hidden) {
    $("middle").classList.remove("boarded", "tabled");
    return;
  }
  const chosen = chosenOrder();
  const board = asBoard();
  const table = asTable();
  renderOrderMenu(chosen, board);
  for (const button of document.querySelectorAll("#view-switch button")) {
    const on = button.dataset.view === (board ? "board" : table ? "table" : "list");
    button.classList.toggle("current", on);
    button.setAttribute("aria-pressed", String(on));
  }
  $("view-switch").hidden = !boardable();
  $("middle").classList.toggle("boarded", board);
  $("middle").classList.toggle("tabled", table);
  renderBoardMenu(board ? chosen : { ...chosen, view: "list" }, orderKey(), shown());
  renderTableMenu(table, tableColumns(chosen), (columns) => chooseOrder(orderKey(), { table: { columns } }));
}

export function wireOrder() {
  $("order-button").addEventListener("click", () => {
    if (!orderMenuOpen) {
      closeBoardMenu();
      closeTableMenu();
    }
    setOrderMenu(!orderMenuOpen);
  });
  // Esc or a click beside it closes the menu (ADR 0064 §2), before the
  // keys of the page see Esc.
  window.addEventListener(
    "keydown",
    (event) => {
      if (!orderMenuOpen || event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      setOrderMenu(false);
      $("order-button").focus();
    },
    true,
  );
  document.addEventListener("mousedown", (event) => {
    if (orderMenuOpen && !event.target.closest("#order-menu, #order-button")) setOrderMenu(false);
  });
  // The same for the table's menu "Spalten" (ADR 0115 §2).
  window.addEventListener(
    "keydown",
    (event) => {
      if (!tableMenuOpen() || event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      closeTableMenu();
      $("table-columns").focus();
    },
    true,
  );
  document.addEventListener("mousedown", (event) => {
    if (tableMenuOpen() && !event.target.closest("#table-menu, #table-columns")) closeTableMenu();
  });
  for (const button of document.querySelectorAll("#view-switch button")) {
    button.addEventListener("click", () => chooseOrder(orderKey(), { view: button.dataset.view }));
  }
  $("board-columns").addEventListener("click", () => {
    setOrderMenu(false);
    toggleBoardMenu();
  });
  $("table-columns").addEventListener("click", () => {
    setOrderMenu(false);
    toggleTableMenu();
  });
  $("board-full").addEventListener("click", () => setBoardFull(!boardFull()));
  $("fold-all").addEventListener("click", foldHeadings);
  // The menu „Zeile …“ (ADR 0130 §6), one menu at a time.
  $("row-fields").addEventListener("click", () => {
    if (!rowMenuOpen()) {
      setOrderMenu(false);
      closeBoardMenu();
      closeTableMenu();
    }
    toggleRowMenu();
  });
  window.addEventListener(
    "keydown",
    (event) => {
      if (!rowMenuOpen() || event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      closeRowMenu();
      $("row-fields").focus();
    },
    true,
  );
  document.addEventListener("mousedown", (event) => {
    if (rowMenuOpen() && !event.target.closest("#row-menu, #row-fields")) closeRowMenu();
  });
}

export function renderList() {
  headings = [];
  drawList();
  renderFoldAll();
}

function drawList() {
  order = [];
  if (blocksShown()?.id !== emptyFor) {
    emptyFor = blocksShown()?.id;
    openedEmpty.clear();
  }
  clearBoard();
  replace($("list-title"), title());
  const base = shown().kind === "perspective" && perspective() ? baseFilter(perspective()) : state.filter;
  const closed = filterNow() !== base;
  for (const button of document.querySelectorAll("#filter button")) {
    button.classList.toggle("current", button.dataset.filter === filterNow());
    button.disabled = !filtered() || closed;
  }
  // Each block has its own filter (ADR 0116 §5).
  $("filter").hidden = Boolean(blocksShown());
  $("filter").title = closed ? tr("Die Suche fragt danach: es gilt „{filter}“", { filter: { all: tr("Alle"), remaining: tr("Verbleibend") }[filterNow()] }) : "";
  renderOrder();
  // The calendar fills the middle and scrolls its hours itself.
  $("middle").classList.toggle("calendar", shown().kind === "calendar");
  if (shown().kind === "stuck") {
    const projects = stuckProjects();
    replace(
      $("list"),
      projects.length
        ? el(
            "ul",
            { class: "rows" },
            projects.map((p) =>
              el(
                "li",
                {
                  class: "row",
                  onclick: () => update({ view: { kind: "project", project: p.id }, selected: null }),
                },
                [
                  el("span", { class: "title", text: p.title }),
                  el("span", { class: "meta" }, el("span", { class: "reason", text: tr("keine nächste Aktion") })),
                ],
              ),
            ),
          )
        : el("p", { class: "empty", text: tr("Kein Projekt steckt fest.") }),
    );
    return;
  }
  if (shown().kind === "review") {
    place = "none";
    const due = reviewProjects();
    replace($("list"), due.length ? due.map(reviewSection) : el("p", { class: "empty", text: tr("Kein Review fällig.") }));
    return;
  }
  if (shown().kind === "time") {
    replace($("list"), el("div", { class: "time report-view" }, reportView()));
    return;
  }
  if (shown().kind === "calendar") {
    replace($("list"), calendarView());
    return;
  }
  if (shown().kind === "forecast") {
    place = "full";
    replace($("list"), [forecastStrip(), ...state.forecast.map(forecastSection)]);
    return;
  }
  if (shown().kind === "search" && wantsAppointments(state.searchChips)) {
    replace($("list"), appointmentSearch());
    return;
  }
  const blocks = blocksShown();
  if (blocks) {
    drawBlocks(blocks);
    return;
  }
  const statuses = state.statuses.filter((s) => inView(s));
  const chosen = chosenOrder();
  const compare = comparer(chosen);
  place = chosen.group === "project" || shown().kind === "project" ? "none" : chosen.group === "folder" ? "project" : "full";
  if (asBoard()) {
    replace($("list"), boardView(boardContext(chosen, compare)));
    return;
  }
  if (asTable()) {
    drawTable($("list"), tableContext(statuses, chosen, compare));
    return;
  }
  const sections = groups(statuses, chosen.group).map(([key, heading, list, subtitle]) => section(heading, list, compare, subtitle, key));
  const drawn = sections.filter(Boolean);
  replace($("list"), elsewhereHint(), drawn.length ? drawn : el("p", { class: "empty", text: tr("Nichts hier.") }));
}

// A search's hits that another own scope alone shows: how many, by scope,
// and a click there; the hits themselves stay unshown (ADR 0124 §3).
function elsewhereHint() {
  if (shown().kind !== "search") return null;
  const counts = new Map();
  for (const s of state.elsewhere) {
    if (inView(s) && (!filtered() || matches(s))) counts.set(s.elsewhere, (counts.get(s.elsewhere) ?? 0) + 1);
  }
  if (!counts.size) return null;
  return el(
    "p",
    { class: "elsewhere-hint" },
    [...counts].map(([scope, n]) =>
      el("button", {
        type: "button",
        class: "link",
        text: tr("{n} Treffer nur im Scope {scope} →", { n, scope: byId(state.scopes, scope)?.title ?? "?" }),
        onclick: () => chooseScope(scope),
      }),
    ),
  );
}

// --- Block perspectives (ADR 0116 §5) ----------------------------------

// The arrangement a fixed view has here when nothing is chosen.
const viewOrder = () => ({ view: "list", group: "project", sort: "position", desc: false, then: null, thenDesc: false });

// Opens what a block shows: its perspective with its search, or the view.
function openBlock(section) {
  if (section.target) update({ view: { kind: "perspective", perspective: section.target.id }, ...searchOf(section.target), selected: null });
  else if (section.block?.view && VIEWS.some((v) => v.kind === section.block.view)) update({ view: { kind: section.block.view, project: null }, search: "", searchChips: [], selected: null });
}

// The empty blocks opened by a click since the block perspective
// `emptyFor` was opened; another view forgets them.
const openedEmpty = new Set();
let emptyFor;

function openEmpty(fold) {
  if (!openedEmpty.delete(fold)) openedEmpty.add(fold);
  renderList();
}

// A block perspective: each block a section with its heading and count,
// its rows arranged as its perspective arranges them, as a list or a
// table (a board is drawn as a list).
function drawBlocks(p) {
  place = "full";
  const statuses = state.statuses.filter((s) => !s.outside);
  const shows = (q, s) => holds(q, s) && passes(filterOf(q), s);
  const sections = assemble(p, perspectives(), statuses, shows);
  drawTables($("list"), (table) => {
    const drawn = sections.map((block, i) => blockSection(p, block, i, table));
    return drawn.length ? drawn : el("p", { class: "empty", text: tr("Nichts hier.") });
  });
}

function blockSection(p, block, i, table) {
  const prefix = `block:${p.id}:${i}:`;
  const fold = laneFold(`block:${p.id}:${i}`);
  const count = new Set(block.tasks.map((s) => s.task.id)).size;
  // An empty block opens folded and stays out of "fold all".
  const hits = count > 0 || Boolean(block.missing);
  if (hits) headings.push(fold);
  const folded = blockFolded(hits, state.folded.has(fold), openedEmpty.has(fold));
  const click = (event) => (hits ? foldClick(fold, event) : openEmpty(fold));
  const viewTitle = block.block?.view ? VIEWS.find((v) => v.kind === block.block.view)?.title : null;
  // A search of the block's own is called by its words in the page's
  // language (ADR 0143 §2).
  const queryTitle = block.query ? queryWords(block.block.query) || tr("Alle Aufgaben") : null;
  const name = block.block?.title ?? viewTitle ?? queryTitle ?? block.title;
  const opens = Boolean(block.target && !block.missing) || Boolean(viewTitle);
  const head = el("div", { class: "head block-head" }, [
    el("button", {
      type: "button",
      class: "fold",
      "aria-label": folded ? tr("aufklappen") : tr("zuklappen"),
      "aria-expanded": String(!folded),
      title: tr("Alt-Klick: nur diesen Block offen"),
      text: folded ? "▸" : "▾",
      onclick: click,
    }),
    el("h3", {
      text: [block.icon, name].filter(Boolean).join(" "),
      class: opens ? "opens" : null,
      title: opens ? tr("öffnen") : null,
      onclick: opens ? () => openBlock(block) : click,
    }),
    el("span", { class: "count", text: String(count) }),
    block.missing ? el("span", { class: "missing", text: tr("fehlt") }) : null,
    // Its heading opens nothing; instead it can become a perspective (§4).
    block.query
      ? el("button", {
          type: "button",
          class: "more",
          text: "⋯",
          title: tr("Als Perspektive sichern …"),
          "aria-label": tr("Als Perspektive sichern …"),
          onclick: () => openBlocks(p, { extract: i }),
        })
      : null,
  ]);
  const kind = `block${folded ? " folded" : ""}${hits ? "" : " no-hits"}`;
  if (folded || block.missing) return el("section", { class: kind }, [head]);
  const visited = order.length;
  const chosen = block.query ? { ...orderOf(block.query) } : block.target && !block.missing ? { ...orderOf(block.target) } : viewOrder();
  const compare = comparer(chosen);
  const orderAt = block.query ? `block-query:${p.id}:${i}` : block.target ? `perspective:${block.target.id}` : `block-view:${block.block?.view}`;
  let body;
  if (!block.tasks.length) body = el("p", { class: "empty", text: tr("Nichts hier.") });
  else if (chosen.view === "table") body = table(tableContext(block.tasks, chosen, compare, orderAt, prefix));
  else {
    const drawn = groups(block.tasks, chosen.group)
      .map(([key, heading, list, subtitle]) => section(heading, list, compare, subtitle, key, prefix))
      .filter(Boolean);
    body = drawn.length ? drawn : el("p", { class: "empty", text: tr("Nichts hier.") });
  }
  if (order.length < visited) order.length = visited;
  return el("section", { class: kind }, [head, el("div", { class: "block-body" }, body)]);
}

// What the board needs of the list (ADR 0061 §1): the same view, filter
// and sorting; lanes from the menu Gruppieren (§8); a card is a row.
function boardContext(chosen, compare) {
  const view = shown();
  // Heute and Markiert keep only what is open; their column "Erledigt"
  // asks the same without that (§6).
  const done = (s) => (view.kind === "today" ? Boolean(s.today) : view.kind === "flagged" ? s.task.flagged : inView(s));
  const group = laneGroup(chosen);
  return {
    chosen,
    view,
    filter: filtered() ? filterNow() : "remaining",
    open: (s) => inView(s) && (!filtered() || matches(s)),
    done,
    lanes: (statuses) => {
      const lanes = groups(statuses, group).map(([key, heading, list]) => [key, heading, list]);
      headings = lanes.filter(([, heading]) => heading).map(([key]) => laneFold(key));
      return lanes;
    },
    foldClick,
    compare,
    card: (s) => {
      const node = row({ ...s, group: null }, 0, false);
      node.classList.add("card");
      return node;
    },
    visit: (id) => {
      if (!order.includes(id)) order.push(id);
    },
  };
}

// What the table needs of the list (ADR 0115 §3): the list's sections
// and trees, as entries instead of elements; a heading folds as it does
// in the list, and j/k visit the rows in the same order.
function tableContext(statuses, chosen, compare, orderAt = orderKey(), prefix = "") {
  const entry = (status, depth, context, _why, bare) => ({ status, depth, context, bare });
  const sections = [];
  for (const [key, heading, list, subtitle] of groups(statuses, chosen.group)) {
    const entries = [];
    const visited = order.length;
    tree(list, entries, compare, 0, entry);
    if (!entries.length) continue;
    if (!heading) {
      sections.push({ heading: null, entries, folded: false });
      continue;
    }
    const fold = laneFold(`${prefix}${key}`);
    headings.push(fold);
    const folded = state.folded.has(fold);
    if (folded) order.length = visited;
    sections.push({ fold, heading, subtitle, entries, folded });
  }
  return {
    chosen,
    columns: tableColumns(chosen),
    sections,
    lead,
    lines,
    rowClick,
    foldClick,
    sortBy: (sort) => chooseOrder(orderAt, chosen.sort === sort ? { desc: !chosen.desc } : { sort, desc: false }),
    setColumns: (columns) => chooseOrder(orderAt, { table: { columns } }),
  };
}

// --- ist:termin (ADR 0047 §14, ADR 0048) ------------------------------

// The next occurrence of each appointment, asked of the server once for
// the appointments as they are; a new sync asks again.
let next = { for: null, byId: null };
function nextOccurrences() {
  if (next.for === state.appointments) return next.byId;
  next = { for: state.appointments, byId: null };
  const asked = next;
  const from = today();
  api
    .appointments(from, addDays(from, 400))
    .then((list) => {
      const byId = new Map();
      for (const i of list) if (!byId.has(i.appointment)) byId.set(i.appointment, i);
      asked.byId = byId;
      if (next === asked) update({});
    })
    .catch((error) => update({ message: error.message }));
  return null;
}

const DATED = ["due", "planned", "defer", "completed", "created", "modified"];

// Appointments instead of tasks: every free word in title, place or note;
// a date chip asks of the next occurrence's first day; once each.
function appointmentSearch() {
  const byId = nextOccurrences();
  if (!byId) return el("p", { class: "empty", text: tr("Termine werden geladen …") });
  const words = state.search.toLowerCase().split(/\s+/).filter(Boolean);
  const ranges = state.searchChips
    .filter((c) => DATED.includes(c.kind))
    .map((c) => ({ range: rangeOf(c.value), not: c.not }))
    .filter((r) => r.range);
  const hits = [];
  for (const a of state.appointments) {
    const hay = `${a.title}\n${a.location ?? ""}\n${a.note ?? ""}`.toLowerCase();
    if (!words.every((w) => hay.includes(w))) continue;
    const occurrence = byId.get(a.id);
    if (!occurrence) continue;
    const day = occurrence.start.date ?? localDayOf(occurrence.start.at);
    const fits = ranges.every(({ range, not }) => {
      const inside = range.any || (!range.none && !range.overdue && (!range.from || day >= range.from) && (!range.to || day <= range.to));
      return not ? !inside : inside;
    });
    if (fits) hits.push({ day, occurrence });
  }
  // By day; on a day whole days first, then by start.
  const key = ({ day, occurrence }) => `${day} ${occurrence.start.date ? "0" : `1${new Date(occurrence.start.at.replace(/\[[^\]]*\]$/, "")).toISOString()}`}`;
  hits.sort((x, y) => (key(x) < key(y) ? -1 : key(x) > key(y) ? 1 : 0));
  if (!hits.length) return el("p", { class: "empty", text: tr("Kein Termin passt.") });
  return el("section", { class: "appointment-search" }, [
    el("ul", { class: "appointments" }, hits.map(({ day, occurrence }) => appointmentLine(occurrence, `${shortDay({ date: day })} ${whenText(occurrence)}`))),
  ]);
}

// The day of a zoned time in this browser.
function localDayOf(at) {
  const d = new Date(at.replace(/\[[^\]]*\]$/, ""));
  const pad = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}
