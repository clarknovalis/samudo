// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The language of the interface (ADR 0105): German is the key. `tr("…")`
// looks the German text up in the English table when the page is English
// and falls back to German; `{name}` is filled from `vars` after the
// lookup. The choice is per browser like the theme (ADR 0026 §2):
// `system` follows navigator.language, `de` and `en` hold. A change
// reloads the page, since tables like the key map are made at load.

import { EN } from "./en.js";

const KEY = "samu.lang";
export const LANG_CHOICES = [
  ["system", "System"],
  ["de", "Deutsch"],
  ["en", "English"],
];

export function langChoice() {
  try {
    return localStorage.getItem(KEY) || "system";
  } catch {
    return "system";
  }
}

const systemLang = () => {
  const code = typeof navigator === "undefined" ? "" : navigator.language || "";
  return code.toLowerCase().startsWith("de") ? "de" : "en";
};

let current = null;

// `de` or `en`, as chosen or as the system says.
export function lang() {
  if (current) return current;
  const choice = langChoice();
  current = choice === "de" || choice === "en" ? choice : systemLang();
  return current;
}

export function setLang(choice) {
  try {
    if (choice === "system") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, choice);
  } catch {
    // Private mode: nothing to keep, so nothing changes.
    return;
  }
  location.reload();
}

// The locale for Intl: German as Germany writes it; English as the
// browser's English, else British (day before month, 24 hours).
export function locale() {
  if (lang() === "de") return "de-DE";
  const code = typeof navigator === "undefined" ? "" : navigator.language || "";
  return code.toLowerCase().startsWith("en") ? code : "en-GB";
}

const fill = (text, vars) => (vars ? text.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k]) : m)) : text);

// A key may begin with a context, `Suche|Verwerfen`, where one German word
// wants two English ones; German shows it without.
const plain = (key) => key.replace(/^[\p{L}]+\|/u, "");

export function tr(key, vars) {
  return fill((lang() === "en" && EN[key]) || plain(key), vars);
}

// The page as it comes from the server, once at the start: its texts and
// its aria-label, title and placeholder through the table. What already
// carries its own `lang` (the demo banner) stays.
export function translatePage(root = document.body) {
  document.documentElement.lang = lang();
  if (lang() === "de") return;
  const own = (node) => node.parentElement?.closest("[lang]:not(html)");
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const key = node.nodeValue.trim();
    if (key && EN[key] && !own(node)) node.nodeValue = node.nodeValue.replace(key, EN[key]);
  }
  for (const attr of ["aria-label", "title", "placeholder"]) {
    for (const element of root.querySelectorAll(`[${attr}]`)) {
      const key = element.getAttribute(attr);
      if (EN[key]) element.setAttribute(attr, EN[key]);
    }
  }
}

// For tests: the language without storage or navigator.
export function forceLang(code) {
  current = code;
}
