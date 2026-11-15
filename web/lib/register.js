// Making an account, filling it with demo data, deleting it (ADR 0099
// §2, §5, §7, §8). The sign-in page asks the server once whether it takes
// new accounts (`GET /register`) and shows the form below its own only
// then; in the mode `demo` also „Ausprobieren“, which needs no name: the
// server makes one and a password, shown here once.
//
// Settings > Konto holds „Demodaten holen“ and „Konto löschen“.

import { api } from "./api.js";
import { enter, gateDevice, leave } from "./account.js";
import { $, el, replace } from "./dom.js";
import { offerOwnAccount } from "./demo.js";
import { stopListening } from "./live.js";
import { state } from "./state.js";
import { lang, locale, tr } from "./i18n.js";

const LANGUAGE_NAMES = { de: "Deutsch", en: "English" };

// The server's mode, once asked: off, demo or open.
let mode = "off";
let languages = [];

// The demo data in the page's language (ADR 0105 §2).
const preferred = lang;

function languageOptions(select, none) {
  const options = languages.map((lang) => el("option", { value: lang, text: LANGUAGE_NAMES[lang] ?? lang }));
  if (none) options.unshift(el("option", { value: "", text: tr("keine") }));
  replace(select, options);
  select.value = languages.includes(preferred()) ? preferred() : (languages[0] ?? "");
}

export async function wireRegister() {
  try {
    ({ mode, languages = [] } = await api.registration());
  } catch {
    // An older server, or none reached: no form.
    return;
  }
  if (mode !== "demo" && mode !== "open") return;
  const form = $("gate-register");
  const error = $("register-error");
  const tryButton = $("register-try");
  languageOptions($("register-demo"), mode === "open");
  replace(
    $("register-hint"),
    mode === "demo"
      ? tr("Ein Demo-Konto, gefüllt mit Beispielen. Es wird gelöscht, wenn es eine Weile nicht benutzt wird.")
      : tr("Ein eigenes Konto auf diesem Server, auf Wunsch mit Beispielen gefüllt."),
  );
  tryButton.hidden = mode !== "demo";
  form.hidden = false;

  const make = async (body, button) => {
    replace(error, "");
    button.disabled = true;
    const device = gateDevice();
    try {
      const made = await api.register({ ...body, device });
      $("register-password").value = "";
      enter(await api.session(), device);
      if (made.password) showMade(made.account, made.password);
    } catch (e) {
      replace(error, e.status === 429 ? `${tr("Gerade zu viele neue Konten.")} ${e.message}` : e.message);
    } finally {
      button.disabled = false;
    }
  };
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    make(
      {
        name: $("register-name").value.trim(),
        password: $("register-password").value,
        demo: $("register-demo").value || undefined,
      },
      form.querySelector("button[type=submit]"),
    );
  });
  tryButton.addEventListener("click", () => make({ demo: $("register-demo").value || undefined }, tryButton));
  if (mode === "demo") {
    offerOwnAccount(() => {
      tryButton.scrollIntoView({ block: "center" });
      tryButton.focus();
    });
  }
}

// The name and password the server made, this once (ADR 0099 §8).
function showMade(account, password) {
  const close = () => {
    document.removeEventListener("keydown", onKey);
    overlay.remove();
  };
  const onKey = (event) => {
    if (event.key === "Escape") close();
  };
  const copied = el("p", { class: "quiet", "aria-live": "polite" });
  const copy = el("button", {
    type: "button",
    text: tr("Kopieren"),
    onclick: async () => {
      try {
        await navigator.clipboard.writeText(`${account}\n${password}`);
        copied.textContent = "Kopiert.";
      } catch {
        copied.textContent = tr("Nicht kopiert; bitte abschreiben.");
      }
    },
  });
  const box = el("div", { class: "box made", role: "dialog", "aria-modal": "true", "aria-label": tr("Dein Demo-Konto") }, [
    el("h2", { text: tr("Dein Demo-Konto") }),
    el("p", { text: tr("Notiere Name und Passwort, wenn du später wiederkommen willst. Sie werden nur jetzt gezeigt.") }),
    el("dl", {}, [
      el("dt", { text: tr("Konto") }),
      el("dd", {}, el("code", { id: "made-account", text: account })),
      el("dt", { text: tr("Passwort") }),
      el("dd", {}, el("code", { id: "made-password", text: password })),
    ]),
    copied,
    el("div", { class: "made-actions" }, [copy, el("button", { type: "button", class: "primary", text: tr("Notiert"), onclick: close })]),
  ]);
  const overlay = el("div", { class: "overlay" }, [box]);
  document.addEventListener("keydown", onKey);
  document.body.append(overlay);
  copy.focus();
}

// Settings > Konto. Built once and kept, so a redraw from /events does
// not take what was typed.
let pane = null;

export function accountPane() {
  if (!pane) pane = buildPane();
  pane.update();
  return pane.node;
}

function buildPane() {
  const who = el("p");
  const until = el("p", { class: "quiet" });

  // Demo data afterwards (ADR 0099 §7).
  const lang = el("select", { id: "demo-lang", "aria-label": tr("Sprache der Demodaten") });
  const demoNote = el("p", { class: "quiet", "aria-live": "polite" });
  const demoButton = el("button", { type: "button", text: tr("Demodaten holen") });
  const demoForm = el("div", { class: "account-row" }, [lang, demoButton]);
  demoButton.addEventListener("click", async () => {
    demoButton.disabled = true;
    demoNote.textContent = "";
    try {
      const added = await api.addDemo(lang.value);
      demoNote.textContent = added.whole
        ? tr("Das Konto war leer und hat alles bekommen.")
        : tr("Die Demodaten liegen unter dem Ordner „Demo“ und dem Tag „Demo“.");
    } catch (e) {
      demoNote.textContent = e.message;
    } finally {
      demoButton.disabled = false;
    }
  });
  const demo = el("section", { class: "account-part" }, [
    el("h4", { text: tr("Demodaten holen") }),
    el("p", {
      class: "quiet",
      text:
        tr("In ein leeres Konto kommt alles: Projekte, Aufgaben, Tags, Habits, Perspektiven, Scopes und Vorlagen. In ein Konto mit Inhalt kommt nur, was unter einem Ordner „Demo“ und einem Tag „Demo“ liegt; wieder weg damit, indem man den Ordner samt Aufgaben löscht."),
    }),
    demoForm,
    demoNote,
  ]);

  // Deleting one's own account (ADR 0099 §5): the password once more, and
  // a second click.
  const password = el("input", {
    id: "delete-password",
    type: "password",
    autocomplete: "current-password",
    "aria-label": tr("Dein Passwort, noch einmal"),
    placeholder: tr("Dein Passwort, noch einmal"),
  });
  const deleteNote = el("p", { class: "quiet", "aria-live": "polite" });
  const deleteButton = el("button", { type: "button", class: "danger", text: tr("Konto löschen") });
  let armed = false;
  const disarm = () => {
    armed = false;
    deleteButton.textContent = tr("Konto löschen");
  };
  password.addEventListener("input", disarm);
  deleteButton.addEventListener("click", async () => {
    deleteNote.textContent = "";
    if (!password.value) {
      deleteNote.textContent = tr("Bitte das Passwort eingeben.");
      password.focus();
      return;
    }
    if (!armed) {
      armed = true;
      deleteButton.textContent = tr("Wirklich endgültig löschen");
      deleteNote.textContent = tr("Alles in diesem Konto ist danach weg, auf allen Geräten.");
      return;
    }
    deleteButton.disabled = true;
    try {
      await api.deleteAccount(password.value);
      password.value = "";
      disarm();
      stopListening();
      leave(tr("Konto gelöscht."));
    } catch (e) {
      deleteNote.textContent = e.status === 401 ? tr("Das Passwort stimmt nicht.") : e.message;
      disarm();
    } finally {
      deleteButton.disabled = false;
    }
  });
  const remove = el("section", { class: "account-part" }, [
    el("h4", { text: tr("Konto löschen") }),
    el("p", {
      class: "quiet",
      text: tr("Löscht das Konto mit allen Aufgaben, Anhängen und Sitzungen. Geht nur bei einem Konto, das hier angelegt wurde."),
    }),
    el("div", { class: "account-row" }, [password, deleteButton]),
    deleteNote,
  ]);

  const node = el("div", { class: "account-pane" }, [who, until, demo, remove]);
  const update = () => {
    const session = state.session;
    replace(who, session ? tr("Konto {account}, angemeldet als Gerät {device}.", { account: session.account, device: session.device }) : "");
    until.hidden = !session?.demo_until;
    if (session?.demo_until) {
      const [y, m, d] = session.demo_until.split("-").map(Number);
      replace(
        until,
        tr("Ein Demo-Konto: gelöscht am {day}, wenn es bis dahin nicht benutzt wird.", { day: new Date(y, m - 1, d).toLocaleDateString(locale(), { day: "numeric", month: "long", year: "numeric" }) }),
      );
    }
    // Only a session of the whole account (ADR 0095).
    demo.hidden = Boolean(session?.grant);
    remove.hidden = Boolean(session?.grant) || session?.kind === "agent" || !(session?.demo_until || mode !== "off");
    if (!languages.length) languages = ["de", "en"];
    if (!lang.options.length) languageOptions(lang, false);
  };
  return { node, update };
}
