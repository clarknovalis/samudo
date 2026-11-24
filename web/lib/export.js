// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Tasks out of the web UI (ADR 0145): one task or the picks, in one of the
// forms of exporttext.js, to the clipboard or into a file. The menus and
// the bar above the picks take their entries from here.

import { $, el } from "./dom.js";
import { ENDING, FORMS, fileName, outline, render, taskRows } from "./exporttext.js";
import { tr } from "./i18n.js";
import { info } from "./message.js";
import { api } from "./api.js";
import { perspectives } from "./perspectives.js";
import { byId, state, statusOf, update } from "./state.js";
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
  return taskRows(
    statuses.map((s) => s.task),
    (id) => byId(state.tags, id)?.title,
  ).map((row) => ({ ...row, cells: cells.get(row.id) }));
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
  const name = fileName($("list-title")?.textContent, today(), form);
  saveFile(name, `${textOf(statuses, form)}\n`, form);
}

const TYPES = { json: "application/json", tsv: "text/tab-separated-values", text: "text/markdown", org: "text/org" };

// A file to save, made here and handed to a download link (§3).
function saveFile(name, text, form) {
  const type = TYPES[form] ?? "text/plain";
  const url = URL.createObjectURL(new Blob([text], { type: `${type};charset=utf-8` }));
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

// --- The section Export of the settings (ADR 0145 §7) ----------------------

const ALL = "all";
// What the section has chosen, kept while the settings are open.
let choice = { source: "inbox", form: "org", closed: false, busy: false };

const plain = (p) => !p.blocks?.length;
const open = (p) => p.status !== "completed" && p.status !== "dropped";
const byTitle = (a, b) => a.title.localeCompare(b.title);

function sourceName(source) {
  if (source === "inbox") return "Inbox";
  if (source === ALL) return tr("Konto");
  const [kind, id] = source.split(":");
  return byId(kind === "project" ? state.projects : state.perspectives, id)?.title ?? kind;
}

// The tasks of a source as the list has them, in outline order.
async function statusesOf(source, closed) {
  const [kind, id] = source.split(":");
  let query = closed ? "&all=true" : "";
  if (kind === "project") query += `&project=${encodeURIComponent(id)}`;
  if (kind === "perspective") query += `&perspective=${encodeURIComponent(id)}`;
  const bare = await api.bareStatuses(query, state.scope);
  let statuses = bare.filter((b) => !b.outside).map((b) => statusOf(b.id)).filter(Boolean);
  if (kind === "inbox") statuses = statuses.filter((s) => !s.task.project || !byId(state.projects, s.task.project));
  const at = new Map(statuses.map((s) => [s.task.id, s]));
  return outline(statuses.map((s) => s.task)).map((t) => at.get(t.id));
}

async function download() {
  const { source, form, closed } = choice;
  choice = { ...choice, busy: true };
  update({});
  try {
    if (source === ALL) {
      const account = await api.exportAccount();
      saveFile(fileName(tr("Konto"), today(), "json"), `${JSON.stringify(account, null, 2)}\n`, "json");
    } else {
      const statuses = await statusesOf(source, closed);
      if (!statuses.length) update({ message: info(tr("Nichts zu exportieren.")) });
      else saveFile(fileName(sourceName(source), today(), form), `${textOf(statuses, form)}\n`, form);
    }
  } catch (error) {
    update({ message: error.message });
  } finally {
    choice = { ...choice, busy: false };
    update({});
  }
}

export function exportPane() {
  const set = (patch) => {
    choice = { ...choice, ...patch };
    update({});
  };
  const option = (value, text) => el("option", { value, text, selected: choice.source === value });
  const group = (label, list, kind) =>
    list.length ? el("optgroup", { label }, list.map((x) => option(`${kind}:${x.id}`, x.title))) : null;
  const whole = choice.source === ALL;
  return el("div", { class: "export-settings" }, [
    el("p", {
      class: "quiet",
      text: tr("Eine Liste als Datei, ohne vorher Zeilen auszuwählen. Projekte und Perspektiven gelten im gewählten Scope, wie in der Liste."),
    }),
    el("label", { class: "field", for: "export-source", text: tr("Was") }),
    el(
      "select",
      { id: "export-source", onchange: (e) => set({ source: e.target.value }) },
      [
        option("inbox", "Inbox"),
        group(tr("Perspektiven"), perspectives().filter(plain), "perspective"),
        group(tr("Projekte"), state.projects.filter(open).sort(byTitle), "project"),
        el("optgroup", { label: tr("Konto") }, [option(ALL, tr("Alles: das ganze Konto als JSON, ohne Dateien"))]),
      ],
    ),
    el("label", { class: "field", for: "export-form", text: tr("Format") }),
    el(
      "select",
      { id: "export-form", disabled: whole, onchange: (e) => set({ form: e.target.value }) },
      FORMS.map((form) => el("option", { value: form, text: `${FORM_LABEL[form]} (.${ENDING[form]})`, selected: choice.form === form })),
    ),
    el("label", { class: "toggle" }, [
      el("input", { type: "checkbox", checked: choice.closed, disabled: whole, onchange: (e) => set({ closed: e.target.checked }) }),
      el("span", { text: tr("auch Erledigtes und Verworfenes") }),
    ]),
    el("button", { type: "button", class: "primary", disabled: choice.busy, text: choice.busy ? tr("Wird geholt …") : tr("Herunterladen"), onclick: download }),
    el("p", {
      class: "quiet",
      text: whole
        ? tr("Eine vollständige Sicherung mit den Dateien der Anhänge macht `samu export` auf der Kommandozeile.")
        : tr("Dasselbe auf der Kommandozeile: `samu list --project … --format org`, ebenso mit --perspective und --all."),
    }),
  ]);
}
