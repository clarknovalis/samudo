// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The English tables against the keys in the code (ADR 0105 §5), run by
// `make check` with node, no packages:
// - every `tr("…")` in web/ and every text, aria-label, title and
//   placeholder of web/index.html has an entry in web/lib/en.js;
// - every entry there is used;
// - the same for `tr("…")` in android/ against En.kt, once that exists.
// It fails when it finds no key at all, so a broken pattern cannot pass.

import { readdirSync, readFileSync, existsSync } from "node:fs";
import { join } from "node:path";

const root = new URL("../../", import.meta.url).pathname;
const { EN } = await import(join(root, "web/lib/en.js"));

function files(dir, ext, skip = []) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (skip.some((s) => path.includes(s))) continue;
    if (entry.isDirectory()) out.push(...files(path, ext, skip));
    else if (entry.name.endsWith(ext)) out.push(path);
  }
  return out;
}

// A double-quoted literal's text; JS and Kotlin escapes are JSON's here.
const unescape = (raw) => JSON.parse(`"${raw}"`);

function keysIn(paths, call) {
  const pattern = new RegExp(`\\b${call}\\(\\s*"((?:[^"\\\\]|\\\\.)*)"`, "g");
  const found = new Map();
  for (const path of paths) {
    // Comment lines name tr("…") without being keys.
    const code = readFileSync(path, "utf8")
      .split("\n")
      .filter((line) => !/^\s*(\/\/|\*)/.test(line))
      .join("\n");
    for (const m of code.matchAll(pattern)) {
      const key = unescape(m[1]);
      if (!found.has(key)) found.set(key, path.slice(root.length));
    }
  }
  return found;
}

// Texts that read the same in both languages, or are names.
const SAME = new Set(["samudo", "samudō", "System", "Deutsch", "English", "Board", "Scope", "Name", "Details", "Status", "Filter", "Inbox", "Tags", "Forecast", "Properties", "Scopes", "Habits", "Review", "Stuck"]);

function pageKeys() {
  const html = readFileSync(join(root, "web/index.html"), "utf8")
    .split("\n")
    // The demo banner carries its own language (lang="de" / lang="en").
    .filter((line) => !/<p lang="/.test(line))
    .join("\n");
  const keys = new Map();
  for (const m of html.matchAll(/>([^<>]+)</g)) {
    const text = m[1].trim();
    if (/\p{L}{2,}/u.test(text)) keys.set(text, "web/index.html");
  }
  for (const m of html.matchAll(/(?:aria-label|title|placeholder)="([^"]+)"/g)) keys.set(m[1], "web/index.html");
  return keys;
}

let failed = 0;
const complain = (text) => {
  failed += 1;
  console.error(text);
};

const web = keysIn(files(join(root, "web"), ".js", ["/tests/", "lib/en.js", "qrcode"]), "tr");
for (const [key, where] of pageKeys()) if (!SAME.has(key)) web.set(key, where);
if (web.size === 0) complain("i18n: no tr(\"…\") in web/ at all: the pattern is broken");
for (const [key, where] of web) {
  if (!(key in EN) && !SAME.has(key)) complain(`i18n: no English for ${JSON.stringify(key)} (${where})`);
}
for (const key of Object.keys(EN)) {
  if (!web.has(key)) complain(`i18n: English for ${JSON.stringify(key)}, which nothing uses`);
}

const enKt = join(root, "android/app/src/main/java/de/samudo/app/En.kt");
let app = 0;
if (existsSync(enKt)) {
  const table = new Map();
  for (const m of readFileSync(enKt, "utf8").matchAll(/^\s*"((?:[^"\\]|\\.)*)"\s+to\s+"((?:[^"\\]|\\.)*)",?\s*$/gm)) {
    table.set(unescape(m[1]), unescape(m[2]));
  }
  const used = keysIn(files(join(root, "android/app/src"), ".kt", ["En.kt"]), "tr");
  app = used.size;
  if (used.size === 0) complain("i18n: no tr(\"…\") in android/ at all: the pattern is broken");
  for (const [key, where] of used) if (!table.has(key) && !SAME.has(key)) complain(`i18n: no English for ${JSON.stringify(key)} (${where})`);
  for (const key of table.keys()) if (!used.has(key)) complain(`i18n: English for ${JSON.stringify(key)} in En.kt, which nothing uses`);
}

// The German of samu-mobile's messages in the app (I18n.kt, MESSAGES): each
// English text, its `{name}`s aside, must still be one samu-mobile says,
// and the German must fill the same names.
const i18nKt = join(root, "android/app/src/main/java/de/samudo/app/I18n.kt");
let messages = 0;
if (existsSync(i18nKt)) {
  const kt = readFileSync(i18nKt, "utf8");
  const start = kt.indexOf("private val MESSAGES");
  const block = kt.slice(start, kt.indexOf(").map", start));
  // samu-mobile passes some of samu-core's on, as the refusals of tagfields.
  const rust = [...files(join(root, "crates/samu-mobile/src"), ".rs"), join(root, "crates/samu-core/src/tagfields.rs")]
    .map((path) => readFileSync(path, "utf8"))
    .join("\n");
  const names = (text) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(",");
  for (const m of block.matchAll(/^\s*"((?:[^"\\]|\\.)*)"\s+to\s*\n?\s*"((?:[^"\\]|\\.)*)",?\s*$/gm)) {
    const [english, german] = [unescape(m[1]), unescape(m[2])];
    messages += 1;
    for (const piece of english.split(/\{\w+\}/)) {
      if (piece && !rust.includes(piece)) complain(`i18n: samu-mobile no longer says ${JSON.stringify(piece)} (I18n.kt, MESSAGES)`);
    }
    if (names(english) !== names(german)) complain(`i18n: ${JSON.stringify(german)} fills other names than ${JSON.stringify(english)}`);
  }
  if (messages === 0) complain("i18n: no message in I18n.kt MESSAGES: the pattern is broken");
}

console.log(`i18n: web ${web.size} keys, app ${existsSync(enKt) ? `${app} keys` : "no En.kt yet"}, ${messages} messages, ${failed} problems`);
process.exit(failed ? 1 : 0);
