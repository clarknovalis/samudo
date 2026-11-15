// The scope in the header (ADR 0035 §7): always visible, a menu to switch,
// and how much is due outside it. What a scope shows is computed by the
// server (`/available?scope=`); this page only names the choice.

import { api } from "./api.js";
import { $, el, replace } from "./dom.js";
import { COLORS, scopeDot } from "./colors.js";
import { openSettings } from "./settings.js";
import { byId, chooseScope, setScopeAuto, state, takeSchedule, update } from "./state.js";
import { tr } from "./i18n.js";

const chosen = () => (state.scope ? byId(state.scopes, state.scope) : null);

export function renderScope() {
  const scope = chosen();
  replace($("scope-name"), (scope ? scope.title : tr("Kein Scope")) + (state.scopeAuto ? " · auto" : ""));
  // The colour of the chosen scope (ADR 0060 §4); none leaves all neutral.
  const color = scope && COLORS.some(([c]) => c === scope.color) ? scope.color : null;
  if (color) document.body.dataset.scopeColor = color;
  else delete document.body.dataset.scopeColor;
  const dot = $("scope-dot");
  dot.hidden = !color;
  if (color) dot.dataset.color = color;
  $("scope-button").classList.toggle("on", Boolean(scope));
  $("scope-button").setAttribute("aria-expanded", String(state.scopeMenu));

  const outside = state.outside.length;
  const hint = $("scope-outside");
  // Without a scope, what only its own scope shows is hidden; when due it
  // still counts here (ADR 0069 §7).
  hint.hidden = outside === 0;
  replace(hint, scope ? tr("{n} fällig außerhalb", { n: outside }) : tr("{n} fällig verborgen", { n: outside }));
  hint.title = scope ? tr("Scope ablegen, um es zu sehen") : tr("In Scopes, die es nennen; Scope wählen");

  const menu = $("scope-menu");
  menu.hidden = !state.scopeMenu;
  if (!state.scopeMenu) return;
  const pick = (id, title) =>
    el(
      "li",
      { class: (state.scope ?? null) === id ? "current" : null },
      el("button", { type: "button", role: "menuitemradio", "aria-checked": String((state.scope ?? null) === id), onclick: () => chooseScope(id) }, [
        scopeDot(id ? byId(state.scopes, id)?.color : null),
        el("span", { text: title }),
      ]),
    );
  const scopes = [...state.scopes].sort((a, b) => a.position - b.position || a.title.localeCompare(b.title));
  replace(
    menu,
    el("ul", { role: "menu" }, [pick(null, tr("Kein Scope")), ...scopes.map((s) => pick(s.id, s.title))]),
    el("button", {
      type: "button",
      class: "auto",
      role: "menuitemcheckbox",
      "aria-checked": String(state.scopeAuto),
      text: `${state.scopeAuto ? "✓ " : ""}${tr("Automatisch nach Zeitplan")}`,
      title: tr("Der Zeitplan der Scopes wählt; eine Wahl von Hand gilt bis zum nächsten Fensterrand."),
      onclick: async () => {
        const on = !state.scopeAuto;
        setScopeAuto(on, on ? await api.scheduled().catch(() => null) : state.scheduled);
      },
    }),
    el("button", {
      type: "button",
      class: "edit",
      text: tr("Scopes bearbeiten …"),
      onclick: () => {
        update({ scopeMenu: false });
        openSettings(state.scope);
      },
    }),
  );
}

// Asks the server what the schedule picks: at once, every minute, and
// when the next edge comes, while this browser follows it.
export async function refreshSchedule() {
  if (!state.scopeAuto || !state.session) return;
  try {
    takeSchedule(await api.scheduled());
  } catch {
    // Offline or signed out: the last answer stands.
  }
}

export function wireScope() {
  setInterval(refreshSchedule, 60_000);
  $("scope-button").addEventListener("click", () => update({ scopeMenu: !state.scopeMenu }));
  // The hint takes the lens off, so what is due shows.
  // Without a scope it opens the menu: the hidden is in a scope.
  $("scope-outside").addEventListener("click", () =>
    state.scope ? chooseScope(null) : update({ scopeMenu: true }),
  );
  document.addEventListener("click", (event) => {
    if (state.scopeMenu && !event.target.closest(".scope")) update({ scopeMenu: false });
  });
}
