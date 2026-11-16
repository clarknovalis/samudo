// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Clocking (ADR 0046): the status bar with the running clock, `c` on a
// task and the section „Zeit“ in the inspector. The view „Zeit“ became the
// tab „Zeit“ of the report (ADR 0081 §1, report.js).
//
// Starting and stopping are ordinary changes; that one clock runs and that
// a clock stops with its task, the server sees to (§2, §3). The running
// length here is the one thing counted in the browser: the seconds since a
// start the server sent.

import { api, change } from "./api.js";
import { $, el, replace, uuidv7 } from "./dom.js";
import { connected } from "./live.js";
import { inEffect, pretty } from "./keymap.js";
import { byId, isOpen, state, update } from "./state.js";
import { addDays, captureWarningText, estimateText, today } from "./words.js";
import { locale, tr } from "./i18n.js";

// The local day of an instant as `YYYY-MM-DD`.
function dayOf(date) {
  const pad = (n) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

// „heute“, „gestern“ or „Mo 09.11.“ for an instant.
export function dayWord(at) {
  const day = dayOf(new Date(at));
  if (day === today()) return tr("heute");
  if (day === addDays(today(), -1)) return tr("gestern");
  return new Date(at).toLocaleDateString(locale(), { weekday: "short", day: "2-digit", month: "2-digit" });
}

const hhmm = (at) => new Date(at).toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" });

// A booked clock as a chip (ADR 0081 §3): „⏱ gestern 14:00–16:00 · 2:00“.
export function bookedText(clock) {
  const seconds = (Date.parse(clock.end) - Date.parse(clock.start)) / 1000;
  return `⏱ ${dayWord(clock.start)} ${hhmm(clock.start)}–${hhmm(clock.end)} · ${length(seconds)}`;
}

// When `x:` completes, as a chip.
export const completedText = (at) => `✓ erledigt ${dayWord(at)} ${hhmm(at)}`;

// `1:05` for an hour and five minutes.
export function length(seconds) {
  const minutes = Math.max(0, Math.floor(seconds / 60));
  return `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, "0")}`;
}

const secondsOf = (clock, now = Date.now()) =>
  Math.max(0, ((clock.end ? Date.parse(clock.end) : now) - Date.parse(clock.start)) / 1000);

// The running clock the server keeps (§2): the latest start, the larger id.
export function running() {
  return (
    state.clocks
      .filter((c) => !c.end)
      .sort((a, b) => Date.parse(b.start) - Date.parse(a.start) || b.id.localeCompare(a.id))[0] ?? null
  );
}

const ownerTitle = (id) => byId(state.tasks, id)?.title ?? byId(state.projects, id)?.title ?? "?";

async function write(changes) {
  try {
    await api.write(changes);
    update({ message: null });
  } catch (error) {
    update({ message: error.message });
  }
}

export function clockIn(owner, start = new Date()) {
  const now = new Date().toISOString();
  return write([
    change.create("clock", { id: uuidv7(), created: now, modified: now, owner, start: start.toISOString() }),
  ]);
}

export function clockOut(clock = running()) {
  if (clock) return write([change.set(clock.id, "end", new Date().toISOString())]);
  return null;
}

// `c`: start on this task, or stop when its clock is the one running.
export function toggleClock(status) {
  const on = running();
  if (on && on.owner === status.task.id) return clockOut(on);
  if (!isOpen(status.task)) {
    update({ message: tr("Auf Erledigtem läuft keine Uhr.") });
    return null;
  }
  return clockIn(status.task.id);
}

// --- The status bar (ADR 0020 §4) --------------------------------------

// The clock key as the account has it (ADR 0079 §1).
const clockKey = () => pretty(inEffect(state.accountSettings?.keys).byId.get("clock"));

export function renderStatusbar() {
  const bar = $("statusbar");
  const session = state.session;
  const on = running();
  // Tabs of several accounts side by side, one per profile (ADR 0086 §8).
  document.title = session ? `samudo · ${session.account}` : "samudo";
  replace(bar, [
    el("span", { class: "who", text: session ? `${session.account} · ${session.device}` : "" }),
    on
      ? el(
          "button",
          {
            type: "button",
            class: "clock running",
            title: tr("Uhr stoppen"),
            onclick: () => clockOut(on),
          },
          [
            el("span", { class: "dot", "aria-hidden": "true" }),
            el("span", { class: "what", text: ownerTitle(on.owner) }),
            el("span", { class: "len", id: "statusbar-length", text: length(secondsOf(on)) }),
            el("span", { class: "stop", text: tr("Stopp") }),
          ],
        )
      : el("span", { class: "clock quiet", text: clockKey() ? tr("keine Uhr läuft · {key} startet", { key: clockKey() }) : tr("keine Uhr läuft") }),
    syncState(session),
  ]);
}

// Live or not; through the replica on this computer also whether it
// reaches the server and what waits in its outbox (ADR 0066 §6).
function syncState(session) {
  const replica = session?.replica;
  if (!connected()) return el("span", { class: "sync", text: tr("getrennt") });
  if (!replica) return el("span", { class: "sync ok", text: "live" });
  const waiting = replica.outbox === 1 ? tr("1 Änderung im Ausgang") : tr("{n} Änderungen im Ausgang", { n: replica.outbox });
  const title = [
    `${tr("Vertreter von")} ${replica.upstream}`,
    replica.last_sync ? tr("zuletzt abgeglichen {when}", { when: new Date(replica.last_sync).toLocaleString(locale()) }) : tr("noch nie abgeglichen"),
    replica.last_error ? `${tr("Fehler:")} ${replica.last_error}` : null,
  ]
    .filter(Boolean)
    .join("\n");
  if (!replica.online) {
    return el("span", { class: "sync offline", title, text: `Offline · ${waiting}` });
  }
  const text = replica.last_error ? tr("Abgleich mit Fehler") : replica.outbox ? waiting : "live";
  return el("span", { class: replica.last_error ? "sync offline" : "sync ok", title, text });
}

// The length ticks on without drawing anything else.
setInterval(() => {
  const on = running();
  const node = $("statusbar-length");
  if (on && node) node.textContent = length(secondsOf(on));
}, 15000);

// --- The inspector -----------------------------------------------------

// `datetime-local` wants the local time without a zone.
const localInput = (date) => {
  const shifted = new Date(date.getTime() - date.getTimezoneOffset() * 60000);
  return shifted.toISOString().slice(0, 16);
};

const timeText = (text) =>
  new Date(text).toLocaleString(locale(), { weekday: "short", day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit" });

export function clockSection(task, section) {
  const mine = state.clocks
    .filter((c) => c.owner === task.id)
    .sort((a, b) => Date.parse(b.start) - Date.parse(a.start));
  const on = running();
  const total = mine.reduce((sum, c) => sum + secondsOf(c), 0);
  const estimate = task.estimate ? ` · ${tr("Schätzung")} ${estimateText(task.estimate)}` : "";
  const start = el("input", { type: "datetime-local", "aria-label": tr("Beginn") });
  const end = el("input", { type: "datetime-local", "aria-label": tr("Ende") });
  const hour = new Date(Date.now() - 3600000);
  start.value = localInput(hour);
  end.value = localInput(new Date());
  const add = () => {
    const from = new Date(start.value);
    const to = new Date(end.value);
    if (Number.isNaN(from.getTime()) || Number.isNaN(to.getTime()) || to < from) {
      update({ message: tr("Das Ende liegt vor dem Beginn.") });
      return;
    }
    const now = new Date().toISOString();
    write([
      change.create("clock", {
        id: uuidv7(),
        created: now,
        modified: now,
        owner: task.id,
        start: from.toISOString(),
        end: to.toISOString(),
      }),
    ]);
  };
  const booked = el("input", {
    type: "text",
    class: "booked",
    placeholder: tr("gestern 2h · 14-15:30 · 45m"),
    "aria-label": tr("Zeit wie nach z:"),
    onkeydown: (event) => {
      if (event.key !== "Enter") return;
      event.preventDefault();
      event.stopPropagation();
      book();
    },
  });
  // Through POST /capture with the task: the parser of `z:` everywhere.
  const book = async () => {
    const text = booked.value.trim();
    if (!text) return;
    try {
      const answer = await api.capture(`z:${text}`, null, false, null, null, task.id);
      const warnings = (answer.warnings ?? []).map(captureWarningText);
      if (!answer.clocks?.length) {
        update({ message: warnings[0] ?? tr("Keine Zeit gebucht.") });
        return;
      }
      booked.value = "";
      update({ message: warnings.length ? warnings.join(" · ") : null });
    } catch (error) {
      update({ message: error.message });
    }
  };
  return section(
    tr("Zeit"),
    el("div", { class: "clocks" }, [
      el("p", { class: "total" }, [
        el("strong", { text: length(total) }),
        ` geclockt${estimate}`,
        task.estimate && total > periodSeconds(task.estimate) ? el("span", { class: "over", text: ` · ${tr("drüber")}` }) : null,
      ]),
      el("div", { class: "actions" }, [
        on && on.owner === task.id
          ? el("button", { type: "button", class: "primary", text: tr("Stopp (c)"), onclick: () => clockOut(on) })
          : isOpen(task)
            ? el("button", { type: "button", text: tr("Start (c)"), onclick: () => clockIn(task.id) })
            : null,
      ]),
      mine.length
        ? el(
            "ul",
            { class: "clocklist" },
            mine.map((c) =>
              el("li", { class: c.end ? null : "running" }, [
                el("span", { text: `${timeText(c.start)} – ${c.end ? new Date(c.end).toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" }) : tr("läuft")}` }),
                el("span", { class: "len", text: length(secondsOf(c)) }),
                el("button", {
                  type: "button",
                  class: "link",
                  text: tr("löschen"),
                  "aria-label": tr("Clock löschen"),
                  onclick: () => write([change.delete(c.id)]),
                }),
              ]),
            ),
          )
        : null,
      // ADR 0081 §3: one field in the syntax of `z:`; the exact fields folded.
      el("details", { class: "addclock" }, [
        el("summary", { text: tr("Nachtragen") }),
        el("div", { class: "row-inputs" }, [booked, el("button", { type: "button", text: tr("Buchen"), onclick: book })]),
        el("details", { class: "exact" }, [
          el("summary", { text: tr("genau") }),
          el("div", { class: "row-inputs" }, [start, "–", end, el("button", { type: "button", text: tr("Hinzufügen"), onclick: add })]),
        ]),
      ]),
    ]),
  );
}

// "PT1H30M" → 5400.
function periodSeconds(period) {
  const m = /^PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?$/.exec(period ?? "");
  if (!m) return Infinity;
  return (Number(m[1] ?? 0) * 60 + Number(m[2] ?? 0)) * 60 + Number(m[3] ?? 0);
}
