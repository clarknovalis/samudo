// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The search language of ADR 0042: words that become chips, and whether a
// task matches the chips and the free text. Only reads state; draws
// nothing. Availability, state, due and today come from the server
// (ADR 0006); the date ranges here are days of the local calendar.

import { subtree } from "./placeops.js";
import { isKey, SYSTEM } from "./props.js";
import { byId, holdsUp, isOpen, state } from "./state.js";
import { lang } from "./i18n.js";
import { addDays, dayOf, localDay, pathOf, tagPath, today } from "./words.js";

const lower = (text) => text.toLocaleLowerCase("de");

// The date fields, by the word before the colon.
export const DATES = {
  fällig: "due",
  geplant: "planned",
  start: "defer",
  erledigt: "completed",
  erstellt: "created",
  geändert: "modified",
};
const DATE_NAME = Object.fromEntries(Object.entries(DATES).map(([word, kind]) => [kind, word]));

// German and English (ADR 0105 §3), as samu_core::query::lang: both are
// read always, a chip keeps the German word and is shown in the language
// of the page.
const KEYS = [
  ["fällig", "due"],
  ["geplant", "planned"],
  ["start", "defer"],
  ["erledigt", "done"],
  ["erstellt", "created"],
  ["geändert", "modified"],
  ["ordner", "folder"],
  ["zustand", "state"],
  ["ist", "is"],
  ["prio", "prio"],
  ["prop", "prop"],
];
const VALUES = [
  ["heute", "today"],
  ["morgen", "tomorrow"],
  ["gestern", "yesterday"],
  ["woche", "week"],
  ["überfällig", "overdue"],
  ["mit", "any"],
  ["ohne", "none"],
  ["markiert", "flagged"],
  ["verfügbar", "available"],
  ["wartet", "waiting"],
  ["offen", "open"],
  ["erledigt", "done"],
  ["verworfen", "dropped"],
  ["gruppe", "group"],
  ["notiz", "note"],
  ["wiederholt", "repeating"],
  ["geclockt", "clocked"],
  ["blocker", "blocker"],
  ["blockiert", "blocked"],
  ["termin", "appointment"],
];
const SPELLED = { faellig: "fällig", geaendert: "geändert" };
// A key in either language (lower case) as the German one, else null.
const canonicalKey = (key) => SPELLED[key] ?? KEYS.find(([de, en]) => de === key || en === key)?.[0] ?? null;
const keyIn = (key, l) => (l === "en" ? (KEYS.find(([de]) => de === key)?.[1] ?? key) : key);
const days = (word, from, to) => {
  const m = new RegExp(`^(-?\\d+)${from}$`).exec(word);
  return m ? `${m[1]}${to}` : null;
};
// A value in either language as the German one; a date or name as it is.
const canonicalValue = (value) => VALUES.find(([, en]) => en === value)?.[0] ?? days(value, "d", "t") ?? value;
const valueIn = (value, l) => (l === "en" ? (VALUES.find(([de]) => de === value)?.[1] ?? days(value, "t", "d") ?? value) : value);

const AHEAD = ["heute", "morgen", "woche", "7t", "30t", "mit", "ohne"];
const BEHIND = ["heute", "gestern", "-7t", "-30t", "mit", "ohne"];
// What the list offers after each date word; any range works with any.
export const RANGES = {
  due: ["überfällig", ...AHEAD],
  planned: ["überfällig", ...AHEAD],
  defer: AHEAD,
  completed: BEHIND,
  created: BEHIND,
  modified: BEHIND,
};

export const IS = ["markiert", "verfügbar", "wartet", "offen", "erledigt", "verworfen", "gruppe", "notiz", "wiederholt", "geclockt", "blocker", "blockiert", "termin"];

// `ist:termin` turns the search to appointments (ADR 0047 §14); a scope's
// filter does not take it, so its field does not offer it.
export const wantsAppointments = (chips) => chips.some((c) => c.kind === "is" && c.value === "termin" && !c.not);

// The tasks with a clock on them (ADR 0046 §6), made again when the clocks change.
let clocked = { clocks: null, ids: new Set() };
function clockedIds() {
  if (clocked.clocks !== state.clocks) clocked = { clocks: state.clocks, ids: new Set(state.clocks.map((c) => c.owner)) };
  return clocked.ids;
}
const ESTIMATES = ["<15m", "<30m", "<1h", ">1h", "mit", "ohne"];
// `prio:` (ADR 0056 §4); `prio:-` is `prio:ohne`.
const PRIORITIES = ["a", "b", "c", "mit", "ohne"];

// What the list offers in an empty field: the kinds themselves.
export const KINDS = [
  ["#", "#Projekt"],
  ["@", "@Tag"],
  ["ordner:", "ordner:Ordner"],
  ["fällig:", "fällig:woche"],
  ["geplant:", "geplant:heute"],
  ["start:", "start:morgen"],
  ["erledigt:", "erledigt:-7t"],
  ["erstellt:", "erstellt:-7t"],
  ["geändert:", "geändert:heute"],
  ["ist:", "ist:markiert"],
  ["zustand:", "zustand:Name"],
  ["prop:", "prop:schlüssel"],
  ["prio:", "prio:a"],
  ["~", "~<15m"],
];
const KINDS_EN = [
  ["#", "#Project"],
  ["@", "@Tag"],
  ["folder:", "folder:Folder"],
  ["due:", "due:week"],
  ["planned:", "planned:today"],
  ["defer:", "defer:tomorrow"],
  ["done:", "done:-7d"],
  ["created:", "created:-7d"],
  ["modified:", "modified:today"],
  ["is:", "is:flagged"],
  ["state:", "state:Name"],
  ["prop:", "prop:key"],
  ["prio:", "prio:a"],
  ["~", "~<15m"],
];

const folderPath = (folder) => pathOf(state.folders, folder);
const workflowStates = () => state.tags.filter((t) => t.parent && byId(state.tags, t.parent)?.exclusive);

// --- Ranges -----------------------------------------------------------

const ISO = /^\d{4}-\d{2}-\d{2}$/;

// A range word as { from, to } (days, inclusive, null = open), or
// { overdue }, { any }, { none }; null when it is no range.
export function rangeOf(word) {
  const now = today();
  switch (word) {
    case "heute":
      return { from: now, to: now };
    case "morgen":
      return { from: addDays(now, 1), to: addDays(now, 1) };
    case "gestern":
      return { from: addDays(now, -1), to: addDays(now, -1) };
    case "woche": {
      const sunday = (7 - new Date(`${now}T12:00:00`).getDay()) % 7;
      return { from: now, to: addDays(now, sunday) };
    }
    case "überfällig":
      return { overdue: true };
    case "mit":
      return { any: true };
    case "ohne":
      return { none: true };
    default:
      break;
  }
  const days = /^(-?)(\d{1,4})t$/.exec(word);
  if (days) {
    const n = Number(days[2]);
    return days[1] ? { from: addDays(now, -n), to: now } : { from: now, to: addDays(now, n) };
  }
  if (ISO.test(word)) return { from: word, to: word };
  const span = /^(\d{4}-\d{2}-\d{2})?\.\.(\d{4}-\d{2}-\d{2})?$/.exec(word);
  if (span && (span[1] || span[2])) return { from: span[1] ?? null, to: span[2] ?? null };
  return null;
}

function dayFor(status, kind) {
  const task = status.task;
  switch (kind) {
    case "due":
    case "planned":
    case "defer":
      return dayOf(task[kind]);
    case "completed":
      return localDay(task.completed_at);
    default:
      return localDay(task[kind]);
  }
}

function inRange(status, kind, range) {
  const day = dayFor(status, kind);
  if (range.none) return !day;
  if (!day) return false;
  if (range.any) return true;
  if (range.overdue) {
    if (kind === "due") return status.due === "overdue";
    return isOpen(status.task) && day < today();
  }
  return (!range.from || day >= range.from) && (!range.to || day <= range.to);
}

// --- Estimates --------------------------------------------------------

// "PT1H30M" → 90; null when there is none.
export function minutes(period) {
  const m = /^PT(?:(\d+)H)?(?:(\d+)M)?$/.exec(period ?? "");
  if (!m || (!m[1] && !m[2])) return null;
  return Number(m[1] ?? 0) * 60 + Number(m[2] ?? 0);
}

function estimateTest(word) {
  if (word === "mit") return (n) => n !== null;
  if (word === "ohne") return (n) => n === null;
  const m = /^(<=|>=|<|>|=)?(\d+)(m|h)$/.exec(word);
  if (!m) return null;
  const limit = Number(m[2]) * (m[3] === "h" ? 60 : 1);
  const op = m[1] ?? "=";
  return (n) => {
    if (n === null) return false;
    if (op === "<") return n < limit;
    if (op === "<=") return n <= limit;
    if (op === ">") return n > limit;
    if (op === ">=") return n >= limit;
    return n === limit;
  };
}

// --- Words and chips --------------------------------------------------

const unquote = (text) => (text.length > 1 && text.startsWith('"') && text.endsWith('"') ? text.slice(1, -1) : text);
const quoted = (name) => (/\s/.test(name) ? `"${name}"` : name);

// The words of a search text; a name in quotes stays one word.
export const WORD = /-?(?:[#@~]|[\p{L}]+:)"[^"]*"|\S+/gu;

function findBy(list, name, path) {
  const want = lower(name);
  const exact = list.filter((o) => lower(path(o)) === want);
  if (exact.length === 1) return exact[0];
  const titled = list.filter((o) => lower(o.title) === want);
  return titled.length === 1 ? titled[0] : null;
}

// A word as a chip { kind, value, not }, or null when it is free text.
export function parseWord(word) {
  let text = word;
  const not = text.startsWith("-") && text.length > 1;
  if (not) text = text.slice(1);
  const chip = (kind, value) => ({ kind, value, not });
  if (text.startsWith("#")) {
    const name = unquote(text.slice(1));
    if (!name) return null;
    if (lower(name) === "inbox") return chip("project", "inbox");
    const project = findBy(state.projects, name, (p) => p.title);
    return project ? chip("project", project.id) : null;
  }
  if (text.startsWith("@")) {
    const name = unquote(text.slice(1));
    const tag = name ? findBy(state.tags, name, tagPath) : null;
    return tag ? chip("tag", tag.id) : null;
  }
  if (text.startsWith("~")) {
    const value = canonicalValue(text.slice(1));
    return estimateTest(value) ? chip("estimate", value) : null;
  }
  const colon = /^([\p{L}]+):(.+)$/u.exec(text);
  if (!colon) return null;
  const key = canonicalKey(lower(colon[1]));
  const value = unquote(colon[2]);
  if (DATES[key]) {
    const v = canonicalValue(lower(value));
    return rangeOf(v) ? chip(DATES[key], v) : null;
  }
  if (key === "ordner") {
    const folder = findBy(state.folders, value, folderPath);
    return folder ? chip("folder", folder.id) : null;
  }
  if (key === "ist") {
    const v = canonicalValue(lower(value));
    return IS.includes(v) ? chip("is", v) : null;
  }
  if (key === "prio") {
    const v = value === "-" ? "ohne" : canonicalValue(lower(value));
    return PRIORITIES.includes(v) ? chip("priority", v) : null;
  }
  if (key === "zustand") {
    const tag = findBy(workflowStates(), value, (t) => t.title);
    return tag ? chip("state", tag.id) : null;
  }
  // ADR 0054 §8: `prop:jira` has it, `prop:jira=HAUS-12` has this value.
  if (key === "prop") {
    const at = value.indexOf("=");
    const name = lower(at < 0 ? value : value.slice(0, at));
    if (!isKey(name)) return null;
    return chip("property", at < 0 ? name : `${name}=${value.slice(at + 1)}`);
  }
  return null;
}

// The value of a property at a task: its own, else the inherited one.
const propertyOf = (s, key) => s.task.properties?.[key] ?? s.inherited?.[key]?.value;

// A chip as the word that makes it, in the language of the page.
export function chipText(chip, l = lang()) {
  const sign = chip.not ? "-" : "";
  const key = (de) => keyIn(de, l);
  const value = (de) => valueIn(de, l);
  switch (chip.kind) {
    case "project":
      return `${sign}#${chip.value === "inbox" ? "Inbox" : quoted(byId(state.projects, chip.value)?.title ?? "?")}`;
    case "tag": {
      const tag = byId(state.tags, chip.value);
      return `${sign}@${tag ? quoted(tagPath(tag)) : "?"}`;
    }
    case "folder": {
      const folder = byId(state.folders, chip.value);
      return `${sign}${key("ordner")}:${folder ? quoted(folderPath(folder)) : "?"}`;
    }
    case "state":
      return `${sign}${key("zustand")}:${quoted(byId(state.tags, chip.value)?.title ?? "?")}`;
    case "is":
      return `${sign}${key("ist")}:${value(chip.value)}`;
    case "estimate":
      return `${sign}~${value(chip.value)}`;
    case "priority":
      return `${sign}prio:${value(chip.value)}`;
    case "property":
      return `${sign}prop:${quoted(chip.value)}`;
    default:
      return `${sign}${key(DATE_NAME[chip.kind])}:${value(chip.value)}`;
  }
}

// The colour a chip takes, as in the capture window.
export const chipClass = (chip) =>
  ({ project: "project", folder: "project", tag: "tag", state: "tag", due: "due" })[chip.kind] ?? "plain";

// --- Matching ---------------------------------------------------------

function test(chip) {
  switch (chip.kind) {
    case "project":
      return chip.value === "inbox"
        ? (s) => s.inbox
        : (s) => !s.inbox && s.task.project === chip.value;
    case "folder": {
      const folders = subtree(state.folders, chip.value);
      return (s) => {
        const project = !s.inbox && s.task.project ? byId(state.projects, s.task.project) : null;
        return Boolean(project?.folder && folders.has(project.folder));
      };
    }
    case "tag": {
      const tags = subtree(state.tags, chip.value);
      return (s) => (s.task.tags ?? []).some((t) => tags.has(t.tag));
    }
    case "state":
      return (s) => s.state === chip.value;
    case "estimate": {
      const fits = estimateTest(chip.value);
      return (s) => fits(minutes(s.task.estimate));
    }
    case "is":
      return isTest(chip.value);
    case "priority":
      if (chip.value === "mit") return (s) => Boolean(s.task.priority);
      if (chip.value === "ohne") return (s) => !s.task.priority;
      return (s) => lower(s.task.priority ?? "") === chip.value;
    case "property": {
      const at = chip.value.indexOf("=");
      const key = at < 0 ? chip.value : chip.value.slice(0, at);
      const want = at < 0 ? null : lower(chip.value.slice(at + 1));
      return (s) => {
        const have = propertyOf(s, key);
        return have !== undefined && (want === null || lower(have) === want);
      };
    }
    default: {
      const range = rangeOf(chip.value);
      return (s) => inRange(s, chip.kind, range);
    }
  }
}

function isTest(word) {
  switch (word) {
    case "markiert":
      return (s) => s.task.flagged;
    case "verfügbar":
      return (s) => isOpen(s.task) && s.available;
    case "wartet":
      return (s) => isOpen(s.task) && !s.available;
    case "offen":
      return (s) => isOpen(s.task);
    case "erledigt":
      return (s) => Boolean(s.task.completed_at);
    case "verworfen":
      return (s) => Boolean(s.task.dropped_at);
    case "gruppe":
      return (s) => Boolean(s.group);
    case "notiz":
      return (s) => Boolean(s.task.note?.trim());
    case "wiederholt":
      return (s) => Boolean(s.task.repeat || s.task.repeat_of);
    case "geclockt":
      return (s) => clockedIds().has(s.task.id);
    case "blocker":
      return (s) => holdsUp(s.task.id);
    // The reason samu-core found (53a0d8e9), its own blocker or its project's.
    case "blockiert":
      return (s) => isOpen(s.task) && (s.reasons ?? []).some((r) => r.reason === "blocked_by");
    // A task is no appointment; `-ist:termin` is the search as before.
    case "termin":
      return () => false;
    default:
      return () => false;
  }
}

// One test for the whole search: chips of one kind OR, kinds AND, a
// negated chip must not hold; every free word in title, note or id, so
// the end of an id finds its task (01a1c271).
export function matcher(chips, text) {
  const kinds = new Map();
  for (const chip of chips) {
    if (!kinds.has(chip.kind)) kinds.set(chip.kind, { any: [], none: [] });
    kinds.get(chip.kind)[chip.not ? "none" : "any"].push(test(chip));
  }
  const words = lower(text).split(/\s+/).filter(Boolean);
  return (status) => {
    for (const { any, none } of kinds.values()) {
      if (any.length && !any.some((t) => t(status))) return false;
      if (none.some((t) => t(status))) return false;
    }
    if (!words.length) return true;
    const task = status.task;
    const hay = lower(`${task.title}\n${task.note ?? ""}\n${task.id}`);
    return words.every((w) => hay.includes(w));
  };
}

// A chip that asks for what is closed: the filter then acts as „Alle“
// (ADR 0042 §4).
export const asksForClosed = (chips) =>
  chips.some(
    (c) =>
      !c.not &&
      ((c.kind === "is" && (c.value === "erledigt" || c.value === "verworfen")) ||
        (c.kind === "completed" && c.value !== "ohne")),
  );

// A chip that asks for what waits (`ist:wartet`, `ist:blockiert`, `start:`):
// „Verfügbar“ then acts as „Verbleibend“, which shows it.
export const asksForWaiting = (chips) =>
  chips.some(
    (c) =>
      !c.not &&
      ((c.kind === "is" && (c.value === "wartet" || c.value === "blockiert")) || (c.kind === "defer" && c.value !== "ohne")),
  );

// --- What to offer ----------------------------------------------------

const MAX = 8;

function ranked(names, query) {
  const q = lower(query);
  const starts = names.filter((n) => lower(n.name).startsWith(q));
  const inside = names.filter((n) => !lower(n.name).startsWith(q) && lower(n.name).includes(q));
  const by = (a, b) => a.name.localeCompare(b.name, "de");
  return [...starts.sort(by), ...inside.sort(by)];
}

const asChip = (kind, value, not) => ({ kind, value, not });

// The keys of every property, own or inherited; after `key=` the values
// that key has.
function propertyOffers(value, not) {
  const at = value.indexOf("=");
  if (at < 0) {
    const keys = new Set();
    for (const s of state.statuses) {
      for (const k of Object.keys(s.task.properties ?? {})) keys.add(k);
      for (const k of Object.keys(s.inherited ?? {})) keys.add(k);
    }
    return ranked(
      [...keys].filter((k) => !SYSTEM.includes(k)).map((k) => ({ name: k, chip: asChip("property", k, not) })),
      value,
    );
  }
  const key = lower(value.slice(0, at));
  const values = new Set(state.statuses.map((s) => propertyOf(s, key)).filter((v) => v !== undefined));
  return ranked(
    [...values].map((v) => ({ name: `${key}=${v}`, chip: asChip("property", `${key}=${v}`, not) })),
    value,
  );
}

// What the list under the field offers for the word at the cursor:
// { label, chip } makes a chip, { label, insert } replaces the word.
export function offers(word, { inScope = false, lang: l = lang() } = {}) {
  const not = word.startsWith("-") && word.length > 1;
  const text = not ? word.slice(1) : word;
  const sign = not ? "-" : "";
  const kinds = l === "en" ? KINDS_EN : KINDS;
  const key_ = (de) => `${keyIn(de, l)}:`;
  // The German words of a list, shown in the page's language, by what is typed.
  const words = (prefix, kind, list, start) =>
    list
      .map((w) => [valueIn(w, l), w])
      .filter(([shown]) => shown.startsWith(start))
      .map(([shown, w]) => ({ label: `${sign}${prefix}${shown}`, chip: asChip(kind, w, not) }));
  const out = [];
  const add = (list) => out.push(...list);
  const projects = () =>
    state.projects
      .filter((p) => p.status !== "dropped")
      .map((p) => ({ name: p.title, chip: asChip("project", p.id, not) }))
      .concat([{ name: "Inbox", chip: asChip("project", "inbox", not) }]);
  const tags = () => state.tags.filter((t) => t.status !== "dropped").map((t) => ({ name: tagPath(t), chip: asChip("tag", t.id, not) }));
  const folders = () =>
    state.folders.filter((f) => f.status !== "dropped").map((f) => ({ name: folderPath(f), chip: asChip("folder", f.id, not) }));
  const label = (prefix) => (o) => ({ label: `${sign}${prefix}${o.name}`, chip: o.chip });

  if (!text) return kinds.map(([insert, hint]) => ({ label: hint, insert: `${sign}${insert}`, kind: true }));
  if (text.startsWith("#")) add(ranked(projects(), unquote(text.slice(1))).map(label("#")));
  else if (text.startsWith("@")) add(ranked(tags(), unquote(text.slice(1))).map(label("@")));
  else if (text.startsWith("~")) {
    add(words("~", "estimate", ESTIMATES, text.slice(1)));
  } else {
    const colon = /^([\p{L}]+):(.*)$/u.exec(text);
    if (colon) {
      const key = canonicalKey(lower(colon[1]));
      const value = unquote(colon[2]);
      const v = lower(value);
      if (DATES[key]) {
        const kind = DATES[key];
        add(words(key_(key), kind, RANGES[kind], v));
      } else if (key === "ordner") add(ranked(folders(), value).map(label(key_("ordner"))));
      else if (key === "prio") add(words("prio:", "priority", PRIORITIES, v));
      else if (key === "ist") add(words(key_("ist"), "is", IS.filter((w) => !(inScope && w === "termin")), v));
      else if (key === "zustand") {
        const states = workflowStates().map((t) => ({ name: t.title, chip: asChip("state", t.id, not) }));
        add(ranked(states, value).map(label(key_("zustand"))));
      } else if (key === "prop") add(propertyOffers(value, not).map(label("prop:")));
    } else {
      // A plain word: the kinds it begins, and the places and tags it names.
      add(kinds.filter(([insert]) => insert.length > 1 && insert.startsWith(lower(text))).map(([insert, hint]) => ({ label: hint, insert: `${sign}${insert}`, kind: true })));
      if (text.length >= 2) {
        add(ranked(projects(), text).slice(0, 3).map(label("#")));
        add(ranked(tags(), text).slice(0, 3).map(label("@")));
        add(ranked(folders(), text).slice(0, 2).map(label(key_("ordner"))));
      }
    }
  }
  return out.slice(0, MAX);
}
