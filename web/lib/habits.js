// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Habits (ADR 0091): a task with the field `habit`. What it is today, its
// streak and its history come from GET /habits, computed in samu-core;
// here they are drawn, and taps become entries over /changes.

import { api, change } from "./api.js";
import { el, uuidv7 } from "./dom.js";
import { state, update } from "./state.js";
import { addDays } from "./words.js";
import { tr } from "./i18n.js";

// How many weeks the calendar of the inspector shows.
export const WEEKS = 12;

// The status of a habit by its task, or null.
export function habitOf(id) {
  return state.habits.find((h) => h.task.id === id) ?? null;
}

const PER = { day: tr("Habit|am Tag"), week: tr("pro Woche"), month: tr("im Monat") };

// The goal in words, like „3× pro Woche“, „8 Glas am Tag“, „vermeiden“.
export function goalText(habit) {
  if (!habit) return "";
  if (habit.kind === "avoid") return tr("vermeiden");
  const per = PER[habit.per] ?? habit.per;
  if (habit.unit) return `${habit.times} ${habit.unit} ${per}`;
  return habit.times === 1 && habit.per === "day" ? tr("täglich") : tr("{n}× {per}", { n: habit.times, per });
}

async function write(changes) {
  try {
    await api.write(changes);
    update({ message: null });
  } catch (error) {
    update({ message: error.message });
  }
}

// An entry for `day` (ADR 0091 §2): every tap is one.
export function enter(task, status, day, extra = {}) {
  const now = new Date().toISOString();
  const entry = { id: uuidv7(), created: now, modified: now, habit: task, day, status, ...extra };
  return write([change.create("habit_entry", entry)]);
}

// The day the server counts as today: its own, in the account's zone.
const todayOf = (h) => h.today;

// What stands in a row instead of the tick (ADR 0091 §6): ✓ to build, +1
// with the amount when it has a unit, „frei seit“ with „Rückfall“ to
// avoid; the streak beside it.
export function habitControl(status) {
  const h = habitOf(status.task.id);
  if (!h) return null;
  const habit = h.task.habit;
  const id = status.task.id;
  const stop = (run) => (event) => {
    event.stopPropagation();
    run();
  };
  const streak = el("span", {
    class: "habit-streak",
    title: tr("Serie {streak}, längste {longest}", { streak: h.streak, longest: h.longest }),
    text: h.streak ? `🔥${h.streak}` : "",
  });
  if (habit.kind === "avoid") {
    return el("span", { class: "habit-control" }, [
      el("span", { class: `habit-free${h.day === "fail" ? " failed" : ""}`, text: tr("frei seit {n} T", { n: h.streak }) }),
      el("button", {
        type: "button",
        class: "habit-relapse",
        title: tr("Rückfall heute"),
        text: tr("Rückfall"),
        onclick: stop(() => enter(id, "fail", todayOf(h))),
      }),
    ]);
  }
  const done = h.period.met;
  const tap = stop(() => enter(id, "done", todayOf(h)));
  if (habit.unit || h.period.target > 1) {
    return el("span", { class: "habit-control" }, [
      el("button", {
        type: "button",
        class: `habit-tap${done ? " met" : ""}`,
        title: `+1${habit.unit ? ` ${habit.unit}` : ""}`,
        text: "+1",
        onclick: tap,
      }),
      el("span", {
        class: `habit-amount${done ? " met" : ""}`,
        text: `${h.period.amount}/${h.period.target}`,
      }),
      streak,
    ]);
  }
  return el("span", { class: "habit-control" }, [
    el("button", {
      type: "button",
      class: `habit-tap${done ? " met" : ""}`,
      "aria-pressed": done ? "true" : "false",
      title: done ? tr("heute geschafft") : tr("heute geschafft?"),
      text: "✓",
      onclick: tap,
    }),
    streak,
  ]);
}

// The entries of the context menu for a habit (ADR 0091 §6).
export function habitMenu(task) {
  const h = habitOf(task.id);
  if (!h) return [];
  const day = todayOf(h);
  const avoid = h.task.habit.kind === "avoid";
  return [
    { label: "Habit", header: true },
    avoid ? null : { label: tr("Heute geschafft"), run: () => enter(task.id, "done", day) },
    { label: avoid ? tr("Rückfall heute") : tr("Heute nicht geschafft"), run: () => enter(task.id, "fail", day) },
    { label: tr("Pause heute"), run: () => enter(task.id, "pause", day) },
    "-",
  ].filter(Boolean);
}

// The day picked in the calendar of the inspector, per task.
let picked = null;

const STATE_WORDS = { done: tr("geschafft"), partial: tr("teils"), fail: tr("nicht geschafft"), pause: tr("Pause"), none: tr("nichts") };
const ENTRY_WORDS = { done: tr("geschafft"), fail: tr("nicht geschafft"), pause: tr("Pause") };

// The part `series` of the inspector (ADR 0091 §6): goal, streak, rate,
// the last weeks as a calendar from Monday, and the entries of a picked
// day with buttons to enter there afterwards.
export function seriesSection(task, section) {
  if (!task.habit) return null;
  const h = habitOf(task.id);
  if (!h) return section(tr("Habit|Serie"), el("p", { class: "quiet", text: goalText(task.habit) }));
  const avoid = task.habit.kind === "avoid";
  const rate = (r) => (r.of ? `${Math.round((100 * r.met) / r.of)} % (${r.met}/${r.of})` : "–");
  const facts = el("dl", { class: "habit-facts" }, [
    el("dt", { text: tr("Habit|Ziel") }),
    el("dd", { text: goalText(task.habit) }),
    el("dt", { text: avoid ? tr("frei seit") : tr("Habit|Serie") }),
    el("dd", { text: avoid ? `${h.streak} Tagen` : `${h.streak}` }),
    el("dt", { text: tr("längste") }),
    el("dd", { text: `${h.longest}` }),
    el("dt", { text: tr("4 Wochen") }),
    el("dd", { text: rate(h.recent) }),
    el("dt", { text: tr("dieses Jahr") }),
    el("dd", { text: rate(h.year) }),
    !avoid && h.task.habit.per !== "day"
      ? el("dt", { text: h.task.habit.per === "week" ? tr("diese Woche") : tr("dieser Monat") })
      : null,
    !avoid && h.task.habit.per !== "day" ? el("dd", { text: `${h.period.amount}/${h.period.target}` }) : null,
  ]);
  return section(tr("Habit|Serie"), el("div", { class: "habit-series" }, [facts, calendar(task, h), dayEntries(task, h)]));
}

// Monday-based weeks as columns, today in the last; future days empty.
function calendar(task, h) {
  const byDay = new Map(h.history.map((d) => [d.day, d]));
  const today = h.today;
  const weekday = (new Date(`${today}T12:00:00`).getDay() + 6) % 7;
  const start = addDays(today, -weekday - 7 * (WEEKS - 1));
  const columns = [];
  for (let w = 0; w < WEEKS; w += 1) {
    const cells = [];
    for (let d = 0; d < 7; d += 1) {
      const day = addDays(start, w * 7 + d);
      if (day > today) {
        cells.push(el("span", { class: "habit-day future" }));
        continue;
      }
      const s = byDay.get(day);
      const kind = s?.state ?? "none";
      const chosen = picked?.task === task.id && picked.day === day;
      cells.push(
        el("button", {
          type: "button",
          class: `habit-day ${kind}${chosen ? " picked" : ""}${day === today ? " today" : ""}`,
          title: `${day}: ${STATE_WORDS[kind] ?? kind}${s?.amount > 1 ? ` (${s.amount})` : ""}`,
          "aria-label": `${day}: ${STATE_WORDS[kind] ?? kind}`,
          onclick: () => {
            picked = chosen ? null : { task: task.id, day };
            update({});
          },
        }),
      );
    }
    columns.push(el("div", { class: "habit-week" }, cells));
  }
  return el("div", { class: "habit-calendar", role: "group", "aria-label": tr("Verlauf") }, columns);
}

// The picked day: its entries to take back, and buttons to enter there.
function dayEntries(task, h) {
  if (picked?.task !== task.id) {
    return el("p", { class: "quiet", text: tr("Ein Tag zum Nachtragen: anklicken.") });
  }
  const day = picked.day;
  const entries = h.entries.filter((e) => e.day === day || (e.until && e.day <= day && day <= e.until));
  const avoid = task.habit.kind === "avoid";
  return el("div", { class: "habit-picked" }, [
    el("h5", { text: day }),
    entries.length
      ? el(
          "ul",
          { class: "habit-entries" },
          entries.map((e) =>
            el("li", {}, [
              el("span", {
                text: `${ENTRY_WORDS[e.status] ?? e.status}${e.amount ? ` ${e.amount}` : ""}${e.until ? ` ${tr("bis {day}", { day: e.until })}` : ""}`,
              }),
              el("button", {
                type: "button",
                class: "remove",
                "aria-label": tr("Eintrag löschen"),
                text: "✕",
                onclick: () => write([change.delete(e.id)]),
              }),
            ]),
          ),
        )
      : el("p", { class: "quiet", text: tr("Kein Eintrag.") }),
    el("div", { class: "habit-enter" }, [
      avoid ? null : el("button", { type: "button", text: tr("geschafft"), onclick: () => enter(task.id, "done", day) }),
      el("button", { type: "button", text: avoid ? tr("Rückfall") : tr("nicht geschafft"), onclick: () => enter(task.id, "fail", day) }),
      el("button", { type: "button", text: tr("Pause"), onclick: () => enter(task.id, "pause", day) }),
    ]),
  ]);
}

// How far back the history is asked: the calendar's first Monday.
export function historyFrom(day) {
  const weekday = (new Date(`${day}T12:00:00`).getDay() + 6) % 7;
  return addDays(day, -weekday - 7 * (WEEKS - 1));
}
