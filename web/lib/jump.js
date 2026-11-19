// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Jumping (db3fe005): the targets of the palette's `go` mode and the
// fuzzy match over them. Pure on what it is given, so the node test can
// call it; palette.js passes the state in.

import { pathOf } from "./words.js";

const byPosition = (a, b) => a.position - b.position || a.title.localeCompare(b.title);

// A score when every character of `query` occurs in `text` in order, else
// null; lower is better. A hit at a word start counts more than one inside.
export function fuzzy(query, text) {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  let score = 0;
  let at = 0;
  for (const ch of q) {
    if (ch === " ") continue;
    const found = t.indexOf(ch, at);
    if (found < 0) return null;
    const start = found === 0 || /[\s›·\-_/]/.test(t[found - 1]);
    score += (found - at) + (start ? 0 : 2);
    at = found + 1;
  }
  return score;
}

// `targets` filtered and ordered by `query`; all of them, as given, when
// it is empty.
export function rank(query, targets) {
  if (!query.trim()) return targets;
  return targets
    .map((target) => ({ target, score: fuzzy(query, target.title) }))
    .filter((m) => m.score !== null)
    .sort((a, b) => a.score - b.score)
    .map((m) => m.target);
}

// What `o` offers, in this order: the fixed views, the perspectives, the
// projects and folders the scope shows (`projects`, `shownFolders`) with
// their folder path, every tag with its path, and the settings. `note`
// adds to a project's title (where another scope alone shows it).
export function jumpTargets({ views, perspectives, projects, folders, shownFolders, tags, settings, note = (t) => t }) {
  const folderPath = (id) => {
    const folder = folders.find((f) => f.id === id);
    return folder ? pathOf(folders, folder) : null;
  };
  const shown = new Set(shownFolders);
  return [
    ...views.map((v) => ({ kind: "view", view: v.kind, title: v.title })),
    ...[...perspectives].sort(byPosition).map((p) => ({ kind: "perspective", id: p.id, title: p.title })),
    ...[...projects].sort(byPosition).map((p) => {
      const path = p.folder ? folderPath(p.folder) : null;
      return { kind: "project", id: p.id, title: note(path ? `${path}/${p.title}` : p.title, p) };
    }),
    ...folders
      .filter((f) => shown.has(f.id))
      .map((f) => ({ kind: "folder", id: f.id, title: pathOf(folders, f) }))
      .sort((a, b) => a.title.localeCompare(b.title)),
    ...tags.map((t) => ({ kind: "tag", id: t.id, title: `@${pathOf(tags, t)}` })).sort((a, b) => a.title.localeCompare(b.title)),
    { kind: "settings", title: settings },
  ];
}
