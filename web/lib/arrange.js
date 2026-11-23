// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The arranged sidebar (ADR 0142): the upper part is one ordered list of
// entries, fixed views and perspectives, and any entry but Inbox can be
// hidden. Kept in this browser under `samu.sidebarEntries` as
// `{"order": [key…], "hidden": [key…]}`, a key being `view:<kind>` or
// `perspective:<id>`. Nothing here touches the page.

import { tr } from "./i18n.js";

// Each view with its stroke icon (ADR 0055 §7), in its default order.
export const VIEWS = [
  { kind: "inbox", title: "Inbox", icon: ["M3 13 5.5 5h13L21 13v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z", "M3 13h5l1.5 3h5l1.5-3h5"] },
  { kind: "everything", title: tr("Alle Projekte"), icon: ["M12 3 21 8l-9 5-9-5z", "M3 12.5l9 5 9-5", "M3 16.5l9 5 9-5"] },
  {
    kind: "today",
    title: tr("Heute"),
    icon: [
      "M12 8a4 4 0 1 0 0 8 4 4 0 1 0 0-8z",
      "M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4",
    ],
  },
  { kind: "forecast", title: "Forecast", icon: ["M3 17l6-6 4 4 8-8", "M15 7h6v6"] },
  { kind: "calendar", title: tr("Kalender"), icon: ["M5 5h14a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z", "M8 3v4M16 3v4M4 10h16"] },
  { kind: "flagged", title: tr("Markiert"), icon: ["M5 21V4", "M5 4h12l-2.5 4.5L17 13H5"] },
  { kind: "stuck", title: "Stuck", icon: ["M12 3.5 21.5 20h-19z", "M12 10v4.5", "M12 17.3h.01"] },
  { kind: "review", title: "Review", icon: ["M2.5 12s3.5-6.5 9.5-6.5 9.5 6.5 9.5 6.5-3.5 6.5-9.5 6.5S2.5 12 2.5 12z", "M12 9a3 3 0 1 0 0 6 3 3 0 1 0 0-6z"] },
  // ADR 0081 §1: „Bericht“ took the place of „Zeit“; the kind stays.
  { kind: "time", title: tr("Bericht"), icon: ["M4 20V11", "M10 20V5", "M16 20v-6", "M3 20h18"] },
];

export const INBOX = "view:inbox";
export const viewKey = (kind) => `view:${kind}`;
export const perspectiveKey = (id) => `perspective:${id}`;

// What a key names: { kind: "view", view } or { kind: "perspective", id }.
export function parseKey(key) {
  if (typeof key !== "string") return null;
  if (key.startsWith("view:")) return { kind: "view", view: key.slice(5) };
  if (key.startsWith("perspective:")) return { kind: "perspective", id: key.slice(12) };
  return null;
}

// The arrangement as it counts, from the JSON this browser kept (§4):
// known keys only, each once; a deleted perspective and an unknown key
// drop out; a view not stored yet goes in at its default place, behind
// the view that precedes it by default; Inbox is never hidden.
export function arrangementOf(stored, viewKinds, perspectiveIds) {
  let kept = null;
  try {
    kept = stored ? JSON.parse(stored) : null;
  } catch {
    kept = null;
  }
  const views = viewKinds.map(viewKey);
  const hidden = Array.isArray(kept?.hidden) ? known(kept.hidden, views, perspectiveIds).filter((k) => k !== INBOX) : [];
  if (!kept || !Array.isArray(kept.order)) return { order: views, hidden };
  const order = known(kept.order, views, perspectiveIds);
  views.forEach((key, i) => {
    if (order.includes(key)) return;
    const before = views.slice(0, i).reverse().find((k) => order.includes(k));
    order.splice(before ? order.indexOf(before) + 1 : 0, 0, key);
  });
  return { order, hidden };
}

function known(keys, views, perspectiveIds) {
  const ids = new Set(perspectiveIds);
  const seen = new Set();
  return keys.filter((key) => {
    const what = parseKey(key);
    const ok =
      what &&
      !seen.has(key) &&
      (what.kind === "view" ? views.includes(key) : ids.has(what.id));
    if (ok) seen.add(key);
    return ok;
  });
}

// `key` before `beforeKey` in the upper list, or at its end when that is
// null or not there; a perspective from the section below comes in this
// way (§3).
export function moveBefore(a, key, beforeKey = null) {
  if (key === beforeKey) return a;
  const order = a.order.filter((k) => k !== key);
  const at = beforeKey ? order.indexOf(beforeKey) : -1;
  order.splice(at < 0 ? order.length : at, 0, key);
  return { order, hidden: a.hidden };
}

// A perspective back to the section "Perspectives"; a view stays where it is.
export function toPerspectives(a, key) {
  if (parseKey(key)?.kind !== "perspective") return a;
  return { order: a.order.filter((k) => k !== key), hidden: a.hidden };
}

export function setHidden(a, key, on) {
  if (key === INBOX) return a;
  const hidden = a.hidden.filter((k) => k !== key);
  if (on) hidden.push(key);
  return { order: a.order, hidden };
}

export const showAll = (a) => ({ order: a.order, hidden: [] });

export const isHidden = (a, key) => a.hidden.includes(key);

// The perspectives of the section below: neither in the upper list nor
// hidden, in their own order.
export const lowerPerspectives = (a, ids) => ids.filter((id) => !a.order.includes(perspectiveKey(id)) && !a.hidden.includes(perspectiveKey(id)));

// --- This browser ------------------------------------------------------

const KEY = "samu.sidebarEntries";
// What was saved last; in private mode the arrangement lasts as long as
// the page.
let saved;

export function loadArrangement(perspectiveIds) {
  if (saved === undefined) {
    try {
      saved = localStorage.getItem(KEY);
    } catch {
      saved = null;
    }
  }
  return arrangementOf(saved, VIEWS.map((v) => v.kind), perspectiveIds);
}

export function saveArrangement(a) {
  saved = JSON.stringify({ order: a.order, hidden: a.hidden });
  try {
    localStorage.setItem(KEY, saved);
  } catch {
    // Private mode: kept in `saved` only.
  }
}
