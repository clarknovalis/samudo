// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// A repetition in the inspector (ADR 0007 §3, 85b3799a): the RRULE taken
// apart into what the form edits, put together again, and said in words.
// The form knows FREQ, INTERVAL, plain BYDAY on a weekly rule, one
// BYMONTHDAY or one numbered BYDAY on a monthly rule, one BYMONTH and
// BYMONTHDAY on a yearly one, WKST, UNTIL and COUNT; every other part
// stays in `rest` and goes back unchanged. The server checks the rule
// (samu_core::repeat::check); nothing here decides.
//
// `ruleEditor` is the same form for an appointment (ADR 0100 §8): no mode
// from OmniFocus, and a rule it cannot show whole stays as text until it
// is replaced.

import { el } from "./dom.js";
import { addDays, shortDay } from "./words.js";
import { lang, tr } from "./i18n.js";

export const FREQS = [
  ["DAILY", tr("täglich"), [tr("Einheit|Tag"), tr("Tage")]],
  ["WEEKLY", tr("wöchentlich"), [tr("Woche"), tr("Wochen")]],
  ["MONTHLY", tr("monatlich"), [tr("Monat"), tr("Monate")]],
  ["YEARLY", tr("jährlich"), [tr("Jahr"), tr("Jahre")]],
];

export const DAYS = [
  ["MO", tr("Mo")],
  ["TU", tr("Di")],
  ["WE", tr("Mi")],
  ["TH", tr("Do")],
  ["FR", tr("Fr")],
  ["SA", tr("Sa")],
  ["SU", tr("So")],
];

export const MODES = [
  ["fixed", tr("fester Plan")],
  ["due_after_completion", tr("Fällig ab Erledigung")],
  ["defer_after_completion", tr("Start ab Erledigung")],
];

const PLAIN_DAY = /^(MO|TU|WE|TH|FR|SA|SU)$/;
const NTH_DAY = /^([+-]?)(\d{1,2})(MO|TU|WE|TH|FR|SA|SU)$/;
const MONTH_DAY = /^[+-]?\d{1,2}$/;

const pad = (n) => String(n).padStart(2, "0");

// "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH;COUNT=5" → the form's parts.
// With `local`, an UNTIL in UTC is the day it falls on here, as an
// appointment's end written by `ruleEditor` is the end of a local day.
export function parseRule(rule, { local = false } = {}) {
  const form = { freq: null, interval: 1, byday: [], nth: null, monthday: null, bymonth: null, wkst: null, until: null, count: null, rest: [] };
  for (const part of (rule ?? "").split(";").filter(Boolean)) {
    const at = part.indexOf("=");
    const key = at < 0 ? part.toUpperCase() : part.slice(0, at).toUpperCase();
    const value = at < 0 ? "" : part.slice(at + 1).toUpperCase();
    const nth = NTH_DAY.exec(value);
    if (key === "FREQ" && !form.freq) form.freq = value;
    else if (key === "INTERVAL" && /^\d+$/.test(value)) form.interval = Number(value);
    else if (key === "BYDAY" && !form.byday.length && value.split(",").every((d) => PLAIN_DAY.test(d))) form.byday = value.split(",");
    else if (key === "BYDAY" && !form.nth && nth && Number(nth[2]) >= 1 && Number(nth[2]) <= 5)
      form.nth = { n: Number(nth[2]) * (nth[1] === "-" ? -1 : 1), day: nth[3] };
    else if (key === "BYMONTHDAY" && form.monthday === null && MONTH_DAY.test(value) && Number(value) !== 0 && Math.abs(Number(value)) <= 31)
      form.monthday = Number(value);
    else if (key === "BYMONTH" && form.bymonth === null && /^\d{1,2}$/.test(value) && Number(value) >= 1 && Number(value) <= 12)
      form.bymonth = Number(value);
    else if (key === "WKST" && !form.wkst && PLAIN_DAY.test(value)) form.wkst = value;
    else if (key === "UNTIL" && /^\d{8}/.test(value)) form.until = untilDay(value, local);
    else if (key === "COUNT" && /^\d+$/.test(value)) form.count = Number(value);
    else form.rest.push(part);
  }
  // Parts that mean something else on another frequency (days on a
  // monthly rule are every Monday of the month): the form leaves them as
  // they are.
  if (form.byday.length && form.freq !== "WEEKLY") {
    form.rest.push(`BYDAY=${form.byday.join(",")}`);
    form.byday = [];
  }
  if (form.nth && form.freq !== "MONTHLY") {
    form.rest.push(`BYDAY=${nthCode(form.nth)}`);
    form.nth = null;
  }
  if (form.monthday !== null && (form.nth || (form.freq !== "MONTHLY" && form.freq !== "YEARLY"))) {
    form.rest.push(`BYMONTHDAY=${form.monthday}`);
    form.monthday = null;
  }
  if (form.bymonth !== null && form.freq !== "YEARLY") {
    form.rest.push(`BYMONTH=${form.bymonth}`);
    form.bymonth = null;
  }
  return form;
}

// The day of an UNTIL: its date, or with `local` the day here of a time in
// UTC.
function untilDay(value, local) {
  const m = /^(\d{4})(\d\d)(\d\d)T(\d\d)(\d\d)(\d\d)Z$/.exec(value);
  if (local && m) {
    const d = new Date(Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]));
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  }
  return `${value.slice(0, 4)}-${value.slice(4, 6)}-${value.slice(6, 8)}`;
}

const nthCode = (nth) => `${nth.n}${nth.day}`;

// The form's parts → the rule, FREQ first, the parts it does not know
// after the ones it does. `until` writes the end day; by default the end
// of that day in UTC: a date alone is refused next to a start in UTC
// (samu_core::repeat::Parts::rule does the same).
export function buildRule(form, { until = (day) => `${day.replaceAll("-", "")}T235959Z` } = {}) {
  const parts = [`FREQ=${form.freq}`];
  if (form.interval > 1) parts.push(`INTERVAL=${form.interval}`);
  if (form.freq === "WEEKLY" && form.byday.length) {
    const order = DAYS.map(([code]) => code);
    const days = [...new Set(form.byday)].sort((a, b) => order.indexOf(a) - order.indexOf(b));
    parts.push(`BYDAY=${days.join(",")}`);
  }
  if (form.freq === "YEARLY" && form.bymonth) parts.push(`BYMONTH=${form.bymonth}`);
  if (form.freq === "MONTHLY" && form.nth) parts.push(`BYDAY=${nthCode(form.nth)}`);
  else if ((form.freq === "MONTHLY" || form.freq === "YEARLY") && form.monthday) parts.push(`BYMONTHDAY=${form.monthday}`);
  if (form.wkst) parts.push(`WKST=${form.wkst}`);
  parts.push(...form.rest);
  if (form.until) parts.push(`UNTIL=${until(form.until)}`);
  if (form.count) parts.push(`COUNT=${form.count}`);
  return parts.join(";");
}

// Whether the form can show the rule: a frequency it offers.
export function formFits(form) {
  return FREQS.some(([code]) => code === form.freq);
}

// Whether the form shows all of it: nothing left over (ADR 0100 §8).
export const formShowsAll = (form) => formFits(form) && !form.rest.length;

const dayName = (code) => DAYS.find(([c]) => c === code)?.[1] ?? code;

// "am 2. Di", "am letzten Fr", "am 15.", "am vorletzten Tag"; in English
// "on the 2nd Tue", "on the last Fri", "on the 15th" (ADR 0105).
const ordinal = (n) => {
  if (lang() !== "en") return `${n}.`;
  const tens = n % 100;
  const end = tens >= 11 && tens <= 13 ? "th" : ({ 1: "st", 2: "nd", 3: "rd" })[n % 10] ?? "th";
  return `${n}${end}`;
};
function monthlyWords(form) {
  if (form.nth) {
    const day = dayName(form.nth.day);
    if (form.nth.n === -1) return tr("am letzten {day}", { day });
    if (form.nth.n < 0) return tr("am {n}-letzten {day}", { n: ordinal(-form.nth.n), day });
    return tr("am {n} {day}", { n: ordinal(form.nth.n), day });
  }
  if (form.monthday === -1) return tr("am letzten Tag");
  if (form.monthday < 0) return tr("am {n}-letzten Tag", { n: ordinal(-form.monthday) });
  if (form.monthday) return tr("am {n}", { n: ordinal(form.monthday) });
  return null;
}

// "alle 2 Wochen Mo, Do · Fällig ab Erledigung · 5 Mal"
export function repeatText(repeat, { local = false } = {}) {
  if (!repeat) return "";
  const form = parseRule(repeat.rule, { local });
  const freq = FREQS.find(([code]) => code === form.freq);
  let text = !freq
    ? repeat.rule
    : form.interval > 1
      ? tr("alle {n} {unit}", { n: form.interval, unit: freq[2][1] })
      : freq[1];
  if (form.byday.length) {
    text += ` ${form.byday.map(dayName).join(", ")}`;
  }
  if (form.freq === "MONTHLY" && monthlyWords(form)) text += ` ${monthlyWords(form)}`;
  if (freq && form.rest.length) text += ` (${form.rest.join(";")})`;
  const parts = [text];
  if (repeat.mode === "fixed") {
    if (repeat.catch_up) parts.push(tr("holt auf"));
  } else {
    parts.push(MODES.find(([code]) => code === repeat.mode)?.[1] ?? repeat.mode);
  }
  if (form.until) parts.push(tr("bis {day}", { day: shortDay({ date: form.until }) }));
  if (form.count) parts.push(tr("{n} Mal", { n: form.count }));
  return parts.join(" · ");
}

// "nächste: fällig Do 19.11." from the status's `next_repeat`.
export function nextText(next) {
  if (!next) return "";
  if (next.ended) return tr("endet mit dieser");
  const which = [
    ["due", tr("fällig")],
    ["defer", tr("Start")],
    ["planned", tr("geplant")],
  ].find(([field]) => next[field]);
  return which ? tr("nächste: {which} {day}", { which: which[1], day: shortDay(next[which[0]]) }) : "";
}

// --- The editor of an appointment's rule (ADR 0100 §8) ----------------------

// How an appointment's UNTIL is written: a date next to a start over whole
// days, as iCalendar wants; else the end of that day here, in UTC.
export function untilWriter(allDay) {
  if (allDay) return (day) => day.replaceAll("-", "");
  return (day) => {
    const d = new Date(`${day}T23:59:59`);
    return `${d.getUTCFullYear()}${pad(d.getUTCMonth() + 1)}${pad(d.getUTCDate())}T${pad(d.getUTCHours())}${pad(d.getUTCMinutes())}${pad(d.getUTCSeconds())}Z`;
  };
}

// The rule as the editor would write it for this kind of start, when the
// editor shows all of it; else the rule unchanged.
export function normalRule(rule, allDay) {
  const form = parseRule(rule, { local: true });
  return rule && formShowsAll(form) ? buildRule(form, { until: untilWriter(allDay) }) : rule;
}

const weekdayOf = (day) => DAYS[(new Date(`${day}T12:00:00`).getDay() + 6) % 7][0];

// The numbered weekdays a start day can repeat on in a month: its n-th,
// and the last when it is.
function nthChoices(day) {
  const date = Number(day.slice(8, 10));
  const d = new Date(`${day}T12:00:00`);
  const length = new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate();
  const code = weekdayOf(day);
  const out = [{ n: Math.ceil(date / 7), day: code }];
  if (date + 7 > length) out.push({ n: -1, day: code });
  return out;
}

// The form for the rule of an appointment starting on `start`
// (YYYY-MM-DD): frequency and interval, the weekdays of a weekly rule,
// the day of a monthly one, the end. `onChange` gets the new rule, or ""
// for none. A rule the form cannot show whole is shown as text and stays
// as it is until it is replaced.
export function ruleEditor({ rule, start, allDay, onChange }) {
  const form = parseRule(rule, { local: true });
  const write = (patch) => onChange(buildRule({ ...form, ...patch }, { until: untilWriter(allDay) }));
  const button = (attrs) => el("button", { type: "button", ...attrs });
  if (rule && !formShowsAll(form)) {
    return el("div", { class: "repeat-form repeat-foreign" }, [
      el("code", { class: "repeat-rule-text", text: rule }),
      el("p", { class: "quiet", text: tr("Diese Regel kann das Formular nicht ganz zeigen; sie bleibt, wie sie ist.") }),
      el("div", { class: "repeat-row" }, [
        button({ text: tr("Ersetzen"), onclick: () => onChange(`FREQ=${formFits(form) ? form.freq : "WEEKLY"}`) }),
        button({ text: tr("Keine Wiederholung"), onclick: () => onChange("") }),
      ]),
    ]);
  }
  const rows = [];
  const unit = FREQS.find(([code]) => code === form.freq)?.[2];
  rows.push(
    el("div", { class: "repeat-row" }, [
      el(
        "select",
        {
          id: "appointment-rrule",
          "aria-label": tr("Häufigkeit"),
          onchange: (event) =>
            event.target.value
              ? write({ freq: event.target.value, byday: [], nth: null, monthday: null, bymonth: null })
              : onChange(""),
        },
        [
          el("option", { value: "", text: tr("keine"), selected: !rule }),
          ...FREQS.map(([code, label]) => el("option", { value: code, text: label, selected: form.freq === code })),
        ],
      ),
      rule ? tr("alle") : null,
      rule
        ? el("input", {
            type: "number",
            min: "1",
            class: "repeat-interval",
            "aria-label": tr("Abstand"),
            value: String(form.interval),
            onchange: (event) => {
              const n = Number(event.target.value);
              if (Number.isInteger(n) && n >= 1 && n !== form.interval) write({ interval: n });
            },
          })
        : null,
      rule && unit ? (form.interval > 1 ? unit[1] : unit[0]) : null,
    ]),
  );
  if (!rule) return el("div", { class: "repeat-form" }, rows);

  if (form.freq === "WEEKLY") {
    // Without days the rule repeats on the start's weekday; that one is
    // shown pressed.
    const own = weekdayOf(start);
    const days = form.byday.length ? form.byday : [own];
    rows.push(
      el(
        "div",
        { class: "repeat-row repeat-days", role: "group", "aria-label": tr("Wochentage") },
        DAYS.map(([code, label]) => {
          const on = days.includes(code);
          return button({
            class: on ? "current" : null,
            "aria-pressed": on ? "true" : "false",
            text: label,
            onclick: () => {
              const next = on ? days.filter((d) => d !== code) : [...days, code];
              if (!next.length) return;
              write({ byday: next.length === 1 && next[0] === own ? [] : next });
            },
          });
        }),
      ),
    );
  }

  if (form.freq === "MONTHLY") {
    const date = Number(start.slice(8, 10));
    const options = [["day", tr("am {n}", { n: ordinal(date) })]];
    if (form.monthday && form.monthday !== date) options.push([`md:${form.monthday}`, monthlyWords({ monthday: form.monthday })]);
    const nths = nthChoices(start);
    if (form.nth && !nths.some((x) => nthCode(x) === nthCode(form.nth))) nths.push(form.nth);
    for (const nth of nths) options.push([`nth:${nthCode(nth)}`, monthlyWords({ nth })]);
    const current = form.nth ? `nth:${nthCode(form.nth)}` : form.monthday && form.monthday !== date ? `md:${form.monthday}` : "day";
    rows.push(
      el("div", { class: "repeat-row" }, [
        el(
          "select",
          {
            "aria-label": tr("Tag im Monat"),
            onchange: (event) => {
              const [kind, value] = event.target.value.split(":");
              if (kind === "day") write({ nth: null, monthday: null });
              else if (kind === "md") write({ nth: null, monthday: Number(value) });
              else {
                const m = NTH_DAY.exec(value);
                write({ monthday: null, nth: { n: Number(m[2]) * (m[1] === "-" ? -1 : 1), day: m[3] } });
              }
            },
          },
          options.map(([value, text]) => el("option", { value, text, selected: value === current })),
        ),
      ]),
    );
  }

  const end = form.count ? "count" : form.until ? "until" : "never";
  rows.push(
    el("div", { class: "repeat-row" }, [
      tr("endet"),
      el(
        "select",
        {
          "aria-label": tr("Ende der Wiederholung"),
          onchange: (event) => {
            const to = event.target.value;
            if (to === "never") write({ until: null, count: null });
            else if (to === "until") write({ count: null, until: form.until ?? addDays(start, 30) });
            else write({ until: null, count: form.count ?? 10 });
          },
        },
        [
          ["never", tr("nie")],
          ["until", tr("am")],
          ["count", tr("nach")],
        ].map(([value, text]) => el("option", { value, text, selected: value === end })),
      ),
      end === "until"
        ? el("input", {
            type: "date",
            "aria-label": tr("Letzter Tag"),
            value: form.until,
            onchange: (event) => {
              if (/^\d{4}-\d\d-\d\d$/.test(event.target.value)) write({ until: event.target.value });
            },
          })
        : null,
      end === "count"
        ? el("input", {
            type: "number",
            min: "1",
            class: "repeat-interval",
            "aria-label": tr("Anzahl"),
            value: String(form.count),
            onchange: (event) => {
              const n = Number(event.target.value);
              if (Number.isInteger(n) && n >= 1) write({ count: n });
            },
          })
        : null,
      end === "count" ? tr("Terminen") : null,
    ]),
  );
  return el("div", { class: "repeat-form" }, rows);
}
