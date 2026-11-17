// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The search field on top (ADR 0026 §4) with chips (ADR 0042): a word the
// search language knows becomes a chip once taken from the list or ended
// with a space; the rest is free text for title and note. What matches is
// decided in query.js.

import { $, el, replace } from "./dom.js";
import { chosenOrder } from "./list.js";
import { captureFrom, discardSearch, edited, saveSearch } from "./perspectives.js";
import { addTo, chipClass, chipText, isOr, mayOr, offers, parseWord, tidy, WORD } from "./query.js";
import { startPerspectiveDraft } from "./sidebar.js";
import { shown, state, update } from "./state.js";
import { tr } from "./i18n.js";

// The list under the field: { items, index, start, end } while it shows.
let offer = null;

// A chip goes into the last group (ADR 0121 §4), `oder` starts a new one.
const addChip = (chip) => addTo(state.searchChips, chip);

// The word the cursor stands in, with where it starts and ends.
function wordAtCursor(input) {
  const at = input.selectionStart ?? input.value.length;
  for (const m of input.value.matchAll(WORD)) {
    if (m.index <= at && at <= m.index + m[0].length) return { text: m[0], start: m.index, end: m.index + m[0].length };
  }
  return { text: "", start: at, end: at };
}

// A word being typed into a chip is not free text yet.
const tokenish = (word) => /^-?(?:[#@~]|[\p{L}]+:)/u.test(word);

function freeText(input) {
  const word = wordAtCursor(input);
  if (!word.text || !tokenish(word.text)) return input.value;
  return input.value.slice(0, word.start) + input.value.slice(word.end);
}

// Words ended with a space that the language knows become chips.
function takeFinished(input) {
  let value = input.value;
  let chips = state.searchChips;
  const found = [];
  for (const m of value.matchAll(WORD)) {
    const end = m.index + m[0].length;
    if (end >= value.length || !/\s/.test(value[end])) continue;
    const chip = parseWord(m[0]);
    if (chip) found.push({ start: m.index, end: end + 1, chip });
  }
  if (!found.length) return chips;
  for (const f of found) chips = addTo(chips, f.chip);
  // Cut back to front, so the earlier places stay where they are.
  let caret = input.selectionStart ?? value.length;
  for (const f of found.reverse()) {
    value = value.slice(0, f.start) + value.slice(f.end);
    if (caret >= f.end) caret -= f.end - f.start;
    else if (caret > f.start) caret = f.start;
  }
  input.value = value;
  input.setSelectionRange(caret, caret);
  return chips;
}

// `chips`: the field's, which a refresh may not have put into the state yet.
function computeOffer(input, chips = state.searchChips) {
  if (document.activeElement !== input) return null;
  const word = wordAtCursor(input);
  if (!word.text && input.value.trim()) return null;
  const items = offers(word.text, { or: mayOr(chips) });
  if (!items.length) return null;
  // Exactly what is typed already: nothing to offer.
  if (items.length === 1 && items[0].label === word.text && items[0].chip) return null;
  const index = offer ? Math.min(offer.index, items.length - 1) : 0;
  return { items, index, start: word.start, end: word.end };
}

function refresh() {
  const input = $("search-input");
  const searchChips = takeFinished(input);
  offer = computeOffer(input, searchChips);
  const search = freeText(input);
  // Focus or a step of the cursor: only the list changes.
  if (search === state.search && searchChips === state.searchChips) renderSearch();
  else update({ search, searchChips, selected: null });
}

function accept(input, item) {
  const value = input.value;
  if (item.chip) {
    const rest = value.slice(offer.end).replace(/^\s+/, "");
    const before = value.slice(0, offer.start);
    input.value = before + rest;
    input.setSelectionRange(before.length, before.length);
    offer = null;
    const searchChips = addChip(item.chip);
    offer = computeOffer(input, searchChips);
    update({ search: freeText(input), searchChips, selected: null });
    return;
  }
  input.value = value.slice(0, offer.start) + item.insert + value.slice(offer.end);
  const caret = offer.start + item.insert.length;
  input.setSelectionRange(caret, caret);
  offer = null;
  refresh();
}

function clearAll(input) {
  input.value = "";
  offer = null;
  input.blur();
  update({ search: "", searchChips: [] });
}

function chipNode(chip, i) {
  const text = chipText(chip);
  const remove = () => {
    update({ searchChips: tidy(state.searchChips.filter((_, j) => j !== i)), selected: null });
    $("search-input").focus();
  };
  // `oder` between two groups: nothing to negate; × joins them.
  if (isOr(chip)) {
    return el("span", { class: "chip or" }, [
      el("span", { class: "word", text }),
      el("button", { type: "button", class: "remove", text: "×", "aria-label": tr("{name} entfernen", { name: text }), onclick: remove }),
    ]);
  }
  return el("span", { class: `chip ${chipClass(chip)}${chip.not ? " not" : ""}` }, [
    el("button", {
      type: "button",
      class: "word",
      text,
      title: chip.not ? tr("wieder einschließen") : tr("ausschließen"),
      onclick: () => {
        const searchChips = state.searchChips.map((c, j) => (j === i ? { ...c, not: !c.not } : c));
        update({ searchChips, selected: null });
      },
    }),
    el("button", {
      type: "button",
      class: "remove",
      text: "×",
      "aria-label": tr("{name} entfernen", { name: text }),
      onclick: remove,
    }),
  ]);
}

// What the search is as a perspective (ADR 0090 §5, §6): in one, its
// changes to keep or drop; in a search of its own, a way to keep it.

function renderSave() {
  const box = $("search-save");
  const button = (text, title, onclick, cls = null) =>
    el("button", { type: "button", class: cls, text, title, onmousedown: (event) => event.preventDefault(), onclick });
  let buttons = [];
  if (edited()) {
    buttons = [
      button(tr("Speichern"), tr("Die Suche in der Perspektive speichern"), () => saveSearch(), "primary"),
      button(tr("Suche|Verwerfen"), tr("Zurück zur gespeicherten Suche"), () => discardSearch($("search-input"))),
    ];
  } else if (shown().kind === "search") {
    buttons = [button(tr("☆ Als Perspektive"), tr("Diese Suche als Perspektive sichern"), () => startPerspectiveDraft(captureFrom(chosenOrder())))];
  }
  box.hidden = !buttons.length;
  replace(box, buttons);
}

export function renderSearch() {
  renderSave();
  replace($("search-chips"), state.searchChips.map(chipNode));
  $("search-input").placeholder = state.searchChips.length ? "" : tr("Suchen: Text, #Projekt, @Tag, fällig:woche, ist:markiert …");
  const list = $("search-suggest");
  if (document.activeElement !== $("search-input")) offer = null;
  list.hidden = !offer;
  replace(
    list,
    offer
      ? offer.items.map((item, i) =>
          el("li", {
            class: [i === offer.index ? "current" : null, item.kind ? "kind" : null].filter(Boolean).join(" ") || null,
            text: item.label,
            onmousedown: (event) => {
              event.preventDefault();
              offer.index = i;
              accept($("search-input"), item);
            },
          }),
        )
      : [],
  );
}

export function wireSearch() {
  const input = $("search-input");
  input.addEventListener("input", refresh);
  input.addEventListener("focus", refresh);
  input.addEventListener("click", refresh);
  input.addEventListener("blur", () => {
    offer = null;
    renderSearch();
  });
  $("search").addEventListener("click", (event) => {
    if (event.target.id === "search" || event.target.id === "search-chips") input.focus();
  });
  input.addEventListener("keydown", (event) => {
    const key = event.ctrlKey ? `C-${event.key}` : event.key;
    if (key === "Backspace" && input.selectionStart === 0 && input.selectionEnd === 0 && state.searchChips.length) {
      event.preventDefault();
      update({ searchChips: state.searchChips.slice(0, -1), selected: null });
      return;
    }
    if (offer) {
      const step = (n) => {
        offer.index = (offer.index + n + offer.items.length) % offer.items.length;
        renderSearch();
      };
      if (key === "ArrowDown" || key === "C-n") step(1);
      else if (key === "ArrowUp" || key === "C-p") step(-1);
      else if (key === "Tab" || key === "Enter") accept(input, offer.items[offer.index]);
      else if (key === "Escape") {
        offer = null;
        renderSearch();
      } else if (key === "ArrowLeft" || key === "ArrowRight" || key === "Home" || key === "End") {
        setTimeout(refresh, 0);
        return;
      } else return;
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    if (key === "Escape") {
      event.stopPropagation();
      clearAll(input);
    } else if (key === "ArrowLeft" || key === "ArrowRight" || key === "Home" || key === "End") {
      setTimeout(refresh, 0);
    }
  });
}
