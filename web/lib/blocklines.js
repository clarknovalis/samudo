// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The blocks of a block perspective as text, one line per block, like
// org-super-agenda (ADR 0143 §3): `:today` is the fixed view, `:note
// #Project` or `:note <id>` the note of a project or task (ADR 0146 §4),
// the exact
// name of a plain perspective a reference to it, any other line a search
// of the block's own; ` | ` starts the heading. A word that looks like a
// chip but names nothing (`@Nirgends`) is refused, as the CLI refuses it.
// The CLI reads the same lines (`samu perspective blocks`). No DOM here,
// so node can test it.

import { knownView } from "./blocks.js";
import { tr } from "./i18n.js";
import { chipText, missedChip, searchFrom, tidy, withGroups, withOrs, WORD } from "./query.js";
import { byId, state } from "./state.js";

const norm = (text) => (text ?? "").trim().replace(/\s+/g, " ");
const same = (a, b) => JSON.stringify(a ?? []) === JSON.stringify(b ?? []);

// A block's own search as it reads in the search field.
export const queryWords = (query, l) =>
  [...tidy(withOrs(query?.chips ?? [])).map((chip) => chipText(chip, l)), norm(query?.text)].filter(Boolean).join(" ");

// The project or task a note block names, as its line writes it: a
// project as the search writes it, a task by the end of its id.
const noteWords = (id, l) => (byId(state.projects, id) ? chipText({ kind: "project", value: id }, l) : id.slice(-8));

// The id a `:note` line names (ADR 0146 §4): `#Project` as the search
// reads it, or a project's or task's full id, `samu:` address, or the
// start or end of one. { id } or { error }.
function noteTarget(target) {
  if (!target) return { error: tr("„:note“ nennt kein Projekt und keine Aufgabe.") };
  if (target.startsWith("#")) {
    const { searchChips, search } = searchFrom(target);
    const [chip] = searchChips;
    if (searchChips.length === 1 && !search.trim() && chip.kind === "project" && !chip.not && chip.value !== "inbox") return { id: chip.value };
    return { error: tr("Kein Projekt „{name}“.", { name: target.slice(1).replace(/^"(.*)"$/, "$1") }) };
  }
  const wanted = target.toLowerCase().replace(/^samu:/, "");
  const ids = [...state.projects, ...state.tasks].map((o) => o.id).filter((id) => id === wanted || id.endsWith(wanted) || id.startsWith(wanted));
  if (ids.length === 1) return { id: ids[0] };
  if (!ids.length) return { error: tr("Kein Projekt und keine Aufgabe „{id}“.", { id: target }) };
  return { error: tr("„{id}“ passt auf mehrere: {ids}", { id: target, ids: ids.map((id) => id.slice(-8)).join(", ") }) };
}

// A block as its line; `perspectiveOf(id)` finds a referenced one.
export function lineOf(block, perspectiveOf, l) {
  const body = block.note
    ? `:note ${noteWords(block.note, l)}`
    : block.view
    ? `:${block.view}`
    : block.perspective
      ? perspectiveOf(block.perspective)?.title ?? block.perspective
      : block.query
        ? queryWords(block.query, l)
        : "";
  return block.title ? `${body} | ${block.title}` : body;
}

// One line as { kind, block } or { kind: "error", error }: kind is
// "view", "note", "perspective" or "query". `self` is the block perspective's id,
// which is no block of itself.
export function parseLine(line, perspectives, self = null) {
  let body = line;
  let title = null;
  const at = line.lastIndexOf(" | ");
  if (at >= 0) {
    body = line.slice(0, at);
    title = line.slice(at + 3).trim();
    if (!title) return { kind: "error", error: tr("Die Überschrift nach | ist leer.") };
  } else if (/\s\|\s*$/.test(line)) {
    return { kind: "error", error: tr("Die Überschrift nach | ist leer.") };
  }
  body = body.trim();
  const titled = (block) => (title ? { ...block, title } : block);
  const note = /^:note(?:\s+(.*))?$/s.exec(body);
  if (note) {
    const found = noteTarget((note[1] ?? "").trim());
    return found.error ? { kind: "error", error: found.error } : { kind: "note", block: titled({ note: found.id }) };
  }
  if (body.startsWith(":")) {
    const view = body.slice(1);
    if (!knownView(view)) return { kind: "error", error: tr("„{view}“ ist keine Ansicht für einen Block.", { view }) };
    return { kind: "view", block: titled({ view }) };
  }
  const name = body.replace(/^"(.*)"$/, "$1").toLowerCase();
  const found = perspectives.find((p) => p.title.toLowerCase() === name);
  if (found) {
    if (found.id === self) return { kind: "error", error: tr("Eine Perspektive ist kein Block von sich selbst.") };
    if (found.blocks?.length) return { kind: "error", error: tr("„{title}“ hat selbst Blöcke und ist kein Block.", { title: found.title }) };
    return { kind: "perspective", block: titled({ perspective: found.id }) };
  }
  // A word that looks like a chip but names nothing is refused, not
  // searched as text: a block would find nothing and not say why.
  for (const [word] of body.matchAll(WORD)) {
    const missed = missedChip(word);
    if (!missed) continue;
    const error =
      missed.kind === "tag"
        ? tr("Kein Tag „{name}“.", { name: missed.name })
        : missed.kind === "project"
          ? tr("Kein Projekt „{name}“.", { name: missed.name })
          : tr("„{word}“ ist kein gültiges Suchwort.", { word: missed.name });
    return { kind: "error", error };
  }
  const { searchChips, search } = searchFrom(body);
  const chips = withGroups(searchChips);
  const text = norm(search);
  const query = { ...(chips.length ? { chips } : {}), ...(text ? { text } : {}) };
  return { kind: "query", block: titled({ query }) };
}

// The lines of `text` as blocks; blank lines do not count. A search that
// is the same as one of `previous` keeps that block's filter, arrangement
// and view (§3): the text sets only the search. { blocks, rows } with a
// row per line that counts: { line, kind, block, error }; blocks only
// when no row is an error, else null.
export function fromLines(text, previous, perspectives, self = null) {
  const rows = text
    .split("\n")
    .filter((line) => line.trim())
    .map((line) => ({ line, ...parseLine(line, perspectives, self) }));
  for (const row of rows) {
    if (row.kind !== "query") continue;
    const { chips, text: words } = row.block.query;
    const kept = previous.find((b) => b.query && same(b.query.chips, chips) && norm(b.query.text) === (words ?? ""));
    if (kept) row.block = { ...row.block, query: { ...kept.query, ...row.block.query } };
  }
  const blocks = rows.some((r) => r.kind === "error") ? null : rows.map((r) => r.block);
  return { blocks, rows };
}

// A reference turned into a block of its own with a copy of the
// perspective's search, under the same heading (§4).
export function detached(block, perspective) {
  const query = {};
  for (const field of ["text", "chips", "filter", "arrange", "view", "board", "table"]) {
    if (perspective[field] !== undefined && perspective[field] !== null) query[field] = perspective[field];
  }
  return { query, title: block.title ?? perspective.title };
}
