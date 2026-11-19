// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Changing a tag's workflow fields afterwards (e076022f): whether it is a
// workflow, the default workflow, its status, and for a state its key and
// whether it asks for a note. The rules are those of `samu tag set` and
// samu-core's tagfields, held to the same answers by
// testdata/tagfields/cases.json.

// What a tag is: "workflow", "state" (a child of one) or "tag".
export function tagRole(tags, tag) {
  if (tag.exclusive) return "workflow";
  const parent = tags.find((t) => t.id === tag.parent);
  return parent?.exclusive ? "state" : "tag";
}

// The changes for `fields` ({exclusive, default_workflow, status, key,
// ask_note}, each left out when unchanged; key null takes it off), as
// [tag, field, value], or { error } with one of: nothing, no_workflow,
// in_use, not_a_state, key_length, key_taken.
export function tagChanges(tags, projects, id, fields) {
  const tag = tags.find((t) => t.id === id);
  if (!tag) return { error: "unknown" };
  const given = (name) => Object.hasOwn(fields, name) && fields[name] !== undefined;
  if (!["exclusive", "default_workflow", "status", "key", "ask_note"].some(given)) return { error: "nothing" };
  const exclusive = given("exclusive") ? fields.exclusive : Boolean(tag.exclusive);
  const parent = tags.find((t) => t.id === tag.parent);
  const isState = Boolean(parent?.exclusive);
  const changes = [];
  if (given("exclusive") && fields.exclusive !== Boolean(tag.exclusive)) {
    if (!fields.exclusive && projects.some((p) => p.workflow === id)) return { error: "in_use" };
    changes.push([id, "exclusive", fields.exclusive]);
    if (!fields.exclusive && tag.default_workflow && fields.default_workflow !== true) {
      changes.push([id, "default_workflow", false]);
    }
  }
  if (given("default_workflow")) {
    const on = fields.default_workflow;
    if (on && !exclusive) return { error: "no_workflow" };
    if (on !== Boolean(tag.default_workflow)) changes.push([id, "default_workflow", on]);
    if (on) {
      for (const other of tags) {
        if (other.default_workflow && other.id !== id) changes.push([other.id, "default_workflow", false]);
      }
    }
  }
  if (given("status")) changes.push([id, "status", fields.status]);
  if ((Object.hasOwn(fields, "key") || given("ask_note")) && !isState) return { error: "not_a_state" };
  if (Object.hasOwn(fields, "key")) {
    const key = fields.key;
    if (key !== null) {
      if ([...key].length !== 1) return { error: "key_length" };
      if (tags.some((t) => t.id !== id && t.parent === tag.parent && t.key === key)) return { error: "key_taken" };
    }
    changes.push([id, "key", key]);
  }
  if (given("ask_note")) changes.push([id, "ask_note", fields.ask_note]);
  return { changes };
}
