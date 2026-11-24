// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// A note shown rendered and edited in place (ADR 0058 §6), the same in
// the inspector and in a note block of a block perspective (ADR 0146 §3):
// the rendered text with a pencil; the pencil or a double click turns it
// into a field, an empty note is the field at once (or, `quiet`, a line
// that invites one), and leaving the field saves and shows it rendered
// again. A checkbox is ticked in the rendered text.

import { change } from "./api.js";
import { $, el, icon } from "./dom.js";
import { richText } from "./rich.js";
import { update } from "./state.js";
import { tr } from "./i18n.js";

const PENCIL = ["M4 20h4L19 9l-4-4L4 16v4z", "M13.5 6.5l4 4"];

// The place whose note is being edited, and its write on the way.
let editing = null;
let saving = null;

// `key` names the place (one note may stand in two), `id` the project or
// task, `text` its note, `write(changes)` sends, `field` is the id of the
// field to type in.
export function noteEditor({ key, id, text, write, field, quiet = false }) {
  const start = () => {
    editing = key;
    update({});
    requestAnimationFrame(() => {
      const node = $(field);
      if (!node) return;
      node.focus();
      node.setSelectionRange(node.value.length, node.value.length);
    });
  };
  const open = editing === key;
  if (!text.trim() && !open && quiet) {
    return el("button", { type: "button", class: "link note-start", text: tr("Notiz schreiben"), onclick: start });
  }
  if (!text.trim() || open) {
    return el("textarea", {
      id: field,
      rows: Math.max(6, Math.min(24, text.split("\n").length + 1)),
      "aria-label": tr("Notiz"),
      text,
      onchange: (event) => {
        saving = write([change.set(id, "note", event.target.value || null)]);
      },
      onblur: async () => {
        if (editing !== key) return;
        await saving;
        saving = null;
        editing = null;
        update({});
      },
    });
  }
  return el(
    "div",
    {
      class: "note-view",
      ondblclick: (event) => {
        if (event.target.closest("a, input, summary, button")) return;
        start();
      },
    },
    [
      el(
        "button",
        {
          type: "button",
          class: "edit-toggle",
          title: tr("Bearbeiten (Doppelklick)"),
          "aria-label": tr("Notiz bearbeiten"),
          onclick: start,
        },
        icon(PENCIL),
      ),
      richText(text, { onText: (next) => write([change.set(id, "note", next)]) }),
    ],
  );
}
