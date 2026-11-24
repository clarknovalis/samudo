// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The dialog „Blöcke …“ of a block perspective (ADR 0116 §6, ADR 0143,
// ADR 0146 §4): its blocks in order, each another perspective, a fixed
// view, a search of its own or the note of a project or task, with an
// optional heading. As a list: adding, dragging or
// ↑ ↓ to order, removing, a search's filter, arrangement and view,
// „Lösen“ (a reference becomes a search of its own) and „Als Perspektive
// sichern“ (a search becomes a perspective). As text: one line per block
// (blocklines.js) with a preview of what each becomes. And the switch
// „Doppelte nur einmal“. Saving writes `blocks`, `dedupe` and the
// perspectives made from searches, in one write.

import { api, change } from "./api.js";
import { el, replace } from "./dom.js";
import { tr } from "./i18n.js";
import { assemble, isBlocks, knownView, VIEWS } from "./blocks.js";
import { detached, fromLines, lineOf, parseLine, queryWords } from "./blocklines.js";
import { GROUPS, SORTS } from "./list.js";
import { perspectiveOf, perspectiveShows, perspectives, perspectiveValue } from "./perspectives.js";
import { noteSource, state, update } from "./state.js";

const VIEW_TITLES = { inbox: () => "Inbox", today: () => tr("Heute"), flagged: () => tr("Markiert") };
const viewTitle = (view) => VIEW_TITLES[view]?.() ?? view;
const FILTERS = [
  ["available", () => tr("Verfügbar")],
  ["remaining", () => tr("Verbleibend")],
  ["all", () => tr("Alle")],
];
const DISPLAYS = [
  ["list", () => tr("Liste")],
  ["table", () => tr("Tabelle")],
];

// Whether another perspective shows `p` as a block: then it gets none.
export const isBlockOfAnother = (p) => state.perspectives.some((o) => o.id !== p.id && (o.blocks ?? []).some((b) => b.perspective === p.id));

// `extract`: the index of a block whose search is to become a perspective
// right away, from the menu of its section (ADR 0143 §4).
export function openBlocks(p, { extract = null } = {}) {
  if (isBlockOfAnother(p)) {
    update({ message: tr("„{title}“ ist Block einer anderen Perspektive und bekommt keine Blöcke.", { title: p.title }) });
    return;
  }
  // A copy to change; only „Sichern“ writes it. Perspectives made from
  // searches wait in `made` until then.
  let blocks = (p.blocks ?? []).map((b) => structuredClone(b));
  const made = [];
  // Searches whose words name nothing, by block, until they are fixed.
  const errors = new Map();
  let asText = false;
  let naming = extract;
  let dragged = null;
  const known = (id) => perspectiveOf(id) ?? made.find((m) => m.id === id) ?? null;
  const all = () => [...perspectives(), ...made];
  const dedupe = el("input", { type: "checkbox", id: "blocks-dedupe" });
  dedupe.checked = p.dedupe !== false;
  const list = el("ul", { class: "block-list" });
  const choice = el("select", { "aria-label": tr("Block hinzufügen") });
  const text = el("textarea", {
    class: "block-lines",
    rows: "8",
    spellcheck: "false",
    "aria-label": tr("Blöcke als Text, eine Zeile je Block"),
    oninput: () => drawPreview(),
  });
  const preview = el("ul", { class: "block-preview" });
  const help = el("p", {
    class: "hint",
    text: tr("Eine Zeile je Block: „:today“ ist die Ansicht, „:note #Projekt“ die Notiz des Projekts, der Name einer Perspektive verweist auf sie, alles andere ist eine eigene Suche. Nach „ | “ steht die Überschrift."),
  });
  // What the text does not hold (§3).
  const kept = el("p", {
    class: "hint",
    text: tr("Filter, Gruppieren, Sortieren und Darstellung einer Suche stellst du in der Listenansicht ein; sie bleiben erhalten, solange die Suche gleich bleibt."),
  });
  const toggle = el("button", { type: "button", onclick: () => switchMode() });
  const close = () => {
    document.removeEventListener("keydown", onKey);
    overlay.remove();
  };
  const onKey = (event) => {
    if (event.key === "Escape") close();
  };

  // How many tasks a block shows now, on its own.
  const statuses = () => state.statuses.filter((s) => !s.outside);
  const shows = perspectiveShows;
  const countOf = (block, i) => {
    const one = { id: `${p.id}:preview:${i}`, blocks: [block], dedupe: true };
    const [section] = assemble(one, all(), statuses(), shows);
    return section?.missing ? null : section?.tasks.length ?? 0;
  };

  // The name a block shows when it has no heading of its own.
  function nameOf(block) {
    if (block.note) {
      const found = noteSource(block.note);
      return found ? [found.icon, found.title].filter(Boolean).join(" ") : tr("fehlt");
    }
    if (block.view) return knownView(block.view) ? viewTitle(block.view) : `${block.view} (${tr("fehlt")})`;
    if (block.query) return queryWords(block.query) || tr("Alle Aufgaben");
    const target = known(block.perspective);
    if (!target) return tr("fehlt");
    return [target.icon, target.title].filter(Boolean).join(" ");
  }

  const move = (from, to) => {
    if (to < 0 || to >= blocks.length || from === to) return;
    const [moved] = blocks.splice(from, 1);
    blocks.splice(to, 0, moved);
    draw();
  };

  // A select over `options` [[value, label]] that calls `set(value)`,
  // under its name: „State“ alone does not say it groups.
  const select = (label, options, value, set) =>
    el("label", { class: "field" }, [
      el("span", { text: label }),
      el(
        "select",
        { onchange: (event) => set(event.target.value) },
        options.map(([v, text]) => el("option", { value: v, text: typeof text === "function" ? text() : text, selected: v === value })),
      ),
    ]);

  // The fields of a block's own search in the list: the search as words,
  // the filter, grouping, sorting and list or table.
  function queryFields(block) {
    const query = block.query;
    const arrange = query.arrange ?? {};
    const setArrange = (patch) => {
      const next = { group: arrange.group ?? "project", sort: arrange.sort ?? "position", ...patch };
      if (next.group === "project" && next.sort === "position") delete query.arrange;
      else query.arrange = next;
      draw();
    };
    const wrong = errors.get(block);
    return el("div", { class: "block-query" }, [
      el("label", { class: "field search" }, [
        el("span", { text: tr("Suche") }),
        el("input", {
          type: "text",
          value: wrong?.text ?? queryWords(query),
          placeholder: tr("Suche, wie im Suchfeld"),
          "aria-invalid": wrong ? "true" : null,
          onchange: (event) => {
            const text = event.target.value;
            const parsed = parseLine(text.replaceAll(" | ", " "), []);
            // A word that names nothing keeps what was typed and blocks
            // saving until it is fixed (ADR 0143 §3, Changes).
            if (parsed.kind === "error") errors.set(block, { text, error: parsed.error });
            else {
              errors.delete(block);
              const { chips, text: words } = parsed.block.query;
              if (chips) query.chips = chips;
              else delete query.chips;
              if (words) query.text = words;
              else delete query.text;
            }
            draw();
          },
        }),
      ]),
      select(tr("Filter"), FILTERS, query.filter ?? "available", (v) => {
        query.filter = v;
        draw();
      }),
      select(tr("Gruppieren"), GROUPS, arrange.group ?? "project", (v) => setArrange({ group: v })),
      select(tr("Sortieren"), SORTS, arrange.sort ?? "position", (v) => setArrange({ sort: v })),
      select(tr("Darstellung"), DISPLAYS, query.view === "table" ? "table" : "list", (v) => {
        query.view = v;
      }),
      wrong ? el("p", { class: "error", text: wrong.error }) : null,
    ]);
  }

  // A name for the perspective a search becomes, in place of its row.
  function nameRow(block, i) {
    const input = el("input", { type: "text", placeholder: tr("Name der neuen Perspektive"), "aria-label": tr("Name der neuen Perspektive") });
    const stop = () => {
      naming = null;
      draw();
    };
    const done = () => {
      const title = input.value.trim();
      if (!title) return;
      if (all().some((o) => o.title.toLowerCase() === title.toLowerCase())) {
        update({ message: tr("Es gibt schon eine Perspektive „{title}“.", { title }) });
        return;
      }
      const value = perspectiveValue(title, block.query, made.length);
      made.push(value);
      const heading = block.title ?? (queryWords(block.query) || tr("Alle Aufgaben"));
      blocks[i] = { perspective: value.id, ...(heading !== title ? { title: heading } : {}) };
      stop();
    };
    input.addEventListener("keydown", (event) => {
      if (event.key === "Enter") done();
      if (event.key === "Escape") {
        event.stopPropagation();
        stop();
      }
    });
    queueMicrotask(() => input.focus());
    return el("li", { class: "block-naming" }, [
      input,
      el("button", { type: "button", class: "primary", text: tr("Anlegen"), onclick: done }),
      el("button", { type: "button", text: tr("Abbrechen"), onclick: stop }),
    ]);
  }

  function row(block, i) {
    if (naming === i && block.query) return nameRow(block, i);
    const target = block.perspective ? known(block.perspective) : null;
    const count = countOf(block, i);
    return el(
      "li",
      {
        draggable: "true",
        class: block.query ? "block-row query" : "block-row",
        ondragstart: (event) => {
          dragged = i;
          event.dataTransfer.effectAllowed = "move";
          event.dataTransfer.setData("text/plain", String(i));
        },
        ondragover: (event) => {
          if (dragged === null) return;
          event.preventDefault();
        },
        ondrop: (event) => {
          event.preventDefault();
          if (dragged !== null) move(dragged, i);
          dragged = null;
        },
        ondragend: () => {
          dragged = null;
        },
      },
      [
        // A search of its own shows its search first, the rest below it.
        block.query ? queryFields(block) : null,
        el("span", { class: "grip", "aria-hidden": "true", text: "⠿" }),
        block.query ? null : el("span", { class: "block-name", text: nameOf(block) }),
        el("span", { class: "count", title: tr("Treffer jetzt"), text: count === null ? "–" : String(count) }),
        el("input", {
          type: "text",
          value: block.title ?? "",
          // Empty, the block is called by what it shows: say so.
          placeholder: nameOf(block),
          title: tr("Eigene Überschrift; leer: „{name}“", { name: nameOf(block) }),
          "aria-label": tr("Überschrift von {name}", { name: nameOf(block) }),
          oninput: (event) => {
            const title = event.target.value.trim();
            if (title) block.title = title;
            else delete block.title;
          },
        }),
        target && !isBlocks(target)
          ? el("button", {
              type: "button",
              text: tr("Lösen"),
              title: tr("Eigene Suche aus einer Kopie von „{title}“", { title: target.title }),
              onclick: () => {
                blocks[i] = detached(block, target);
                draw();
              },
            })
          : null,
        block.query
          ? el("button", {
              type: "button",
              text: tr("Als Perspektive sichern …"),
              onclick: () => {
                naming = i;
                draw();
              },
            })
          : null,
        el("button", { type: "button", text: "↑", "aria-label": tr("nach oben"), disabled: i === 0, onclick: () => move(i, i - 1) }),
        el("button", { type: "button", text: "↓", "aria-label": tr("nach unten"), disabled: i === blocks.length - 1, onclick: () => move(i, i + 1) }),
        el("button", {
          type: "button",
          text: "✕",
          "aria-label": tr("{name} entfernen", { name: nameOf(block) }),
          onclick: () => {
            blocks.splice(i, 1);
            draw();
          },
        }),
      ],
    );
  }

  // The preview below the text: what each line becomes and its hits now.
  function drawPreview() {
    const { rows } = fromLines(text.value, blocks, all(), p.id);
    const kinds = { view: tr("Ansicht"), note: tr("Notiz"), perspective: tr("Perspektive"), query: tr("Suche") };
    replace(
      preview,
      rows.length
        ? rows.map((r, i) => {
            if (r.kind === "error") return el("li", { class: "error", text: `${r.line.trim()} — ${r.error}` });
            const count = countOf(r.block, i);
            return el("li", { class: r.kind }, [
              el("span", { class: "kind", text: kinds[r.kind] }),
              el("span", { class: "block-name", text: r.block.title ?? nameOf(r.block) }),
              el("span", { class: "count", text: count === null ? "–" : String(count) }),
            ]);
          })
        : [el("li", { class: "quiet", text: tr("Noch kein Block: ohne Blöcke ist es eine einfache Perspektive.") })],
    );
  }

  // The lines read as blocks, or null with a message when one is none.
  function read() {
    const { blocks: lines } = fromLines(text.value, blocks, all(), p.id);
    if (!lines) update({ message: tr("Eine Zeile ist kein Block; siehe die Vorschau.") });
    return lines;
  }

  // From the list to the text and back; back only when every line holds.
  function switchMode() {
    if (asText) {
      const lines = read();
      if (!lines) return;
      blocks = lines;
      asText = false;
    } else {
      if ([...errors.keys()].some((b) => blocks.includes(b))) {
        update({ message: tr("Eine Suche nennt etwas, das es nicht gibt; siehe den Block.") });
        return;
      }
      text.value = blocks.map((b) => lineOf(b, known)).join("\n");
      asText = true;
    }
    draw();
    (asText ? text : choice).focus();
  }

  function draw() {
    toggle.textContent = asText ? tr("Als Liste") : tr("Als Text");
    for (const [node, shown] of [
      [list, !asText],
      [choice, !asText],
      [text, asText],
      [preview, asText],
      [help, asText],
      [kept, asText],
    ]) {
      node.hidden = !shown;
    }
    if (asText) {
      drawPreview();
      return;
    }
    replace(
      list,
      blocks.length
        ? blocks.map((block, i) => row(block, i))
        : [el("li", { class: "quiet", text: tr("Noch kein Block: ohne Blöcke ist es eine einfache Perspektive.") })],
    );
    // What can be added: a search of its own, the fixed views, and every
    // plain perspective but this one (§2).
    const plain = all().filter((o) => o.id !== p.id && !isBlocks(o));
    // A task's note is added by its line (ADR 0146 §4).
    const projects = state.projects
      .filter((o) => o.status === "active" || o.status === "on_hold")
      .sort((a, b) => a.title.localeCompare(b.title));
    replace(choice, [
      el("option", { value: "", text: tr("Block hinzufügen …") }),
      el("option", { value: "query:", text: tr("Eigene Suche") }),
      el("optgroup", { label: tr("Ansichten") }, VIEWS.map((v) => el("option", { value: `view:${v}`, text: viewTitle(v) }))),
      plain.length
        ? el("optgroup", { label: tr("Perspektiven") }, plain.map((o) => el("option", { value: `perspective:${o.id}`, text: [o.icon, o.title].filter(Boolean).join(" ") })))
        : null,
      projects.length
        ? el("optgroup", { label: tr("Notiz eines Projekts") }, projects.map((o) => el("option", { value: `note:${o.id}`, text: [o.icon, o.title].filter(Boolean).join(" ") })))
        : null,
    ]);
  }

  choice.addEventListener("change", () => {
    const [kind, value] = choice.value.split(/:(.*)/);
    if (kind === "view") blocks.push({ view: value });
    if (kind === "perspective") blocks.push({ perspective: value });
    if (kind === "query") blocks.push({ query: {} });
    if (kind === "note") blocks.push({ note: value });
    draw();
    if (kind === "query") list.querySelector("li:last-child .block-query input")?.focus();
    else choice.focus();
  });

  const save = async () => {
    if (!asText && [...errors.keys()].some((b) => blocks.includes(b))) {
      update({ message: tr("Eine Suche nennt etwas, das es nicht gibt; siehe den Block.") });
      return;
    }
    if (asText) {
      const lines = read();
      if (!lines) return;
      blocks = lines;
    }
    // Only the perspectives a block still points at are made.
    const making = made.filter((m) => blocks.some((b) => b.perspective === m.id));
    const changes = making.map((m) => change.create("perspective", m));
    if (JSON.stringify(blocks) !== JSON.stringify(p.blocks ?? [])) changes.push(change.set(p.id, "blocks", blocks));
    if (dedupe.checked !== (p.dedupe !== false)) changes.push(change.set(p.id, "dedupe", dedupe.checked));
    if (changes.length) {
      try {
        await api.write(changes);
      } catch (error) {
        update({ message: error.message });
        return;
      }
      for (const m of making) if (!perspectiveOf(m.id)) state.perspectives = [...state.perspectives, m];
      Object.assign(p, { blocks, dedupe: dedupe.checked });
    }
    close();
    // What was edited is seen: the block perspective, with an empty
    // search field. A search hides the blocks even with the perspective
    // open, since the field there searches everything (ADR 0116 §6).
    if (isBlocks(p)) {
      const field = document.getElementById("search-input");
      if (field) field.value = "";
      update({ view: { kind: "perspective", perspective: p.id }, search: "", searchChips: [], selected: null });
    } else update({});
  };

  const box = el("div", { class: "box blocks", role: "dialog", "aria-modal": "true", "aria-label": tr("Blöcke") }, [
    el("div", { class: "blocks-head" }, [el("h2", { text: tr("Blöcke von „{title}“", { title: p.title }) }), toggle]),
    list,
    choice,
    help,
    text,
    preview,
    kept,
    el("label", { class: "check", for: "blocks-dedupe" }, [dedupe, el("span", { text: tr("Doppelte nur einmal: eine Aufgabe nur im ersten Block, der sie zeigt") })]),
    el("div", { class: "buttons" }, [
      el("button", { type: "button", text: tr("Abbrechen"), onclick: close }),
      el("button", { type: "button", class: "primary", text: tr("Sichern"), onclick: save }),
    ]),
  ]);
  const overlay = el("div", { class: "overlay" }, [box]);
  overlay.addEventListener("click", (event) => {
    if (event.target === overlay) close();
  });
  draw();
  // The menu that opened it closes.
  update({});
  document.addEventListener("keydown", onKey);
  document.body.append(overlay);
  if (naming === null) choice.focus();
}
