// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The writes of web/lib/api.js with a drawing (onWriting): drawn before the
// request goes out, taken back when the server refuses, and the reload
// behind them not waited for. Run by `make check` with node, no packages;
// the server is a stand-in for `fetch`.

const root = new URL("../../", import.meta.url);

const log = [];
let answer = () => ({ ok: true, status: 200, body: [{ outcome: "applied", seq: 1 }] });
globalThis.fetch = async (path) => {
  log.push(`post ${path}`);
  const { ok, status, body } = answer(String(path));
  return { ok, status, text: async () => JSON.stringify(body) };
};

const { api, change, onWriting, onWritten } = await import(new URL("web/lib/api.js", root));

let failed = 0;
const same = (what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    failed += 1;
    console.error(`not ok  ${what}: ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
  }
};

// A reload that finishes only when told to.
let finish = null;
onWritten(() => {
  log.push("reload");
  return new Promise((resolve) => {
    finish = () => {
      log.push("reloaded");
      resolve();
    };
  });
});
let draws = true;
onWriting((changes) => {
  if (!draws) return null;
  log.push(`draw ${changes.map((c) => c.op).join(",")}`);
  return () => log.push("undo");
});

// Drawn first, then sent; back before the reload is done.
log.length = 0;
await api.write([change.set("a", "note", "x")]);
same("drawn, sent, reload not waited for", log, ["draw set", "post /changes?stamp=server", "reload"]);
finish();

// Refused: taken back, the reload shows what went before it.
log.length = 0;
answer = () => ({ ok: true, status: 200, body: [{ outcome: "applied", seq: 1 }, { outcome: "rejected", reason: "no" }] });
let thrown = null;
await api.write([change.set("a", "note", "x"), change.set("b", "note", "y")]).catch((e) => {
  thrown = e.message;
});
same("refused: taken back and reloaded", [log, thrown], [["draw set,set", "post /changes?stamp=server", "undo", "reload"], "no"]);
finish();

// Failed on the way: taken back, nothing to reload.
log.length = 0;
answer = () => ({ ok: false, status: 503, body: { message: "down" } });
await api.write([change.set("a", "note", "x")]).catch(() => {});
same("failed: taken back", log, ["draw set", "post /changes?stamp=server", "undo"]);

// Without a drawing the write waits for the reload, as before.
log.length = 0;
draws = false;
answer = () => ({ ok: true, status: 200, body: [{ outcome: "applied", seq: 1 }] });
let back = false;
const waiting = api.write([change.set("a", "note", "x")]).then(() => {
  back = true;
});
await new Promise((resolve) => setTimeout(resolve, 0));
same("not drawn: still waiting for the reload", back, false);
finish();
await waiting;
same("not drawn: back after the reload", log, ["post /changes?stamp=server", "reload", "reloaded"]);
draws = true;

// Delete: the route's delete drawn, a preview not.
log.length = 0;
answer = () => ({ ok: true, status: 200, body: { deleted: 1, moved: 0, untagged: 0, seq: 2 } });
await api.deleteObject("a", { preview: true });
await api.deleteObject("a", { preview: false });
same("delete drawn, preview not", log, ["post /objects/a/delete", "draw delete", "post /objects/a/delete", "reload"]);
finish();

// Close: taken back when the server wrote nothing (open ones below).
log.length = 0;
answer = () => ({ ok: true, status: 200, body: { open: 2, closed: 0, lifted: 0 } });
await api.closeObject("a", { field: "completed", children: "ask" });
same("close without a write taken back", log, ["draw close", "post /objects/a/close", "undo"]);
log.length = 0;
answer = () => ({ ok: true, status: 200, body: { open: 0, closed: 1, lifted: 0, seq: 3 } });
await api.closeObject("a", { field: "completed", children: "ask" });
same("close drawn and reloaded", log, ["draw close", "post /objects/a/close", "reload"]);
finish();

// A drawing that fails: the write goes out all the same and waits.
log.length = 0;
answer = () => ({ ok: true, status: 200, body: [{ outcome: "applied", seq: 4 }] });
onWriting(() => {
  throw new Error("broken");
});
const quiet = console.error;
console.error = () => {};
const sent = api.write([change.set("a", "note", "x")]);
await new Promise((resolve) => setTimeout(resolve, 0));
finish();
await sent;
console.error = quiet;
same("a broken drawing still writes", log, ["post /changes?stamp=server", "reload", "reloaded"]);

if (failed) process.exit(1);
console.log("ok   writes are drawn first and not held up by the reload");
