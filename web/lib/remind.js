// Reminders in the inspector (ADR 0030): the ones set on a task, made and
// deleted as ordinary objects over POST /changes, like `samu remind`.
//
// When one goes off is computed by samu-core on the phone (§4, §5); this
// only says what is set and never works out a time of its own.

import { api, change } from "./api.js";
import { el, uuidv7 } from "./dom.js";
import { state, update } from "./state.js";
import { parsePeriod, periodText } from "./words.js";
import { locale, tr } from "./i18n.js";

const ANCHORS = { due: tr("Fällig"), planned: tr("Geplant"), defer: tr("Start") };

async function write(changes) {
  try {
    await api.write(changes);
    update({ message: null });
  } catch (error) {
    update({ message: error.message });
  }
}

// `datetime-local` wants the local time without a zone.
const localInput = (date) => {
  const shifted = new Date(date.getTime() - date.getTimezoneOffset() * 60000);
  return shifted.toISOString().slice(0, 16);
};

const instantText = (text) =>
  new Date(text).toLocaleString(locale(), {
    weekday: "short",
    day: "2-digit",
    month: "2-digit",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });

// "1 Tag vor Fällig", "zu Geplant", "am Fr. 20.11.2026, 08:30".
export function reminderText(reminder, task) {
  // ADR 0093: every day at its time; a habit only while the day is open.
  if (reminder.anchor === "daily") {
    const time = (reminder.time ?? "").slice(0, 5);
    return task.habit ? tr("täglich {time}, solange offen", { time }) : tr("täglich {time}", { time });
  }
  if (reminder.anchor === "at") return reminder.at ? tr("am {when}", { when: instantText(reminder.at) }) : tr("zu einem Zeitpunkt");
  const name = ANCHORS[reminder.anchor];
  if (!name) return "unbekannte Art (aus einer neueren samu)";
  const text = reminder.before ? tr("{length} vor {name}", { length: periodText(reminder.before), name }) : tr("zu {name}", { name });
  // ADR 0030 §1: without the date it points at, it reminds of nothing.
  return task[reminder.anchor] ? text : `${text} – ${tr("ohne Datum, meldet sich nicht")}`;
}

// Those before a date first, as made; then those at an instant, by it.
const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
function mine(task) {
  return state.reminders
    .filter((r) => r.owner === task.id)
    .sort((a, b) => cmp(a.at ?? "", b.at ?? "") || cmp(a.created, b.created));
}

function adder(task) {
  const anchor = el("select", { "aria-label": tr("Erinnern") }, [
    el("option", { value: "due", text: tr("vor Fällig") }),
    el("option", { value: "planned", text: tr("vor Geplant") }),
    el("option", { value: "defer", text: tr("vor Start") }),
    el("option", { value: "at", text: tr("zum Zeitpunkt") }),
    el("option", { value: "daily", text: tr("täglich um") }),
  ]);
  const before = el("input", {
    type: "text",
    placeholder: tr("z. B. 1d, 2h, 30min"),
    title: tr("Wie lange vor dem Datum; leer = zum Datum selbst"),
    "aria-label": tr("Wie lange vorher"),
  });
  const hour = new Date(Date.now() + 3600000);
  hour.setMinutes(0, 0, 0);
  const at = el("input", { type: "datetime-local", "aria-label": tr("Zeitpunkt"), value: localInput(hour), hidden: true });
  const time = el("input", { type: "time", "aria-label": tr("Uhrzeit"), value: "20:00", hidden: true });
  anchor.addEventListener("change", () => {
    at.hidden = anchor.value !== "at";
    time.hidden = anchor.value !== "daily";
    before.hidden = anchor.value === "at" || anchor.value === "daily";
  });
  const add = () => {
    const now = new Date().toISOString();
    const value = { id: uuidv7(), created: now, modified: now, owner: task.id, anchor: anchor.value };
    if (anchor.value === "daily") {
      if (!/^\d\d:\d\d$/.test(time.value)) {
        update({ message: tr("Keine gültige Uhrzeit.") });
        return;
      }
      value.time = `${time.value}:00`;
    } else if (anchor.value === "at") {
      const when = new Date(at.value);
      if (Number.isNaN(when.getTime())) {
        update({ message: tr("Kein gültiger Zeitpunkt.") });
        return;
      }
      value.at = when.toISOString();
    } else {
      const period = parsePeriod(before.value);
      if (period === undefined) {
        update({ message: tr("„{text}“ ist keine Dauer (z. B. 1d, 2h, 30min).", { text: before.value }) });
        return;
      }
      if (period) value.before = period;
    }
    write([change.create("reminder", value)]);
  };
  before.addEventListener("keydown", (event) => {
    if (event.key === "Enter") add();
  });
  return el("details", { class: "addclock" }, [
    el("summary", { text: tr("Erinnerung setzen") }),
    el("div", { class: "row-inputs" }, [anchor, before, at, time, el("button", { type: "button", text: tr("Hinzufügen"), onclick: add })]),
  ]);
}

export function reminderSection(task, section) {
  const list = mine(task);
  return section(
    tr("Erinnerungen"),
    el("div", { class: "reminders" }, [
      // ADR 0030 §2: a due date reminds without an object.
      task.due ? el("p", { class: "quiet", text: tr("Fällig meldet sich von selbst (auf dem Handy).") }) : null,
      list.length
        ? el(
            "ul",
            { class: "clocklist" },
            list.map((r) =>
              el("li", {}, [
                el("span", { text: reminderText(r, task) }),
                el("span"),
                el("button", {
                  type: "button",
                  class: "link",
                  text: tr("löschen"),
                  "aria-label": tr("Erinnerung löschen"),
                  onclick: () => write([change.delete(r.id)]),
                }),
              ]),
            ),
          )
        : null,
      adder(task),
    ]),
  );
}
