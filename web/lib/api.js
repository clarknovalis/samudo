// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The only way to the server (ADR 0025). Relative paths throughout, so this
// works on a server of its own, on localhost and later inside a desktop shell.

import { tr } from "./i18n.js";

export class ApiError extends Error {
  constructor(message, status, code) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

async function request(path, options) {
  const response = await fetch(path, options);
  const text = await response.text();
  if (!response.ok) {
    let message = text;
    let code;
    try {
      const parsed = JSON.parse(text);
      message = parsed.message ?? text;
      code = parsed.error;
    } catch {
      // Not JSON: an answer from something in front of the server.
    }
    // Only the server can, and the replica on this computer cannot reach
    // it (ADR 0066 §5).
    if (code === "offline") message = tr("Nur mit Verbindung zum Server möglich.");
    throw new ApiError(message || `${response.status}`, response.status, code);
  }
  return text ? JSON.parse(text) : null;
}

const get = (path) => request(path);
const post = (path, body) =>
  request(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

// A path through a scope (ADR 0035 §5), or as it is without one.
const scoped = (path, scope) =>
  scope ? `${path}${path.includes("?") ? "&" : "?"}scope=${encodeURIComponent(scope)}` : path;

// The server stamps online writes itself (ADR 0019 §4); this is a
// placeholder it replaces.
const hlc = () => ({ wall: 0, counter: 0, device: "web" });

// Called after every write the server applied, so the page reloads at
// once instead of waiting for the round trip over `/events`.
let reload = () => {};
export function onWritten(callback) {
  reload = callback;
}

// Several writes with one reload at the end (bulk actions): each write
// used to wait for its own reload of every view before the next one went
// out, which made twenty tasks take twenty reloads.
let held = 0;
let owed = false;
export const holding = () => held > 0;
const written = () => {
  if (!held) return reload();
  owed = true;
  return undefined;
};
// Runs `run`; the writes in it reload once, when it returns or throws, and
// only if one of them was applied.
export async function together(run) {
  held += 1;
  try {
    return await run();
  } finally {
    held -= 1;
    if (!held && owed) {
      owed = false;
      await reload();
    }
  }
}

export const api = {
  session: () => get("/session"),
  login: (account, password, device) => post("/login", { account, password, device }),
  logout: () => post("/logout", {}),
  // The sessions of the account, and ending one (ADR 0095 §7); only for a
  // person's session of the whole account.
  sessions: () => get("/sessions"),
  endSession: (id) => post(`/sessions/${encodeURIComponent(id)}/end`, {}),
  // Whether this server takes new accounts, without a session (ADR 0099 §2).
  registration: () => get("/register"),
  // A new account, signed in by its cookie; `password` only when the
  // server made it.
  register: (body) => post("/register", body),
  // The account of the session goes with everything in it (ADR 0099 §5).
  deleteAccount: (password) => post("/account/delete", { password }),
  // Demo data into this account (ADR 0099 §7).
  async addDemo(lang) {
    const added = await post("/demo", { lang });
    await written();
    return added;
  },
  // A pairing code for a phone, after the password once more (ADR 0031).
  pairCode: (device, password) => post("/pair/code", { device, password }),
  // What changed after a change number (ADR 0004 §5): every live object
  // after 0, the deleted ones under `gone`.
  objects: (after) => get(`/changes?after=${after}`),
  // What samu-core computed for every task, done ones too, under the
  // task's `id`: the tasks themselves come from `/changes` (ADR 0065).
  // With a scope only what it shows, and what is due outside marked
  // `outside`.
  // With `elsewhere` too what another own scope alone shows, marked with
  // it (ADR 0124 §2).
  statuses: (scope) => get(scoped("/available?all=true&bare=true&elsewhere=true", scope)),
  // Every open project with what samu-core computed: available, stuck;
  // those of another own scope marked `elsewhere`.
  projectStatuses: (scope) => get(scoped("/available/projects?elsewhere=true", scope)),
  // The open habits the scope shows (ADR 0091 §4), with the history and
  // entries from `from` on.
  habits: (scope, from) => get(scoped(`/habits?from=${from}`, scope)),
  // Forecast as samu-core computes it (ADR 0045), through the scope.
  forecast: (scope) => get(scoped("/forecast", scope)),
  // The occurrences of some days, repetitions unfolded (ADR 0048); with a
  // scope only its calendars (ADR 0047 §12).
  appointments: (from, to, scope) => get(scoped(`/appointments?from=${from}&to=${to}`, scope)),
  // ADR 0101 §5: reminders that go off from `from` to `to` (instants).
  alarms: (from, to, tasks, appointments) =>
    get(`/alarms?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}&tasks=${tasks}&appointments=${appointments}`),
  // An edit or a delete of an appointment or of occurrences of its
  // repetition (ADR 0068 §5); samu-core makes the changes.
  async editAppointment(id, edit) {
    const answer = await post(`/appointments/${encodeURIComponent(id)}/edit`, edit);
    await written();
    return answer;
  },
  // Where a calendar comes from (ADR 0047 §3): sealed on the server,
  // answered with nothing.
  calendarSource: (id, source) =>
    request(`/calendars/${encodeURIComponent(id)}/source`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(source),
    }),
  // Fetches now (§7) and answers the calendar as it is after; what went
  // wrong is under `error`.
  async refreshCalendar(id) {
    const calendar = await post(`/calendars/${encodeURIComponent(id)}/refresh`, {});
    await written();
    return calendar;
  },
  // The collections on a CalDAV server (§8); makes nothing.
  discoverCalendars: (source) => post("/calendars/discover", source),
  // The others on the server of a CalDAV calendar, with its stored source
  // (ADR 0100 §1).
  discoverFrom: (id) => post(`/calendars/${encodeURIComponent(id)}/discover`, {}),
  // An iCalendar file into a writable calendar (ADR 0100 §4): each UID as
  // it is in the file; the calendar is fetched afterwards.
  async importCalendar(id, file) {
    const answer = await request(
      `/calendars/${encodeURIComponent(id)}/import?name=${encodeURIComponent(file.name)}`,
      { method: "POST", headers: { "Content-Type": "text/calendar" }, body: file },
    );
    await written();
    return answer;
  },
  // The time clocked over days (ADR 0046 §5), through the scope.
  clockReport: (from, to, by, scope) =>
    get(scoped(`/clocks/report?from=${from}&to=${to}&by=${by}`, scope)),
  // Done, worked on and time (ADR 0080 §4); `q` in the search language.
  report: (from, to, by, scope, q, dropped) =>
    get(
      scoped(
        `/report?from=${from}&to=${to}&by=${by}&q=${encodeURIComponent(q)}${dropped ? "&dropped=true" : ""}`,
        scope,
      ),
    ),
  // What the scopes' schedules pick now (ADR 0043 §7).
  scheduled: () => get("/scopes/scheduled"),
  // The folders the views show, an empty one among them (ADR 0037 §4).
  folders: (scope) => get(scoped("/available/folders", scope)),
  // Quick capture (ADR 0027): with `preview`, what would be made; the
  // scope chosen here fills in what the line names not (ADR 0043 §6).
  // With `task`, the line edits that task instead (ADR 0078 §3).
  async capture(text, project, preview, scope, parent = null, task = null) {
    const answer = await post("/capture", {
      text,
      project: project ?? undefined,
      parent: parent ?? undefined,
      scope: scope ?? undefined,
      task: task ?? undefined,
      preview,
    });
    if (!preview) written();
    return answer;
  },
  // Several lines at once (ADR 0129): a task per line, an indented line
  // under the line above it, all or none.
  async captureLines(text, project, preview, scope, parent = null) {
    const answer = await post("/capture/lines", {
      text,
      project: project ?? undefined,
      parent: parent ?? undefined,
      scope: scope ?? undefined,
      preview,
    });
    if (!preview) written();
    return answer;
  },
  // Notes and comments as document trees (ADR 0058), one per text.
  render: (texts) => post("/render", { texts: texts.map((text) => ({ text })) }),
  logbook: (id) => get(`/objects/${encodeURIComponent(id)}/logbook`),
  // The account's logbook, newest first (ADR 0123 §4).
  accountLogbook: ({ since, kinds = [], device = "" }) => {
    const query = [`since=${encodeURIComponent(since)}`, "limit=500"];
    if (kinds.length) query.push(`kind=${kinds.join(",")}`);
    if (device) query.push(`device=${encodeURIComponent(device)}`);
    return get(`/logbook?${query.join("&")}`);
  },
  // A file under its SHA-256 (ADR 0039 §4); answers its hash, size and the
  // type it was first recorded with.
  putBlob: (hash, type, body) =>
    request(`/blobs/${hash}`, { method: "PUT", headers: { "Content-Type": type }, body }),
  // Where <img>, <audio> and a download find a file; the cookie gets
  // them through.
  // Asks for the transcript of an audio attachment (ADR 0140 §3); the
  // text comes later, with the changes.
  transcribe: (id) => post(`/attachments/${encodeURIComponent(id)}/transcribe`, {}),
  blobUrl: (hash) => `/blobs/${encodeURIComponent(hash)}`,
  // Deletes with what hangs on it (ADR 0036); with `preview`, the counts.
  async deleteObject(id, body) {
    const answer = await post(`/objects/${encodeURIComponent(id)}/delete`, body);
    if (!body.preview) await written();
    return answer;
  },

  // Completes or drops a task, with what is open below it closed along,
  // lifted out, or with `ask` not yet (ADR 0128 §3).
  async closeObject(id, body) {
    const answer = await post(`/objects/${encodeURIComponent(id)}/close`, body);
    if (answer.seq != null) await written();
    return answer;
  },

  // Sends changes and fails unless every one was applied.
  async write(changes) {
    const outcomes = await post("/changes?stamp=server", changes);
    for (const outcome of outcomes) {
      if (outcome.outcome === "rejected") throw new ApiError(outcome.reason, 422, "rejected");
      if (outcome.outcome !== "applied") {
        throw new ApiError(`not applied: ${JSON.stringify(outcome)}`, 409, outcome.outcome);
      }
    }
    // Waits for the reload, so what was written is drawn when this returns.
    await written();
    return outcomes;
  },
};

export const change = {
  create: (kind, value) => ({ op: "create", object: value.id, kind, value, hlc: hlc() }),
  set: (object, field, value, note) => ({ op: "set", object, field, value, note, hlc: hlc() }),
  // "Geprüft" (ADR 0088 §3): reviewed now, and no date set by hand.
  reviewed: (project) => [
    { op: "set", object: project, field: "last_reviewed", value: new Date().toISOString(), hlc: hlc() },
    { op: "set", object: project, field: "next_review", value: null, hlc: hlc() },
  ],
  addTag: (object, tag, note) => ({
    op: "add",
    object,
    field: "tags",
    element: tag,
    position: 0,
    note,
    hlc: hlc(),
  }),
  // The members of a scope (ADR 0035 §1): a set like tags.
  addMember: (scope, member) => ({
    op: "add",
    object: scope,
    field: "members",
    element: member,
    position: 0,
    hlc: hlc(),
  }),
  removeMember: (scope, member) => ({ op: "remove", object: scope, field: "members", element: member, hlc: hlc() }),
  delete: (object) => ({ op: "delete", object, hlc: hlc() }),
  removeTag: (object, tag, note) => ({
    op: "remove",
    object,
    field: "tags",
    element: tag,
    note,
    hlc: hlc(),
  }),
};
