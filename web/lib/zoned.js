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

// `when` ({date} or {at}) on the day `day` (YYYY-MM-DD): a whole day stays
// one; an instant keeps its time of day and its zone, with the offset the
// zone has on the new day. Null for no day.
export function onDay(when, day) {
  if (!day) return null;
  const m = AT.exec(when?.at ?? "");
  if (!m) return { date: day };
  const [, hh, mm, ss = "00", frac = "", zone] = m;
  const [y, mo, d] = day.split("-").map(Number);
  let wall;
  try {
    wall = Date.UTC(y, mo - 1, d, Number(hh), Number(mm), Number(ss));
    // Twice: the offset at the guess decides the instant, and the instant
    // may lie on the other side of a change of the offset.
    const offset = offsetAt(zone, wall - offsetAt(zone, wall) * 60000);
    const sign = offset < 0 ? "-" : "+";
    const abs = Math.abs(offset);
    return { at: `${day}T${hh}:${mm}:${ss}${frac}${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}[${zone}]` };
  } catch {
    // A zone this browser does not know: the day, rather than a wrong time.
    return { date: day };
  }
}
