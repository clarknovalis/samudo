// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The calendar view (ADR 0047 §11): a day or a week, hours as a grid, a
// band of whole days above it. Appointments in their calendar's colour and
// tasks planned at a time as blocks. A click on a free place makes a new
// appointment of an hour there, a click on a free place of the band one
// over the whole day (ADR 0068 §11); the sheet is in appointment.js.
// A month of six weeks and a list of the coming days (ADR 0100 §5–7) show
// the same occurrences as lines, a day at a time.
//
// The occurrences come from `GET /appointments`, unfolded by samu-core
// (ADR 0047 §5); nothing here computes a repetition. Times are drawn in
// this browser's zone, as Forecast draws them.

import { api } from "./api.js";
import { clock, dayOfDate, instant, startNew, tinted, writableCalendars } from "./appointment.js";
import { el } from "./dom.js";
import { minutes } from "./query.js";
import { isOpen, state, update } from "./state.js";
import { addDays, shortDay, today } from "./words.js";
import { locale, tr } from "./i18n.js";

// Pixels per hour; the CSS reads the same number through --hour.
const HOUR = 44;
// A block is drawn at least this long, so a short one stays readable and
// counts as overlapping what it covers on screen.
const SHORTEST = 25;
// What a task planned at a time lasts without planned_duration or an
// estimate (ADR 0047 §11).
const TASK_MINUTES = 30;

// Day, week, month or list, this browser only; the day shown starts at
// today.
function stored() {
  try {
    return JSON.parse(localStorage.getItem("samu.calendar")) ?? {};
  } catch {
    return {};
  }
}
const narrow = () => typeof window !== "undefined" && window.innerWidth < 800;
const MODES = ["day", "week", "month", "list"];
// Days the list shows at first, and again with each „Mehr“ (ADR 0100 §6).
const LIST_DAYS = 30;
let choice = {
  mode: MODES.includes(stored().mode) ? stored().mode : narrow() ? "day" : "week",
  day: today(),
  listDays: LIST_DAYS,
};

function choose(patch) {
  choice = { ...choice, ...patch };
  if (patch.mode) choice.listDays = LIST_DAYS;
  try {
    localStorage.setItem("samu.calendar", JSON.stringify({ mode: choice.mode }));
  } catch {
    // Private mode: the choice lasts as long as the page.
  }
  update({});
}

// What the calendar shows, for the address (ADR 0075 §7): the day null
// when it is today.
export function calendarChoice() {
  return { mode: choice.mode, day: choice.day === today() ? null : choice.day };
}

// The same from an address; what is missing stays.
export function setCalendarChoice({ mode = null, day = null }) {
  const patch = {};
  if (MODES.includes(mode)) patch.mode = mode;
  if (day) patch.day = day;
  if (Object.keys(patch).length) choose(patch);
}

// Monday of the week a day is in.
function monday(day) {
  const weekday = (new Date(`${day}T12:00:00`).getDay() + 6) % 7;
  return addDays(day, -weekday);
}

// The first of the month a day is in.
const firstOfMonth = (day) => `${day.slice(0, 8)}01`;

// The same day of another month, as near as that month has it; the
// arrows of the month step with it.
export function addMonths(day, months) {
  const [y, m] = [Number(day.slice(0, 4)), Number(day.slice(5, 7)) - 1 + months];
  const year = y + Math.floor(m / 12);
  const month = ((m % 12) + 12) % 12;
  const last = new Date(year, month + 1, 0).getDate();
  const date = Math.min(Number(day.slice(8, 10)), last);
  return `${year}-${pad(month + 1)}-${pad(date)}`;
}

const span = (first, length) => Array.from({ length }, (_, i) => addDays(first, i));

export function shownDays() {
  switch (choice.mode) {
    case "day":
      return [choice.day];
    // Six weeks from the Monday before the first (ADR 0100 §5).
    case "month":
      return span(monday(firstOfMonth(choice.day)), 42);
    // From today on, always (§6).
    case "list":
      return span(today(), choice.listDays);
    default:
      return span(monday(choice.day), 7);
  }
}

// --- Time on the wire ---------------------------------------------------

const pad = (n) => String(n).padStart(2, "0");

// The local start of a day as a Date.
const midnight = (day) => new Date(`${day}T00:00:00`);

// --- What is drawn --------------------------------------------------------

// The occurrences of the days shown, asked once per days and scope, and
// again after a sync brought new appointments or calendars.
let fetched = { key: null, data: null, error: null, for: null };
function occurrences(from, to) {
  const key = JSON.stringify([from, to, state.scope]);
  const current = [state.appointments, state.calendars];
  if (fetched.key === key && fetched.for?.every((x, i) => x === current[i])) return fetched;
  fetched = { key, data: fetched.key === key ? fetched.data : null, error: null, for: current };
  const asked = fetched;
  api
    .appointments(from, to, state.scope)
    .then((data) => {
      asked.data = data;
      if (fetched === asked) update({});
    })
    .catch((error) => {
      asked.error = error.message;
      if (fetched === asked) update({});
    });
  return fetched;
}

// Tasks planned at a time, as [start, end) with their status; a task
// planned for a whole day is not a block (ADR 0047 §11).
function plannedTasks() {
  const blocks = [];
  for (const status of state.statuses) {
    const task = status.task;
    if (!task.planned?.at || task.dropped_at) continue;
    const start = instant(task.planned.at);
    if (Number.isNaN(start.getTime())) continue;
    const length = minutes(task.planned_duration) ?? minutes(task.estimate) ?? TASK_MINUTES;
    blocks.push({ kind: "task", status, start, end: new Date(start.getTime() + length * 60000) });
  }
  return blocks;
}

// Appointments at a time as blocks; whole days go to the band.
function timedAppointments(list) {
  const blocks = [];
  for (const a of list) {
    if (!a.start.at || a.status === "cancelled") continue;
    const start = instant(a.start.at);
    const end = a.end?.at ? instant(a.end.at) : start;
    blocks.push({ kind: "appointment", occurrence: a, start, end: end < start ? start : end });
  }
  return blocks;
}

// A block's part on one day, in minutes from midnight, or null.
function onDay(block, day) {
  const from = midnight(day);
  const to = midnight(addDays(day, 1));
  const zeroLength = block.end.getTime() === block.start.getTime();
  if (block.start >= to || (zeroLength ? block.start < from : block.end <= from)) return null;
  const top = block.start <= from ? 0 : block.start.getHours() * 60 + block.start.getMinutes();
  const bottom = block.end >= to ? 24 * 60 : block.end.getHours() * 60 + block.end.getMinutes();
  return { ...block, top, bottom: Math.max(bottom, top + SHORTEST), continued: block.start < from };
}

// Side by side where blocks overlap: each gets a lane, and every block of
// a run of overlapping ones shares the run's number of lanes.
export function lanes(parts) {
  const sorted = [...parts].sort((a, b) => a.top - b.top || b.bottom - a.bottom);
  let run = [];
  let runEnd = -1;
  let ends = [];
  const close = () => {
    for (const part of run) part.lanes = ends.length;
    run = [];
    ends = [];
  };
  for (const part of sorted) {
    if (part.top >= runEnd) {
      close();
      runEnd = -1;
    }
    let lane = ends.findIndex((end) => end <= part.top);
    if (lane < 0) {
      lane = ends.length;
      ends.push(part.bottom);
    } else ends[lane] = part.bottom;
    part.lane = lane;
    run.push(part);
    runEnd = Math.max(runEnd, part.bottom);
  }
  close();
  return sorted;
}

// Whole-day appointments over the days shown: first and last column,
// `end` excluded as in iCalendar (ADR 0047 §4).
function bandBars(list, days) {
  const first = days[0];
  const last = days[days.length - 1];
  const bars = [];
  for (const a of list) {
    if (!a.start.date || a.status === "cancelled") continue;
    const start = a.start.date;
    const end = a.end?.date && a.end.date > start ? addDays(a.end.date, -1) : start;
    if (end < first || start > last) continue;
    const from = start < first ? 0 : days.indexOf(start);
    const to = end > last ? days.length - 1 : days.indexOf(end);
    bars.push({ occurrence: a, from, to, cutBefore: start < first, cutAfter: end > last });
  }
  return bars.sort((x, y) => x.from - y.from || y.to - y.from - (x.to - x.from));
}

// --- A day as lines (month and list) -----------------------------------------

// The local days an occurrence lies on: whole days up to the day before
// `end` (iCalendar), times from the day of their start to the day of their
// end, an end at midnight not counting (ADR 0100 §5, as Forecast).
export function daysOf(a) {
  if (a.start.date) {
    const last = a.end?.date && a.end.date > a.start.date ? addDays(a.end.date, -1) : a.start.date;
    return { first: a.start.date, last };
  }
  const start = instant(a.start.at);
  const end = a.end?.at ? instant(a.end.at) : start;
  const first = dayOfDate(start);
  if (end <= start) return { first, last: first };
  let last = dayOfDate(end);
  if (end.getTime() === midnight(last).getTime()) last = addDays(last, -1);
  return { first, last: last < first ? first : last };
}

// The day of a task's `planned`, in this browser's zone.
function plannedDay(task) {
  if (task.planned?.date) return task.planned.date;
  if (!task.planned?.at) return null;
  const at = instant(task.planned.at);
  return Number.isNaN(at.getTime()) ? null : dayOfDate(at);
}

// What lies on each day shown, as a map from day to lines: appointments
// over the whole day first, then those with a start, then the tasks
// planned on that day (ADR 0100 §5). Cancelled ones are missing as
// everywhere.
export function linesByDay(list, statuses, days) {
  const lines = new Map(days.map((day) => [day, []]));
  const first = days[0];
  const last = days[days.length - 1];
  for (const a of list) {
    if (a.status === "cancelled") continue;
    const on = daysOf(a);
    if (on.last < first || on.first > last) continue;
    const start = a.start.at ? instant(a.start.at) : null;
    for (let day = on.first < first ? first : on.first; day <= on.last && day <= last; day = addDays(day, 1)) {
      const continued = day !== on.first;
      const end = a.end?.at ? instant(a.end.at) : null;
      let when = null;
      if (start && !continued) when = clock(start);
      else if (start) when = end && day === on.last && dayOfDate(end) === day ? tr("bis {day}", { day: clock(end) }) : tr("ganzer Tag");
      lines.get(day).push({ kind: "appointment", occurrence: a, allDay: !start, when, order: start && !continued ? start.getTime() : -1 });
    }
  }
  for (const status of statuses) {
    const task = status.task;
    if (task.dropped_at) continue;
    const day = plannedDay(task);
    if (!day || !lines.has(day)) continue;
    const at = task.planned.at ? instant(task.planned.at) : null;
    lines.get(day).push({ kind: "task", status, allDay: !at, when: at ? clock(at) : null, order: at ? at.getTime() : -1 });
  }
  const rank = (line) => (line.kind === "task" ? 2 : line.allDay ? 0 : 1);
  for (const day of days) {
    lines.get(day).sort((x, y) => rank(x) - rank(y) || x.order - y.order || titleOf(x).localeCompare(titleOf(y)));
  }
  return lines;
}

const titleOf = (line) => (line.kind === "task" ? line.status.task.title : line.occurrence.title) ?? "";

// One line of a day: a button that opens the appointment or the task.
function lineNode(line) {
  if (line.kind === "task") {
    const task = line.status.task;
    return el(
      "button",
      {
        type: "button",
        class: `line task${isOpen(task) ? "" : " done"}${state.selected === task.id ? " selected" : ""}`,
        title: task.title,
        onclick: (event) => {
          event.stopPropagation();
          update({ selected: task.id, appointment: null, pane: "detail" });
        },
      },
      [line.when ? el("span", { class: "when", text: line.when }) : null, el("span", { class: "what", text: `☐ ${task.title}` })],
    );
  }
  const a = line.occurrence;
  const node = el(
    "button",
    {
      type: "button",
      class: `line appointment${line.allDay ? " allday" : ""}${a.status === "tentative" ? " tentative" : ""}${state.appointment === a ? " selected" : ""}`,
      title: [a.title, a.location].filter(Boolean).join(" · "),
      onclick: (event) => {
        event.stopPropagation();
        chooseAppointment(a);
      },
    },
    [
      line.when ? el("span", { class: "when", text: line.when }) : null,
      el("span", { class: "what", text: a.title }),
      a.location ? el("span", { class: "where", text: a.location }) : null,
    ],
  );
  return tinted(node, a.calendar);
}

// Lines a day of the month shows before „+N“.
const MONTH_LINES = 4;

const weekdayShort = new Intl.DateTimeFormat(locale(), { weekday: "short" });

function monthView(days, lines) {
  const month = choice.day.slice(0, 7);
  const names = days.slice(0, 7).map((day) => el("span", { class: "weekday", text: weekdayShort.format(new Date(`${day}T12:00:00`)) }));
  const cells = days.map((day) => {
    const all = lines.get(day);
    const shown = all.length > MONTH_LINES ? all.slice(0, MONTH_LINES - 1) : all;
    const rest = all.length - shown.length;
    return el(
      "div",
      {
        class: `cell${day.slice(0, 7) === month ? "" : " other"}${day === today() ? " today" : ""}${canMake() ? " makes" : ""}`,
        title: canMake() ? tr("Klick: ganztägiger Termin") : null,
        onclick: (event) => {
          if (event.target === event.currentTarget && canMake()) startNew({ day });
        },
      },
      [
        el("button", {
          type: "button",
          class: "date",
          title: tr("Diesen Tag zeigen"),
          text: String(Number(day.slice(8, 10))),
          onclick: () => choose({ mode: "day", day }),
        }),
        ...shown.map(lineNode),
        rest
          ? el("button", { type: "button", class: "more", title: tr("Diesen Tag zeigen"), text: `+${rest}`, onclick: () => choose({ mode: "day", day }) })
          : null,
      ],
    );
  });
  return el("div", { class: "cal-month" }, [el("div", { class: "weekdays" }, names), el("div", { class: "weeks" }, cells)]);
}

const listDay = new Intl.DateTimeFormat(locale(), { weekday: "long", day: "numeric", month: "long" });

function listView(days, lines, loading) {
  const groups = days
    .filter((day) => lines.get(day).length)
    .map((day) =>
      el("section", { class: `listday${day === today() ? " today" : ""}` }, [
        el("button", {
          type: "button",
          class: "listhead",
          title: tr("Diesen Tag zeigen"),
          text: day === today() ? `Heute, ${listDay.format(new Date(`${day}T12:00:00`))}` : listDay.format(new Date(`${day}T12:00:00`)),
          onclick: () => choose({ mode: "day", day }),
        }),
        ...lines.get(day).map(lineNode),
      ]),
    );
  return el("div", { class: "cal-list" }, [
    !groups.length && !loading ? el("p", { class: "empty", text: tr("Nichts in den nächsten {n} Tagen.", { n: days.length }) }) : null,
    ...groups,
    // At most 400 days are asked at once (samu-core).
    days.length + LIST_DAYS <= 400
      ? el("button", { type: "button", class: "list-more", text: tr("Mehr"), onclick: () => choose({ listDays: choice.listDays + LIST_DAYS }) })
      : null,
  ]);
}

// --- Drawing ----------------------------------------------------------------

// The time on a block: its start, or where it ends when it began on an
// earlier day.
const startWords = (part) => {
  if (!part.continued) return clock(part.start);
  return part.bottom >= 24 * 60 ? tr("ganzer Tag") : tr("bis {day}", { day: clock(part.end) });
};

const chooseAppointment = (occurrence) => update({ appointment: occurrence, selected: null, pane: "detail" });

function appointmentBlock(part) {
  const a = part.occurrence;
  const tentative = a.status === "tentative";
  const node = el(
    "button",
    {
      type: "button",
      class: `block appointment${tentative ? " tentative" : ""}${state.appointment === a ? " selected" : ""}`,
      title: [a.title, a.location].filter(Boolean).join(" · "),
      onclick: () => chooseAppointment(a),
    },
    [
      el("span", { class: "when", text: startWords(part) }),
      el("span", { class: "what", text: a.title }),
      a.location ? el("span", { class: "where", text: a.location }) : null,
    ],
  );
  return tinted(node, a.calendar);
}

function taskBlock(part) {
  const task = part.status.task;
  return el(
    "button",
    {
      type: "button",
      class: `block task${isOpen(task) ? "" : " done"}${state.selected === task.id ? " selected" : ""}`,
      title: task.title,
      onclick: () => update({ selected: task.id, appointment: null, pane: "detail" }),
    },
    [el("span", { class: "when", text: `${startWords(part)} ☐` }), el("span", { class: "what", text: task.title })],
  );
}

function place(node, part) {
  node.style.top = `${(part.top / 60) * HOUR}px`;
  node.style.height = `${((part.bottom - part.top) / 60) * HOUR - 2}px`;
  node.style.left = `calc(${(part.lane / part.lanes) * 100}% + 2px)`;
  node.style.width = `calc(${100 / part.lanes}% - 4px)`;
  // Too short for two lines: time and title side by side.
  if (part.bottom - part.top < 50) node.classList.add("short");
  return node;
}

function column(day, blocks) {
  const parts = lanes(blocks.map((b) => onDay(b, day)).filter(Boolean));
  const children = parts.map((part) => place(part.kind === "task" ? taskBlock(part) : appointmentBlock(part), part));
  if (day === today()) {
    const now = new Date();
    const line = el("div", { class: "now", "aria-hidden": "true" });
    line.style.top = `${((now.getHours() * 60 + now.getMinutes()) / 60) * HOUR}px`;
    children.push(line);
  }
  return el("div", { class: `col${day === today() ? " today" : ""}${canMake() ? " makes" : ""}`, onclick: (event) => makeAt(event, day) }, children);
}

// Whether a click on a free place makes an appointment: only with a
// calendar to write into (ADR 0068 §1).
const canMake = () => writableCalendars().length > 0;

// A click on a free place of a day's column: a new appointment of an hour
// from the half hour clicked.
function makeAt(event, day) {
  if (event.target !== event.currentTarget || !canMake()) return;
  const y = event.offsetY;
  const half = Math.max(0, Math.min(47, Math.floor((y / HOUR) * 2)));
  const start = new Date(`${day}T${pad(Math.floor(half / 2))}:${half % 2 ? "30" : "00"}:00`);
  startNew({ start });
}

// A click on a free place of the band: a new appointment over that day.
function makeAllDay(event, days) {
  if (event.target !== event.currentTarget || !canMake()) return;
  const box = event.currentTarget.getBoundingClientRect();
  const index = Math.floor(((event.clientX - box.left) / box.width) * days.length);
  const day = days[Math.max(0, Math.min(days.length - 1, index))];
  startNew({ day });
}

const monthYear = new Intl.DateTimeFormat(locale(), { month: "long", year: "numeric" });
const longDay = new Intl.DateTimeFormat(locale(), { weekday: "long", day: "numeric", month: "long", year: "numeric" });

function heading(days) {
  if (choice.mode === "month") return monthYear.format(new Date(`${choice.day.slice(0, 8)}15T12:00:00`));
  if (choice.mode === "list") return tr("Ab heute, {n} Tage", { n: days.length });
  if (days.length === 1) return longDay.format(new Date(`${days[0]}T12:00:00`));
  const first = new Date(`${days[0]}T12:00:00`);
  const last = new Date(`${days[days.length - 1]}T12:00:00`);
  const a = monthYear.format(first);
  const b = monthYear.format(last);
  return a === b ? a : `${a} – ${b}`;
}

// The day one arrow leads to: a day, a week or a month on.
function stepped(direction) {
  if (choice.mode === "month") return addMonths(choice.day, direction);
  return addDays(choice.day, direction * (choice.mode === "day" ? 1 : 7));
}

function toolbar(days, loading) {
  return el("div", { class: "cal-controls" }, [
    // The list always starts today; there is nothing to page through.
    choice.mode === "list"
      ? null
      : el("div", { class: "nav", role: "group", "aria-label": tr("Blättern") }, [
          el("button", { type: "button", "aria-label": tr("zurück"), title: tr("zurück"), text: "‹", onclick: () => choose({ day: stepped(-1) }) }),
          el("button", { type: "button", text: tr("Heute"), onclick: () => choose({ day: today() }) }),
          el("button", { type: "button", "aria-label": tr("weiter"), title: tr("weiter"), text: "›", onclick: () => choose({ day: stepped(1) }) }),
        ]),
    el("h2", { text: heading(days) }),
    loading ? el("span", { class: "quiet", text: tr("lädt …") }) : null,
    el(
      "div",
      { class: "modes", role: "group", "aria-label": tr("Ansicht") },
      [
        ["day", tr("Ansicht|Tag")],
        ["week", tr("Ansicht|Woche")],
        ["month", tr("Ansicht|Monat")],
        ["list", tr("Liste")],
      ].map(([mode, title]) =>
        el("button", { type: "button", class: choice.mode === mode ? "current" : null, "aria-pressed": String(choice.mode === mode), text: title, onclick: () => choose({ mode }) }),
      ),
    ),
  ]);
}

// Where the hours were scrolled to, kept across drawing; 7:00 at first.
let scrolled = 7 * HOUR;

export function calendarView() {
  const days = shownDays();
  const { data, error } = occurrences(days[0], days[days.length - 1]);
  const list = data ?? [];
  if (choice.mode === "month" || choice.mode === "list") {
    const lines = linesByDay(list, state.statuses, days);
    return el("div", { class: `cal cal-${choice.mode}` }, [
      toolbar(days, !data && !error),
      error ? el("p", { class: "empty", text: `${tr("Termine nicht geladen:")} ${error}` }) : null,
      choice.mode === "month" ? monthView(days, lines) : listView(days, lines, !data && !error),
    ]);
  }
  const blocks = [...timedAppointments(list), ...plannedTasks()];
  const bars = bandBars(list, days);
  const columns = `--days: ${days.length}`;

  const head = el(
    "div",
    { class: "cal-head", style: columns },
    [
      el("span", { class: "gutter" }),
      ...days.map((day) =>
        el("button", {
          type: "button",
          class: `dayhead${day === today() ? " today" : ""}`,
          title: choice.mode === "week" ? tr("Diesen Tag zeigen") : null,
          text: shortDay({ date: day }),
          onclick: () => choose({ mode: "day", day }),
        }),
      ),
    ],
  );

  const band = el("div", { class: "cal-band", style: columns }, [
    el("span", { class: "gutter", text: tr("ganztägig") }),
    el(
      "div",
      { class: `bars${canMake() ? " makes" : ""}`, title: canMake() ? tr("Klick: ganztägiger Termin") : null, onclick: (event) => makeAllDay(event, days) },
      bars.map((bar) => {
        const a = bar.occurrence;
        const node = el(
          "button",
          {
            type: "button",
            class: `bar${a.status === "tentative" ? " tentative" : ""}${bar.cutBefore ? " cut-before" : ""}${bar.cutAfter ? " cut-after" : ""}${state.appointment === a ? " selected" : ""}`,
            title: [a.title, a.location].filter(Boolean).join(" · "),
            text: a.title,
            onclick: () => chooseAppointment(a),
          },
        );
        node.style.gridColumn = `${bar.from + 1} / ${bar.to + 2}`;
        return tinted(node, a.calendar);
      }),
    ),
  ]);

  const hours = el(
    "div",
    { class: "hours", "aria-hidden": "true" },
    Array.from({ length: 24 }, (_, h) => el("span", { text: h ? `${pad(h)}:00` : "" })),
  );
  const body = el(
    "div",
    { class: "cal-body", onscroll: (event) => (scrolled = event.target.scrollTop) },
    el("div", { class: "cal-grid", style: `${columns}; --hour: ${HOUR}px` }, [hours, ...days.map((day) => column(day, blocks))]),
  );
  // Drawn into the page by the caller; then the hours go back where they were.
  queueMicrotask(() => {
    body.scrollTop = scrolled;
  });

  return el("div", { class: `cal cal-${choice.mode}` }, [
    toolbar(days, !data && !error),
    error ? el("p", { class: "empty", text: `${tr("Termine nicht geladen:")} ${error}` }) : null,
    head,
    bars.length || canMake() ? band : null,
    body,
  ]);
}
