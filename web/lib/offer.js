// Names offered while typing a `#project`, `@tag` or `/template` in a line
// of quick capture (ADR 0027 §5, ADR 0073 §3): in the capture window and
// in the edit dialog (ADR 0078 §1) alike.

import { state } from "./state.js";
import { tagPath } from "./words.js";

// The `#…`/`@…`/`/…` word the cursor stands in, if any.
function wordAtCursor(input) {
  const at = input.selectionStart ?? input.value.length;
  const before = input.value.slice(0, at);
  const start = Math.max(before.lastIndexOf(" "), before.lastIndexOf("\t")) + 1;
  const word = before.slice(start);
  if (!/^[#@/]/.test(word) || word.startsWith('#"') || word.startsWith('@"') || word.startsWith('//')) return null;
  let end = at;
  while (end < input.value.length && !/\s/.test(input.value[end])) end += 1;
  return { sign: word[0], query: word.slice(1).toLowerCase(), start, end };
}

// What to offer at the cursor of `input`; `previous` keeps the chosen line
// while one types on.
export function offerFor(input, previous) {
  const word = wordAtCursor(input);
  if (!word) return null;
  const names =
    word.sign === "#"
      ? state.projects.filter((p) => p.status === "active" || p.status === "on_hold").map((p) => p.title)
      : word.sign === "/"
        ? (state.templates ?? []).map((t) => t.title)
        : state.tags.filter((t) => t.status !== "dropped").map(tagPath);
  const starts = names.filter((n) => n.toLowerCase().startsWith(word.query));
  const inside = names.filter((n) => !n.toLowerCase().startsWith(word.query) && n.toLowerCase().includes(word.query));
  const items = [...starts.sort(), ...inside.sort()].slice(0, 8);
  // Nothing to offer, or exactly what is typed already.
  if (!items.length || (items.length === 1 && items[0].toLowerCase() === word.query)) return null;
  const index = previous && previous.sign === word.sign ? Math.min(previous.index, items.length - 1) : 0;
  return { ...word, items, index };
}

// Write the chosen name into the line, quoted when it holds a space.
export function acceptOffer(input, offer) {
  const name = offer.items[offer.index];
  const written = /\s/.test(name) ? `${offer.sign}"${name}"` : `${offer.sign}${name}`;
  const value = input.value;
  const rest = value.slice(offer.end).replace(/^\s*/, "");
  input.value = `${value.slice(0, offer.start)}${written} ${rest}`;
  const caret = offer.start + written.length + 1;
  input.setSelectionRange(caret, caret);
}
