// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// web/lib/checklist.js against the cases of samu-core's checklist module
// (ADR 0141). Run by `make check` with node, no packages.

import assert from "node:assert/strict";

const root = new URL("../../", import.meta.url);
const { lines, itemsOf, progress, toSubtask, fromSubtask, carries, allToSubtasks, allFromSubtasks, placing, spliced, pasted } =
  await import(
  new URL("web/lib/checklist.js", root)
);

// The same text as the Rust test `pasted_lists_become_items`.
const text =
  "- [ ] Milch\n* Brot\n  + Käse\n1. Eier\n2) Butter\n[x] Salz\n- [X] Pfeffer\n\n   \n[ ]\nhat [x] drin\n[x]kein Leerzeichen";
assert.deepEqual(
  lines(text).map((l) => [l.text, l.checked]),
  [
    ["Milch", false],
    ["Brot", false],
    ["Käse", false],
    ["Eier", false],
    ["Butter", false],
    ["Salz", true],
    ["Pfeffer", true],
    ["hat [x] drin", false],
    ["[x]kein Leerzeichen", false],
  ],
);
assert.deepEqual(lines("a\r\nb"), [
  { text: "a", checked: false },
  { text: "b", checked: false },
]);
assert.deepEqual(lines(""), []);

const now = "2026-11-21T09:00:00Z";
let n = 0;
const hlc = () => ({ wall: ++n, counter: 0, device: "t" });
const task = (id, parent, position, more = {}) => ({ id, title: `t${id}`, parent, position, project: "p", ...more });
const item = (id, owner, position, checked = false) => ({ id, owner, text: `i${id}`, position, checked });

// Order and progress.
const items = [item("b", "T", 2, true), item("a", "T", 1), item("c", "U", 0)];
assert.deepEqual(itemsOf(items, "T").map((i) => i.id), ["a", "b"]);
assert.deepEqual(progress(items, "T"), [1, 2]);
assert.deepEqual(progress(items, "none"), [0, 0]);

// An item becomes the last subtask and goes.
const tasks = [task("T", null, 0), task("s1", "T", 4)];
const to = toSubtask({ tasks, items, item: "b", id: "new", now, hlc });
assert.equal(to.length, 2);
assert.equal(to[0].op, "create");
assert.equal(to[0].kind, "task");
assert.deepEqual(
  { title: to[0].value.title, parent: to[0].value.parent, position: to[0].value.position, project: to[0].value.project, done: to[0].value.completed_at },
  { title: "ib", parent: "T", position: 5, project: "p", done: now },
);
assert.deepEqual([to[1].op, to[1].object], ["delete", "b"]);
assert.equal(toSubtask({ tasks, items, item: "a", id: "x", now, hlc })[0].value.completed_at, undefined);
assert.throws(() => toSubtask({ tasks, items, item: "zz", id: "x", now, hlc }));

// All items at once: in order, each after the one before it.
let ids = 0;
const all = allToSubtasks({ tasks, items, task: "T", now, hlc, newId: () => `n${++ids}` });
assert.deepEqual(all.filter((c) => c.op === "create").map((c) => [c.value.title, c.value.position]), [["ia", 5], ["ib", 6]]);
assert.deepEqual(all.filter((c) => c.op === "delete").map((c) => c.object), ["a", "b"]);
assert.deepEqual(allToSubtasks({ tasks, items, task: "none", now, hlc, newId: () => "x" }), []);

// A plain subtask becomes the last item and goes.
const from = fromSubtask({ tasks, items, task: "s1", id: "it", now, hlc });
assert.equal(from[0].kind, "checklist_item");
assert.deepEqual(
  { owner: from[0].value.owner, text: from[0].value.text, position: from[0].value.position, checked: from[0].value.checked },
  { owner: "T", text: "ts1", position: 3, checked: undefined },
);
assert.deepEqual([from[1].op, from[1].object], ["delete", "s1"]);
const done = [task("T", null, 0), task("s2", "T", 0, { completed_at: now })];
assert.equal(fromSubtask({ tasks: done, items: [], task: "s2", id: "it", now, hlc })[0].value.checked, true);

// Every plain subtask at once: in order, after the existing items; the
// flagged one stays and is named.
{
  const subs = [task("T", null, 0), task("p", "T", 1), task("q", "T", 0, { flagged: true }), task("r", "T", 2, { completed_at: now })];
  let n = 0;
  const got = allFromSubtasks({ tasks: subs, items, task: "T", now, hlc, newId: () => `m${++n}` });
  assert.deepEqual(got.changes.filter((c) => c.op === "create").map((c) => [c.value.text, c.value.position, c.value.checked]), [["tp", 3, undefined], ["tr", 4, true]]);
  assert.deepEqual(got.changes.filter((c) => c.op === "delete").map((c) => c.object), ["p", "r"]);
  assert.deepEqual(got.skipped.map((s) => s.id), ["q"]);
  assert.throws(() => allFromSubtasks({ tasks: [task("T", null, 0), task("q", "T", 0, { flagged: true })], items: [], task: "T", now, hlc, newId: () => "x" }), /a flag/);
  assert.throws(() => allFromSubtasks({ tasks: [task("T", null, 0)], items: [], task: "T", now, hlc, newId: () => "x" }));
}

// What an item cannot keep is refused and named, never lost.
for (const [more, what] of [
  [{ note: "x" }, "a note"],
  [{ due: { date: "2026-12-01" } }, "dates"],
  [{ tags: [{ tag: "g", position: 0 }] }, "tags"],
  [{ estimate: "PT1H" }, "an estimate"],
  [{ flagged: true }, "a flag"],
  [{ priority: "A" }, "a priority"],
  [{ properties: { k: "v" } }, "properties"],
]) {
  assert.equal(carries(task("s", "T", 0, more)), what);
  assert.throws(
    () => fromSubtask({ tasks: [task("T", null, 0), task("s", "T", 0, more)], items: [], task: "s", id: "i", now, hlc }),
    (e) => e.message.includes(what),
  );
}
assert.equal(carries(task("s", "T", 0, { note: "  " })), null);

// What hangs on the subtask as an object of its own goes with the task
// when it is deleted, so each kind is refused and named.
{
  const pair = [task("T", null, 0), task("s", "T", 0), task("u", "T", 1)];
  const on = { owner: "s" };
  for (const [around, what] of [
    [{ comments: [on] }, "comments"],
    [{ attachments: [on] }, "attachments"],
    [{ clocks: [on] }, "clocked time"],
    [{ reminders: [on] }, "reminders"],
    [{ links: [{ owner: "s", uri: "https://example.org/", kind: "ref" }] }, "links"],
    [{ links: [{ owner: "u", uri: "samu:s", kind: "blocked_by" }] }, "a task that waits for it"],
  ]) {
    assert.throws(
      () => fromSubtask({ tasks: pair, items: [], around, task: "s", id: "i", now, hlc }),
      (e) => e.message.includes(what),
    );
  }
  // A plain reference to it from elsewhere is no reason.
  fromSubtask({ tasks: pair, items: [], around: { links: [{ owner: "u", uri: "samu:s", kind: "ref" }] }, task: "s", id: "i", now, hlc });
  // A checklist of its own would go with the task.
  assert.throws(
    () => fromSubtask({ tasks: pair, items: [{ id: "x", owner: "s", position: 0 }], task: "s", id: "i", now, hlc }),
    /a checklist of its own/,
  );
  // The whole list: the one with a comment stays and is named.
  const got = allFromSubtasks({ tasks: pair, items: [], around: { comments: [on] }, task: "T", now, hlc, newId: () => "n" });
  assert.equal(got.changes.length, 2);
  assert.deepEqual(got.skipped.map((x) => x.id), ["s"]);
  assert.match(got.skipped[0].reason, /comments/);
}
assert.throws(() => fromSubtask({ tasks, items, task: "T", id: "i", now, hlc }), /not a subtask/);
// A refusal carries its case and what it is about, for the page to word.
assert.throws(
  () => fromSubtask({ tasks, items, task: "T", id: "i", now, hlc }),
  (e) => e.code === "noParent" && e.what === undefined,
);
assert.throws(
  () => fromSubtask({ tasks: [task("T", null, 0), task("s", "T", 0, { flagged: true })], items: [], task: "s", id: "i", now, hlc }),
  (e) => e.code === "carries" && e.what === "a flag",
);
assert.throws(
  () => allFromSubtasks({ tasks: [task("T", null, 0)], items: [], task: "T", now, hlc, newId: () => "x" }),
  (e) => e.code === "noPlain",
);
{
  const got = allFromSubtasks({
    tasks: [task("T", null, 0), task("a", "T", 0), task("b", "T", 1, { priority: "A" })],
    items: [],
    task: "T",
    now,
    hlc,
    newId: () => "n",
  });
  assert.deepEqual(
    got.skipped.map((x) => [x.id, x.error.code, x.error.what]),
    [["b", "carries", "a priority"]],
  );
}
assert.throws(
  () => fromSubtask({ tasks: [...tasks, task("g", "s1", 0)], items, task: "s1", id: "i", now, hlc }),
  /subtasks of its own/,
);
assert.throws(
  () => fromSubtask({ tasks: [task("T", null, 0), task("s", "T", 0, { dropped_at: now })], items: [], task: "s", id: "i", now, hlc }),
  /dropped/,
);

console.log("checklist ok");

// New lines go after the item they were typed under, and only what gets
// another place is written.
{
  const list = [
    { id: "a", position: 0 },
    { id: "b", position: 1 },
    { id: "c", position: 2 },
  ];
  const fresh = [{ id: "n", isNew: true, value: { id: "n", text: "neu" } }];
  assert.deepEqual(
    spliced(list, "a", fresh).map((i) => i.id),
    ["a", "n", "b", "c"],
  );
  assert.deepEqual(
    spliced(list, null, fresh).map((i) => i.id),
    ["a", "b", "c", "n"],
  );
  assert.deepEqual(
    spliced(list, "gone", fresh).map((i) => i.id),
    ["a", "b", "c", "n"],
    "after an item that is gone: at the end",
  );
  assert.deepEqual(placing(spliced(list, "a", fresh)), [
    { create: { id: "n", text: "neu", position: 1 } },
    { set: "b", position: 2 },
    { set: "c", position: 3 },
  ]);
  assert.deepEqual(placing(list), [], "nothing moves, nothing is written");
  assert.deepEqual(placing(spliced(list, "c", [])), [], "no line, no item");
}

// Pasting into an empty item: the first line is its own text, the others
// follow; into one with a text, all follow.
{
  const got = lines("Milch\n[x] Brot\nEier");
  assert.deepEqual(pasted("  ", got), { own: got[0], rest: [got[1], got[2]] });
  assert.deepEqual(pasted("Käse", got), { own: null, rest: got });
}
