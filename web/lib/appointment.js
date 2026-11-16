// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The appointment in the inspector (ADR 0047 §11, ADR 0068 §11): read,
// and in a writable calendar edited, deleted and made new.
//
// A new appointment goes over `/changes` as `create` (ADR 0068 §3); an
// edit or a delete of one that is there goes to
// `POST /appointments/{id}/edit` (§5), where samu-core turns „only this /
// this and following / all“ into changes. Nothing here computes a
// repetition.

import { api, change } from "./api.js";
import { el, uuidv7 } from "./dom.js";
import { normalRule, repeatText, ruleEditor } from "./repeat.js";
import { alarmChoice, alarmWords } from "./alarms.js";
import { byId, state, update } from "./state.js";
import { addDays, shortDay } from "./words.js";
import { tr } from "./i18n.js";

// --- Time on the wire ---------------------------------------------------

// A zoned time of the wire as a Date: `…+01:00[Europe/Berlin]`.
export const instant = (at) => new Date(at.replace(/\[[^\]]*\]$/, ""));

const pad = (n) => String(n).padStart(2, "0");
export const dayOfDate = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
export const clock = (d) => `${pad(d.getHours())}:${pad(d.getMinutes())}`;

// What `datetime-local` shows and gives: the local time without a zone.
const localInput = (d) => `${dayOfDate(d)}T${clock(d)}`;

// A local `YYYY-MM-DDTHH:MM` as a time of the wire, in this browser's zone
// with the offset it has then; null when it is no time.
export function wireAt(local) {
  if (!/^\d{4}-\d\d-\d\dT\d\d:\d\d$/.test(local)) return null;
  const d = new Date(`${local}:00`);
  if (Number.isNaN(d.getTime())) return null;
  const offset = -d.getTimezoneOffset();
  const sign = offset < 0 ? "-" : "+";
  const abs = Math.abs(offset);
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  return { at: `${local}:00${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}[${zone}]` };
}

// Sets the calendar's colour as a custom property, never as raw style
// text, so a colour cannot carry other declarations.
export function tinted(node, calendarId) {
  const color = byId(state.calendars, calendarId)?.color;
  if (color) node.style.setProperty("--cal", color);
  return node;
}

// When an occurrence is, in words: whole days with the last day included,
// times in this browser's zone.
export function whenWords(a) {
  if (a.start.date) {
    const last = a.end?.date && a.end.date > a.start.date ? addDays(a.end.date, -1) : a.start.date;
    return last === a.start.date ? `${shortDay(a.start)}, ${tr("ganztägig")}` : `${shortDay(a.start)} – ${shortDay({ date: last })}, ${tr("ganztägig")}`;
  }
  const start = instant(a.start.at);
  const end = a.end?.at ? instant(a.end.at) : null;
  const startText = `${shortDay({ date: dayOfDate(start) })} ${clock(start)}`;
  if (!end || end.getTime() === start.getTime()) return startText;
  return dayOfDate(end) === dayOfDate(start) ? `${startText}–${clock(end)}` : `${startText} – ${shortDay({ date: dayOfDate(end) })} ${clock(end)}`;
}

// --- Who may write --------------------------------------------------------

// The calendars a new appointment can go into (ADR 0068 §1).
export const writableCalendars = () =>
  state.calendars.filter((c) => c.writable && c.kind === "caldav").sort((a, b) => (a.position ?? 0) - (b.position ?? 0));

// The calendar a new appointment goes into without asking (§10).
function defaultCalendar() {
  const id = state.accountSettings?.default_calendar;
  return writableCalendars().some((c) => c.id === id) ? id : null;
}

// Why an occurrence cannot be changed, or null when it can (§1, §2).
function readOnly(occurrence) {
  if (occurrence.draft) return null;
  if (!byId(state.calendars, occurrence.calendar)?.writable) return tr("nur lesbar");
  if (byId(state.appointments, occurrence.appointment)?.meeting) return tr("Einladung, nur lesbar");
  return null;
}

// Whether the occurrence belongs to a repetition: then an edit asks which
// occurrences it is about (§4).
function inSeries(occurrence) {
  const a = byId(state.appointments, occurrence.appointment);
  if (!a) return false;
  if (a.recurrence_id) return true;
  return Boolean(a.rrule || a.rdate?.length);
}

// --- The form ---------------------------------------------------------------

// What is being typed, kept across drawing: the inspector is drawn again
// on every change of state, and the fields with it.
let form = null;

// The fields of an occurrence as the form holds them: whole days with the
// last day included, times as `datetime-local`.
function fieldsOf(o) {
  const allDay = Boolean(o.start.date);
  let start;
  let end;
  if (allDay) {
    start = o.start.date;
    end = o.end?.date && o.end.date > start ? addDays(o.end.date, -1) : start;
  } else {
    const s = instant(o.start.at);
    start = localInput(s);
    end = o.end?.at ? localInput(instant(o.end.at)) : localInput(new Date(s.getTime() + 3600000));
  }
  const rrule = byId(state.appointments, o.appointment)?.rrule ?? o.rrule ?? "";
  return {
    title: o.title ?? "",
    allDay,
    start,
    end,
    location: o.location ?? "",
    note: o.note ?? "",
    busy: o.busy !== false,
    calendar: o.calendar ?? "",
    rrule,
  };
}

function startEditing(occurrence) {
  const base = fieldsOf(occurrence);
  form = { for: occurrence, base, fields: { ...base }, confirm: false, error: null, busy: false };
}

function stopEditing() {
  form = null;
}

// A new appointment from the calendar view or Forecast: one hour from
// `start` (a Date), or a whole `day`. Opens the form; nothing is written
// before „Anlegen“.
export function startNew({ start = null, day = null }) {
  if (!writableCalendars().length) {
    update({ message: tr("Kein schreibbarer Kalender: einen CalDAV-Kalender unter Einstellungen › Kalender einrichten.") });
    return;
  }
  const draft = {
    draft: true,
    appointment: null,
    calendar: defaultCalendar() ?? "",
    title: "",
    start: day ? { date: day } : wireAt(localInput(start)),
    end: day ? { date: addDays(day, 1) } : wireAt(localInput(new Date(start.getTime() + 3600000))),
    busy: true,
  };
  startEditing(draft);
  update({ appointment: draft, selected: null, pane: "detail" });
  queueMicrotask(() => document.getElementById("appointment-title")?.focus());
}

// The form's times as the wire wants them, or a reason they are not.
function times(f) {
  if (f.allDay) {
    if (!/^\d{4}-\d\d-\d\d$/.test(f.start)) return { error: tr("Kein Beginn.") };
    const last = /^\d{4}-\d\d-\d\d$/.test(f.end) ? f.end : f.start;
    if (last < f.start) return { error: tr("Das Ende liegt vor dem Beginn.") };
    return { start: { date: f.start }, end: { date: addDays(last, 1) } };
  }
  const start = wireAt(f.start);
  const end = wireAt(f.end);
  if (!start || !end) return { error: tr("Beginn und Ende brauchen Datum und Uhrzeit.") };
  if (new Date(`${f.end}:00`) < new Date(`${f.start}:00`)) return { error: tr("Das Ende liegt vor dem Beginn.") };
  return { start, end };
}

// What the form changed, as a patch for the edit route (ADR 0068 §5);
// `location` and `note` are cleared with an empty text, `rrule` with
// `clear`.
function patchOf(f, base) {
  const patch = {};
  const clear = [];
  const title = f.title.trim();
  if (title !== base.title) patch.title = title;
  if (f.allDay !== base.allDay || f.start !== base.start || f.end !== base.end) {
    const t = times(f);
    if (t.error) return { error: t.error };
    patch.start = t.start;
    patch.end = t.end;
  }
  if (f.location.trim() !== base.location) patch.location = f.location.trim();
  if (f.note !== base.note) patch.note = f.note;
  if (f.busy !== base.busy) patch.busy = f.busy;
  if (f.calendar !== base.calendar) patch.calendar = f.calendar;
  // An UNTIL follows the kind of start (ADR 0100 §8): written again only
  // when the rule was changed.
  const rrule = f.rrule.trim() === base.rrule ? base.rrule : normalRule(f.rrule.trim(), f.allDay);
  if (rrule !== base.rrule) {
    if (rrule) patch.rrule = rrule;
    else clear.push("rrule");
  }
  if (clear.length) patch.clear = clear;
  return { patch };
}

// A new appointment as the value of `create` (ADR 0068 §3): the id is
// also its UID.
function created(f) {
  const t = times(f);
  if (t.error) return { error: t.error };
  if (!f.calendar) return { error: tr("Welcher Kalender?") };
  const id = uuidv7();
  const now = new Date().toISOString();
  const value = {
    id,
    created: now,
    modified: now,
    calendar: f.calendar,
    uid: id,
    title: f.title.trim(),
    start: t.start,
    end: t.end,
    status: "confirmed",
    busy: f.busy,
  };
  if (f.location.trim()) value.location = f.location.trim();
  if (f.note) value.note = f.note;
  if (f.rrule.trim()) value.rrule = normalRule(f.rrule.trim(), f.allDay);
  return { value };
}

async function run(work) {
  form.busy = true;
  form.error = null;
  update({});
  try {
    await work();
  } catch (error) {
    if (form) {
      form.busy = false;
      form.error = error.message;
    }
    update({});
  }
}

function save(range) {
  const f = form.fields;
  const occurrence = form.for;
  if (!f.title.trim()) {
    form.error = tr("Ein Termin braucht einen Titel.");
    update({});
    return;
  }
  if (occurrence.draft) {
    const { value, error } = created(f);
    if (error) {
      form.error = error;
      update({});
      return;
    }
    run(async () => {
      await api.write([change.create("appointment", value)]);
      stopEditing();
      const shown = { appointment: value.id, calendar: value.calendar, title: value.title, start: value.start, end: value.end, location: value.location, note: value.note, busy: value.busy, status: "confirmed" };
      update({ appointment: shown, message: null });
    });
    return;
  }
  const { patch, error } = patchOf(f, form.base);
  if (error) {
    form.error = error;
    update({});
    return;
  }
  if (!Object.keys(patch).length) {
    stopEditing();
    update({});
    return;
  }
  const series = inSeries(occurrence);
  run(async () => {
    await api.editAppointment(occurrence.appointment, {
      instance: series ? occurrence.start : undefined,
      range: series ? range : "all",
      patch,
    });
    stopEditing();
    // One occurrence of a repetition may now be another object; the view
    // asks the server again, and the sheet closes until it is chosen anew.
    if (series) update({ appointment: null, pane: "list", message: null });
    else update({ appointment: { ...occurrence, ...shownPatch(patch) }, message: null });
  });
}

// What a saved patch looks like on the occurrence shown.
function shownPatch(patch) {
  const shown = {};
  for (const key of ["title", "start", "end", "busy", "calendar"]) if (key in patch) shown[key] = patch[key];
  if ("location" in patch) shown.location = patch.location || undefined;
  if ("note" in patch) shown.note = patch.note || undefined;
  return shown;
}

function remove(range) {
  const occurrence = form?.for;
  if (!occurrence) return;
  const series = inSeries(occurrence);
  run(async () => {
    await api.editAppointment(occurrence.appointment, {
      instance: series ? occurrence.start : undefined,
      range: series ? range : "all",
      delete: true,
    });
    stopEditing();
    update({ appointment: null, pane: "list", message: null });
  });
}

// --- Drawing ----------------------------------------------------------------

const STATUS = { tentative: tr("vorläufig"), cancelled: tr("abgesagt") };

// The choice for a repetition (§11), or one button for the rest.
const RANGES = [
  ["this", tr("Nur dieser")],
  ["following", tr("Dieser und folgende")],
  ["all", tr("Alle")],
];

function readView(a) {
  const calendar = byId(state.calendars, a.calendar);
  const why = readOnly(a);
  const full = byId(state.appointments, a.appointment);
  const facts = [
    [tr("Wann"), whenWords(a)],
    full?.rrule ? [tr("Wiederholung"), repeatText({ rule: full.rrule, mode: "fixed" }, { local: true })] : full?.recurrence_id ? [tr("Wiederholung"), tr("geänderter Termin einer Serie")] : null,
    a.location ? [tr("Ort"), a.location] : null,
    // Shown also with the switch off: what a phone will do (ADR 0101 §9).
    full?.alarms?.length
      ? [tr("Erinnerung"), full.alarms.map((x) => alarmWords(x, full)).join(", ") + (alarmChoice().appointments ? "" : " (hier aus)")]
      : null,
    [tr("Kalender"), calendar?.title ?? "?"],
    STATUS[a.status] ? ["Status", STATUS[a.status]] : null,
    a.busy === false ? [tr("Belegt"), tr("frei")] : null,
  ].filter(Boolean);
  const swatch = tinted(el("span", { class: "swatch", "aria-hidden": "true" }), a.calendar);
  return [
    el("button", { type: "button", class: "back", text: tr("‹ Zurück"), onclick: () => update({ pane: "list" }) }),
    el("p", { class: "where", text: why ? `${tr("Termin")} · ${why}` : tr("Termin") }),
    el("h2", { class: "appointment-title" }, [swatch, a.title]),
    el(
      "dl",
      { class: "facts" },
      facts.flatMap(([term, value]) => [el("dt", { text: term }), el("dd", { text: value })]),
    ),
    why
      ? null
      : el("div", { class: "actions appointment-actions" }, [
          el("button", {
            type: "button",
            text: tr("Bearbeiten"),
            onclick: () => {
              startEditing(a);
              update({});
              queueMicrotask(() => document.getElementById("appointment-title")?.focus());
            },
          }),
          el("button", {
            type: "button",
            text: tr("Löschen"),
            onclick: () => {
              startEditing(a);
              form.confirm = true;
              update({});
            },
          }),
        ]),
    a.link
      ? el("a", { class: "open-original", href: a.link, target: "_blank", rel: "noopener noreferrer", text: tr("Im Kalender öffnen ↗") })
      : null,
    a.note ? el("div", {}, [el("h4", { text: tr("Notiz") }), el("p", { class: "appointment-note", text: a.note })]) : null,
  ];
}

// Asks before deleting, in the sheet itself: a dialog of the browser
// would block the page.
function confirmView(a) {
  const series = inSeries(a);
  const cancel = el("button", {
    type: "button",
    text: tr("Abbrechen"),
    onclick: () => {
      stopEditing();
      update({});
    },
  });
  return [
    el("p", { class: "where", text: tr("Termin löschen") }),
    el("h2", { class: "appointment-title" }, [tinted(el("span", { class: "swatch", "aria-hidden": "true" }), a.calendar), a.title]),
    el("p", { text: whenWords(a) }),
    el("p", { class: "quiet", text: series ? tr("Welche Termine der Serie?") : tr("Der Termin verschwindet auch im Kalender.") }),
    el("div", { class: "actions appointment-actions" }, [
      ...(series
        ? RANGES.map(([range, text]) =>
            el("button", { type: "button", class: "danger", text, disabled: form.busy, onclick: () => remove(range) }),
          )
        : [el("button", { type: "button", class: "danger", text: tr("Löschen"), disabled: form.busy, onclick: () => remove("all") })]),
      cancel,
    ]),
    form.error ? el("p", { class: "error", role: "alert", text: form.error }) : null,
  ];
}

function field(id, label, input) {
  return [el("dt", {}, el("label", { for: id, text: label })), el("dd", {}, input)];
}

function editView(a) {
  const f = form.fields;
  const set = (key) => (event) => {
    f[key] = event.target.type === "checkbox" ? event.target.checked : event.target.value;
  };
  const timeType = f.allDay ? "date" : "datetime-local";
  const toggleAllDay = (event) => {
    const allDay = event.target.checked;
    if (allDay === f.allDay) return;
    if (allDay) {
      f.start = f.start.slice(0, 10);
      f.end = f.end.slice(0, 10) < f.start ? f.start : f.end.slice(0, 10);
    } else {
      f.start = `${f.start}T09:00`;
      f.end = `${f.start.slice(0, 10)}T10:00`;
    }
    f.allDay = allDay;
    update({});
  };
  // Moving the start takes the end along, by the same length.
  const moveStart = (event) => {
    const before = f.start;
    f.start = event.target.value;
    if (f.allDay) {
      if (/^\d{4}-\d\d-\d\d$/.test(before) && /^\d{4}-\d\d-\d\d$/.test(f.start) && /^\d{4}-\d\d-\d\d$/.test(f.end)) {
        const days = Math.round((new Date(`${f.end}T12:00:00`) - new Date(`${before}T12:00:00`)) / 86400000);
        f.end = addDays(f.start, days);
      }
    } else if (wireAt(before) && wireAt(f.start) && wireAt(f.end)) {
      const length = new Date(`${f.end}:00`) - new Date(`${before}:00`);
      f.end = localInput(new Date(new Date(`${f.start}:00`).getTime() + length));
    }
    update({});
  };
  const calendars = writableCalendars();
  const series = !a.draft && inSeries(a);
  const instance = Boolean(byId(state.appointments, a.appointment)?.recurrence_id);
  // An occurrence alone keeps its calendar and has no rule (ADR 0068 §3).
  const onlyWhole = f.calendar !== form.base.calendar || f.rrule.trim() !== form.base.rrule;
  const cancel = el("button", {
    type: "button",
    text: tr("Abbrechen"),
    disabled: form.busy,
    onclick: () => {
      const draft = a.draft;
      stopEditing();
      update(draft ? { appointment: null, pane: "list" } : {});
    },
  });
  const buttons = a.draft
    ? [el("button", { type: "submit", class: "primary", text: tr("Anlegen"), disabled: form.busy })]
    : series
      ? RANGES.map(([range, text]) =>
          el("button", {
            type: "button",
            class: range === "this" ? "primary" : null,
            text,
            disabled: form.busy || (range === "this" && onlyWhole),
            title: range === "this" && onlyWhole ? tr("Ein einzelner Termin einer Serie behält Kalender und Wiederholung.") : null,
            onclick: () => save(range),
          }),
        )
      : [el("button", { type: "submit", class: "primary", text: tr("Sichern"), disabled: form.busy })];
  return el(
    "form",
    {
      class: "appointment-form",
      onsubmit: (event) => {
        event.preventDefault();
        if (!series) save("all");
      },
      onkeydown: (event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          cancel.click();
        }
      },
    },
    [
      el("p", { class: "where", text: a.draft ? tr("Neuer Termin") : series ? tr("Termin einer Serie bearbeiten") : tr("Termin bearbeiten") }),
      el("input", {
        class: "title",
        id: "appointment-title",
        value: f.title,
        placeholder: tr("Titel"),
        "aria-label": tr("Titel"),
        oninput: set("title"),
      }),
      el("dl", { class: "facts" }, [
        ...field(
          "appointment-allday",
          tr("Ganztägig"),
          el("input", { id: "appointment-allday", type: "checkbox", checked: f.allDay, onchange: toggleAllDay }),
        ),
        ...field("appointment-start", tr("Beginn"), el("input", { id: "appointment-start", type: timeType, value: f.start, onchange: moveStart })),
        ...field(
          "appointment-end",
          f.allDay ? tr("Letzter Tag") : tr("Ende"),
          el("input", { id: "appointment-end", type: timeType, value: f.end, oninput: set("end") }),
        ),
        ...field("appointment-location", tr("Ort"), el("input", { id: "appointment-location", value: f.location, oninput: set("location") })),
        ...field(
          "appointment-calendar",
          tr("Kalender"),
          instance
            ? el("span", { text: byId(state.calendars, f.calendar)?.title ?? "?" })
            : el(
                "select",
                {
                  id: "appointment-calendar",
                  onchange: (event) => {
                    f.calendar = event.target.value;
                    update({});
                  },
                },
                [
                  f.calendar ? null : el("option", { value: "", text: tr("Kalender wählen …"), selected: true }),
                  ...calendars.map((c) => el("option", { value: c.id, text: c.title, selected: c.id === f.calendar })),
                ],
              ),
        ),
        ...field(
          "appointment-busy",
          tr("Belegt"),
          el("input", { id: "appointment-busy", type: "checkbox", checked: f.busy, onchange: set("busy") }),
        ),
        instance
          ? null
          : field(
              "appointment-rrule",
              tr("Wiederholung"),
              ruleEditor({
                rule: f.rrule,
                start: f.start.slice(0, 10),
                allDay: f.allDay,
                onChange: (rule) => {
                  f.rrule = rule;
                  update({});
                },
              }),
            ),
      ].flat()),
      el("label", { class: "field", for: "appointment-note", text: tr("Notiz") }),
      el("textarea", { id: "appointment-note", rows: "4", oninput: set("note") }, f.note),
      series ? el("p", { class: "quiet", text: tr("Sichern für:") }) : null,
      el("div", { class: "actions appointment-actions" }, [...buttons, cancel]),
      form.error ? el("p", { class: "error", role: "alert", text: form.error }) : null,
    ],
  );
}

export function appointmentDetail(a) {
  // Another appointment was chosen: what was typed for the last one goes.
  if (form && form.for !== a) stopEditing();
  if (!form) return readView(a);
  if (form.confirm) return confirmView(a);
  return [
    el("button", { type: "button", class: "back", text: tr("‹ Zurück"), onclick: () => update({ pane: "list" }) }),
    editView(a),
  ];
}
