// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Bulk actions reload once (web/lib/api.js, `together`): the writes inside
// do not each wait for a reload of every view. Run by `make check` with
// node, no packages; the server is a stand-in for `fetch`.

const root = new URL("../../", import.meta.url);

let posts = 0;
globalThis.fetch = async (path) => {
  posts += 1;
  const fails = String(path).includes("bad");
  return {
    ok: !fails,
    status: fails ? 422 : 200,
    text: async () => (fails ? JSON.stringify({ message: "no" }) : JSON.stringify({ seq: posts })),
  };
};

const { api, holding, onWritten, together } = await import(new URL("web/lib/api.js", root));

let failed = 0;
const same = (what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    failed += 1;
    console.error(`not ok  ${what}: ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
  }
};

let reloads = 0;
onWritten(async () => {
  reloads += 1;
});

// Outside: every write reloads, as before.
await api.deleteObject("a", { preview: false });
await api.deleteObject("b", { preview: false });
same("a reload per write outside", reloads, 2);

// Inside: one reload for all, after the last write.
reloads = 0;
posts = 0;
let during = null;
await together(async () => {
  for (const id of ["a", "b", "c"]) await api.deleteObject(id, { preview: false });
  during = [reloads, holding()];
});
same("all writes went out", posts, 3);
same("no reload while it runs", during, [0, true]);
same("one reload at the end", [reloads, holding()], [1, false]);

// Nothing written, nothing reloaded: a preview does not count.
reloads = 0;
await together(() => api.deleteObject("a", { preview: true }));
same("no reload without a write", reloads, 0);

// A failure in the middle still shows what was written before it.
reloads = 0;
let thrown = null;
try {
  await together(async () => {
    await api.deleteObject("a", { preview: false });
    await api.deleteObject("bad", { preview: false });
  });
} catch (error) {
  thrown = error.message;
}
same("the failure comes through", thrown, "no");
same("what was written is reloaded", [reloads, holding()], [1, false]);

// Nested: the outermost one reloads.
reloads = 0;
await together(async () => {
  await together(() => api.deleteObject("a", { preview: false }));
  same("the inner one waits", reloads, 0);
});
same("the outer one reloads", reloads, 1);

if (failed) process.exit(1);
console.log("ok   bulk actions reload once");
