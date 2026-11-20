// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Refile (ADR 0020 §4): a palette with a fuzzy search over the inbox,
// projects and action groups. ↵ moves there, ⇥ makes the task a child.
// Indenting in the list (ADR 0021 §5) and the project select in the
// inspector go through the same `moveTo`, so there is one way to move.
// "Verschieben nach…" in the sidebar opens it for a project or folder;
// then the targets are the folders and the top (ADR 0041 §5).
// "Blockiert durch…" opens it over the open tasks (ae5a3206): ↵ makes the
// one chosen a blocker of the task.
// `o` opens it to jump to a view, project, folder, tag or perspective
// (db3fe005).

import { api, change } from "./api.js";
import { $, el, replace, uuidv7 } from "./dom.js";
import { byPosition, movePlace, subtree } from "./placeops.js";
import { info } from "./message.js";
import { byId, chooseScope, elsewhereOf, isOpen, state, statusOf, update } from "./state.js";
import { placeOf } from "./words.js";
import { tr } from "./i18n.js";
import { jumpTargets, rank } from "./jump.js";
import { openObject } from "./address.js";
import { reportTab } from "./report.js";
import { openSettings } from "./settings.js";
import { chooseView, VIEWS } from "./sidebar.js";

const INBOX = { kind: "inbox", project: null, parent: null, title: "Inbox" };

// Moves a task under `project` and `parent` (either may be null), last
// among its new siblings, in one write. Its descendants go along into
// `project` and keep their parents, as in the app (ADR 0040 §3).
export function moveTo(task, project, parent) {
  return moveMany([task], project, parent);
}

// The same for many tasks (ADR 0052 §2): each last there in the order
// given, in one write. A task whose ancestor moves as well stays under it
// (§3).
export async function moveMany(tasks, project, parent) {
  const changes = moveChanges(tasks, project, parent);
  if (!changes) {
    update({ message: tr("Eine Aufgabe kann nicht unter sich selbst oder darunter.") });
    return;
  }
  const from = tasks.map((t) => ({ id: t.id, project: t.project ?? null, parent: t.parent ?? null }));
  try {
    await api.write(changes);
    update({ message: movedAway(project, from) });
  } catch (error) {
    update({ message: error.message });
  }
}

// `title · nur im Scope Exit` for a project another own scope alone shows
// (ADR 0124 §4).
export function withScope(title, project) {
  const scope = elsewhereOf(project);
  return scope ? `${title} · ${tr("nur im Scope {scope}", { scope: byId(state.scopes, scope)?.title ?? "?" })}` : title;
}

// Moved where this scope does not look: said so, with the way there and
// back (ADR 0124 §5); otherwise no message.
function movedAway(project, from) {
  const scope = elsewhereOf(project);
  if (!scope) return null;
  const title = byId(state.projects, project)?.title ?? "?";
  const back = async () => {
    const changes = [];
    for (const { id, project: p, parent } of from) {
      const task = byId(state.tasks, id);
      if (task) changes.push(...(moveChanges([task], p, parent) ?? []));
    }
    try {
      await api.write(changes);
      update({ message: null });
    } catch (error) {
      update({ message: error.message });
    }
  };
  return info(
    tr("Verschoben nach {project}, nur im Scope {scope} sichtbar.", { project: title, scope: byId(state.scopes, scope)?.title ?? "?" }),
    [
      [tr("Dorthin"), () => {
        chooseScope(scope);
        update({ view: { kind: "project", project }, message: null });
      }],
      [tr("Rückgängig"), back],
    ],
  );
}

// The changes of such a move, unwritten; null when a task would go under
// itself. The edit dialog writes them with its other changes (ADR 0078).
export function moveChanges(tasks, project, parent) {
  const ids = new Set(tasks.map((t) => t.id));
  const roots = tasks.filter((t) => !ancestors(t.id).some((a) => ids.has(a)));
  if (parent && roots.some((t) => t.id === parent || ancestors(parent).includes(t.id))) return null;
  const moving = new Set(roots.map((t) => t.id));
  const siblings = state.statuses.filter(
    (s) =>
      !moving.has(s.task.id) &&
      (s.task.project ?? null) === project &&
      (s.task.parent ?? null) === parent,
  );
  let position = siblings.reduce((max, s) => Math.max(max, s.task.position + 1), 0);
  const changes = [];
  for (const task of roots) {
    changes.push(
      change.set(task.id, "project", project),
      change.set(task.id, "parent", parent),
      change.set(task.id, "position", position),
    );
    position += 1;
    for (const t of descendants(task.id)) {
      if ((t.project ?? null) !== project) changes.push(change.set(t.id, "project", project));
    }
  }
  return changes;
}

// The parents above `id`, nearest first.
export function ancestors(id) {
  const out = [];
  let at = byId(state.tasks, id)?.parent ?? null;
  while (at && !out.includes(at)) {
    out.push(at);
    at = byId(state.tasks, at)?.parent ?? null;
  }
  return out;
}

// `>`: child of the sibling drawn above it. `<`: one level up, right
// after its old parent's siblings.
export function indent(status) {
  const task = status.task;
  const above = siblingAbove(status);
  if (!above) {
    update({ message: tr("Darüber steht keine Aufgabe, unter die sie rücken könnte.") });
    return null;
  }
  return moveTo(task, task.project ?? null, above.task.id);
}

// The open sibling right above a task, the one `>` puts it under.
export function siblingAbove(status) {
  const task = status.task;
  return state.statuses
    .filter(
      (s) =>
        s.task.id !== task.id &&
        isOpen(s.task) &&
        (s.task.project ?? null) === (task.project ?? null) &&
        (s.task.parent ?? null) === (task.parent ?? null) &&
        s.task.position < task.position,
    )
    .sort((a, b) => b.task.position - a.task.position)[0] ?? null;
}

export function outdent(status) {
  const task = status.task;
  if (!task.parent) {
    update({ message: tr("Die Aufgabe steht schon ganz oben.") });
    return null;
  }
  const parent = statusOf(task.parent)?.task;
  return moveTo(task, task.project ?? null, parent?.parent ?? null);
}

// Every task below `id`, also the closed ones and those the scope hides.
export function descendants(id) {
  const out = [];
  const ids = new Set([id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const t of state.tasks) {
      if (t.parent && ids.has(t.parent) && !ids.has(t.id)) {
        ids.add(t.id);
        out.push(t);
        grew = true;
      }
    }
  }
  return out;
}

// Ids of `id` and everything below it: no target may be among them.
function family(id) {
  const out = new Set([id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const s of state.statuses) {
      if (s.task.parent && out.has(s.task.parent) && !out.has(s.task.id)) {
        out.add(s.task.id);
        grew = true;
      }
    }
  }
  return out;
}

// The folders as targets for a project or folder, each with its path,
// leaving out a folder's own subtree.
function placeTargets(id, kind) {
  const own = kind === "folder" ? subtree(state.folders, id) : new Set();
  const out = [{ kind: "top", folder: null, title: tr("Oben") }];
  const walk = (parent, path, seen) => {
    for (const f of state.folders.filter((x) => (x.parent ?? null) === parent).sort(byPosition)) {
      if (seen.has(f.id) || own.has(f.id)) continue;
      seen.add(f.id);
      const title = path ? `${path} › ${f.title}` : f.title;
      out.push({ kind: "folder", folder: f.id, title });
      walk(f.id, title, seen);
    }
  };
  walk(null, "", new Set());
  return out;
}

const placeObject = (p) => byId(p.kind === "folder" ? state.folders : state.projects, p.id);

function targets(taskIds) {
  const own = new Set(taskIds.flatMap((id) => [...family(id)]));
  const projects = state.projects
    .filter((p) => p.status !== "dropped" && p.status !== "completed")
    .sort((a, b) => a.position - b.position)
    .map((p) => ({ kind: "project", project: p.id, parent: null, title: withScope(p.title, p.id) }));
  const groups = state.statuses
    .filter((s) => s.group && isOpen(s.task) && !own.has(s.task.id))
    .map((s) => {
      const project = s.task.project ?? null;
      const where = project ? byId(state.projects, project)?.title ?? "?" : "Inbox";
      return { kind: "group", project, parent: s.task.id, title: `${where} › ${s.task.title}` };
    });
  return [INBOX, ...projects, ...groups];
}

// The `blocked_by` links of a task with the task each names, as
// samu-core's `blockers::of`.
export function blockersOf(owner) {
  return state.links
    .filter((l) => l.owner === owner && l.kind === "blocked_by" && l.uri.startsWith("samu:"))
    .map((link) => ({ link, target: link.uri.slice("samu:".length) }));
}

// Whether `target` may block `owner`, as samu-core's `blockers::check`:
// not itself, not twice, and not one that waits for `owner` already.
export function blockerAllowed(owner, target) {
  if (owner === target || !byId(state.tasks, target)) return false;
  if (blockersOf(owner).some((b) => b.target === target)) return false;
  const seen = new Set();
  const todo = [target];
  while (todo.length) {
    const at = todo.pop();
    if (at === owner) return false;
    if (seen.has(at)) continue;
    seen.add(at);
    for (const b of blockersOf(at)) todo.push(b.target);
  }
  return true;
}

function blockTargets(owner) {
  return state.statuses
    .filter((s) => isOpen(s.task) && blockerAllowed(owner, s.task.id))
    .map((s) => ({ kind: "task", task: s.task.id, title: `${placeOf(s.task)} › ${s.task.title}` }));
}

// Makes `target` a blocker of `owner`.
export async function addBlocker(owner, target) {
  const now = new Date().toISOString();
  const link = { id: uuidv7(), created: now, modified: now, owner, uri: `samu:${target}`, kind: "blocked_by" };
  try {
    await api.write([change.create("link", link)]);
    update({ message: null });
  } catch (error) {
    update({ message: error.message });
  }
}

function matches() {
  const p = state.palette;
  if (!p) return [];
  const all =
    p.mode === "go"
      ? goTargets()
      : p.mode === "place"
        ? placeTargets(p.id, p.kind)
        : p.mode === "block"
          ? blockTargets(p.tasks[0])
          : targets(p.tasks);
  return rank(p.query, all);
}

// What `o` jumps to (db3fe005): views, perspectives, projects, folders,
// tags, the settings.
function goTargets() {
  return jumpTargets({
    views: VIEWS,
    perspectives: state.perspectives,
    projects: state.projectStatuses.map((s) => s.project),
    folders: state.folders,
    shownFolders: state.shownFolders,
    tags: state.tags,
    settings: tr("Einstellungen"),
    note: (title, project) => withScope(title, project.id),
  });
}

function jump(target) {
  if (target.kind === "view") {
    if (target.view === "time") reportTab("time");
    chooseView({ kind: target.view, project: null });
  } else if (target.kind === "settings") openSettings();
  else openObject(target.id);
}

// For one task, or with `statuses` for many (ADR 0052 §4).
export function openPalette(status, statuses = [status]) {
  update({ palette: { mode: "task", tasks: statuses.map((s) => s.task.id), query: "", index: 0 } });
  const input = $("palette-input");
  input.value = "";
  input.focus();
}

// For jumping to a view, project, folder, tag or perspective (db3fe005).
export function openGoPalette() {
  update({ palette: { mode: "go", query: "", index: 0 } });
  const input = $("palette-input");
  input.value = "";
  input.focus();
}

// For a project or folder of the sidebar (ADR 0041 §5).
export function openPlacePalette(id, kind) {
  update({ palette: { mode: "place", id, kind, query: "", index: 0 } });
  const input = $("palette-input");
  input.value = "";
  input.focus();
}

// For the blockers of one task (ae5a3206).
export function openBlockPalette(status) {
  update({ palette: { mode: "block", tasks: [status.task.id], query: "", index: 0 } });
  const input = $("palette-input");
  input.value = "";
  input.focus();
}

function close() {
  update({ palette: null });
  $("palette-input").blur();
}

function choose(asChild) {
  const p = state.palette;
  const target = matches()[p.index];
  if (p.mode === "go") {
    close();
    if (target) jump(target);
    return;
  }
  if (p.mode === "place") {
    const object = placeObject(p);
    close();
    if (target && object) movePlace(object, p.kind, target.folder);
    return;
  }
  if (p.mode === "block") {
    close();
    if (target) addBlocker(p.tasks[0], target.task);
    return;
  }
  const tasks = p.tasks.map((id) => statusOf(id)?.task).filter(Boolean);
  close();
  if (!target || !tasks.length) return;
  if (target.kind === "group") {
    const group = statusOf(target.parent)?.task;
    if (asChild) moveMany(tasks, target.project, target.parent);
    else moveMany(tasks, target.project, group?.parent ?? null);
  } else moveMany(tasks, target.project, null);
}

export function wirePalette() {
  const input = $("palette-input");
  input.addEventListener("input", () => update({ palette: { ...state.palette, query: input.value, index: 0 } }));
  input.addEventListener("keydown", (event) => {
    const p = state.palette;
    if (!p) return;
    const step = (n) => {
      const count = matches().length;
      if (count) update({ palette: { ...p, index: (p.index + n + count) % count } });
    };
    const key = event.ctrlKey ? `C-${event.key}` : event.key;
    switch (key) {
      case "ArrowDown":
      case "C-n":
        step(1);
        break;
      case "ArrowUp":
      case "C-p":
        step(-1);
        break;
      case "Enter":
        choose(false);
        break;
      case "Tab":
        choose(true);
        break;
      case "Escape":
        close();
        break;
      default:
        return;
    }
    event.preventDefault();
    event.stopPropagation();
  });
  $("palette").addEventListener("click", (event) => {
    if (event.target.id === "palette") close();
  });
}

const KINDS = () => ({
  group: tr("Gruppe"),
  project: tr("Projekt"),
  folder: tr("Ordner"),
  task: tr("Aufgabe"),
  view: tr("Ansicht"),
  perspective: tr("Perspektive"),
  tag: "Tag",
  settings: "",
});

export function renderPalette() {
  const p = state.palette;
  $("palette").hidden = !p;
  if (!p) return;
  const go = p.mode === "go";
  const place = p.mode === "place";
  const block = p.mode === "block";
  if (go) {
    replace($("palette-task"), tr("Springen zu …"));
  } else {
    const subject = place ? placeObject(p) : p.tasks.length === 1 ? statusOf(p.tasks[0])?.task : null;
    const what = subject ? tr("„{title}“", { title: subject.title }) : place ? "" : tr("{n} Aufgaben", { n: p.tasks.length });
    const verb = place ? tr("verschieben") : block ? tr("wartet auf …") : tr("umhängen");
    replace($("palette-task"), what ? `${what} ${verb}` : "");
  }
  replace($("palette-hint"), go ? tr("↵ öffnen · Esc abbrechen") : tr("↵ verschieben · ⇥ als Kind · Esc abbrechen"));
  $("palette-input").placeholder = go
    ? tr("Ansicht, Projekt, Ordner, Tag")
    : place
      ? tr("Ordner")
      : block
        ? tr("Aufgabe")
        : tr("Projekt oder Gruppe");
  const found = matches();
  replace(
    $("palette-list"),
    found.length
      ? found.map((target, i) =>
          el(
            "li",
            {
              class: i === p.index ? "current" : null,
              onmousedown: (event) => {
                event.preventDefault();
                update({ palette: { ...p, index: i } });
                choose(false);
              },
            },
            [
              el("span", { text: target.title }),
              el("span", { class: "count", text: KINDS()[target.kind] ?? "" }),
            ],
          ),
        )
      : el("li", { class: "empty", text: go ? tr("Nichts gefunden.") : block ? tr("Keine Aufgabe, die blockieren könnte.") : tr("Kein Ziel.") }),
  );
  $("palette-list").querySelector(".current")?.scrollIntoView({ block: "nearest" });
}
