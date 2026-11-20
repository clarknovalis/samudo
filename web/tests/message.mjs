// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The kind of a message on the message line: web/lib/message.js. Run by
// `make check` with node, no packages.

import assert from "node:assert/strict";

const root = new URL("../../", import.meta.url);
const { info, messageParts } = await import(new URL("web/lib/message.js", root));

// Text alone is an error, as every failure has always been shown.
assert.deepEqual(messageParts("No such object."), { text: "No such object.", actions: [], kind: "error" });
assert.deepEqual(messageParts(null), { text: "", actions: [], kind: "error" });

// A confirmation is the quiet kind and keeps its buttons.
const back = () => {};
assert.deepEqual(messageParts(info("Link copied.")), { text: "Link copied.", actions: [], kind: "info" });
assert.deepEqual(messageParts(info("Moved.", [["Undo", back]])), { text: "Moved.", actions: [["Undo", back]], kind: "info" });

// An object without a kind, as messages with buttons were before, stays an error.
assert.equal(messageParts({ text: "x", actions: [] }).kind, "error");

// Nothing to say is no message at all, so `info(cond ? text : null)` clears the line.
assert.equal(info(null), null);
assert.equal(info(""), null);

console.log("message: ok");
