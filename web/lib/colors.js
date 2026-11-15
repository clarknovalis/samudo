// The colours of scopes (ADR 0060): names on the wire, shades in style.css.

import { el } from "./dom.js";
import { tr } from "./i18n.js";

// The palette of scopes (ADR 0060 §1), in the order the settings offer it.
export const COLORS = [
  ["violet", tr("Violett")],
  ["blue", tr("Blau")],
  ["teal", tr("Petrol")],
  ["green", tr("Grün")],
  ["yellow", tr("Gelb")],
  ["orange", tr("Orange")],
  ["red", tr("Rot")],
  ["pink", tr("Rosa")],
];

// A dot in a scope's colour; a grey one without.
export function scopeDot(color) {
  const known = COLORS.some(([c]) => c === color);
  return el("span", { class: known ? "scope-dot" : "scope-dot none", "data-color": known ? color : null, "aria-hidden": "true" });
}
