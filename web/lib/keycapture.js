// Settings › Tastatur (ADR 0079 §4): every action with the key in effect,
// change, remove, back to the default; and the capture: press a key (or
// two, §6), then it is free, refused or taken — taken offers to swap, to
// take it over or to cancel. Saved is `settings.keys` as a whole, only the
// differences from the defaults (§2).

import { api, change } from "./api.js";
import { el } from "./dom.js";
import { ACTIONS, PLACES, action, clashesWith, inEffect, normalize, pretty, refused, tidy } from "./keymap.js";
import { state, update } from "./state.js";
import { tr } from "./i18n.js";

// The capture under way: { id, steps, phase: "listen" | "decide" }, or null.
let capture = null;
let secondTimer = null;
// The search field, kept across redraws so typing goes on.
let searchField = null;

const own = () => ({ ...(state.accountSettings?.keys ?? {}) });

async function save(keys) {
  const s = state.accountSettings;
  const value = tidy(keys);
  // At once on this page; the server's answer follows over /events.
  s.keys = value;
  update({});
  try {
    await api.write([change.set(s.id, "keys", value)]);
  } catch (error) {
    update({ message: error.message });
  }
}

function startCapture(id) {
  capture = { id, steps: [], phase: "listen" };
  update({});
}

function endCapture() {
  clearTimeout(secondTimer);
  capture = null;
  update({});
}

function decide() {
  clearTimeout(secondTimer);
  if (capture) capture.phase = "decide";
  update({});
}

// While listening, every key goes to the capture and nowhere else: it runs
// before keys.js (capture phase) and stops the event.
function onKey(event) {
  if (!capture || !state.settings) return;
  // Esc ends the capture, not the settings.
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    return endCapture();
  }
  if (capture.phase !== "listen") return;
  event.preventDefault();
  event.stopPropagation();
  const key = normalize(event);
  if (!key) return;
  capture.steps.push(key);
  if (capture.steps.length === 2) return decide();
  // A short wait for a second key (§6).
  clearTimeout(secondTimer);
  secondTimer = setTimeout(decide, 1500);
  update({});
}

export function wireKeyCapture() {
  document.addEventListener("keydown", onKey, true);
}

const titleOf = (id) => action(id)?.title ?? id;

// What to do with the captured binding.
function verdict(byId) {
  const binding = capture.steps.join(" ");
  const blocked = refused(binding);
  if (blocked) return { binding, blocked };
  return { binding, taken: clashesWith(byId, capture.id, binding) };
}

function captureBox(byId) {
  const a = action(capture.id);
  const head = el("p", {}, [`${tr("Taste für")} `, el("strong", { text: a.title }), " …"]);
  if (capture.phase === "listen") {
    return el("div", { class: "key-capture", role: "dialog", "aria-label": tr("Taste aufnehmen") }, [
      head,
      el("p", { class: "key-big", text: capture.steps.length ? `${pretty(capture.steps[0])} …` : tr("Taste drücken") }),
      el("p", { class: "quiet", text: capture.steps.length ? tr("Noch eine Taste für eine Folge, sonst bleibt es bei dieser.") : tr("Esc bricht ab.") }),
      el("div", { class: "buttons" }, [
        capture.steps.length ? el("button", { type: "button", text: tr("Nur diese Taste"), onclick: decide }) : null,
        el("button", { type: "button", text: tr("Abbrechen"), onclick: endCapture }),
      ]),
    ]);
  }
  const { binding, blocked, taken } = verdict(byId);
  const mine = own();
  const old = byId.get(capture.id) ?? null;
  const take = (extra = {}) => {
    save({ ...mine, ...extra, [capture.id]: binding });
    endCapture();
  };
  let body;
  if (blocked) {
    body = [
      el("p", { class: "warn", text: tr("{key} ist fest vergeben und lässt sich nicht belegen.", { key: pretty(blocked) }) }),
      el("div", { class: "buttons" }, [
        el("button", { type: "button", text: tr("Andere Taste"), onclick: () => startCapture(capture.id) }),
        el("button", { type: "button", text: tr("Abbrechen"), onclick: endCapture }),
      ]),
    ];
  } else if (!taken.length) {
    body = [
      el("p", { text: binding === old ? tr("Das ist schon die Taste dieser Aktion.") : "Frei." }),
      el("div", { class: "buttons" }, [
        el("button", { type: "button", class: "primary", text: tr("Übernehmen"), onclick: () => take() }),
        el("button", { type: "button", text: tr("Andere Taste"), onclick: () => startCapture(capture.id) }),
        el("button", { type: "button", text: tr("Abbrechen"), onclick: endCapture }),
      ]),
    ];
  } else {
    const names = taken.map((id) => `${titleOf(id)} (${pretty(byId.get(id))})`).join(", ");
    // Swapping gives the other action this one's old key; with more than
    // one in the way there is nothing to swap with.
    const swap = taken.length === 1 && old ? { [taken[0]]: old } : null;
    body = [
      el("p", { class: "warn", text: tr("{key} ist schon belegt: {names}.", { key: pretty(binding), names }) }),
      el("div", { class: "buttons" }, [
        swap
          ? el("button", {
              type: "button",
              text: tr("Tauschen ({name} bekommt {key})", { name: titleOf(taken[0]), key: pretty(old) }),
              onclick: () => take(swap),
            })
          : null,
        el("button", {
          type: "button",
          class: "primary",
          text: taken.length === 1 ? tr("Übernehmen (die andere verliert ihre Taste)") : tr("Übernehmen (die anderen verlieren ihre Taste)"),
          onclick: () => take(Object.fromEntries(taken.map((id) => [id, null]))),
        }),
        el("button", { type: "button", text: tr("Abbrechen"), onclick: endCapture }),
      ]),
    ];
  }
  return el("div", { class: "key-capture", role: "dialog", "aria-label": tr("Taste aufnehmen") }, [
    head,
    el("p", { class: "key-big", text: pretty(binding) }),
    ...body,
  ]);
}

function row(a, byId, mine) {
  const binding = byId.get(a.id) ?? null;
  const changed = a.id in mine;
  return el("li", { class: capture?.id === a.id ? "current" : null }, [
    el("span", { class: "key-title" }, [a.title, changed ? el("span", { class: "quiet", text: ` · ${tr("eigene")}` }) : null]),
    el("kbd", { class: binding ? null : "none", text: binding ? pretty(binding) : "—" }),
    el("span", { class: "key-actions" }, [
      el("button", { type: "button", text: tr("Ändern"), onclick: () => startCapture(a.id) }),
      binding
        ? el("button", { type: "button", text: tr("Entfernen"), onclick: () => save({ ...mine, [a.id]: null }) })
        : null,
      changed
        ? el("button", {
            type: "button",
            text: a.key ? tr("Vorgabe ({days})", { days: pretty(a.key) }) : tr("Vorgabe (keine)"),
            onclick: () => {
              const next = { ...mine };
              delete next[a.id];
              save(next);
            },
          })
        : null,
    ]),
  ]);
}

export function keysPane() {
  const s = state.accountSettings;
  if (!s) return el("p", { class: "empty", text: tr("Der Server kennt noch keine Einstellungen.") });
  const { byId, displaced } = inEffect(s.keys);
  const mine = own();
  if (!searchField) {
    searchField = el("input", {
      type: "search",
      placeholder: tr("Aktion oder Taste suchen"),
      "aria-label": tr("Aktion oder Taste suchen"),
      oninput: () => update({}),
    });
  }
  const query = searchField.value.trim().toLowerCase();
  const hit = (a) =>
    !query || a.title.toLowerCase().includes(query) || pretty(byId.get(a.id)).toLowerCase().includes(query);
  const groups = PLACES.map(([place, title]) => {
    const list = ACTIONS.filter((a) => a.place === place && hit(a));
    return list.length
      ? el("section", {}, [el("h4", { text: title }), el("ul", { class: "key-list" }, list.map((a) => row(a, byId, mine)))])
      : null;
  });
  return el("div", { class: "keys-settings" }, [
    el("div", { class: "toolbar" }, [
      searchField,
      el("button", {
        type: "button",
        text: tr("Alles auf Vorgabe"),
        disabled: !Object.keys(mine).length,
        onclick: () => save({}),
      }),
    ]),
    capture ? captureBox(byId) : null,
    displaced.length
      ? el(
          "p",
          { class: "quiet" },
          `${tr("Vorgaben, die einer eigenen Belegung weichen:")} ${displaced.map((id) => `${titleOf(id)} (${pretty(action(id).key)})`).join(", ")}.`,
        )
      : null,
    ...groups,
    el("p", {
      class: "quiet",
      text: tr("Gilt für das Konto, in jedem Browser. Esc ist fest; in Eingabefeldern gelten keine Einzeltasten. „t, dann Kürzel“ nimmt das Kürzel aus dem Zustand."),
    }),
  ]);
}
