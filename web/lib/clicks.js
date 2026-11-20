// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// A double click on a row, counted here instead of by the browser's
// dblclick. The first click selects the task and the list is drawn anew,
// which replaces the row; Firefox then fires no dblclick, because the
// second click lands on another element. Counting by task id and time
// works whatever the drawing did in between.

// The longest gap between the two clicks, in milliseconds; desktops
// default to 400 to 500.
export const DOUBLE_MS = 450;

// A function (id, at) → true when this click is the second on the same
// id within DOUBLE_MS of the first. A third click starts a new pair.
export function clickCounter(gap = DOUBLE_MS) {
  let last = null;
  return (id, at) => {
    const second = last !== null && last.id === id && at - last.at >= 0 && at - last.at <= gap;
    last = second ? null : { id, at };
    return second;
  };
}
