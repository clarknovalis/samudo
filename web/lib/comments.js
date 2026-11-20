// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Comments and attachments in the inspector (ADR 0039 §3, §5, §6).
//
// One composer under the comments takes text and files: pasted or dropped
// images, files picked, sound recorded with MediaRecorder. With text it
// becomes a comment and the files its attachments; without, the files hang
// on the task itself. A file goes up first (PUT /blobs/{sha256}), then the
// objects in one write. Changing or deleting is offered only on what the
// same kind of session wrote; the server holds the rule either way.

import { api, change } from "./api.js";
import { $, el, uuidv7 } from "./dom.js";
import { richText } from "./rich.js";
import { info } from "./message.js";
import { state, update } from "./state.js";
import { stamp } from "./words.js";
import { tr } from "./i18n.js";

// The largest file the server keeps (ADR 0039 §4).
const MAX_BYTES = 50 * 1024 * 1024;

// The composer of one task: { owner, text, files: [File] }. Lives here,
// not in state.js: typing must not draw the page again.
let draft = { owner: null, text: "", files: [] };
// The recorder while it runs: { owner, recorder, chunks }.
let recording = null;

function draftFor(owner) {
  if (draft.owner !== owner) draft = { owner, text: "", files: [] };
  return draft;
}

const sessionKind = () => state.session?.kind ?? "human";
// A comment or attachment without an author counts as a person's.
const kindOf = (object) => object.created_by?.kind ?? "human";
const mine = (object) => kindOf(object) === sessionKind();

const byAge = (a, b) => a.created.localeCompare(b.created) || a.id.localeCompare(b.id);

export const commentsOf = (owner) => state.comments.filter((c) => c.owner === owner).sort(byAge);
export const attachmentsOf = (owner) => state.attachments.filter((a) => a.owner === owner).sort(byAge);

function sizeText(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function author(object) {
  const by = object.created_by;
  return el("span", { class: "author" }, [
    by?.device ?? "",
    kindOf(object) === "agent" ? el("span", { class: "agent", text: "Agent" }) : null,
  ]);
}

async function write(changes) {
  try {
    await api.write(changes);
    update({ message: null });
    return true;
  } catch (error) {
    update({ message: error.message });
    return false;
  }
}

// Asks once more inline, never with a browser dialog.
let confirming = null;

function deleteButton(object) {
  if (confirming === object.id) {
    return el("span", { class: "confirm" }, [
      el("button", {
        type: "button",
        class: "danger",
        text: tr("Löschen"),
        onclick: () => {
          confirming = null;
          write([change.delete(object.id)]);
        },
      }),
      el("button", {
        type: "button",
        text: tr("Abbrechen"),
        onclick: () => {
          confirming = null;
          update({});
        },
      }),
    ]);
  }
  return el("button", {
    type: "button",
    class: "danger-quiet",
    text: tr("löschen"),
    onclick: () => {
      confirming = object.id;
      update({});
    },
  });
}

// A picture as a preview, sound and film with a player, anything else as
// a download. The cookie gets <img> and <audio> through (ADR 0039 §4).
function attachmentView(a) {
  const url = api.blobUrl(a.blob);
  const type = a.media_type.split(";")[0];
  let view;
  if (type.startsWith("image/")) {
    view = el("a", { href: url, target: "_blank", rel: "noopener" }, el("img", { src: url, alt: a.name, loading: "lazy" }));
  } else if (type.startsWith("audio/")) {
    view = el("audio", { src: url, controls: true, preload: "metadata" });
  } else if (type.startsWith("video/")) {
    view = el("video", { src: url, controls: true, preload: "metadata" });
  }
  return el("li", { class: "attachment" }, [
    view,
    el("div", { class: "line" }, [
      el("a", { href: url, download: a.name, text: a.name }),
      el("span", { class: "quiet", text: sizeText(a.size) }),
      a.created_by?.kind === "agent" ? el("span", { class: "agent", text: "Agent" }) : null,
      mine(a) ? deleteButton(a) : null,
    ]),
  ]);
}

function attachmentList(owner) {
  const list = attachmentsOf(owner);
  return list.length ? el("ul", { class: "attachments" }, list.map(attachmentView)) : null;
}

let editing = null;

function commentView(c) {
  // The server stamps `modified` a moment after `created` when it makes
  // one; edited means later than that.
  const edited = Date.parse(c.modified) - Date.parse(c.created) > 60_000;
  const body =
    editing === c.id
      ? el("div", { class: "edit" }, [
          el("textarea", { id: "comment-edit", rows: 4, "aria-label": tr("Kommentar"), text: c.body }),
          el("div", { class: "actions" }, [
            el("button", {
              type: "button",
              class: "primary",
              text: tr("Sichern"),
              onclick: async () => {
                const text = $("comment-edit").value.trim();
                if (!text) return;
                if (await write([change.set(c.id, "body", text)])) editing = null;
                update({});
              },
            }),
            el("button", {
              type: "button",
              text: tr("Abbrechen"),
              onclick: () => {
                editing = null;
                update({});
              },
            }),
          ]),
        ])
      : el(
          "div",
          { class: "body" },
          // Checkboxes switch only where the comment may be changed.
          richText(c.body, { onText: mine(c) ? (next) => write([change.set(c.id, "body", next)]) : undefined }),
        );
  return el("li", { class: "comment" }, [
    el("div", { class: "head" }, [
      author(c),
      el("span", { class: "when", text: stamp(c.created) + (edited ? ` · ${tr("bearbeitet")}` : "") }),
      mine(c) && editing !== c.id
        ? el("span", { class: "tools" }, [
            el("button", {
              type: "button",
              class: "link",
              text: tr("bearbeiten"),
              onclick: () => {
                editing = c.id;
                update({});
              },
            }),
            deleteButton(c),
          ])
        : null,
    ]),
    body,
    attachmentList(c.id),
  ]);
}

async function sha256(buffer) {
  const digest = await crypto.subtle.digest("SHA-256", buffer);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

// Uploads a file and returns the attachment object for `owner`.
// Uploads a file and answers the attachment for it; the voice note
// (voice.js) sets the owner after.
export async function attachmentFor(owner, file) {
  const buffer = await file.arrayBuffer();
  const hash = await sha256(buffer);
  const blob = await api.putBlob(hash, file.type || "application/octet-stream", buffer);
  const now = new Date().toISOString();
  return {
    id: uuidv7(),
    created: now,
    modified: now,
    owner,
    blob: blob.hash,
    name: file.name || tr("Datei"),
    media_type: blob.media_type,
    size: blob.size,
  };
}

function addFiles(owner, files) {
  const d = draftFor(owner);
  for (const file of files) {
    if (file.size > MAX_BYTES) {
      update({ message: tr("„{name}“ ist größer als 50 MB.", { name: file.name }) });
      continue;
    }
    d.files.push(file);
  }
  update({});
}

let sending = false;

async function send(owner) {
  const d = draftFor(owner);
  const field = $("comment-draft");
  if (field) d.text = field.value;
  const text = d.text.trim();
  if (sending || (!text && !d.files.length)) return;
  sending = true;
  update({ message: d.files.length ? info(tr("Lade hoch …")) : null });
  try {
    const changes = [];
    let target = owner;
    if (text) {
      const now = new Date().toISOString();
      const comment = { id: uuidv7(), created: now, modified: now, owner, body: text };
      changes.push(change.create("comment", comment));
      target = comment.id;
    }
    for (const file of d.files) changes.push(change.create("attachment", await attachmentFor(target, file)));
    await api.write(changes);
    draft = { owner, text: "", files: [] };
    update({ message: null });
  } catch (error) {
    update({ message: error.message });
  } finally {
    sending = false;
  }
}

async function toggleRecording(owner) {
  if (recording) {
    recording.recorder.stop();
    return;
  }
  if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === "undefined") {
    update({ message: tr("Dieser Browser nimmt nicht auf.") });
    return;
  }
  let stream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  } catch (error) {
    update({ message: `${tr("Kein Mikrofon:")} ${error.message}` });
    return;
  }
  const type = MediaRecorder.isTypeSupported?.("audio/webm") ? "audio/webm" : "";
  const recorder = new MediaRecorder(stream, type ? { mimeType: type } : undefined);
  const chunks = [];
  recorder.addEventListener("dataavailable", (event) => event.data.size && chunks.push(event.data));
  recorder.addEventListener("stop", () => {
    for (const track of stream.getTracks()) track.stop();
    const mime = recorder.mimeType || "audio/webm";
    const at = new Date().toISOString().slice(0, 16).replace(/[-:]/g, "").replace("T", "-");
    const file = new File(chunks, `Aufnahme-${at}.webm`, { type: mime });
    recording = null;
    addFiles(owner, [file]);
  });
  recording = { owner, recorder };
  recorder.start();
  update({});
}

function composer(owner) {
  const d = draftFor(owner);
  const picker = el("input", {
    type: "file",
    multiple: true,
    hidden: true,
    onchange: (event) => addFiles(owner, [...event.target.files]),
  });
  const pending = d.files.map((file, index) =>
    el("span", { class: "chip" }, [
      `${file.name} · ${sizeText(file.size)}`,
      el("button", {
        type: "button",
        "aria-label": tr("Datei entfernen"),
        text: "×",
        onclick: () => {
          d.files.splice(index, 1);
          update({});
        },
      }),
    ]),
  );
  const recordingHere = recording?.owner === owner;
  return el("div", { class: "composer" }, [
    el("textarea", {
      id: "comment-draft",
      rows: 3,
      "aria-label": tr("Kommentar schreiben"),
      placeholder: tr("Kommentar schreiben; Bilder einfügen oder hineinziehen"),
      text: d.text,
      oninput: (event) => {
        d.text = event.target.value;
      },
      onkeydown: (event) => {
        if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) send(owner);
      },
      onpaste: (event) => {
        const files = [...(event.clipboardData?.files ?? [])];
        if (files.length) {
          event.preventDefault();
          addFiles(owner, files);
        }
      },
      ondragover: (event) => event.preventDefault(),
      ondrop: (event) => {
        const files = [...(event.dataTransfer?.files ?? [])];
        if (files.length) {
          event.preventDefault();
          addFiles(owner, files);
        }
      },
    }),
    pending.length ? el("div", { class: "chips" }, pending) : null,
    el("div", { class: "actions" }, [
      el("button", {
        type: "button",
        class: "primary",
        text: tr("Senden (Strg+↵)"),
        onclick: () => send(owner),
      }),
      el("button", { type: "button", text: tr("Datei …"), onclick: () => picker.click() }),
      el("button", {
        type: "button",
        class: recordingHere ? "recording" : null,
        text: recordingHere ? tr("■ Aufnahme beenden") : tr("● Aufnehmen"),
        onclick: () => toggleRecording(owner),
      }),
      picker,
    ]),
    el("p", {
      class: "quiet",
      text: tr("Mit Text wird es ein Kommentar und die Dateien seine Anhänge; ohne Text hängen die Dateien an der Aufgabe."),
    }),
  ]);
}

// The two sections for the inspector of `owner`.
export function commentSections(owner, section) {
  const comments = commentsOf(owner);
  const own = attachmentList(owner);
  return [
    own ? section(tr("Anhänge"), own) : null,
    section(
      tr("Kommentare"),
      el("div", {}, [
        comments.length ? el("ol", { class: "comments" }, comments.map(commentView)) : null,
        composer(owner),
      ]),
    ),
  ];
}
