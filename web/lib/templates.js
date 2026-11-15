// Capture templates (ADR 0073): a line of quick capture with only marks
// and reminders for the new task, called as `/name`, or by its key after
// the capture prefix, `C h` (ADR 0084). Here the settings pane to keep them,
// and the buttons under the capture field.

import { api, change } from "./api.js";
import { el, uuidv7 } from "./dom.js";
import { state, update } from "./state.js";
import { tr } from "./i18n.js";

const ANCHORS = [
  ["planned", tr("Geplant")],
  ["due", tr("Fällig")],
  ["defer", tr("Start")],
  ["daily", tr("täglich um")],
];

// The example offered while there is no template yet (ADR 0073 §6).
const EXAMPLE = {
  title: "habit",
  text: tr("@habit p:heute 21:00 every+:1d"),
  reminders: [{ anchor: "planned" }],
};

const sorted = () =>
  [...(state.templates ?? [])].sort((a, b) => a.position - b.position || a.title.localeCompare(b.title));

// The templates with a key, in their order (ADR 0084 §2).
export const keyedTemplates = () => sorted().filter((t) => t.key);

// One letter or digit (ADR 0084 §1); `h` and `H` are two keys.
const KEY = /^[\p{L}\p{N}]$/u;

// The first letter of `title` no template has as its key, for a new one
// (ADR 0084 §3); null when every one is taken.
function freeKey(title) {
  const taken = new Set(sorted().map((t) => t.key).filter(Boolean));
  return [...title].find((c) => KEY.test(c) && !taken.has(c)) ?? null;
}

async function write(changes) {
  try {
    await api.write(changes);
    update({ message: null });
    return true;
  } catch (error) {
    // The server says what it refuses, like free text in a template.
    update({ message: error.message });
    return false;
  }
}

// `15m`, `1h`, `2d` or an ISO 8601 length as the server reads it.
function period(text) {
  const t = text.trim();
  if (!t) return null;
  if (/^P/i.test(t)) return t.toUpperCase();
  const m = /^(\d+)\s*(m|min|h|d)$/i.exec(t);
  if (!m) return undefined;
  const n = m[1];
  const unit = m[2].toLowerCase();
  return unit === "d" ? `P${n}D` : `PT${n}${unit === "h" ? "H" : "M"}`;
}

function periodText(p) {
  if (!p) return "";
  const m = /^P(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?)?$/.exec(p);
  if (!m) return p;
  return [m[1] && `${m[1]} d`, m[2] && `${m[2]} h`, m[3] && `${m[3]} min`].filter(Boolean).join(" ");
}

export function reminderText(r) {
  // ADR 0093: every day at its time.
  if (r.anchor === "daily") return tr("täglich {time}", { time: (r.time ?? "").slice(0, 5) });
  const anchor = ANCHORS.find(([a]) => a === r.anchor)?.[1] ?? r.anchor;
  return r.before ? tr("{length} vor {name}", { length: periodText(r.before), name: anchor }) : tr("zu {name}", { name: anchor });
}

async function make(base) {
  const now = new Date().toISOString();
  const position = Math.max(-1, ...sorted().map((t) => t.position)) + 1;
  let title = base.title;
  const taken = new Set(sorted().map((t) => t.title.toLowerCase()));
  for (let n = 2; taken.has(title.toLowerCase()); n += 1) title = `${base.title}${n}`;
  const id = uuidv7();
  const value = { id, created: now, modified: now, title, text: base.text, reminders: base.reminders, position };
  const key = freeKey(title);
  if (key) value.key = key;
  await write([change.create("template", value)]);
}

// The key of a template: typed into a one-character field; a key another
// template has asks to swap, take it over or cancel (ADR 0084 §3, as the
// recording of ADR 0079 §4).
function keyField(template) {
  const set = (changes) => write(changes);
  const warning = el("span", { class: "template-key-clash", hidden: true });
  const input = el("input", {
    class: "template-key",
    value: template.key ?? "",
    maxlength: "1",
    spellcheck: "false",
    autocomplete: "off",
    "aria-label": tr("Taste nach dem Präfix der Erfassung"),
    onchange: (e) => {
      const key = e.target.value;
      const back = () => {
        e.target.value = template.key ?? "";
        warning.hidden = true;
      };
      if (key === (template.key ?? "")) return back();
      if (!key) return set([change.set(template.id, "key", null)]);
      if (!KEY.test(key)) {
        update({ message: tr("Eine Taste ist ein Buchstabe oder eine Ziffer.") });
        return back();
      }
      const other = sorted().find((t) => t.id !== template.id && t.key === key);
      if (!other) return set([change.set(template.id, "key", key)]);
      // Free the key first, so no step has two templates on it.
      const takeOver = [change.set(other.id, "key", null), change.set(template.id, "key", key)];
      const swap = template.key ? [...takeOver, change.set(other.id, "key", template.key)] : takeOver;
      warning.replaceChildren(
        el("span", { text: tr("„{key}“ ist schon /{name}", { key, name: other.title }) }),
        el("button", { type: "button", text: tr("Tauschen"), onclick: () => set(swap) }),
        el("button", { type: "button", text: tr("Übernehmen"), onclick: () => set(takeOver) }),
        el("button", { type: "button", class: "link", text: tr("Abbrechen"), onclick: back }),
      );
      warning.hidden = false;
    },
  });
  return el("label", { class: "template-key-field", title: tr("Taste nach dem Präfix der Erfassung (C)") }, [
    el("span", { class: "quiet", text: tr("Taste") }),
    input,
    warning,
  ]);
}

function editor(template) {
  const set = (field, value) => write([change.set(template.id, field, value)]);
  const reminders = template.reminders ?? [];
  const anchor = el(
    "select",
    { "aria-label": tr("Anker") },
    ANCHORS.map(([value, text]) => el("option", { value, text })),
  );
  const before = el("input", { class: "before", placeholder: tr("vorher, z. B. 15m"), "aria-label": tr("Wie lange vorher") });
  const time = el("input", { type: "time", value: "20:00", "aria-label": tr("Uhrzeit"), hidden: true });
  anchor.addEventListener("change", () => {
    time.hidden = anchor.value !== "daily";
    before.hidden = anchor.value === "daily";
  });
  const addReminder = () => {
    if (anchor.value === "daily") {
      if (!/^\d\d:\d\d$/.test(time.value)) {
        update({ message: tr("Uhrzeit als HH:MM.") });
        return;
      }
      set("reminders", [...reminders, { anchor: "daily", time: `${time.value}:00` }]);
      return;
    }
    const p = period(before.value);
    if (p === undefined) {
      update({ message: tr("Vorher als 15m, 1h oder 1d.") });
      return;
    }
    set("reminders", [...reminders, p ? { anchor: anchor.value, before: p } : { anchor: anchor.value }]);
  };
  return el("li", { class: "template-edit" }, [
    el("div", { class: "template-head" }, [
      el("span", { class: "call", text: "/" }),
      el("input", {
        class: "template-title",
        value: template.title,
        "aria-label": tr("Name, zugleich der Aufruf"),
        onchange: (e) => {
          const v = e.target.value.trim().replace(/^\//, "");
          if (v && v !== template.title) set("title", v);
          else e.target.value = template.title;
        },
      }),
      keyField(template),
      el("button", {
        type: "button",
        class: "danger-quiet",
        text: tr("Löschen"),
        onclick: () => write([change.delete(template.id)]),
      }),
    ]),
    el("input", {
      class: "template-text",
      value: template.text ?? "",
      placeholder: tr("#Projekt @tag p:heute 21:00 every+:1d // Notiz"),
      spellcheck: "false",
      "aria-label": tr("Kürzel der Schnellerfassung"),
      onchange: async (e) => {
        if (!(await set("text", e.target.value.trim()))) e.target.value = template.text ?? "";
      },
    }),
    el("ul", { class: "template-reminders" }, [
      ...reminders.map((r, i) =>
        el("li", {}, [
          el("span", { text: `${tr("Erinnerung")} ${reminderText(r)}` }),
          el("button", {
            type: "button",
            class: "link",
            text: tr("entfernen"),
            onclick: () => set("reminders", reminders.filter((_, j) => j !== i)),
          }),
        ]),
      ),
      el("li", { class: "add" }, [
        anchor,
        before,
        time,
        el("button", { type: "button", text: tr("+ Erinnerung"), onclick: addReminder }),
      ]),
    ]),
  ]);
}

// Settings > Vorlagen.
export function templatesPane() {
  const list = sorted();
  return el("div", { class: "templates-pane" }, [
    el("p", {
      class: "quiet",
      text: tr("Eine Vorlage ist eine Zeile der Schnellerfassung ohne Titel. In der Erfassung ruft „/name“ sie auf, überall sonst C und ihre Taste; was die Zeile selbst nennt, gewinnt. Relative Daten wie „heute 21:00“ gelten ab dem Erfassen. every+: holt verpasste Tage nicht nach."),
    }),
    list.length ? el("ul", { class: "templates" }, list.map(editor)) : el("p", { class: "empty", text: tr("Noch keine Vorlage.") }),
    el("div", { class: "toolbar" }, [
      el("button", { type: "button", text: tr("+ Vorlage"), onclick: () => make({ title: "vorlage", text: "", reminders: [] }) }),
      list.length ? null : el("button", { type: "button", text: tr("Beispiel Habit"), onclick: () => make(EXAMPLE) }),
    ]),
  ]);
}

// The buttons under the capture field: each puts `/name` into the line,
// before a note, so the preview shows what comes of it (ADR 0073 §3).
export function templateButtons(input, changed) {
  const list = sorted();
  if (!list.length) return [];
  return list.map((t) =>
    el("button", {
      type: "button",
      class: "chip template",
      title: [t.text, ...(t.reminders ?? []).map((r) => `${tr("Erinnerung")} ${reminderText(r)}`)].filter(Boolean).join(" · "),
      onmousedown: (event) => event.preventDefault(),
      onclick: () => {
        const call = `/${t.title}`;
        const value = input.value.replace(new RegExp(`(^|\\s)/${t.title}(?=\\s|$)`, "i"), "").trimEnd();
        const at = value.indexOf(" // ");
        input.value = at >= 0 ? `${value.slice(0, at)} ${call}${value.slice(at)}` : `${value}${value ? " " : ""}${call} `;
        input.focus();
        changed();
      },
    }, [`/${t.title}`, t.key ? el("kbd", { text: t.key }) : null]),
  );
}
