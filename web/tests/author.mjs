// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The report's `autor:` in both languages (ADR 0080 §2, ADR 0105 §3):
// web/lib/author.js. Run by `make check` with node, no packages.

import assert from "node:assert/strict";

const root = new URL("../../", import.meta.url);
const { authorOffers, authorWord, knownAuthor, parseAuthor } = await import(new URL("web/lib/author.js", root));

// Read in either language, kept German; a person's work is `ich`, named by
// the account.
for (const [word, value] of [
  ["autor:alle", "alle"],
  ["author:all", "alle"],
  ["Author:Agent", "agent"],
  ["autor:alice", "ich"],
  ["author:Alice", "ich"],
  ["autor:ich", "ich"],
  ["author:me", "ich"],
  ["autor:mensch", "ich"],
  ["author:human", "ich"],
]) {
  assert.deepEqual(parseAuthor(word, "Alice"), { kind: "author", value }, word);
}
for (const word of ["author:someone", "autor:", "auhor:all", "author:alle:x", "autor:bob"]) {
  assert.equal(parseAuthor(word, "alice"), null, word);
}
// Without a session the person's word is still known, an account name not.
assert.deepEqual(parseAuthor("autor:ich", null), { kind: "author", value: "ich" });
assert.equal(parseAuthor("autor:alice", null), null);

// A chip kept from before the account's name.
assert.deepEqual(knownAuthor("bob"), { kind: "author", value: "ich" });
assert.deepEqual(knownAuthor("agent"), { kind: "author", value: "agent" });

// Shown in the page's language, a person's work as the account.
assert.equal(authorWord("alle", "alice", "de"), "autor:alle");
assert.equal(authorWord("alle", "alice", "en"), "author:all");
assert.equal(authorWord("agent", "alice", "en"), "author:agent");
assert.equal(authorWord("ich", "Alice", "en"), "author:alice");
assert.equal(authorWord("ich", null, "de"), "autor:ich");

// Offered for what is typed in either language, labelled in the page's.
const labels = (word, l) => authorOffers(word, "alice", l).map((o) => o.label);
assert.deepEqual(labels("author:a", "en"), ["author:alice", "author:agent", "author:all"]);
assert.deepEqual(labels("author:a", "de"), ["autor:alice", "autor:agent", "autor:alle"]);
assert.deepEqual(labels("autor:al", "en"), ["author:alice", "author:all"]);
assert.deepEqual(labels("auth", "de"), ["autor:alice", "autor:agent", "autor:alle"]);
assert.deepEqual(labels("x", "en"), []);
assert.deepEqual(labels("", "en"), []);
assert.deepEqual(authorOffers("author:all", "alice", "en")[0].chip, { kind: "author", value: "alle" });

console.log("author.mjs: ok");
