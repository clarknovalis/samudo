// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// A write drawn before the server answers (web/lib/local.js): the changes
// on copies of the objects, as samu_core::overlay applies them, and taken
// back exactly. Run by `make check` with node, no packages.

const root = new URL("../../", import.meta.url);
const { applyLocal, drawsAtOnce } = await import(new URL("web/lib/local.js", root));

let failed = 0;
const same = (what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    failed += 1;
    console.error(`not ok  ${what}: ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
  }
};

const NOW = "2026-01-02T03:04:05.000Z";
const task = (id, more = {}) => [id, { kind: "task", object: { id, title: id, position: 0, tags: [], ...more } }];
const world = () =>
  new Map([
    task("a", { project: "p", properties: { ort: "Keller" } }),
    task("b", { parent: "a", project: "p" }),
    task("c", { parent: "b", project: "p" }),
    task("d", { parent: "a", project: "p", completed_at: "2025-12-01T00:00:00Z" }),
    task("e"),
    ["p", { kind: "project", object: { id: "p", title: "P", tags: [] } }],
    ["w", { kind: "tag", object: { id: "w", title: "Workflow", exclusive: true } }],
    ["w1", { kind: "tag", object: { id: "w1", title: "Eins", parent: "w" } }],
    ["w2", { kind: "tag", object: { id: "w2", title: "Zwei", parent: "w" } }],
    ["x", { kind: "tag", object: { id: "x", title: "Frei" } }],
    ["s", { kind: "scope", object: { id: "s", title: "S", members: ["p"] } }],
  ]);
const field = (objects, id, name) => objects.get(id)?.object[name];

// A field, a field taken away with null, a property by key.
{
  const objects = world();
  const original = objects.get("a");
  const undo = applyLocal(objects, [
    { op: "set", object: "a", field: "note", value: "neu" },
    { op: "set", object: "a", field: "project", value: null },
    { op: "set", object: "a", field: "properties.ticket", value: "T-1" },
    { op: "set", object: "a", field: "properties.ort", value: null },
  ]);
  same("set a field", field(objects, "a", "note"), "neu");
  same("null takes the field away", "project" in objects.get("a").object, false);
  same("properties by key", field(objects, "a", "properties"), { ticket: "T-1" });
  same("the entry drawn before stays as it was", original.object.note, undefined);
  undo();
  same("undo puts the very entry back", objects.get("a") === original, true);
}

// Tags: added at the end, a state of a workflow replaces its sibling,
// removed; members of a scope.
{
  const objects = world();
  applyLocal(objects, [
    { op: "add", object: "e", field: "tags", element: "x", position: 0 },
    { op: "add", object: "e", field: "tags", element: "w1", position: 1 },
    { op: "add", object: "e", field: "tags", element: "w2", position: 2 },
    { op: "add", object: "e", field: "tags", element: "x", position: 3 },
  ]);
  same("tags, a state replacing its sibling", field(objects, "e", "tags").map((t) => t.tag), ["x", "w2"]);
  applyLocal(objects, [{ op: "remove", object: "e", field: "tags", element: "x" }]);
  same("a tag removed", field(objects, "e", "tags").map((t) => t.tag), ["w2"]);
  applyLocal(objects, [
    { op: "add", object: "s", field: "members", element: "e", position: 0 },
    { op: "remove", object: "s", field: "members", element: "p" },
  ]);
  same("members of a scope", field(objects, "s", "members"), ["e"]);
}

// A plain delete takes the object alone; the route's delete a task with
// everything below it.
{
  const objects = world();
  const undo = applyLocal(objects, [{ op: "delete", object: "b" }]);
  same("a plain delete", ["b", "c"].map((id) => objects.has(id)), [false, true]);
  undo();
  applyLocal(objects, [{ op: "delete", object: "a", below: true }]);
  same("the route takes the tasks below", ["a", "b", "c", "d", "e"].map((id) => objects.has(id)), [
    false,
    false,
    false,
    false,
    true,
  ]);
}

// Created, and taken back.
{
  const objects = world();
  const undo = applyLocal(objects, [{ op: "create", object: "n", kind: "task", value: { id: "n", title: "neu" } }]);
  same("created", field(objects, "n", "title"), "neu");
  undo();
  same("taken back", objects.has("n"), false);
}

// Closing: with `close` the open ones below along, with `lift` the open
// children up to the task's place, with `ask` and open ones below nothing.
{
  let objects = world();
  applyLocal(objects, [{ op: "close", object: "a", field: "completed", children: "close" }], NOW);
  same("closed along", ["a", "b", "c"].map((id) => field(objects, id, "completed_at")), [NOW, NOW, NOW]);
  same("the closed one below keeps its date", field(objects, "d", "completed_at"), "2025-12-01T00:00:00Z");

  objects = world();
  applyLocal(objects, [{ op: "close", object: "b", field: "dropped", children: "lift" }], NOW);
  same("dropped", field(objects, "b", "dropped_at"), NOW);
  same("the open child lifted", field(objects, "c", "parent"), "a");

  objects = world();
  applyLocal(objects, [{ op: "close", object: "a", field: "completed", children: "ask" }], NOW);
  same("ask with open ones below writes nothing", field(objects, "a", "completed_at"), undefined);
  applyLocal(objects, [{ op: "close", object: "c", field: "completed", children: "ask" }], NOW);
  same("ask with none open below closes", field(objects, "c", "completed_at"), NOW);
}

// What is not known here is left alone; undo leaves alone what a reload
// put there in the meantime.
{
  const objects = world();
  const before = JSON.stringify([...objects]);
  applyLocal(objects, [{ op: "move", object: "a" }, { op: "set", object: "zz", field: "note", value: "x" }]);
  same("unknown left alone", JSON.stringify([...objects]), before);

  const undo = applyLocal(objects, [{ op: "set", object: "e", field: "note", value: "hier" }]);
  const fromServer = { kind: "task", object: { id: "e", title: "e", note: "vom Server" } };
  objects.set("e", fromServer);
  undo();
  same("undo keeps what a reload brought", objects.get("e") === fromServer, true);
}

// What can be drawn at once.
{
  const objects = world();
  same("sets are drawn", drawsAtOnce(objects, [{ op: "set", object: "a", field: "note", value: "" }]), true);
  same("a create waits", drawsAtOnce(objects, [{ op: "create", object: "n", kind: "task", value: {} }]), false);
  same("a task's delete is drawn", drawsAtOnce(objects, [{ op: "delete", object: "a", below: true }]), true);
  same("a project's delete waits", drawsAtOnce(objects, [{ op: "delete", object: "p", below: true }]), false);
}

if (failed) process.exit(1);
console.log("ok   writes are drawn before the server answers");
