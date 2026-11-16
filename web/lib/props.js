// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Properties as a front matter (ADR 0054): a foldable block under the
// title of a task or project, one line `key  value` per property, the own
// ones editable, the inherited ones faint with where they come from. A
// value leads somewhere when it is a URI or its key has a template in the
// account's settings; with "Befehle dieses Rechners" on, a click goes to
// `samu-open:<id>/<key>` and the desktop handler runs this machine's
// command (`samu open-uri`).
//
// `isUri`, `encode` and `linkFor` follow samu-core's `properties.rs` rule
// for rule; change both together.

import { api, change } from "./api.js";
import { el } from "./dom.js";
import { EXAMPLES } from "./addresses.js";
import { byId, state, update } from "./state.js";
import { tr } from "./i18n.js";

// Keys samu writes itself (ADR 0023 §5, ADR 0051): last and faint.
export const SYSTEM = ["org_id", "org_origin", "source"];

export const isKey = (key) => /^[a-z][a-z0-9_-]{0,63}$/.test(key);

// A scheme after RFC 3986, a colon, something after it, no white space.
export const isUri = (value) => /^[A-Za-z][A-Za-z0-9+.-]*:\S+$/.test(value);

// Every byte but the unreserved ones as %XX, like samu-core.
export const encode = (text) =>
  encodeURIComponent(text).replace(/[!'()*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`);

// Where a value leads (ADR 0054 §4): itself as a URI, else the template.
export function linkFor(value, template) {
  const v = (value ?? "").trim();
  if (!v) return null;
  if (isUri(v)) return v;
  const t = (template ?? "").trim();
  return t && t.includes("{value}") ? t.split("{value}").join(encode(v)) : null;
}

// --- Per browser: folded, and whether clicks go to this machine ---------

function read(key, fallback) {
  try {
    const raw = localStorage.getItem(key);
    return raw === null ? fallback : JSON.parse(raw);
  } catch {
    return fallback;
  }
}

function keep(key, value) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Without storage the choice lasts until the page reloads.
  }
}

let unfolded = read("samu.propsOpen", true);
let local = read("samu.openLocal", false);

export const opensLocally = () => local;
export function setOpensLocally(on) {
  local = on;
  keep("samu.openLocal", on);
  update({});
}

// Where a click on the value of `key` at `owner` goes, or nothing.
export function targetOf(owner, key, value) {
  // System keys open nothing on the machine; they lead where they lead.
  if (local && !SYSTEM.includes(key)) return `samu-open:${owner}/${key}`;
  return linkFor(value, state.accountSettings?.property_links?.[key]);
}

// --- The block --------------------------------------------------------

// What is being edited: { owner, key } of a value, or { owner, adding }.
let editing = null;

async function write(changes) {
  try {
    await api.write(changes);
    update({ message: null });
  } catch (error) {
    update({ message: error.message });
  }
}

function save(owner, key, value) {
  stopEditing();
  const v = value.trim();
  write([change.set(owner, `properties.${key}`, v || null)]);
}

// Every key in the account, for the suggestions when adding one.
export function knownKeys() {
  const keys = new Set(Object.keys(state.accountSettings?.property_links ?? {}));
  for (const o of [...state.tasks, ...state.projects]) {
    for (const k of Object.keys(o.properties ?? {})) keys.add(k);
  }
  return [...keys].filter((k) => !SYSTEM.includes(k)).sort();
}

function fromTitle(id) {
  const p = byId(state.projects, id);
  if (p) return tr("von Projekt „{title}“", { title: p.title });
  const t = byId(state.tasks, id);
  return t ? tr("von „{title}“", { title: t.title }) : tr("geerbt");
}

// Fields being typed in, kept across redraws by a name, so a reload from
// /events does not take the text from under the cursor. A new one gets the
// focus once.
const fields = new Map();

function kept(name, make) {
  let field = fields.get(name);
  if (!field) {
    field = make();
    fields.set(name, field);
    requestAnimationFrame(() => {
      if (field.isConnected) {
        field.focus();
        field.select?.();
      }
    });
  }
  return field;
}

function stopEditing() {
  editing = null;
  fields.clear();
}

function valueEditor(owner, key, value) {
  return kept(`edit:${owner}/${key}`, () => el("input", {
    id: "prop-edit",
    class: "prop-input",
    value,
    "aria-label": tr("Wert von {key}", { key }),
    onkeydown: (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        save(owner, key, e.target.value);
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        stopEditing();
        update({});
      }
    },
    onblur: (e) => {
      if (editing?.owner === owner && editing.key === key) save(owner, key, e.target.value);
    },
  }));
}

function valueView(owner, key, value, own) {
  const target = targetOf(owner, key, value);
  const edit = () => {
    stopEditing();
    editing = { owner, key };
    update({});
  };
  const shown = target
    ? el("a", {
        class: "prop-value",
        href: target,
        title: target,
        ...(/^https?:/i.test(target) ? { target: "_blank", rel: "noopener noreferrer" } : {}),
        text: value,
      })
    : el("span", { class: "prop-value", text: value, title: tr("Bearbeiten"), ondblclick: edit, onclick: edit });
  return el("span", { class: "prop-cell" }, [
    shown,
    el("button", {
      type: "button",
      class: "prop-tool",
      title: own ? tr("Bearbeiten") : tr("Eigenen Wert setzen"),
      "aria-label": tr("{key} bearbeiten", { key }),
      text: "✎",
      onclick: edit,
    }),
    own
      ? el("button", {
          type: "button",
          class: "prop-tool",
          title: tr("Entfernen"),
          "aria-label": tr("{name} entfernen", { name: key }),
          text: "×",
          onclick: () => write([change.set(owner, `properties.${key}`, null)]),
        })
      : null,
  ]);
}

function adder(owner) {
  const finish = () => {
    const key = document.getElementById("prop-new-key")?.value.trim().toLowerCase() ?? "";
    const value = document.getElementById("prop-new-value")?.value ?? "";
    if (!isKey(key)) {
      update({ message: tr("„{key}“ ist kein Schlüssel: Kleinbuchstaben, Ziffern, _ und -, vorne ein Buchstabe.", { key }) });
      return;
    }
    if (!value.trim()) {
      update({ message: tr("{key}: ein Wert fehlt.", { key }) });
      return;
    }
    save(owner, key, value);
  };
  const keys = (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      if (e.target.id === "prop-new-key") document.getElementById("prop-new-value")?.focus();
      else finish();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      stopEditing();
      update({});
    }
  };
  const value = fields.get(`new-value:${owner}`) ?? el("input", {
    id: "prop-new-value",
    class: "prop-input",
    placeholder: tr("Wert"),
    "aria-label": tr("Wert"),
    onkeydown: keys,
  });
  fields.set(`new-value:${owner}`, value);
  return [
    el("dt", {}, kept(`new-key:${owner}`, () => el("input", {
      id: "prop-new-key",
      class: "prop-input key",
      list: "prop-keys",
      placeholder: tr("schlüssel"),
      "aria-label": tr("Schlüssel"),
      autocomplete: "off",
      onkeydown: keys,
    }))),
    el("dd", {}, [
      value,
      el("datalist", { id: "prop-keys" }, knownKeys().map((k) => el("option", { value: k }))),
    ]),
  ];
}

// The front matter of a task (with what it inherits) or a project.
export function propertyBlock(owner, own = {}, inherited = {}) {
  const rows = [
    ...Object.entries(own).map(([key, value]) => ({ key, value, own: true })),
    ...Object.entries(inherited).map(([key, i]) => ({ key, value: i.value, own: false, from: i.from })),
  ].sort((a, b) => SYSTEM.includes(a.key) - SYSTEM.includes(b.key) || a.key.localeCompare(b.key));
  const adding = editing?.owner === owner && editing.adding;
  const add = el("button", {
    type: "button",
    class: "prop-add",
    text: tr("+ Property"),
    onclick: () => {
      stopEditing();
      editing = { owner, adding: true };
      unfolded = true;
      keep("samu.propsOpen", true);
      update({});
    },
  });
  if (!rows.length && !adding) return el("div", { class: "props empty" }, add);
  const open = unfolded || adding || editing?.owner === owner;
  const names = rows.filter((r) => !SYSTEM.includes(r.key)).map((r) => r.key);
  const summary = `${rows.length} ${rows.length === 1 ? tr("Property") : "Properties"}${
    names.length ? ` · ${names.slice(0, 3).join(", ")}${names.length > 3 ? " …" : ""}` : ""
  }`;
  return el("section", { class: `props${open ? " open" : ""}`, "aria-label": "Properties" }, [
    el("button", {
      type: "button",
      class: "props-head",
      "aria-expanded": open ? "true" : "false",
      onclick: () => {
        unfolded = !open;
        keep("samu.propsOpen", unfolded);
        if (!unfolded) stopEditing();
        update({});
      },
    }, [el("span", { class: "props-fold", text: open ? "▾" : "▸" }), el("span", { text: summary })]),
    open
      ? el("dl", { class: "props-list" }, [
          ...rows.flatMap((r) => {
            const here = editing?.owner === owner && editing.key === r.key;
            const cls = ["prop", r.own ? null : "inherited", SYSTEM.includes(r.key) ? "system" : null]
              .filter(Boolean)
              .join(" ");
            return [
              el("dt", { class: cls, text: r.key }),
              el("dd", { class: cls }, [
                here ? valueEditor(owner, r.key, r.value) : valueView(owner, r.key, r.value, r.own),
                r.own ? null : el("span", { class: "prop-from", text: fromTitle(r.from) }),
              ]),
            ];
          }),
          ...(adding ? adder(owner) : []),
        ])
      : null,
    open && !adding ? add : null,
  ]);
}

// --- Settings: templates per key, and this browser's switch -----------
//
// Only keys with a template are listed, each with its field, a probe and
// a remove button; one row below adds a template. A change waits for
// tr("Sichern"); what is wrong is said under its row, not at the top.

// Per key: { field, error }; the add row under "+". Kept across redraws so
// a reload from /events does not take the text from under the cursor.
const rows = new Map();
const NEW = "+";

function writeLink(key, template) {
  const id = state.accountSettings?.id;
  if (!id) return Promise.resolve();
  return api.write([change.set(id, `property_links.${key}`, template)]);
}

// What is wrong with a template, or null.
function problem(template) {
  if (!template.includes("{value}")) return tr("Die Vorlage braucht {value} für den Wert.");
  if (!isUri(template.replace("{value}", "x"))) return tr("Die Vorlage muss eine Adresse sein, etwa https://….");
  return null;
}

function rowOf(key) {
  let row = rows.get(key);
  if (!row) {
    row = { field: null, keyField: null, error: null };
    rows.set(key, row);
  }
  return row;
}

async function saveRow(key, row, template) {
  const t = template.trim();
  row.error = t ? problem(t) : null;
  if (row.error) {
    update({});
    return false;
  }
  try {
    await writeLink(key, t || null);
    row.error = null;
    update({});
    return true;
  } catch (error) {
    row.error = error.message;
    update({});
    return false;
  }
}

function templateLine(key, stored) {
  const row = rowOf(key);
  if (!row.field) {
    row.field = el("input", {
      class: "prop-template",
      "aria-label": tr("Vorlage für {key}", { key }),
      value: stored,
      oninput: () => update({}),
      onkeydown: (e) => {
        if (e.key === "Enter") saveRow(key, row, e.target.value);
        if (e.key === "Escape") {
          e.stopPropagation();
          e.target.value = stored;
          row.error = null;
          update({});
        }
      },
    });
  }
  if (document.activeElement !== row.field && row.field.value === row.shown) row.field.value = stored;
  row.shown = stored;
  const changed = row.field.value.trim() !== stored;
  const probe = linkFor("TEST-1", stored);
  return [
    el("dt", { text: key }),
    el("dd", {}, [
      el("div", { class: "prop-template-row" }, [
        row.field,
        changed
          ? el("button", { type: "button", class: "primary", text: tr("Sichern"), onclick: () => saveRow(key, row, row.field.value) })
          : probe
            ? el("a", { href: probe, target: "_blank", rel: "noopener noreferrer", class: "quiet", title: probe, text: tr("Probe ↗") })
            : null,
        el("button", {
          type: "button",
          class: "prop-tool shown",
          title: tr("Vorlage entfernen"),
          "aria-label": tr("Vorlage für {key} entfernen", { key }),
          text: "×",
          onclick: async () => {
            if (await saveRow(key, row, "")) rows.delete(key);
          },
        }),
      ]),
      row.error ? el("p", { class: "prop-error", text: row.error }) : null,
    ]),
  ];
}

function addLine(links) {
  const row = rowOf(NEW);
  if (!row.keyField) {
    row.keyField = el("input", {
      id: "prop-template-key",
      class: "prop-input key",
      placeholder: tr("schlüssel"),
      "aria-label": tr("Schlüssel"),
      list: "prop-template-keys",
      autocomplete: "off",
    });
    row.field = el("input", {
      id: "prop-template-new",
      class: "prop-template",
      placeholder: EXAMPLES.property,
      "aria-label": tr("Vorlage"),
      onkeydown: (e) => e.key === "Enter" && add(),
    });
  }
  const add = async () => {
    const key = row.keyField.value.trim().toLowerCase();
    if (!isKey(key)) {
      row.error = key
        ? tr("„{key}“ ist kein Schlüssel: Kleinbuchstaben, Ziffern, _ und -, vorne ein Buchstabe.", { key })
        : tr("Erst einen Schlüssel eingeben, etwa jira.");
      update({});
      return;
    }
    if (links[key] !== undefined) {
      row.error = tr("Für {key} gibt es schon eine Vorlage; oben ändern.", { key });
      update({});
      return;
    }
    if (!row.field.value.trim()) {
      row.error = tr("Die Vorlage fehlt.");
      update({});
      return;
    }
    if (await saveRow(key, row, row.field.value)) {
      row.keyField.value = "";
      row.field.value = "";
    }
  };
  const unused = knownKeys().filter((k) => links[k] === undefined);
  return el("div", { class: "prop-add-template" }, [
    el("h4", { text: tr("Vorlage hinzufügen") }),
    el("div", { class: "prop-template-row" }, [
      row.keyField,
      row.field,
      el("button", { type: "button", text: tr("Hinzufügen"), onclick: add }),
    ]),
    el("datalist", { id: "prop-template-keys" }, unused.map((k) => el("option", { value: k }))),
    row.error ? el("p", { class: "prop-error", text: row.error }) : null,
    unused.length
      ? el("p", { class: "quiet", text: `${tr("Schlüssel ohne Vorlage:")} ${unused.join(", ")}` })
      : null,
  ]);
}

export function propertiesPane() {
  const s = state.accountSettings;
  if (!s) return el("p", { class: "empty", text: tr("Der Server kennt noch keine Einstellungen.") });
  const links = s.property_links ?? {};
  const keys = Object.keys(links).sort();
  return el("div", { class: "props-settings" }, [
    el("p", { class: "quiet" }, [
      `${tr("Eine Vorlage macht aus dem Wert einer Property einen Link: mit der Vorlage")} `,
      el("code", { text: "…/browse/{value}" }),
      ` ${tr("wird")} `,
      el("code", { text: "jira  HAUS-12" }),
      ` ${tr("zu einem Link auf …/browse/HAUS-12. Ein Wert, der selbst eine Adresse ist (https:, catastr:, mailto: …), braucht keine Vorlage. Gilt für das Konto, auf allen Geräten.")}`,
    ]),
    keys.length
      ? el("dl", { class: "props-templates" }, keys.flatMap((key) => templateLine(key, links[key])))
      : el("p", { class: "empty", text: tr("Noch keine Vorlage.") }),
    addLine(links),
    el("h4", { text: tr("Dieser Browser") }),
    el("label", { class: "toggle" }, [
      el("input", { type: "checkbox", checked: local, onchange: (e) => setOpensLocally(e.target.checked) }),
      el("span", {}, [
        tr("Klicks an die Befehle dieses Rechners geben"),
        el("span", {
          class: "quiet",
          text: ` – ${tr("dann öffnet samu auf diesem Rechner den Wert: mit dem Befehl, der in ~/.config/samu/config.toml unter [open] für den Schlüssel steht, sonst wie ohne Schalter. Braucht make install.")}`,
        }),
      ]),
    ]),
  ]);
}
