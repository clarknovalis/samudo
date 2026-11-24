// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Tasks out of the web UI (ADR 0145): one task or the picks, in one of the
// forms of exporttext.js, to the clipboard or into a file. The menus and
// the bar above the picks take their entries from here.

import { $, el } from "./dom.js";
import { ENDING, FORMS, fileName, render } from "./exporttext.js";
import { tr } from "./i18n.js";
import { info } from "./message.js";
import { byId, isOpen, state, update } from "./state.js";
import { plainCells } from "./table.js";
import { COLUMNS } from "./tabledata.js";
import { today } from "./words.js";

export const FORM_LABEL = {
  ids: tr("IDs"),
  text: tr("Text (Markdown)"),
  org: "Org",
  tsv: "TSV",
  json: "JSON",
};

// The statuses as the rows exporttext.js takes; the cells only for TSV.
function rowsOf(statuses, form) {
  const cells = form === "tsv" ? plainCells(statuses) : new Map();
  return statuses.map((s) => ({
    id: s.task.id,
    parent: s.task.parent ?? null,
    title: s.task.title,
    closed: !isOpen(s.task),
    tags: [...(s.task.tags ?? [])].sort((a, b) => a.position - b.position).map((t) => byId(state.tags, t.tag)?.title).filter(Boolean),
    cells: cells.get(s.task.id),
    object: s.task,
  }));
}

const textOf = (statuses, form) => render(form, rowsOf(statuses, form), COLUMNS);

export async function copyTasks(statuses, form) {
  if (!statuses.length) return;
  const text = textOf(statuses, form);
  try {
    await navigator.clipboard.writeText(text);
    update({ message: info(statuses.length === 1 ? tr("1 Aufgabe kopiert.") : tr("{n} Aufgaben kopiert.", { n: statuses.length })) });
  } catch {
    // No clipboard (no secure context): IDs fit the message line, the
    // longer forms go into a file instead.
    if (form === "ids") update({ message: info(text) });
    else
      update({
        message: info(tr("Keine Zwischenablage auf dieser Seite."), [[tr("Als Datei speichern"), () => exportTasks(statuses, form)]]),
      });
  }
}

export function exportTasks(statuses, form) {
  if (!statuses.length) return;
  const type = { json: "application/json", tsv: "text/tab-separated-values", text: "text/markdown", org: "text/org" }[form] ?? "text/plain";
  const url = URL.createObjectURL(new Blob([`${textOf(statuses, form)}\n`], { type: `${type};charset=utf-8` }));
  const name = fileName($("list-title")?.textContent, today(), form);
  const link = el("a", { href: url, download: name });
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url));
  update({ message: info(tr("Gespeichert als {name}.", { name })) });
}

// The submenus Kopieren and Exportieren of a context menu; `extra` comes
// first in Kopieren (the link of one task).
export function copyEntries(statuses, extra = []) {
  return [
    { label: tr("Kopieren"), sub: [...extra, ...FORMS.map((form) => ({ label: FORM_LABEL[form], run: () => copyTasks(statuses, form) }))] },
    {
      label: tr("Exportieren"),
      sub: FORMS.map((form) => ({ label: `${FORM_LABEL[form]} (.${ENDING[form]})`, run: () => exportTasks(statuses, form) })),
    },
  ];
}

// The choices of the bar: `copy <form>` and `export <form>` (bulk.js).
export function barChoices() {
  return [
    [tr("Kopieren"), FORMS.map((form) => [`copy ${form}`, FORM_LABEL[form]])],
    [tr("Exportieren"), FORMS.map((form) => [`export ${form}`, `${FORM_LABEL[form]} (.${ENDING[form]})`])],
  ];
}

export function runChoice(value, statuses) {
  const [how, form] = value.split(" ");
  if (how === "copy") copyTasks(statuses, form);
  else if (how === "export") exportTasks(statuses, form);
}
