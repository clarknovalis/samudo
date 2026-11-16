// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The report's `autor:` (ADR 0080 §2) in both languages (ADR 0105 §3):
// `author:` and `all` are read always and shown on an English page; the
// chip keeps the German word, which is what the server is sent.
//
// A person's work is the chip value `ich`; it shows as the account's name
// (`autor:alice`), and the account's name, `ich`, `mensch`, `me` and
// `human` all read as it. `me` below is the account's name.

import { lang } from "./i18n.js";

export const AUTHORS = ["ich", "agent", "alle"];
export const author = (value) => ({ kind: "author", value });

const PERSON = ["ich", "mensch", "me", "human"];

// The chip's word in the language `l`.
export function authorWord(value, me, l = lang()) {
  const shown = value === "ich" && me ? me.toLowerCase() : value;
  return l === "en" ? `author:${shown === "alle" ? "all" : shown}` : `autor:${shown}`;
}

// A typed word in either language as the chip, else null.
export function parseAuthor(word, me) {
  const m = /^au(?:t|th)or:(\S+)$/iu.exec(word);
  const value = m && m[1].toLowerCase();
  if (!value) return null;
  if (PERSON.includes(value) || (me && value === me.toLowerCase())) return author("ich");
  const de = value === "all" ? "alle" : value;
  return AUTHORS.includes(de) ? author(de) : null;
}

// A chip kept from before, as one of AUTHORS (an old name is a person's).
export const knownAuthor = (value) => author(AUTHORS.includes(value) ? value : "ich");

// What a word typed in either language may become, labelled in `l`.
export function authorOffers(word, me, l = lang()) {
  const typed = word.toLowerCase();
  if (!typed) return [];
  return AUTHORS.filter((a) => ["de", "en"].some((each) => authorWord(a, me, each).startsWith(typed))).map((a) => ({
    label: authorWord(a, me, l),
    chip: author(a),
  }));
}
