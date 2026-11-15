// Reminders in this browser (ADR 0101 §6, §8): two switches that live
// here alone, never in the account, both off at first. With one on, the
// page asks once for permission to notify, asks the server what goes off
// in the next 24 hours (`GET /alarms`, computed by samu-core), and sets a
// timer for each. Asked again on load, after every change from the event
// stream and every hour. What went off at most ten minutes before the page
// opened comes once more. Which reminders were shown is kept here, so a
// second tab does not show them again. Nothing comes while no page is open.
//
// The words of an appointment's alarms for its sheet (§9) are here too.

import { api } from "./api.js";
import { periodText, shortDay } from "./words.js";
import { shown, update } from "./state.js";
import { locale, tr } from "./i18n.js";

const CHOICE = "samu.alarms";
const SHOWN = "samu.alarms.shown";
// How far back a reminder still comes after the page opened.
const CATCH_UP = 10 * 60 * 1000;
const AHEAD = 24 * 60 * 60 * 1000;
const HOUR = 60 * 60 * 1000;

function read(key, fallback) {
  try {
    return JSON.parse(localStorage.getItem(key)) ?? fallback;
  } catch {
    return fallback;
  }
}

function keep(key, value) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Private mode: it lasts as long as the page.
  }
}

// The switches of this browser: { appointments, tasks }, both off at first.
let choice = { appointments: false, tasks: false, ...read(CHOICE, {}) };
export const alarmChoice = () => ({ ...choice });

const supported = () => typeof window !== "undefined" && "Notification" in window;

// Whether this browser may notify: "granted", "denied", "default", or
// "unsupported".
export const permission = () => (supported() ? Notification.permission : "unsupported");

export async function setAlarmChoice(patch) {
  choice = { ...choice, ...patch };
  keep(CHOICE, choice);
  if ((choice.appointments || choice.tasks) && permission() === "default") {
    try {
      await Notification.requestPermission();
    } catch {
      // An old browser without the promise: asked again next time.
    }
  }
  update({});
  refreshAlarms();
}

// --- Timers -------------------------------------------------------------------

let timers = [];
let hourly = null;
let asking = 0;

function clearTimers() {
  for (const timer of timers) clearTimeout(timer);
  timers = [];
}

export function stopAlarms() {
  clearTimers();
  clearInterval(hourly);
  hourly = null;
  asking += 1;
}

// A reminder's key: what it is about and when it goes off.
const keyOf = (alert) => [alert.at, alert.appointment ?? alert.owner, alert.start?.date ?? alert.start?.at ?? ""].join("|");

// The keys shown, with when; those over two days old are forgotten.
function shownKeys() {
  const shown = read(SHOWN, {});
  const old = Date.now() - 2 * AHEAD;
  for (const [key, at] of Object.entries(shown)) if (at < old) delete shown[key];
  return shown;
}

// Marks a reminder shown; false when it was already.
function markShown(key) {
  const shown = shownKeys();
  if (key in shown) return false;
  shown[key] = Date.now();
  keep(SHOWN, shown);
  return true;
}

export async function refreshAlarms() {
  clearTimers();
  if (!choice.appointments && !choice.tasks) return;
  if (permission() !== "granted") return;
  if (!hourly) hourly = setInterval(refreshAlarms, HOUR);
  const asked = ++asking;
  const now = Date.now();
  let alerts;
  try {
    alerts = await api.alarms(new Date(now - CATCH_UP).toISOString(), new Date(now + AHEAD).toISOString(), choice.tasks, choice.appointments);
  } catch {
    // Offline or signed out: the next change or the next hour asks again.
    return;
  }
  // A newer ask set the timers meanwhile.
  if (asked !== asking) return;
  const shown = shownKeys();
  for (const alert of alerts) {
    const key = keyOf(alert);
    if (key in shown) continue;
    const delay = Math.max(0, new Date(alert.at).getTime() - Date.now());
    timers.push(setTimeout(() => notify(alert, key), delay));
  }
}

const clockOf = (at) => new Date(at.replace(/\[[^\]]*\]$/, "")).toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" });

// The second line of a notification: when and where, or that it is a task.
function body(alert) {
  if (!alert.appointment) return tr("Erinnerung an eine Aufgabe");
  const when = alert.start?.date
    ? `${shortDay(alert.start)}, ${tr("ganztägig")}`
    : [alert.start?.at ? `${shortDay({ date: localDate(alert.start.at) })} ${clockOf(alert.start.at)}` : null, alert.end?.at ? clockOf(alert.end.at) : null]
        .filter(Boolean)
        .join("–");
  return [when, alert.location].filter(Boolean).join(" · ");
}

function localDate(at) {
  const d = new Date(at.replace(/\[[^\]]*\]$/, ""));
  const pad = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function notify(alert, key) {
  if (!markShown(key) || permission() !== "granted") return;
  let notification;
  try {
    // The same tag in another tab replaces instead of adding one.
    notification = new Notification(alert.title || tr("Erinnerung"), { body: body(alert), tag: key });
  } catch {
    return;
  }
  notification.onclick = () => {
    window.focus();
    notification.close();
    if (alert.appointment) {
      const occurrence = {
        appointment: alert.appointment,
        calendar: alert.calendar,
        title: alert.title,
        start: alert.start,
        end: alert.end,
        location: alert.location,
        status: "confirmed",
        busy: true,
      };
      // The sheet of an appointment belongs to the calendar and Forecast.
      const view = ["calendar", "forecast"].includes(shown().kind) ? {} : { view: { kind: "calendar" } };
      update({ ...view, appointment: occurrence, selected: null, pane: "detail" });
    } else update({ selected: alert.owner, appointment: null, pane: "detail" });
  };
}

// --- In words (§9) --------------------------------------------------------------

// "PT15M" → minutes, or null when it is not a plain length.
function minutesOf(period) {
  const m = /^(-?)P(?:(\d+)W)?(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?)?$/.exec(period ?? "");
  if (!m) return null;
  const n = (i) => Number(m[i] ?? 0);
  const total = n(2) * 7 * 1440 + n(3) * 1440 + n(4) * 60 + n(5) + Math.round(n(6) / 60);
  return m[1] ? -total : total;
}

const lengthWords = (minutes) => {
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor((minutes % 1440) / 60);
  const rest = minutes % 60;
  if (!days && !hours) return tr("{n} Min.", { n: rest });
  return periodText(`P${days ? `${days}D` : ""}${hours || rest ? "T" : ""}${hours ? `${hours}H` : ""}${rest ? `${rest}M` : ""}`);
};

// One alarm of an appointment in words: "15 Min. vorher", "5 Min. nach
// dem Ende", "am Vortag um 09:00", "Fr 20.11. 08:00".
export function alarmWords(alarm, appointment) {
  if (alarm.at) {
    const d = new Date(alarm.at);
    const pad = (n) => String(n).padStart(2, "0");
    return `${shortDay({ date: localDate(alarm.at) })} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
  }
  const minutes = minutesOf(alarm.before);
  if (minutes === null) return alarm.before ?? "?";
  const end = alarm.related === "end";
  // Over whole days from midnight (§3): the day and the time it goes off.
  if (!end && appointment?.start?.date && minutes > 0 && minutes % 1440) {
    const days = Math.ceil(minutes / 1440);
    const at = days * 1440 - minutes;
    const pad = (n) => String(n).padStart(2, "0");
    const time = `${pad(Math.floor(at / 60))}:${pad(at % 60)}`;
    return days === 1 ? tr("am Vortag um {time}", { time }) : tr("{n} Tage vorher um {time}", { n: days, time });
  }
  if (minutes === 0) return end ? tr("zum Ende") : tr("zu Beginn");
  const words = lengthWords(Math.abs(minutes));
  if (end) return minutes > 0 ? tr("{length} vor dem Ende", { length: words }) : tr("{length} nach dem Ende", { length: words });
  return minutes > 0 ? tr("{length} vorher", { length: words }) : tr("{length} nach Beginn", { length: words });
}
