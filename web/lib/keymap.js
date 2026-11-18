// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The key bindings (ADR 0079): every action has an id, a title, a default
// binding and the place it works. Keys, the help and the menu are all drawn
// from this table and the account's own bindings (`settings.keys`), so they
// cannot disagree. Pure: no DOM, no state, no imports but the language
// (ADR 0105 §4) — keys.js, menu.js and the settings use it alike.
//
// The defaults are also those of Emacs (emacs/samu.el, ADR 0082 §3): a
// changed default here wants the same change in `samu-list-mode-map'.
//
// A binding is one key with its modifiers in a fixed spelling — `x`, `N`,
// `Shift+ArrowUp`, `Ctrl+k`, `Alt+ArrowLeft`, `Delete`, `Space` — or two of
// them one after the other, with a space between: `g i` (§6).

import { tr } from "./i18n.js";

// Where an action works. `task` works on the list and the board, `list` and
// `board` only there, `any` everywhere. Two places overlap unless one is
// `list` and the other `board`.
export const PLACES = [
  ["any", tr("Überall")],
  ["task", tr("Aufgaben in Liste und Board")],
  ["list", tr("Liste")],
  ["board", "Board"],
  ["go", tr("Springen")],
];

// `fixed` are keys that always do the same as the action and cannot be
// taken (the arrows beside j and k); `hint` explains more in the help.
export const ACTIONS = [
  { id: "next", title: tr("nächste Aufgabe"), key: "j", fixed: ["ArrowDown"], place: "task" },
  { id: "prev", title: tr("vorige Aufgabe"), key: "k", fixed: ["ArrowUp"], place: "task" },
  { id: "open", title: tr("Aufgabe öffnen"), key: "Enter", place: "task" },
  {
    id: "pick",
    title: tr("Aufgabe auswählen"),
    key: "Space",
    place: "task",
    hint: tr("auch Strg-/⌘-Klick; Umschalt-Klick wählt einen Bereich"),
  },
  { id: "complete", title: tr("erledigen / wieder öffnen"), key: "x", place: "task", picks: true },
  { id: "state.cycle", title: tr("Zustand weiterschalten"), key: "s", place: "task", picks: true },
  { id: "state.pick", title: tr("Zustand wählen"), key: "t", place: "task", picks: true, hint: tr("danach das Kürzel des Zustands") },
  { id: "flag", title: tr("markieren"), key: "f", place: "task", picks: true },
  { id: "clock", title: tr("Uhr starten / stoppen"), key: "c", place: "task" },
  { id: "move", title: tr("umhängen (Refile)"), key: "m", place: "task", picks: true },
  { id: "block", title: tr("blockiert durch …"), key: null, place: "task", hint: tr("eine Aufgabe, auf die sie wartet") },
  { id: "indent", title: tr("einrücken"), key: ">", place: "task" },
  { id: "outdent", title: tr("ausrücken"), key: "<", place: "task" },
  {
    id: "shift.up",
    title: tr("eine Stelle nach oben"),
    key: "Shift+ArrowUp",
    place: "list",
    hint: tr("auch in eine offene Gruppe hinein und aus ihrer heraus"),
  },
  { id: "shift.down", title: tr("eine Stelle nach unten"), key: "Shift+ArrowDown", place: "list" },
  { id: "board.left", title: tr("Karte eine Spalte nach links"), key: "Alt+ArrowLeft", place: "board" },
  { id: "board.right", title: tr("Karte eine Spalte nach rechts"), key: "Alt+ArrowRight", place: "board" },
  { id: "fold.all", title: tr("alle Gruppen zu / auf"), key: "Z", place: "task", hint: tr("Alt-Klick auf einen Pfeil: nur diese Gruppe offen") },
  { id: "board.full", title: tr("Board: Vollbild an / aus"), key: "z", place: "board" },
  { id: "capture.child", title: tr("Unteraufgabe anlegen"), key: "N", place: "task" },
  {
    id: "edit",
    title: tr("bearbeiten"),
    key: "e",
    place: "task",
    hint: tr("Titel, gesetzte Werte als Chips mit ✕, dazu @tag !A d:fr #projekt // an die Notiz; bei Ausgewählten für alle"),
  },
  { id: "menu", title: tr("Menü der Aufgabe"), key: "Shift+F10", fixed: ["ContextMenu"], place: "task", hint: tr("auch Rechtsklick") },
  { id: "delete", title: tr("löschen …"), key: "Delete", place: "task", picks: true },
  { id: "drop", title: tr("verwerfen"), key: null, place: "task", picks: true },
  { id: "review.done", title: tr("Projekt als geprüft markieren"), key: "R", place: "task", hint: tr("das Projekt der Aufgabe oder der Ansicht (ADR 0088)") },
  { id: "copy-link", title: tr("Link kopieren"), key: null, place: "task" },
  { id: "date.today", title: tr("fällig heute"), key: null, place: "task", picks: true },
  { id: "date.tomorrow", title: tr("fällig morgen"), key: null, place: "task", picks: true },
  {
    id: "capture",
    title: tr("erfassen"),
    key: "n",
    place: "any",
    hint: tr("#projekt @tag ! !A d:fr s:mo p:morgen ~15m every:1w // Notiz"),
  },
  {
    id: "capture.lines",
    title: tr("mehrere Zeilen erfassen"),
    key: null,
    place: "any",
    hint: tr("eine Aufgabe je Zeile, eingerückt darunter; ⇧↵ im Erfassen wechselt auch dorthin"),
  },
  {
    id: "capture.template",
    title: tr("mit Vorlage erfassen"),
    key: "C",
    place: "any",
    hint: tr("dann die Taste der Vorlage, wie org-capture (Einstellungen > Vorlagen)"),
  },
  { id: "voice", title: tr("Sprachnotiz in die Inbox"), key: "r", place: "any", hint: tr("nochmal legt an, Esc verwirft") },
  {
    id: "search",
    title: tr("suchen"),
    key: "/",
    place: "any",
    hint: tr("Text #projekt @tag ordner: fällig:woche erledigt:-7t ist:markiert zustand: ~<15m, -verneint"),
  },
  { id: "sidebar", title: tr("Seitenleiste: voll, schmal, weg"), key: "[", place: "any" },
  { id: "full", title: tr("Aufgabe in voller Breite / zurück"), key: "]", place: "task" },
  { id: "help", title: tr("diese Hilfe"), key: "?", place: "any" },
  { id: "go.inbox", title: tr("zur Inbox"), key: "g i", place: "go", view: "inbox" },
  { id: "go.forecast", title: tr("zu Forecast"), key: "g f", place: "go", view: "forecast" },
  { id: "go.flagged", title: tr("zu Markiert"), key: "g m", place: "go", view: "flagged" },
  { id: "go.projects", title: tr("zu Alle Projekte"), key: "g p", place: "go", view: "everything" },
  { id: "go.review", title: tr("zu Review"), key: "g r", place: "go", view: "review" },
  { id: "go.time", title: tr("zum Bericht, Zeit"), key: "g z", place: "go", view: "time" },
  { id: "go.calendar", title: tr("zum Kalender"), key: "g k", place: "go", view: "calendar" },
  { id: "go.settings", title: tr("zu den Einstellungen"), key: "g s", place: "go" },
];

export const action = (id) => ACTIONS.find((a) => a.id === id) ?? null;

// Keys the browser keeps or one does not want to lose (§3), and Esc.
export const BLOCKED = ["Escape", "Ctrl+w", "Ctrl+t", "Ctrl+n", "Ctrl+Tab", "Ctrl+l", "Ctrl+r", "F5", "Tab"];
const FIXED = ACTIONS.flatMap((a) => a.fixed ?? []);

const MODIFIERS = ["Ctrl", "Alt", "Shift", "Meta"];
const NAMES = { " ": "Space", Spacebar: "Space", Esc: "Escape", Del: "Delete" };

// One step from its parts. A single printable character carries its Shift
// in itself (`N`, `?`); a named key spells it out (`Shift+ArrowUp`).
function step({ ctrl, alt, shift, meta, key }) {
  if (!key || ["Shift", "Control", "Alt", "Meta", "AltGraph", "OS", "Dead"].includes(key)) return null;
  key = NAMES[key] ?? key;
  const single = [...key].length === 1;
  if (single && /\p{L}/u.test(key)) {
    // With Ctrl or Alt the browser may send the upper case for Shift.
    key = shift ? key.toUpperCase() : key.toLowerCase();
  }
  const mods = [];
  if (ctrl) mods.push("Ctrl");
  if (alt) mods.push("Alt");
  if (shift && !single) mods.push("Shift");
  if (meta) mods.push("Meta");
  return [...mods, key].join("+");
}

// A key event in the fixed spelling, or null for a modifier alone.
export function normalize(event) {
  return step({ ctrl: event.ctrlKey, alt: event.altKey, shift: event.shiftKey, meta: event.metaKey, key: event.key });
}

// A binding as written by hand (`Shift+n`, `ctrl+K`, `g  i`) in the fixed
// spelling, or null when it is none.
export function canon(binding) {
  if (typeof binding !== "string" || !binding.trim()) return null;
  const steps = binding.trim().split(/\s+/).map((text) => {
    const parts = text.split("+");
    // `+` alone, or a binding ending in `++`.
    let key = parts.pop();
    if (key === "" && parts.length && parts.at(-1) === "") {
      parts.pop();
      key = "+";
    } else if (key === "") key = "+";
    const has = (m) => parts.some((p) => p.toLowerCase() === m.toLowerCase() || (m === "Ctrl" && p.toLowerCase() === "control"));
    if (parts.some((p) => !MODIFIERS.some((m) => p.toLowerCase() === m.toLowerCase()) && p.toLowerCase() !== "control")) return null;
    return step({ ctrl: has("Ctrl"), alt: has("Alt"), shift: has("Shift"), meta: has("Meta"), key });
  });
  return steps.includes(null) || steps.length > 2 ? null : steps.join(" ");
}

export const overlaps = (a, b) => !((a === "list" && b === "board") || (a === "board" && b === "list"));

// Whether two bindings get in each other's way: the same, or one is the
// first key of the other (§6: a key is a prefix or a binding, not both).
export function clash(a, b) {
  if (!a || !b) return false;
  if (a === b) return true;
  const first = (x) => x.split(" ")[0];
  const sequence = (x) => x.includes(" ");
  return (sequence(a) && !sequence(b) && first(a) === b) || (sequence(b) && !sequence(a) && first(b) === a);
}

// The bindings in effect: { byId: Map(id → binding|null), displaced: [id] }.
// Own bindings (only differences, from `settings.keys`) come first; a
// default that clashes with one of them falls away (§2). Unknown ids are
// passed over.
export function effective(own = {}) {
  const mine = new Map(
    Object.entries(own ?? {})
      .filter(([id]) => action(id))
      .map(([id, b]) => [id, canon(b)]),
  );
  const byId = new Map();
  const displaced = [];
  for (const a of ACTIONS) {
    if (mine.has(a.id)) {
      byId.set(a.id, mine.get(a.id));
      continue;
    }
    const beaten = [...mine].some(([id, b]) => overlaps(action(id).place, a.place) && clash(b, a.key));
    byId.set(a.id, beaten ? null : a.key);
    if (beaten) displaced.push(a.id);
  }
  return { byId, displaced };
}

// `effective` for the account's bindings, computed again only when they
// change (a new settings object).
let memoFrom;
let memo;
export function inEffect(own) {
  const of = own ?? {};
  if (of !== memoFrom) {
    memoFrom = of;
    memo = effective(of);
  }
  return memo;
}

// The pairs of actions whose bindings clash in overlapping places.
export function conflicts(byId) {
  const out = [];
  const list = ACTIONS.filter((a) => byId.get(a.id));
  for (let i = 0; i < list.length; i++) {
    for (let j = i + 1; j < list.length; j++) {
      const [a, b] = [list[i], list[j]];
      if (overlaps(a.place, b.place) && clash(byId.get(a.id), byId.get(b.id))) out.push([a.id, b.id]);
    }
  }
  return out;
}

// What a binding may not be: a blocked or fixed key in any of its steps.
export function refused(binding) {
  return binding.split(" ").find((s) => BLOCKED.includes(s) || FIXED.includes(s)) ?? null;
}

// The actions in `place` (and the overlapping ones) that `binding` would
// clash with, other than `id`.
export function clashesWith(byId, id, binding) {
  const place = action(id).place;
  return ACTIONS.filter((a) => a.id !== id && overlaps(a.place, place) && clash(byId.get(a.id), binding)).map((a) => a.id);
}

// Own bindings as `settings.keys` keeps them: only what differs from the
// default, unknown ids kept as they were (§2).
export function tidy(own) {
  const out = {};
  for (const [id, b] of Object.entries(own)) {
    const a = action(id);
    if (!a) out[id] = b;
    else if ((b ?? null) !== a.key) out[id] = b ?? null;
  }
  return out;
}

const PRETTY = {
  Space: tr("Leertaste"),
  Enter: "↵",
  Delete: tr("Entf"),
  Backspace: "⌫",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Shift: tr("Umschalt"),
  Ctrl: tr("Strg"),
  Meta: "⌘",
  ContextMenu: tr("Menütaste"),
};

// A binding for reading: `Umschalt+↑`, `g i`, `Entf`.
export function pretty(binding) {
  if (!binding) return "";
  return binding
    .split(" ")
    .map((s) => (s === "+" ? "+" : s.split(/\+(?!$)/).map((p) => PRETTY[p] ?? p).join("+")))
    .join(" ");
}
