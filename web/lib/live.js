// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Listening to `GET /events` (ADR 0004 §5), only while the tab is visible,
// as catastr ADR 0095 §6: a hidden tab holds no connection and catches up
// once when it comes back.

let source = null;
let timer = null;
// Told when the connection opens or breaks: the status bar shows it.
let onConnection = () => {};
export function watchConnection(fn) {
  onConnection = fn;
}

// `reload(seq)` is told the change number the event reports, or nothing
// when the page must load anyway (a reset, a tab back in view).
export function listen(reload) {
  // Events close together load once, for the newest of them.
  let newest = 0;
  let must = false;
  const soon = (seq = null) => {
    if (seq === null) must = true;
    else newest = Math.max(newest, seq);
    clearTimeout(timer);
    timer = setTimeout(() => {
      const upTo = must ? null : newest;
      newest = 0;
      must = false;
      reload(upTo);
    }, 250);
  };
  const seqOf = (event) => {
    try {
      return JSON.parse(event.data).seq ?? null;
    } catch {
      return null;
    }
  };
  const open = () => {
    if (source) return;
    source = new EventSource("/events");
    source.addEventListener("changed", (event) => soon(seqOf(event)));
    source.addEventListener("reset", () => soon());
    source.addEventListener("open", () => onConnection());
    source.addEventListener("error", () => onConnection());
  };
  const close = () => {
    source?.close();
    source = null;
    onConnection();
  };
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) close();
    else {
      open();
      soon();
    }
  });
  if (!document.hidden) open();
}

// Whether `/events` is open: the status bar says so (ADR 0020 §4).
export const connected = () => source?.readyState === EventSource.OPEN;

export function stopListening() {
  source?.close();
  source = null;
}
