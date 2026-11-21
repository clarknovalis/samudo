// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Which language a browser gets (web/lib/i18n.js, ADR 0105 §2): English
// until it chooses, German or the system's language by choice. Run by
// `make check` with node, no packages.

const root = new URL("../../", import.meta.url);
const { choiceOf, langOf } = await import(new URL("web/lib/i18n.js", root));

let failed = 0;
let checks = 0;
const same = (what, got, want) => {
  checks += 1;
  if (got !== want) {
    failed += 1;
    console.error(`not ok  ${what}: ${got}, want ${want}`);
  }
};

// Nothing stored, or something unknown, is English.
same("no choice", choiceOf(null), "en");
same("unknown choice", choiceOf("fr"), "en");
same("stored en", choiceOf("en"), "en");
same("stored de", choiceOf("de"), "de");
same("stored system", choiceOf("system"), "system");

// A German browser that never chose is English all the same.
same("German browser, no choice", langOf(choiceOf(null), "de-DE"), "en");
same("German by choice", langOf("de", "en-US"), "de");
same("English by choice", langOf("en", "de-DE"), "en");
// Only `system` asks the browser.
same("system, German browser", langOf("system", "de-AT"), "de");
same("system, English browser", langOf("system", "en-GB"), "en");
same("system, other browser", langOf("system", "fr-FR"), "en");
same("system, no browser", langOf("system", undefined), "en");

if (failed || !checks) {
  console.error(`lang: ${failed} of ${checks} failed`);
  process.exit(1);
}
console.log(`lang: ${checks} checks ok`);
