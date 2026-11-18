// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Settings (ADR 0035 §8, ADR 0036 §8, ADR 0037 §5): a navigation on the
// left, the chosen section beside it. The first section is Scopes: make one, name it,
// choose its mode, tick its members in the tree of folders, projects and
// tags, delete it. Every edit is an ordinary change over POST /changes; what
// a scope shows is the server's business.
//
// Drawn from `state` like everything else. The title field is kept across
// redraws while its scope stays the same, so a reload from /events does not
// take the text from under the cursor.

import { api, change } from "./api.js";
import { $, el, replace, uuidv7 } from "./dom.js";
import { COLORS, scopeDot } from "./colors.js";
import { placesPane } from "./places.js";
import { calendarsPane, forgetCalendarForms } from "./calendars.js";
import { tagPath } from "./words.js";
import { chipField } from "./chipfield.js";
import { layoutEditor } from "./layout.js";
import { propertiesPane } from "./props.js";
import { templatesPane } from "./templates.js";
import { keysPane } from "./keycapture.js";
import { accountPane } from "./register.js";
import { forgetLogbook, logbookPane } from "./logbook.js";
import { alarmChoice, permission, setAlarmChoice } from "./alarms.js";
import { byId, chooseScope, onlyIn, state, update } from "./state.js";
import { tr } from "./i18n.js";

let titleField = null;
let titleOf = null;
let confirming = null;

export function openSettings(scope = null, section = "scopes") {
  const first = scope ?? [...state.scopes].sort((a, b) => a.position - b.position)[0]?.id ?? null;
  update({ settings: { section, scope: first, place: null }, scopeMenu: false });
}

const SECTIONS = [
  ["scopes", "Scopes"],
  ["places", tr("Projekte und Ordner")],
  ["tags", "Tags"],
  ["forecast", "Forecast"],
  ["calendars", tr("Mehrzahl|Kalender")],
  ["alarms", tr("Erinnerungen")],
  ["properties", "Properties"],
  ["templates", tr("Vorlagen")],
  ["keys", tr("Tastatur")],
  ["account", tr("Konto")],
  ["logbook", "Logbook"],
];

// Reminders in this browser (ADR 0101 §6, §8): switches of this browser
// alone, not of the account, both off at first.
function alarmsPane() {
  const choice = alarmChoice();
  const allowed = permission();
  const toggle = (key, text, hint) =>
    el("label", { class: "toggle" }, [
      el("input", {
        type: "checkbox",
        checked: choice[key],
        disabled: allowed === "unsupported",
        onchange: (e) => setAlarmChoice({ [key]: e.target.checked }),
      }),
      el("span", {}, [text, el("span", { class: "quiet", text: ` – ${hint}` })]),
    ]);
  const why = {
    unsupported: tr("Dieser Browser kann keine Meldungen zeigen."),
    denied: tr("Der Browser hat Meldungen für diese Seite verboten; erlauben lässt es sich in seinen Website-Einstellungen."),
    default: choice.appointments || choice.tasks ? tr("Der Browser fragt noch nach der Erlaubnis.") : null,
    granted: null,
  }[allowed];
  return el("div", { class: "alarm-settings" }, [
    el("p", {
      class: "quiet",
      text: tr("Nur in diesem Browser und nur, solange samudo hier offen ist. Ein Handy, das die Kalender über DAVx5 abgleicht, erinnert an Termine schon selbst."),
    }),
    toggle("appointments", tr("Erinnerungen an Termine"), tr("die Erinnerungen, die im Termin stehen (VALARM)")),
    toggle("tasks", tr("Erinnerungen an Aufgaben"), tr("Fälligkeit und gesetzte Erinnerungen")),
    why ? el("p", { class: "error", role: "status", text: why }) : null,
  ]);
}

// The account's settings for Forecast (ADR 0045 §4): one object, set field
// by field like any object; the same on every device.
function forecastPane() {
  const s = state.accountSettings;
  if (!s) return el("p", { class: "empty", text: tr("Der Server kennt noch keine Einstellungen (älter als samu-api 0.1.15).") });
  const set = (field, value) => {
    s[field] = value;
    write([change.set(s.id, field, value)]);
  };
  const toggle = (field, text, hint) =>
    el("label", { class: "toggle" }, [
      el("input", { type: "checkbox", checked: Boolean(s[field]), onchange: (e) => set(field, e.target.checked) }),
      el("span", {}, [text, hint ? el("span", { class: "quiet", text: ` – ${hint}` }) : null]),
    ]);
  const tags = state.tags
    .filter((t) => t.status !== "dropped")
    .sort((a, b) => tagPath(a).localeCompare(tagPath(b), "de"));
  return el("div", { class: "forecast-settings" }, [
    el("label", { class: "field", for: "forecast-days", text: tr("Tage nach heute; alles danach steht unter „Später“") }),
    el("input", {
      id: "forecast-days",
      type: "number",
      min: "1",
      max: "366",
      value: String(s.forecast_days),
      onchange: (e) => {
        const n = Number(e.target.value);
        if (Number.isInteger(n) && n >= 1 && n <= 366) set("forecast_days", n);
        else e.target.value = String(s.forecast_days);
      },
    }),
    // ADR 0030 §3: a reminder on a date without a time goes off then.
    el("label", { class: "field", for: "remind-time", text: tr("Erinnerungen an Daten ohne Uhrzeit melden sich um") }),
    el("input", {
      id: "remind-time",
      type: "time",
      value: (s.remind_time ?? "09:00").slice(0, 5),
      onchange: (e) => {
        if (/^\d\d:\d\d$/.test(e.target.value)) set("remind_time", `${e.target.value}:00`);
        else e.target.value = (s.remind_time ?? "09:00").slice(0, 5);
      },
    }),
    toggle("forecast_flagged", tr("Abschnitt „Markiert“"), tr("markierte verfügbare Aufgaben, nach Heute")),
    toggle("forecast_defer", tr("Zurückgestelltes"), tr("an dem Tag, an dem das Startdatum es verfügbar macht")),
    toggle("forecast_done", tr("Heute Erledigtes"), tr("bleibt unter Heute stehen")),
    toggle("forecast_events", tr("Termine aus den Kalendern"), tr("an jedem ihrer Tage, über den Aufgaben")),
    el("label", { class: "field", for: "forecast-tag", text: tr("Forecast-Tag: seine Aufgaben (und die der Tags darunter) stehen unter Heute") }),
    el(
      "select",
      { id: "forecast-tag", onchange: (e) => set("forecast_tag", e.target.value || null) },
      [
        el("option", { value: "", text: tr("keiner") }),
        ...tags.map((t) => el("option", { value: t.id, text: tagPath(t), selected: s.forecast_tag === t.id })),
      ],
    ),
    // ADR 0080 §1: what the report counts as worked on.
    el("label", { class: "field", for: "report-worked", text: tr("Bericht: als „bearbeitet“ zählt") }),
    el(
      "select",
      { id: "report-worked", onchange: (e) => set("report_worked", e.target.value) },
      [
        el("option", { value: "events", text: tr("Uhr, Zustand, Erledigt, Kommentar, erledigte Unteraufgabe"), selected: s.report_worked !== "fields" }),
        el("option", { value: "fields", text: tr("dazu jede Änderung eines Felds"), selected: s.report_worked === "fields" }),
      ],
    ),
    el("p", { class: "quiet", text: tr("Gilt für das Konto, auf allen Geräten.") }),
  ]);
}

function nav() {
  return el(
    "nav",
    { class: "settings-nav", "aria-label": tr("Abschnitte") },
    el(
      "ul",
      {},
      SECTIONS.map(([key, text]) =>
        el("li", { class: state.settings.section === key ? "current" : null }, el("button", {
          type: "button",
          "aria-current": state.settings.section === key ? "page" : null,
          text,
          onclick: () => {
            confirming = null;
            if (key === "logbook") forgetLogbook();
            update({ settings: { ...state.settings, section: key, place: null } });
          },
        })),
      ),
    ),
  );
}

export function closeSettings() {
  confirming = null;
  forgetCalendarForms();
  update({ settings: null });
}

async function write(changes) {
  try {
    await api.write(changes);
  } catch (error) {
    update({ message: error.message });
  }
}

async function newScope() {
  const id = uuidv7();
  const now = new Date().toISOString();
  await write([
    change.create("scope", {
      id,
      created: now,
      modified: now,
      title: tr("Neuer Scope"),
      mode: "except",
      members: [],
      position: state.scopes.length,
    }),
  ]);
  update({ settings: { ...state.settings, scope: id } });
  titleField?.focus();
  titleField?.select();
}

const byPosition = (a, b) => a.position - b.position || a.title.localeCompare(b.title);

function scopeList() {
  const scopes = [...state.scopes].sort(byPosition);
  return el("div", { class: "scope-list" }, [
    el(
      "ul",
      {},
      scopes.map((s) =>
        el(
          "li",
          { class: state.settings.scope === s.id ? "current" : null },
          el("button", {
            type: "button",
            onclick: () => {
              confirming = null;
              update({ settings: { ...state.settings, scope: s.id } });
            },
          }, [
            el("span", { text: s.title }),
            state.scope === s.id ? el("span", { class: "quiet", text: tr("hier gewählt") }) : null,
          ]),
        ),
      ),
    ),
    el("button", { type: "button", text: tr("+ Neuer Scope"), onclick: newScope }),
  ]);
}

// One checkbox per folder, project or tag, ticked when it is a member.
function tick(scope, id, title, depth, extra = null) {
  const member = scope.members?.includes(id) ?? false;
  // ADR 0094 §8: what only its own scope shows carries its name here.
  const only = onlyIn(byId([...state.folders, ...state.projects, ...state.tags], id));
  return el("li", { style: depth ? `padding-left: ${depth * 16}px` : null, class: extra }, el("label", {}, [
    el("input", {
      type: "checkbox",
      checked: member,
      onchange: (event) =>
        write([event.target.checked ? change.addMember(scope.id, id) : change.removeMember(scope.id, id)]),
    }),
    el("span", { text: title }),
    only ? el("span", { class: "quiet scope-only-mark", title: tr("Nur im eigenen Scope"), text: ` · ${only}` }) : null,
  ]));
}

// What a capture in this scope gets when its line names none (ADR 0043 §6):
// a project from a list, tags to tick. Written as a whole; nothing clears it.
function captureDefaults(scope) {
  const capture = scope.capture ?? {};
  const save = (next) => {
    const value = next.project || next.tags?.length ? { ...(next.project ? { project: next.project } : {}), ...(next.tags?.length ? { tags: next.tags } : {}) } : null;
    write([change.set(scope.id, "capture", value)]);
  };
  const open = state.projects
    .filter((p) => p.status !== "completed" && p.status !== "dropped")
    .sort((a, b) => a.title.localeCompare(b.title, "de"));
  const project = el(
    "select",
    { "aria-label": tr("Projekt für Neues"), onchange: (e) => save({ ...capture, project: e.target.value || null }) },
    [
      el("option", { value: "", text: tr("Inbox (kein Projekt)") }),
      ...open.map((p) => el("option", { value: p.id, text: p.title, selected: capture.project === p.id })),
    ],
  );
  const tags = state.tags
    .filter((t) => t.status !== "dropped")
    .sort((a, b) => tagPath(a).localeCompare(tagPath(b), "de"))
    .map((t) => {
      const on = (capture.tags ?? []).includes(t.id);
      return el("button", {
        type: "button",
        class: `chip tag-toggle${on ? " on" : ""}`,
        "aria-pressed": on ? "true" : "false",
        text: tagPath(t),
        onclick: () => save({ ...capture, tags: on ? capture.tags.filter((x) => x !== t.id) : [...(capture.tags ?? []), t.id] }),
      });
    });
  return el("div", { class: "capture-defaults" }, [
    el("label", { class: "field", text: tr("Neue Aufgaben in diesem Scope bekommen, wenn die Zeile nichts nennt") }),
    project,
    el("div", { class: "tag-toggles" }, tags),
  ]);
}

// The schedule (ADR 0043 §7): windows of days and a time from-to; one whose
// end is not after its start ends the next day. Written as a whole.
const DAYS = [["mo", tr("Mo")], ["tu", tr("Di")], ["we", tr("Mi")], ["th", tr("Do")], ["fr", tr("Fr")], ["sa", tr("Sa")], ["su", tr("So")]];
const hhmm = (time) => (time ?? "00:00:00").slice(0, 5);

function scheduleEditor(scope) {
  const windows = scope.schedule ?? [];
  // Kept on the object at once, so a second click before the reload
  // builds on the first.
  const save = (next) => {
    scope.schedule = next;
    write([change.set(scope.id, "schedule", next)]);
  };
  const change1 = (i, patch) => save(windows.map((w, j) => (j === i ? { ...w, ...patch } : w)));
  const rows = windows.map((w, i) =>
    el("div", { class: "window" }, [
      el(
        "span",
        { class: "days" },
        DAYS.map(([key, label]) => {
          const on = w.days.includes(key);
          return el("button", {
            type: "button",
            class: `chip day${on ? " on" : ""}`,
            "aria-pressed": on ? "true" : "false",
            text: label,
            onclick: () => {
              const days = on ? w.days.filter((d) => d !== key) : DAYS.map(([k]) => k).filter((k) => k === key || w.days.includes(k));
              if (days.length) change1(i, { days });
            },
          });
        }),
      ),
      el("input", { type: "time", value: hhmm(w.from), "aria-label": tr("von"), onchange: (e) => e.target.value && change1(i, { from: `${e.target.value}:00` }) }),
      el("span", { text: "–" }),
      el("input", { type: "time", value: hhmm(w.to), "aria-label": tr("bis"), onchange: (e) => e.target.value && change1(i, { to: `${e.target.value}:00` }) }),
      el("button", { type: "button", class: "remove", text: "×", "aria-label": tr("Fenster entfernen"), onclick: () => save(windows.filter((_, j) => j !== i)) }),
    ]),
  );
  return el("div", { class: "schedule" }, [
    el("label", { class: "field", text: tr("Zeitplan: wann ein Gerät, das „automatisch“ folgt, diesen Scope wählt") }),
    ...rows,
    el("button", {
      type: "button",
      text: tr("+ Fenster"),
      onclick: () => save([...windows, { days: ["mo", "tu", "we", "th", "fr"], from: "08:00:00", to: "17:00:00" }]),
    }),
  ]);
}

// The inbox, a member of its own (ADR 0043 §1), above the folders.
function inboxTick(scope) {
  return el("li", { class: "inbox" }, el("label", {}, [
    el("input", {
      type: "checkbox",
      checked: Boolean(scope.inbox),
      onchange: (event) => write([change.set(scope.id, "inbox", event.target.checked)]),
    }),
    el("span", { text: "Inbox" }),
  ]));
}

// The calendars whose appointments the scope shows (ADR 0047 §12); none
// ticked means all. Written as a whole.
function calendarTicks(scope) {
  const chosen = scope.calendars ?? [];
  const save = (next) => {
    scope.calendars = next;
    write([change.set(scope.id, "calendars", next)]);
  };
  return [...state.calendars].sort(byPosition).map((c) =>
    el("li", {}, el("label", {}, [
      el("input", {
        type: "checkbox",
        checked: chosen.includes(c.id),
        onchange: (e) => save(e.target.checked ? [...chosen, c.id] : chosen.filter((x) => x !== c.id)),
      }),
      el("span", { class: "swatch", style: c.color ? `background: ${c.color}` : null }),
      el("span", { text: c.title }),
    ])),
  );
}

function folderTree(scope) {
  const rows = [inboxTick(scope)];
  const open = (p) => p.status !== "completed" && p.status !== "dropped";
  const walk = (parent, depth, seen) => {
    for (const folder of state.folders.filter((f) => (f.parent ?? null) === parent).sort(byPosition)) {
      if (seen.has(folder.id)) continue;
      seen.add(folder.id);
      rows.push(tick(scope, folder.id, folder.title, depth, "folder"));
      walk(folder.id, depth + 1, seen);
    }
    for (const project of state.projects.filter((p) => (p.folder ?? null) === parent && open(p)).sort(byPosition)) {
      rows.push(tick(scope, project.id, project.title, depth));
    }
  };
  walk(null, 0, new Set());
  return rows;
}

function tagTree(scope) {
  const rows = [];
  const walk = (parent, depth, seen) => {
    for (const tag of state.tags.filter((t) => (t.parent ?? null) === parent).sort(byPosition)) {
      if (seen.has(tag.id) || tag.status === "dropped") continue;
      seen.add(tag.id);
      rows.push(tick(scope, tag.id, tag.title, depth));
      walk(tag.id, depth + 1, seen);
    }
  };
  walk(null, 0, new Set());
  return rows;
}

// The scope's colour from the palette, or none (ADR 0060 §4).
function colorPicker(scope) {
  const current = scope.color ?? null;
  const swatch = (color, title) =>
    el("button", {
      type: "button",
      role: "radio",
      title,
      "aria-label": title,
      "aria-checked": String(current === color),
      onclick: () => current !== color && write([change.set(scope.id, "color", color)]),
    }, scopeDot(color));
  return el("div", { class: "scope-colors", role: "radiogroup", "aria-label": tr("Farbe") }, [
    swatch(null, tr("Keine Farbe")),
    ...COLORS.map(([color, title]) => swatch(color, title)),
  ]);
}

function editor(scope) {
  if (!scope) {
    return el("div", { class: "scope-edit" }, el("p", {
      class: "empty",
      text: tr("Noch kein Scope. Ein Scope zeigt in allen Ansichten nur einen Teil: etwa nur Privat, oder alles außer IT."),
    }));
  }
  if (titleOf !== scope.id || !titleField) {
    titleOf = scope.id;
    titleField = el("input", {
      id: "scope-title",
      autocomplete: "off",
      onchange: (event) => {
        const title = event.target.value.trim();
        if (title && title !== byId(state.scopes, scope.id)?.title) write([change.set(scope.id, "title", title)]);
      },
      onkeydown: (event) => {
        if (event.key === "Enter") event.target.blur();
      },
    });
    titleField.value = scope.title;
  } else if (document.activeElement !== titleField) {
    titleField.value = scope.title;
  }
  const mode = (value, text) =>
    el("label", { class: "mode" }, [
      el("input", {
        type: "radio",
        name: "scope-mode",
        value,
        checked: scope.mode === value,
        onchange: () => write([change.set(scope.id, "mode", value)]),
      }),
      el("span", { text }),
    ]);
  const here = state.scope === scope.id;
  const deleting = confirming === scope.id;
  return el("div", { class: "scope-edit" }, [
    el("label", { for: "scope-title", class: "field", text: "Name" }),
    titleField,
    el("label", { class: "field", text: tr("Farbe, solange der Scope gewählt ist") }),
    colorPicker(scope),
    el("fieldset", { class: "modes" }, [
      el("legend", { text: tr("Die Ansichten zeigen") }),
      mode("only", tr("nur, was die Häkchen erfassen")),
      mode("except", tr("alles außer dem, was die Häkchen erfassen")),
    ]),
    el("p", {
      class: "quiet",
      text: tr("Ein Ordner gilt mit allem darin, ein Tag mit allen Tags darunter und für jede Aufgabe eines Projekts, das ihn trägt. Die Inbox zählt nur über ihr eigenes Häkchen, nicht über die Tags ihrer Aufgaben. Ohne jedes Häkchen schränken die Mitglieder nicht ein; dann zählt nur der Filter."),
    }),
    el("label", { class: "field", text: tr("Filter: dazu muss eine Aufgabe auch das erfüllen") }),
    chipField(scope.id, scope.filter, (chips) => write([change.set(scope.id, "filter", chips)]), { or: true }),
    el("p", {
      class: "quiet",
      text: tr("Wie in der Suche: Chips einer Art gelten mit oder, verschiedene Arten zusammen, ein Klick auf einen Chip schließt aus, das Wort „oder“ beginnt eine weitere Gruppe. Der Filter wirkt auf Aufgaben, nicht auf Projekte und Ordner."),
    }),
    captureDefaults(scope),
    layoutEditor(scope, write),
    scheduleEditor(scope),
    el("div", { class: "members" }, [
      el("section", {}, [el("h4", { text: tr("Ordner und Projekte") }), el("ul", {}, folderTree(scope))]),
      el("section", {}, [el("h4", { text: "Tags" }), el("ul", {}, tagTree(scope))]),
      state.calendars.length
        ? el("section", {}, [
            el("h4", { text: tr("Mehrzahl|Kalender") }),
            el("ul", {}, calendarTicks(scope)),
            el("p", { class: "quiet", text: tr("Ohne Häkchen zeigt der Scope die Termine aller Kalender.") }),
          ])
        : null,
    ]),
    el("div", { class: "actions" }, [
      el("button", {
        type: "button",
        class: here ? null : "primary",
        text: here ? tr("Hier gewählt · aufheben") : tr("Hier verwenden"),
        onclick: () => chooseScope(here ? null : scope.id),
      }),
      el("button", {
        type: "button",
        class: deleting ? "danger" : null,
        text: deleting ? tr("Wirklich löschen?") : tr("Scope löschen"),
        onclick: async () => {
          if (!deleting) {
            confirming = scope.id;
            update({});
            return;
          }
          confirming = null;
          // Off the lens first, so the reload after the delete asks for
          // no scope that is gone.
          if (state.scope === scope.id) chooseScope(null);
          await write([change.delete(scope.id)]);
          update({ settings: { ...state.settings, scope: null } });
        },
      }),
    ]),
  ]);
}

export function renderSettings() {
  const overlay = $("settings");
  overlay.hidden = !state.settings;
  if (!state.settings) return;
  // Fields are kept across redraws, but building the new page moves them
  // and takes the cursor away; it goes back where it was. Asked first,
  // before anything is built.
  const active = document.activeElement;
  const kept = active && $("settings-body").contains(active) ? active : null;
  const range = kept && typeof kept.selectionStart === "number" ? [kept.selectionStart, kept.selectionEnd] : null;
  const section = state.settings.section ?? "scopes";
  const scope = state.settings.scope ? byId(state.scopes, state.settings.scope) : null;
  const title = SECTIONS.find(([key]) => key === section)?.[1];
  const main =
    section === "scopes"
      ? el("div", { class: "panes scopes" }, [scopeList(), editor(scope)])
      : section === "forecast"
        ? forecastPane()
        : section === "alarms"
          ? alarmsPane()
          : section === "calendars"
          ? calendarsPane()
          : section === "properties"
            ? propertiesPane()
            : section === "templates"
              ? templatesPane()
              : section === "keys"
                ? keysPane()
                : section === "account"
                  ? accountPane()
                  : section === "logbook"
                    ? logbookPane()
                    : placesPane(section);
  replace(
    $("settings-body"),
    el("div", { class: "settings-grid" }, [
      nav(),
      el("section", { class: "settings-main", "aria-label": title }, [el("h3", { text: title }), main]),
    ]),
  );
  if (kept?.isConnected && document.activeElement !== kept) {
    kept.focus();
    if (range) kept.setSelectionRange(...range);
  }
}

export function wireSettings() {
  $("settings-close").addEventListener("click", closeSettings);
  $("settings").addEventListener("click", (event) => {
    if (event.target === $("settings")) closeSettings();
  });
}
