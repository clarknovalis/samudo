// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// What the inspector shows of a transcript (ADR 0140 §2, §3):
// web/lib/transcript.js. Run by `make check` with node, no packages.

import assert from "node:assert/strict";

const root = new URL("../../", import.meta.url);
const { transcriptParts } = await import(new URL("web/lib/transcript.js", root));
const { forceLang } = await import(new URL("web/lib/i18n.js", root));
forceLang("de");

const audio = (more = {}) => ({ media_type: "audio/webm;codecs=opus", ...more });

// Only sound has a transcript.
assert.equal(transcriptParts({ media_type: "image/png", transcript: "x" }, true), null);
assert.equal(transcriptParts({ media_type: "video/webm" }, true), null);

// Nothing yet: the button, where the server has a service.
assert.deepEqual(transcriptParts(audio(), true), { ask: "Transkribieren" });
assert.equal(transcriptParts(audio(), false), null);

// Waiting: no button, with or without a service.
for (const canAsk of [true, false]) {
  assert.deepEqual(transcriptParts(audio({ transcript_state: "pending", transcript: "old" }), canAsk), {
    note: "Transkript läuft …",
  });
}

// Failed: the reason, and the button to try again.
const failed = audio({ transcript_state: "failed", transcript_error: "the transcript service answered 503" });
assert.deepEqual(transcriptParts(failed, true), {
  note: "Kein Transkript: the transcript service answered 503",
  ask: "Erneut versuchen",
});
assert.deepEqual(transcriptParts(failed, false), { note: "Kein Transkript: the transcript service answered 503" });
assert.deepEqual(transcriptParts(audio({ transcript_state: "failed" }), false), { note: "Kein Transkript:" });

// The text; an empty one says that nothing was said.
assert.deepEqual(transcriptParts(audio({ transcript: "Buy milk." }), false), { text: "Buy milk." });
assert.deepEqual(transcriptParts(audio({ transcript: "Buy milk." }), true), {
  text: "Buy milk.",
  ask: "Neu transkribieren",
});
assert.deepEqual(transcriptParts(audio({ transcript: "" }), false), { note: "Nichts gesprochen." });

// A state from a newer server is shown as nothing known.
assert.deepEqual(transcriptParts(audio({ transcript_state: "unknown" }), true), { ask: "Transkribieren" });

// In English too.
forceLang("en");
assert.deepEqual(transcriptParts(audio({ transcript: "" }), true), { note: "Nothing was said.", ask: "Transcribe again" });

console.log("transcript.mjs: ok");
