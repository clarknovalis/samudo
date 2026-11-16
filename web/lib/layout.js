// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// A scope's layout (ADR 0072): which parts of the inspector it shows, the
// rest folded under one line that names what has a value, and how the
// note shows. The layout lives in the scope on the server; whether this
// browser has unfolded the rest is its own, per scope.

import { change } from "./api.js";
import { el } from "./dom.js";
import { byId, state, update } from "./state.js";
import { tr } from "./i18n.js";

// The parts in the inspector's order (ADR 0072 §2), with a German label and
// whether the task has a value there.
export const PARTS = [
  ["properties", "Properties", (t) => Object.keys(t.properties ?? {}).length > 0],
  ["availability", tr("Verfügbarkeit"), (_t, s) => !s.available],
  ["state", tr("Zustand"), (_t, s) => Boolean(s.state)],
  ["project", tr("Projekt"), (t) => Boolean(t.project)],
  ["priority", tr("Priorität"), (t) => Boolean(t.priority)],
  ["defer", tr("Start"), (t) => Boolean(t.defer)],
  ["due", tr("Fällig"), (t) => Boolean(t.due)],
  ["planned", tr("Geplant"), (t) => Boolean(t.planned)],
  ["repeat", tr("Wiederholung"), (t) => Boolean(t.repeat)],
  ["series", tr("Serie"), (t) => Boolean(t.habit)],
  ["estimate", tr("Schätzung"), (t) => Boolean(t.estimate)],
  ["tags", "Tags", (t) => (t.tags ?? []).length > 0],
  ["note", tr("Notiz"), (t) => Boolean(t.note?.trim())],
  ["links", "Links", (t) => state.links.some((l) => l.owner === t.id)],
  ["reminders", tr("Erinnerungen"), (t) => state.reminders.some((r) => r.owner === t.id)],
  ["clock", "Clocking", (t) => state.clocks.some((c) => c.owner === t.id)],
  ["comments", tr("Kommentare"), (t) => state.comments.some((c) => c.owner === t.id)],
  ["attachments", tr("Anhänge"), (t) => state.attachments.some((a) => a.owner === t.id)],
  ["logbook", "Logbook", () => false],
];

// The preset "Habit" (ADR 0072 §5): it only fills the ticks.
export const HABIT = { shown: ["series", "note", "repeat", "planned", "reminders"], note: "rendered" };

// The inspector's layout of the chosen scope, or null: everything shown.
export function inspectorLayout() {
  const scope = state.scope ? byId(state.scopes, state.scope) : null;
  return scope?.layout?.inspector ?? null;
}

const openKey = () => `samu.inspectorOpen.${state.scope}`;

export function unfolded() {
  try {
    return localStorage.getItem(openKey()) === "1";
  } catch {
    return false;
  }
}

function setUnfolded(on) {
  try {
    if (on) localStorage.setItem(openKey(), "1");
    else localStorage.removeItem(openKey());
  } catch {
    // Private window: it stays folded after a reload.
  }
  update({});
}

// Which parts this draw shows: all of them without a layout or unfolded.
export function shownParts() {
  const layout = inspectorLayout();
  if (!layout || unfolded()) return null;
  return new Set(layout.shown ?? []);
}

// The quiet line under the shown parts (ADR 0072 §3): what is folded and
// has a value first, by name, then how many more; tr("Weniger") once unfolded.
export function foldLine(task, status) {
  const layout = inspectorLayout();
  if (!layout) return null;
  if (unfolded()) {
    return el("button", { type: "button", class: "fold-line", text: tr("Weniger"), onclick: () => setUnfolded(false) });
  }
  const shown = new Set(layout.shown ?? []);
  const folded = PARTS.filter(([key]) => !shown.has(key));
  if (!folded.length) return null;
  const valued = folded.filter(([, , has]) => has(task, status)).map(([, label]) => label);
  const rest = folded.length - valued.length;
  const text = valued.length
    ? `Mehr: ${valued.join(", ")}${rest ? ` · ${rest} weitere` : ""}`
    : tr("Mehr ({rest})", { rest });
  return el("button", {
    type: "button",
    class: valued.length ? "fold-line valued" : "fold-line",
    text,
    title: tr("Was der Scope einklappt, aufklappen"),
    onclick: () => setUnfolded(true),
  });
}

// The note's mode in this scope: "rendered" or "default".
export function noteMode() {
  return inspectorLayout()?.note === "rendered" ? "rendered" : "default";
}

// "Gestalt" in the scope settings (ADR 0072 §5): a tick per part, the
// note, tr("Alles zeigen") and the preset "Habit". Written as a whole.
export function layoutEditor(scope, write) {
  const inspector = scope.layout?.inspector ?? null;
  const shown = new Set(inspector?.shown ?? PARTS.map(([key]) => key));
  const save = (next) => {
    const layout = { ...(scope.layout ?? {}) };
    if (next) layout.inspector = next;
    else delete layout.inspector;
    write([change.set(scope.id, "layout", Object.keys(layout).length ? layout : null)]);
  };
  const tick = ([key, label]) =>
    el("label", { class: "toggle" }, [
      el("input", {
        type: "checkbox",
        checked: shown.has(key),
        onchange: (event) => {
          const next = new Set(shown);
          if (event.target.checked) next.add(key);
          else next.delete(key);
          // Kept in the inspector's order.
          save({ ...(inspector ?? {}), shown: PARTS.map(([k]) => k).filter((k) => next.has(k)) });
        },
      }),
      el("span", { text: label }),
    ]);
  return el("div", { class: "layout-editor" }, [
    el("label", { class: "field", text: tr("Gestalt: was der Inspektor zeigt; der Rest steht eingeklappt unter „Mehr“") }),
    el("div", { class: "layout-parts" }, PARTS.map(tick)),
    el("label", { class: "toggle" }, [
      el("input", {
        type: "checkbox",
        checked: inspector?.note === "rendered",
        onchange: (event) =>
          save({ shown: [...shown].filter((k) => PARTS.some(([p]) => p === k)), ...(inspector ?? {}), note: event.target.checked ? "rendered" : "default" }),
      }),
      el("span", {}, [tr("Notiz nur gerendert"), el("span", { class: "quiet", text: ` – ${tr("auch leer kein Eingabefeld, sondern „Notiz schreiben“")}` })]),
    ]),
    el("div", { class: "layout-actions" }, [
      el("button", { type: "button", text: tr("Vorgabe Habit"), onclick: () => save({ ...HABIT }) }),
      el("button", { type: "button", text: tr("Alles zeigen"), disabled: !inspector, onclick: () => save(null) }),
    ]),
    el("p", {
      class: "quiet",
      text: tr("Ort, Titel und Erledigen stehen immer da. Eingeklapptes mit Wert nennt die Zeile „Mehr“ beim Namen. Aufklappen gilt nur in diesem Browser."),
    }),
  ]);
}
