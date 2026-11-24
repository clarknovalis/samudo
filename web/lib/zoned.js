// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// A date of a task moved to another day without losing its time
// (ADR 0144 §3). Nothing here touches the page.

const AT = /^\d{4}-\d\d-\d\dT(\d\d):(\d\d)(?::(\d\d)(\.\d+)?)?(?:Z|[+-]\d\d:\d\d)\[([^\]]+)\]$/;
const pad = (n) => String(n).padStart(2, "0");

// The offset of `zone` at the instant `ms`, in minutes east of UTC.
function offsetAt(zone, ms) {
  const name = new Intl.DateTimeFormat("en-US", { timeZone: zone, timeZoneName: "longOffset" })
    .formatToParts(new Date(ms))
    .find((part) => part.type === "timeZoneName")?.value;
  const m = /GMT([+-])(\d\d):(\d\d)/.exec(name ?? "");
  return m ? (m[1] === "-" ? -1 : 1) * (Number(m[2]) * 60 + Number(m[3])) : 0;
}

// The wall time `hh:mm:ss` plus `frac` on `day` in `zone` as an instant of
// the wire, with the offset the zone has then. Null for a zone this
// browser does not know.
function zonedAt(day, hh, mm, ss, frac, zone) {
  const [y, mo, d] = day.split("-").map(Number);
  try {
    const wall = Date.UTC(y, mo - 1, d, Number(hh), Number(mm), Number(ss));
    // Twice: the offset at the guess decides the instant, and the instant
    // may lie on the other side of a change of the offset.
    const offset = offsetAt(zone, wall - offsetAt(zone, wall) * 60000);
    const sign = offset < 0 ? "-" : "+";
    const abs = Math.abs(offset);
    return `${day}T${hh}:${mm}:${ss}${frac}${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}[${zone}]`;
  } catch {
    return null;
  }
}

// `when` ({date} or {at}) on the day `day` (YYYY-MM-DD): a whole day stays
// one; an instant keeps its time of day and its zone, with the offset the
// zone has on the new day. Null for no day.
export function onDay(when, day) {
  if (!day) return null;
  const m = AT.exec(when?.at ?? "");
  if (!m) return { date: day };
  const [, hh, mm, ss = "00", frac = "", zone] = m;
  const at = zonedAt(day, hh, mm, ss, frac, zone);
  // A zone this browser does not know: the day, rather than a wrong time.
  return at ? { at } : { date: day };
}

// The time of day of `when` as `HH:MM`, "" for a whole day or none.
export function timeOf(when) {
  const m = AT.exec(when?.at ?? "");
  return m ? `${m[1]}:${m[2]}` : "";
}

// The day `day` at the time `time` (`HH:MM`, from a time field) in `zone`,
// the account's (ADR 0144 §2, §3): an empty time is a whole day again.
// Null for no day. A zone this browser does not know keeps the day.
export function atTime(day, time, zone) {
  if (!day) return null;
  const m = /^(\d\d):(\d\d)$/.exec(time ?? "");
  if (!m) return { date: day };
  const at = zone ? zonedAt(day, m[1], m[2], "00", "", zone) : null;
  return at ? { at } : { date: day };
}
