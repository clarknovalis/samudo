// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Settings: projects and folders, and tags (ADR 0036 §8, ADR 0037 §5).
// A tree to choose from in the middle, the inspector of the chosen one on
// the right. Making, renaming, moving and the project's properties are
// ordinary changes; deleting goes through POST /objects/{id}/delete, which
// the server composes, after a preview whose numbers the panel shows.
//
// Fields are kept per object across redraws, so a reload from /events does
// not take the text from under the cursor.

import { api, change } from "./api.js";
import { el } from "./dom.js";
import { byPosition, createPlace, movePlace, subtree } from "./placeops.js";
import { propertyBlock } from "./props.js";
import { byId, isOpen, onlyIn, state, update } from "./state.js";
import { locale, tr } from "./i18n.js";

const fields = new Map();
// The delete being asked about: { id, kind, fate, target, preview, error }.
let pending = null;
// Completed and dropped projects in the tree too (ADR 0037 §3).
let showClosed = false;
// Something just made, whose name field takes the cursor once drawn.
let focusNext = null;
// The tag being dragged to another place among its siblings:
// the order of a workflow's states is the order of the board's columns.
let dragged = null;
// The tag moved by Alt+↑/↓, whose line keeps the cursor once redrawn.
let refocus = null;

const closed = (p) => p.status === "completed" || p.status === "dropped";
const all = () => [...state.projects, ...state.folders, ...state.tags];

const KIND = { folder: tr("Ordner"), project: tr("Projekt"), tag: "Tag" };
const PROJECT_KIND = [
  ["parallel", tr("Parallel")],
  ["sequential", tr("Sequenziell")],
  ["single_actions", tr("Einzelaufgaben")],
];
const STATUS = [
  ["active", tr("Aktiv")],
  ["on_hold", tr("Ruht")],
  ["completed", tr("Erledigt")],
  ["dropped", tr("Verworfen")],
];
const REVIEW = [
  ["", tr("Keine")],
  ["P1W", tr("Jede Woche")],
  ["P2W", tr("Alle 2 Wochen")],
  ["P1M", tr("Jeden Monat")],
  ["P3M", tr("Alle 3 Monate")],
];

function kindOf(id) {
  if (byId(state.projects, id)) return "project";
  if (byId(state.folders, id)) return "folder";
  if (byId(state.tags, id)) return "tag";
  return null;
}

async function write(changes) {
  try {
    await api.write(changes);
    return true;
  } catch (error) {
    update({ message: error.message });
    return false;
  }
}

function choose(id) {
  pending = null;
  update({ settings: { ...state.settings, place: id } });
}

// --- The tree --------------------------------------------------------

const siblingTags = (tag) =>
  state.tags.filter((t) => (t.parent ?? null) === (tag.parent ?? null)).sort(byPosition);

// Moves `tag` before `before` among its siblings, or to their end; the
// parent stays, so a state does not slip into another workflow.
function reorderTag(tag, before) {
  movePlace(tag, "tag", tag.parent ?? null, before);
}

// Alt+↑/↓ on a tag in the tree moves it one place up or down.
function stepTag(tag, by) {
  const list = siblingTags(tag);
  const at = list.findIndex((t) => t.id === tag.id);
  const to = at + by;
  if (at < 0 || to < 0 || to >= list.length) return;
  const rest = list.filter((t) => t.id !== tag.id);
  refocus = tag.id;
  reorderTag(tag, rest[to]?.id ?? null);
}

const clearDrop = () => {
  for (const node of document.querySelectorAll(".place-tree .drop-before, .place-tree .drop-after")) {
    node.classList.remove("drop-before", "drop-after");
  }
};

// Dragging among the tags of one parent; elsewhere it drops nothing.
function tagDrag(tag) {
  const zoneOf = (event) => {
    if (!dragged || dragged.id === tag.id || (dragged.parent ?? null) !== (tag.parent ?? null)) return null;
    const box = event.currentTarget.getBoundingClientRect();
    return event.clientY - box.top < box.height / 2 ? "before" : "after";
  };
  return {
    draggable: "true",
    ondragstart: (event) => {
      dragged = tag;
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", tag.id);
    },
    ondragend: () => {
      dragged = null;
      clearDrop();
    },
    ondragover: (event) => {
      const zone = zoneOf(event);
      if (!zone) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      const node = event.currentTarget;
      if (!node.classList.contains(`drop-${zone}`)) {
        clearDrop();
        node.classList.add(`drop-${zone}`);
      }
    },
    ondragleave: (event) => event.currentTarget.classList.remove("drop-before", "drop-after"),
    ondrop: (event) => {
      const zone = zoneOf(event);
      const moving = dragged;
      dragged = null;
      clearDrop();
      if (!zone) return;
      event.preventDefault();
      const rest = siblingTags(tag).filter((t) => t.id !== moving.id);
      const at = rest.findIndex((t) => t.id === tag.id);
      const before = zone === "before" ? tag.id : (rest[at + 1]?.id ?? null);
      reorderTag(moving, before);
    },
  };
}

function line(object, kind, depth) {
  const meta = [];
  // ADR 0094 §8: shown only in its own scope, named.
  const only = onlyIn(object);
  if (only) meta.push(only);
  if (kind === "project") {
    if (object.kind === "single_actions") meta.push(tr("Einzelaufgaben"));
    if (object.kind === "sequential") meta.push(tr("sequenziell"));
    if (object.status !== "active") meta.push(STATUS.find(([v]) => v === object.status)?.[1].toLowerCase());
  }
  const chosen = state.settings.place === object.id;
  const tag = kind === "tag";
  const button = el(
    "button",
    {
      type: "button",
      class: `place-line ${kind}${kind === "project" && closed(object) ? " closed" : ""}`,
      style: `--depth: ${depth}`,
      "aria-current": chosen ? "true" : null,
      title: tag ? tr("Ziehen oder Alt+↑/↓ ändert die Reihenfolge, bei Zuständen die Spalten des Boards") : null,
      onclick: () => choose(object.id),
      onkeydown: tag
        ? (event) => {
            if (!event.altKey || (event.key !== "ArrowUp" && event.key !== "ArrowDown")) return;
            event.preventDefault();
            stepTag(object, event.key === "ArrowUp" ? -1 : 1);
          }
        : null,
    },
    [
      el("span", { class: "glyph", "aria-hidden": "true", text: kind === "folder" ? "▸" : kind === "tag" ? "#" : "○" }),
      el("span", { class: "title", text: object.title }),
      meta.length ? el("span", { class: "meta", text: meta.filter(Boolean).join(" · ") }) : null,
    ],
  );
  // Only when the redraw took the cursor away, never from a field.
  if (refocus === object.id) {
    requestAnimationFrame(() => {
      if (document.activeElement === document.body) button.focus();
    });
  }
  return el("li", { class: chosen ? "current" : null, ...(tag ? tagDrag(object) : {}) }, button);
}

function placeTree() {
  const rows = [];
  const walk = (parent, depth, seen) => {
    for (const folder of state.folders.filter((f) => (f.parent ?? null) === parent).sort(byPosition)) {
      if (seen.has(folder.id)) continue;
      seen.add(folder.id);
      rows.push(line(folder, "folder", depth));
      walk(folder.id, depth + 1, seen);
    }
    const projects = state.projects.filter((p) => (p.folder ?? null) === parent && (showClosed || !closed(p)));
    for (const project of projects.sort(byPosition)) rows.push(line(project, "project", depth));
  };
  walk(null, 0, new Set());
  return rows;
}

function tagTree() {
  const rows = [];
  const walk = (parent, depth, seen) => {
    for (const tag of state.tags.filter((t) => (t.parent ?? null) === parent).sort(byPosition)) {
      if (seen.has(tag.id)) continue;
      seen.add(tag.id);
      rows.push(line(tag, "tag", depth));
      walk(tag.id, depth + 1, seen);
    }
  };
  walk(null, 0, new Set());
  return rows;
}

// --- Making ----------------------------------------------------------

const now = () => new Date().toISOString();

// Where something new goes: into the chosen folder, beside the chosen
// project, beside the chosen tag.
function hereFolder() {
  const id = state.settings.place;
  if (byId(state.folders, id)) return id;
  return byId(state.projects, id)?.folder ?? null;
}

const NEW_TITLE = { project: tr("Neues Projekt"), folder: tr("Neuer Ordner"), tag: tr("Neuer Tag") };

async function make(kind) {
  const at = kind === "tag" ? (byId(state.tags, state.settings.place)?.parent ?? null) : hereFolder();
  const id = await createPlace(kind, NEW_TITLE[kind], at);
  if (!id) return;
  focusNext = id;
  choose(id);
}

// --- The inspector ---------------------------------------------------

function nameField(object) {
  let field = fields.get(object.id);
  if (!field) {
    field = el("input", {
      id: `place-name-${object.id}`,
      autocomplete: "off",
      onchange: (event) => {
        const current = byId(all(), object.id);
        const title = event.target.value.trim();
        if (!current) return;
        if (title && title !== current.title) write([change.set(object.id, "title", title)]);
        else event.target.value = current.title;
      },
      onkeydown: (event) => {
        if (event.key === "Enter") event.target.blur();
      },
    });
    fields.set(object.id, field);
  }
  if (document.activeElement !== field) field.value = object.title;
  if (focusNext === object.id) {
    focusNext = null;
    requestAnimationFrame(() => {
      field.focus();
      field.select();
    });
  }
  return field;
}

// An emoji for a folder or project (ADR 0059 §5): a few to pick, a field
// for any other, and a way back to none.
const EMOJIS = ["🏠", "💼", "💻", "🛠️", "📚", "🎓", "💰", "🏦", "🛒", "🍳", "🌱", "🌻", "🚗", "✈️", "🏃", "❤️", "👨‍👩‍👧", "🐾", "🎵", "🎨", "📷", "🧾", "📦", "⭐"];

const iconFields = new Map();

function iconPicker(object) {
  const current = object.icon ?? null;
  const put = (icon) => {
    const clean = icon?.trim() || null;
    if (clean !== current) write([change.set(object.id, "icon", clean)]);
  };
  // Kept across redraws, like the name, so typing is not cut off.
  let field = iconFields.get(object.id);
  if (field) {
    if (document.activeElement !== field) field.value = current ?? "";
  } else field = el("input", {
    id: `place-icon-${object.id}`,
    class: "icon-field",
    autocomplete: "off",
    placeholder: "Emoji",
    value: current ?? "",
    onchange: (event) => {
      // The field outlives this draw: ask for the icon as it is now.
      const now = byId(all(), object.id)?.icon ?? null;
      const value = event.target.value.trim() || null;
      if (value && new TextEncoder().encode(value).length > 32) {
        update({ message: tr("Ein Icon ist ein Emoji, höchstens 32 Bytes.") });
        event.target.value = now ?? "";
        return;
      }
      if (value !== now) write([change.set(object.id, "icon", value)]);
    },
    onkeydown: (event) => {
      if (event.key === "Enter") event.target.blur();
    },
  });
  iconFields.set(object.id, field);
  return el("div", { class: "icon-picker" }, [
    field,
    el("button", { type: "button", class: "icon-none", title: tr("Kein Icon"), "aria-label": tr("Kein Icon"), text: "–", onclick: () => put(null) }),
    el("div", { class: "icon-choices" }, EMOJIS.map((emoji) =>
      el("button", {
        type: "button",
        class: emoji === current ? "on" : null,
        "aria-pressed": String(emoji === current),
        text: emoji,
        onclick: () => put(emoji),
      }),
    )),
  ]);
}

// ADR 0094: „Nur im Scope:“ — shown in that scope alone, whatever its
// mode; hidden in every other one and without a scope.
function ownScopePicker(object, kind) {
  const what = kind === "tag" ? tr("Aufgaben mit diesem Tag erscheinen") : kind === "folder" ? tr("Was in diesem Ordner liegt, erscheint") : tr("Dieses Projekt erscheint");
  const options = [["", tr("– überall –")], ...[...state.scopes].sort(byPosition).map((s) => [s.id, s.title])];
  const missing = object.own_scope && !byId(state.scopes, object.own_scope);
  if (missing) options.push([object.own_scope, tr("Scope fehlt")]);
  const scope = byId(state.scopes, object.own_scope);
  return el("div", { class: "own-scope" }, [
    select(
      options,
      object.own_scope,
      (value) => write([change.set(object.id, "own_scope", value || null)]),
      tr("Nur im Scope"),
    ),
    el("p", {
      class: "quiet",
      text: scope
        ? tr("{what} nur im Scope „{scope}“ – dort auch ohne Häkchen –, in keinem anderen und auch ohne Scope nicht.", { what, scope: scope.title })
        : missing
          ? tr("Den gewählten Scope gibt es nicht mehr; darum erscheint es überall.")
          : tr("{what} überall.", { what }),
    }),
  ]);
}

function row(label, control, forId = null) {
  return el("div", { class: "insp-row" }, [el("label", { class: "insp-label", for: forId, text: label }), control]);
}

function select(options, current, onchange, label) {
  return el(
    "select",
    { "aria-label": label, onchange: (event) => onchange(event.target.value) },
    options.map(([value, text]) => el("option", { value, text, selected: value === (current ?? "") })),
  );
}

// Places in a tree as options, indented, leaving out `skip`.
function treeOptions(list, prefix, skip = new Set()) {
  const out = [["", tr("Oben")]];
  const walk = (parent, depth, seen) => {
    for (const o of list.filter((x) => (x.parent ?? null) === parent).sort(byPosition)) {
      if (seen.has(o.id) || skip.has(o.id)) continue;
      seen.add(o.id);
      out.push([o.id, `${" ".repeat(depth)}${prefix}${o.title}`]);
      walk(o.id, depth + 1, seen);
    }
  };
  walk(null, 0, new Set());
  return out;
}

// Done and dropped carry their moment; back to open clears both
// (ADR 0037 §3).
function setStatus(project, status) {
  const at = now();
  write([
    change.set(project.id, "status", status),
    change.set(project.id, "completed_at", status === "completed" ? at : null),
    change.set(project.id, "dropped_at", status === "dropped" ? at : null),
  ]);
}

function projectFields(project) {
  const kinds = el(
    "div",
    { class: "segmented", role: "radiogroup", "aria-label": tr("Art") },
    PROJECT_KIND.map(([value, text]) =>
      el("button", {
        type: "button",
        role: "radio",
        "aria-checked": project.kind === value ? "true" : "false",
        class: project.kind === value ? "on" : null,
        text,
        onclick: () => project.kind !== value && write([change.set(project.id, "kind", value)]),
      }),
    ),
  );
  const workflows = [["", tr("Vorgabe")], ...state.tags.filter((t) => t.exclusive).sort(byPosition).map((t) => [t.id, t.title])];
  const open = state.statuses.filter((s) => s.task.project === project.id && isOpen(s.task)).length;
  return [
    row(tr("Art"), kinds),
    row("Status", select(STATUS, project.status, (v) => setStatus(project, v), "Status")),
    row(tr("Ablauf"), select(workflows, project.workflow, (v) => write([change.set(project.id, "workflow", v || null)]), tr("Ablauf"))),
    row("Review", select(REVIEW, project.review_interval, (v) => write([change.set(project.id, "review_interval", v || null)]), "Review")),
    ...reviewRows(project),
    el("p", { class: "quiet", text: open === 1 ? tr("1 offene Aufgabe") : tr("{n} offene Aufgaben", { n: open }) }),
  ];
}

// When the next review is, set by hand or as samu-core computed it, when
// the last was, and „Geprüft“ (ADR 0088 §2, §5).
function reviewRows(project) {
  if (!project.review_interval) return [];
  const computed = state.projectStatuses.find((p) => p.project.id === project.id);
  const next = el("input", {
    type: "date",
    value: project.next_review ?? computed?.review ?? "",
    "aria-label": tr("Nächstes Review"),
    title: project.next_review ? tr("von Hand gesetzt; leer rechnet wieder aus dem Abstand") : tr("aus dem Abstand gerechnet"),
    onchange: (event) => write([change.set(project.id, "next_review", event.target.value || null)]),
  });
  const last = project.last_reviewed ? new Date(project.last_reviewed).toLocaleDateString(locale()) : tr("noch nie");
  return [
    row(tr("Nächstes Review"), next),
    row(tr("Zuletzt geprüft"), el("span", {}, [
      el("span", { text: `${last} ` }),
      el("button", { type: "button", text: tr("Geprüft"), onclick: () => write(change.reviewed(project.id)) }),
    ])),
  ];
}

function inspector(id) {
  const kind = kindOf(id);
  if (!kind) {
    return el("div", { class: "inspector empty" }, el("p", {
      class: "empty",
      text: tr("Links etwas wählen, um es umzubenennen, zu verschieben oder zu löschen."),
    }));
  }
  const object = byId(all(), id);
  let place;
  if (kind === "project") {
    place = select(treeOptions(state.folders, `${tr("in")} `), object.folder, (v) => movePlace(object, kind, v), tr("Ort"));
  } else if (kind === "folder") {
    place = select(treeOptions(state.folders, `${tr("in")} `, subtree(state.folders, id)), object.parent, (v) =>
      movePlace(object, kind, v), tr("Ort"));
  } else {
    place = select(treeOptions(state.tags, `${tr("unter")} `, subtree(state.tags, id)), object.parent, (v) =>
      movePlace(object, kind, v), tr("Ort"));
  }
  const asking = pending?.id === id;
  return el("div", { class: "inspector" }, [
    el("p", { class: "insp-kind", text: KIND[kind] }),
    row("Name", nameField(object), `place-name-${id}`),
    row(tr("Ort"), place),
    kind !== "tag" ? row("Icon", iconPicker(object), `place-icon-${id}`) : null,
    row(tr("Nur im Scope"), ownScopePicker(object, kind)),
    kind === "project" ? propertyBlock(object.id, object.properties ?? {}) : null,
    ...(kind === "project" ? projectFields(object) : []),
    asking
      ? deletePanel()
      : el("div", { class: "insp-actions" }, el("button", {
          type: "button",
          class: "danger-quiet",
          text: tr("{kind} löschen …", { kind: KIND[kind] }),
          onclick: () => ask(id, kind),
        })),
  ]);
}

// --- Deleting (ADR 0036) ---------------------------------------------

function body() {
  if (pending.kind !== "project") return {};
  if (pending.fate === "project") return pending.target ? { tasks: { project: pending.target } } : null;
  return { tasks: pending.fate };
}

async function preview() {
  const request = body();
  pending.preview = null;
  pending.error = null;
  update({});
  if (!request) return;
  const asked = pending;
  try {
    const answer = await api.deleteObject(asked.id, { ...request, preview: true });
    if (pending === asked) pending.preview = answer;
  } catch (error) {
    if (pending === asked) pending.error = error.message;
  }
  update({});
}

function ask(id, kind) {
  pending = { id, kind, fate: "inbox", target: null, preview: null, error: null };
  preview();
}

async function confirm() {
  const request = body();
  if (!request) return;
  const id = pending.id;
  try {
    await api.deleteObject(id, request);
    pending = null;
    fields.delete(id);
    update({ message: null, settings: { ...state.settings, place: null } });
  } catch (error) {
    pending.error = error.message;
    update({});
  }
}

function counts(answer, kind) {
  if (kind === "project") {
    const n = pending.fate === "delete" ? answer.deleted - 1 : answer.moved;
    if (n === 0) return tr("Das Projekt hat keine Aufgaben.");
    if (pending.fate === "delete") return n === 1 ? tr("1 Aufgabe wird mitgelöscht.") : tr("{n} Aufgaben werden mitgelöscht.", { n });
    return n === 1 ? tr("1 Aufgabe wandert.") : tr("{n} Aufgaben wandern.", { n });
  }
  if (kind === "folder") {
    return answer.moved ? tr("{n} Projekte und Ordner rücken eine Ebene auf.", { n: answer.moved }) : tr("Der Ordner ist leer.");
  }
  return answer.untagged
    ? tr("Der Tag wird von {n} Aufgaben, Projekten und Scopes genommen.", { n: answer.untagged })
    : tr("Niemand trägt den Tag.");
}

function deletePanel() {
  const kind = pending.kind;
  const choice = (value, text, extra = null) =>
    el("label", { class: "mode" }, [
      el("input", {
        type: "radio",
        name: "delete-fate",
        value,
        checked: pending.fate === value,
        onchange: () => {
          pending.fate = value;
          preview();
        },
      }),
      el("span", { text }),
      extra,
    ]);
  const others = state.projects.filter((p) => p.id !== pending.id && !closed(p)).sort(byPosition);
  const target = el(
    "select",
    {
      "aria-label": tr("Zielprojekt"),
      disabled: pending.fate !== "project",
      onchange: (event) => {
        pending.target = event.target.value || null;
        preview();
      },
    },
    [
      el("option", { value: "", text: tr("Projekt wählen …"), selected: !pending.target }),
      ...others.map((p) => el("option", { value: p.id, text: p.title, selected: p.id === pending.target })),
    ],
  );
  return el("div", { class: "delete-panel" }, [
    el("p", { class: "delete-title", text: tr("{kind} löschen", { kind: KIND[kind] }) }),
    kind === "project"
      ? el("fieldset", { class: "modes" }, [
          el("legend", { text: tr("Die Aufgaben des Projekts") }),
          choice("inbox", tr("in die Inbox")),
          choice("project", tr("in ein anderes Projekt"), target),
          choice("delete", tr("mitlöschen")),
        ])
      : null,
    pending.error
      ? el("p", { class: "error", text: pending.error })
      : el("p", { class: "quiet", text: pending.preview ? counts(pending.preview, kind) : "…" }),
    el("div", { class: "actions" }, [
      el("button", { type: "button", text: tr("Abbrechen"), onclick: () => ((pending = null), update({})) }),
      el("button", {
        type: "button",
        class: "danger",
        text: tr("Löschen"),
        disabled: !pending.preview || Boolean(pending.error),
        onclick: confirm,
      }),
    ]),
  ]);
}

// --- The pane --------------------------------------------------------

export function placesPane(section) {
  // An object deleted elsewhere takes its question along.
  if (pending && !kindOf(pending.id)) pending = null;
  const chosen = state.settings.place;
  const fits =
    chosen && (section === "tags" ? kindOf(chosen) === "tag" : ["project", "folder"].includes(kindOf(chosen)));
  const tree = section === "tags" ? tagTree() : placeTree();
  const toolbar =
    section === "tags"
      ? [el("button", { type: "button", text: "+ Tag", onclick: () => make("tag") })]
      : [
          el("button", { type: "button", text: tr("+ Projekt"), onclick: () => make("project") }),
          el("button", { type: "button", text: tr("+ Ordner"), onclick: () => make("folder") }),
          el("label", { class: "toggle" }, [
            el("input", {
              type: "checkbox",
              checked: showClosed,
              onchange: (event) => {
                showClosed = event.target.checked;
                update({});
              },
            }),
            el("span", { text: tr("Erledigte") }),
          ]),
        ];
  return el("div", { class: "panes" }, [
    el("div", { class: "tree-pane" }, [
      el("div", { class: "toolbar" }, toolbar),
      tree.length
        ? el("ul", { class: "place-tree" }, tree)
        : el("p", { class: "empty", text: section === "tags" ? tr("Noch keine Tags.") : tr("Noch keine Projekte.") }),
      el("p", {
        class: "quiet",
        text:
          section === "tags"
            ? tr("Ein Tag mit Kindern oder einer, der einem Projekt als Ablauf dient, lässt sich nicht löschen.")
            : tr("Ein Sammelsurium ist ein Projekt der Art Einzelaufgaben. Ein gelöschter Ordner gibt seinen Inhalt eine Ebene nach oben ab."),
      }),
    ]),
    inspector(fits ? chosen : null),
  ]);
}
