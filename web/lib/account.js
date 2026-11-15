// Signing in and out (ADR 0019, ADR 0025 §4). The session lives in a cookie
// the server sets; this page never sees the token.

import { api } from "./api.js";
import { stopAlarms } from "./alarms.js";
import { $, el, replace } from "./dom.js";
import { openPairing } from "./pair.js";
import { openSettings } from "./settings.js";
import { byId, state, update } from "./state.js";
import { pathOf } from "./words.js";
import { langChoice, locale, setLang, tr } from "./i18n.js";

const DEVICE_KEY = "samu.device";
const THEME_KEY = "samu.theme";

// System, light or dark (ADR 0026 §2): per browser, never synced.
function storedTheme() {
  try {
    return localStorage.getItem(THEME_KEY) || "system";
  } catch {
    return "system";
  }
}

export function applyTheme(theme = storedTheme()) {
  if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
  else delete document.documentElement.dataset.theme;
  for (const button of document.querySelectorAll("[data-theme-choice]")) {
    button.setAttribute("aria-pressed", String(button.dataset.themeChoice === theme));
  }
}

function setTheme(theme) {
  try {
    localStorage.setItem(THEME_KEY, theme);
  } catch {
    // Private mode: the choice lasts as long as the page.
  }
  applyTheme(theme);
}

function defaultDevice() {
  try {
    return localStorage.getItem(DEVICE_KEY) || "web";
  } catch {
    return "web";
  }
}

export function showGate(message) {
  $("gate").hidden = false;
  $("app").hidden = true;
  $("gate-device").value = $("gate-device").value || defaultDevice();
  replace($("gate-error"), message ?? "");
  $("gate-account").focus();
}

export function hideGate() {
  $("gate").hidden = true;
  $("app").hidden = false;
}

// The device named in the sign-in form, for a session started here.
export function gateDevice() {
  return $("gate-device").value.trim() || defaultDevice();
}

let afterSignIn = () => {};

// Into the app with a session just started, by signing in or by making an
// account (ADR 0099 §8).
export function enter(session, device) {
  try {
    localStorage.setItem(DEVICE_KEY, device);
  } catch {
    // Only a convenience.
  }
  update({ session });
  hideGate();
  afterSignIn();
}

// Out of the app, back to the sign-in form.
export function leave(message) {
  stopAlarms();
  update({ session: null, statuses: [], selected: null, settings: null });
  showGate(message);
}

export function wireSignIn(onSignedIn) {
  afterSignIn = onSignedIn;
  $("gate-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const account = $("gate-account").value.trim();
    const device = gateDevice();
    try {
      const session = await api.login(account, $("gate-password").value, device);
      $("gate-password").value = "";
      enter(session, device);
    } catch (error) {
      showGate(error.message);
    }
  });
  // The menu at the foot of the sidebar opens upwards (ADR 0050 §4); Esc
  // and a click beside it close it.
  const menu = $("account-menu");
  const button = $("account-button");
  const toggle = (open) => {
    menu.hidden = !open;
    if (!open) closeSessions();
    button.setAttribute("aria-expanded", String(open));
    if (open) menu.querySelector("[role=menuitem]")?.focus();
  };
  button.addEventListener("click", () => toggle(menu.hidden));
  document.addEventListener("click", (event) => {
    if (!menu.hidden && !event.target.closest(".account")) toggle(false);
  });
  menu.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.stopPropagation();
      toggle(false);
      button.focus();
    }
  });
  for (const choice of document.querySelectorAll("[data-theme-choice]")) {
    choice.addEventListener("click", () => setTheme(choice.dataset.themeChoice));
  }
  // The language (ADR 0105 §2): per browser, a change reloads the page.
  for (const choice of document.querySelectorAll("[data-lang-choice]")) {
    choice.setAttribute("aria-pressed", String(choice.dataset.langChoice === langChoice()));
    choice.addEventListener("click", () => {
      if (choice.dataset.langChoice !== langChoice()) setLang(choice.dataset.langChoice);
    });
  }
  $("settings-open").addEventListener("click", () => {
    toggle(false);
    openSettings(state.scope);
  });
  $("pair-open").addEventListener("click", () => {
    toggle(false);
    openPairing();
  });
  $("account-open").addEventListener("click", () => {
    toggle(false);
    openSettings(state.scope, "account");
  });
  $("sessions-open").addEventListener("click", () => {
    if ($("account-sessions").hidden) openSessions();
    else closeSessions();
  });
  $("help-open").addEventListener("click", () => {
    toggle(false);
    update({ help: true });
  });
  $("logout").addEventListener("click", async () => {
    try {
      await api.logout();
    } finally {
      leave("Abgemeldet.");
    }
  });
}

// --- The grant and the sessions (ADR 0095 §1, §7) ---------------------------

// A granted folder by its path, a project by its title, else its short id,
// as `samu session` shows them.
function grantPlace(id) {
  const folder = byId(state.folders, id);
  if (folder) return pathOf(state.folders, folder);
  return byId(state.projects, id)?.title ?? id.slice(-8);
}

const accessWords = (access) => (access === "ro" ? tr("nur lesen") : access === "rw" ? tr("lesen und schreiben") : access);

export const grantWords = (grant) => `${grant.objects.map(grantPlace).join(", ")} · ${accessWords(grant.access)}`;

// Only a person's session of the whole account sees the others.
const managesSessions = (session) => Boolean(session) && session.kind !== "agent" && !session.grant;

const dayOf = (stamp) => new Date(stamp).toLocaleDateString(locale(), { day: "2-digit", month: "2-digit", year: "numeric" });

function closeSessions() {
  $("account-sessions").hidden = true;
  $("sessions-open").setAttribute("aria-expanded", "false");
}

async function openSessions() {
  const list = $("account-sessions");
  list.hidden = false;
  $("sessions-open").setAttribute("aria-expanded", "true");
  replace(list, el("p", { class: "note", text: tr("Lädt …") }));
  let sessions;
  try {
    sessions = await api.sessions();
  } catch (error) {
    replace(list, el("p", { class: "note", text: error.message }));
    return;
  }
  // The newest first, this one at the top.
  sessions.sort((a, b) => Number(b.current) - Number(a.current) || b.created.localeCompare(a.created));
  replace(list, sessions.map(sessionRow));
}

function sessionRow(session) {
  const name = [session.device, session.kind === "agent" ? "Agent" : null, session.current ? tr("diese Sitzung") : null]
    .filter(Boolean)
    .join(" · ");
  const lines = [
    el("strong", { text: name, title: `${tr("Sitzung")} ${session.id}` }),
    session.grant ? el("span", { text: `${tr("Freigabe:")} ${grantWords(session.grant)}` }) : null,
    el("span", { text: tr("seit {from} · bis {to}", { from: dayOf(session.created), to: dayOf(session.expires) }) }),
  ];
  // Ending this one is signing out, which is below.
  const end = session.current
    ? null
    : el("button", {
        type: "button",
        text: tr("Beenden"),
        title: tr("Sitzung {device} beenden", { device: session.device }),
        onclick: async (event) => {
          event.stopPropagation();
          event.currentTarget.disabled = true;
          try {
            await api.endSession(session.id);
          } catch (error) {
            replace($("account-sessions"), el("p", { class: "note", text: error.message }));
            return;
          }
          openSessions();
        },
      });
  return el("div", { class: "session" }, [el("div", {}, lines), end]);
}

export function renderAccount() {
  const session = state.session;
  // Whether a person or an agent is signed in here (ADR 0039 §2).
  const kind = session?.kind === "agent" ? " · Agent" : "";
  replace($("account-label"), session ? session.account : "samudō");
  replace($("account-name"), session ? `${session.device}${kind}` : "");
  replace($("account-initial"), session ? session.account.slice(0, 1).toUpperCase() : "");
  replace($("account-who"), session ? `${tr("Konto {account} · Gerät {device}", { account: session.account, device: session.device })}${kind}` : "");
  const grant = $("account-grant");
  grant.hidden = !session?.grant;
  replace(grant, session?.grant ? `${tr("Freigabe:")} ${grantWords(session.grant)}` : "");
  $("sessions-open").hidden = !managesSessions(session);
}
