// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The message line at the bottom: what it says, its buttons, and its kind.
// A message is text, or `{ text, actions: [[label, do]], kind }` with
// buttons (ADR 0124 §5). Text alone is an error, the common case; `info()`
// makes a message that reports what went well or what is going on, drawn
// quietly instead of in the colour of an error.

export function info(text, actions = []) {
  return text ? { text, actions, kind: "info" } : null;
}

// `{ text, actions, kind }` of any message, null included.
export function messageParts(message) {
  if (typeof message === "object" && message) {
    return { text: message.text ?? "", actions: message.actions ?? [], kind: message.kind ?? "error" };
  }
  return { text: message ?? "", actions: [], kind: "error" };
}
