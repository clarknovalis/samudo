// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The account's logbook in the settings (ADR 0123 §6): newest first,
// filtered by kind, device and span. A tool to look things up, so only
// here, not in the sidebar. Fetched when the section opens and when a
// filter changes; it does not follow /events.

import { api } from "./api.js";
import { el } from "./dom.js";
import { state, update } from "./state.js";
import { logWords, stamp } from "./words.js";
import { tr } from "./i18n.js";

const RANGES = [
  ["1d", tr("24 Stunden"), 1],
  ["7d", tr("7 Tage"), 7],
  ["30d", tr("30 Tage"), 30],
];
const KINDS = ["deleted", "moved", "completed", "dropped", "reopened", "state"];

const log = { range: "7d", kinds: [], device: "", entries: null, devices: [], asked: null, error: null };

async function fetchLog() {
  const days = RANGES.find(([key]) => key === log.range)?.[2] ?? 7;
  const since = new Date(Date.now() - days * 86400000).toISOString();
  const asked = JSON.stringify([log.range, log.kinds, log.device]);
  log.asked = asked;
  try {
    const entries = await api.accountLogbook({ since, kinds: log.kinds, device: log.device });
    if (log.asked !== asked) return;
    log.entries = entries;
    log.error = null;
    // The devices to choose from: those seen, and the one chosen.
    log.devices = [...new Set([...log.devices, ...entries.map((e) => e.device)])].sort();
  } catch (error) {
    log.error = error.message;
  }
  update({});
}

const OBJECT_WORDS = {
  task: tr("Aufgabe"),
  project: tr("Projekt"),
  folder: tr("Ordner"),
  tag: "Tag",
  clock: "Clock",
  appointment: tr("Termin"),
  comment: tr("Kommentar"),
};

// What a delete took along, or where a move went.
function detail(e) {
  if (e.kind === "deleted" && e.new_value && typeof e.new_value === "object") {
    const parts = [
      [e.new_value.deleted - 1, (n) => tr("{n} mitgelöscht", { n })],
      [e.new_value.moved, (n) => tr("{n} verschoben", { n })],
      [e.new_value.untagged, (n) => tr("{n} ohne den Tag", { n })],
    ];
    return parts
      .filter(([n]) => n > 0)
      .map(([n, text]) => text(n))
      .join(", ");
  }
  if (e.kind === "moved") {
    const name = (id) =>
      id ? (state.projects.find((p) => p.id === id) ?? state.folders?.find((f) => f.id === id) ?? state.tasks.find((t) => t.id === id))?.title ?? id.slice(-8) : tr("Inbox");
    return `${name(e.old_value)} → ${name(e.new_value)}`;
  }
  return e.note ?? "";
}

// To the object, while it is there.
function open(e) {
  if (e.deleted) return null;
  if (e.object_kind === "task") return () => update({ settings: null, selected: e.object, pane: "detail" });
  if (e.object_kind === "project") return () => update({ settings: null, view: { kind: "project", project: e.object }, selected: null });
  return null;
}

export function logbookPane() {
  if (log.entries === null && log.asked === null) fetchLog();
  const set = (patch) => {
    Object.assign(log, patch);
    fetchLog();
  };
  const ranges = el(
    "div",
    { class: "ranges" },
    RANGES.map(([key, text]) =>
      el("button", { type: "button", class: log.range === key ? "current" : null, "aria-pressed": String(log.range === key), text, onclick: () => set({ range: key }) }),
    ),
  );
  const kinds = el(
    "div",
    { class: "ranges" },
    KINDS.map((kind) => {
      const on = log.kinds.includes(kind);
      return el("button", {
        type: "button",
        class: on ? "current" : null,
        "aria-pressed": String(on),
        text: logWords[kind] ?? kind,
        onclick: () => set({ kinds: on ? log.kinds.filter((k) => k !== kind) : [...log.kinds, kind] }),
      });
    }),
  );
  const device = el(
    "select",
    { "aria-label": tr("Gerät"), onchange: (event) => set({ device: event.target.value }) },
    [el("option", { value: "", text: tr("alle Geräte") }), ...log.devices.map((d) => el("option", { value: d, text: d, selected: d === log.device || null }))],
  );
  const entries = log.entries ?? [];
  const body = log.error
    ? el("p", { class: "message", text: log.error })
    : log.entries === null
      ? el("p", { class: "quiet", text: tr("Lädt …") })
      : !entries.length
        ? el("p", { class: "quiet", text: tr("Noch nichts.") })
        : el(
            "ol",
            { class: "logbook account-log" },
            entries.map((e) => {
              const go = open(e);
              const what = e.title ?? e.object.slice(-8);
              return el("li", {}, [
                el("span", { class: "when", text: stamp(e.at) }),
                el("span", { class: "kind", text: logWords[e.kind] ?? e.kind }),
                el("span", { class: "object" }, [
                  el("span", { class: "quiet", text: `${OBJECT_WORDS[e.object_kind] ?? e.object_kind} ` }),
                  go ? el("button", { type: "button", class: "link", text: what, onclick: go }) : el("span", { class: e.deleted ? "gone" : null, text: what }),
                ]),
                el("span", { class: "device", text: e.author_kind === "agent" ? `${e.device} · Agent` : e.device }),
                el("span", { class: "note", text: detail(e) }),
              ]);
            }),
          );
  return el("div", { class: "account-logbook" }, [
    el("p", { class: "quiet", text: tr("Was im Konto gelöscht, verschoben, erledigt … wurde, das Neueste zuerst.") }),
    el("div", { class: "time-controls" }, [ranges, kinds, device]),
    body,
  ]);
}

// Opened afresh each time the section is chosen.
export function forgetLogbook() {
  log.entries = null;
  log.asked = null;
}
