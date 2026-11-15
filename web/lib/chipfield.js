// The filter of a scope in the settings (ADR 0043 §2): chips of the search
// language (ADR 0042 §2), typed with the same list under the field as the
// search, without free text. Every change writes the whole field; what a
// chip means is decided in samu-core, not here.

import { el, replace } from "./dom.js";
import { chipClass, chipText, offers, parseWord, WORD } from "./query.js";
import { tr } from "./i18n.js";

const sameChip = (a, b) => a.kind === b.kind && a.value === b.value;
// The wire leaves `not` out when it is false.
const wire = (chip) => (chip.not ? { kind: chip.kind, value: chip.value, not: true } : { kind: chip.kind, value: chip.value });

function wordAtCursor(input) {
  const at = input.selectionStart ?? input.value.length;
  for (const m of input.value.matchAll(WORD)) {
    if (m.index <= at && at <= m.index + m[0].length) return { text: m[0], start: m.index, end: m.index + m[0].length };
  }
  return { text: "", start: at, end: at };
}

// One field per scope, kept across redraws so typing is not lost.
let field = null;

// `save(chips)` writes the filter; `chips` are the scope's as the server has them.
// `more` lets a field know words of its own (the report's `autor:`, ADR 0080
// §2): `parse(word)` → chip or null, `offers(word)` → items, `text(chip)`
// and `click(chip, i)` for its chips, `fixed(chip)` for one that cannot go.
export function chipField(scopeId, chips, save, more = {}) {
  if (!field || field.scope !== scopeId) field = build(scopeId);
  field.chips = chips ?? [];
  field.save = save;
  field.more = more;
  draw();
  return field.node;
}

function build(scope) {
  const input = el("input", { class: "chip-input", autocomplete: "off", "aria-label": "Filter" });
  const chips = el("span", { class: "chips" });
  const list = el("ul", { class: "suggest", hidden: true });
  const note = el("p", { class: "quiet error", hidden: true });
  const node = el("div", { class: "chip-field" }, [
    el("div", { class: "chip-box", onclick: (e) => e.target.classList.contains("chip-box") && input.focus() }, [chips, input]),
    list,
    note,
  ]);
  const f = { scope, node, input, chipsNode: chips, list, note, chips: [], offer: null, save: () => {}, more: {} };
  input.addEventListener("input", () => refresh(true));
  input.addEventListener("focus", () => refresh(false));
  input.addEventListener("click", () => refresh(false));
  input.addEventListener("blur", () => {
    f.offer = null;
    draw();
  });
  input.addEventListener("keydown", (event) => keydown(event));
  return f;
}

function write(chips) {
  field.chips = chips;
  field.save(chips.map(wire));
  draw();
}

function add(chip) {
  write([...field.chips.filter((c) => !sameChip(c, chip)), chip]);
}

// Words the language knows and ended with a space become chips; a finished
// word it does not know is said below the field and stays.
function refresh(typed) {
  const input = field.input;
  if (typed) {
    const value = input.value;
    for (const m of value.matchAll(WORD)) {
      const end = m.index + m[0].length;
      if (end >= value.length || !/\s/.test(value[end])) continue;
      const chip = parseWord(m[0]) ?? field.more.parse?.(m[0]);
      if (!chip) continue;
      input.value = (value.slice(0, m.index) + value.slice(end + 1)).trimStart();
      add(chip);
      return refresh(true);
    }
  }
  const word = wordAtCursor(input);
  const items =
    document.activeElement === input
      ? [...(field.more.offers?.(word.text) ?? []), ...offers(word.text, { inScope: true })]
      : [];
  const exact = items.length === 1 && items[0].label === word.text && items[0].chip;
  field.offer = items.length && !exact ? { items, index: 0, start: word.start, end: word.end } : null;
  field.note.hidden = true;
  draw();
}

function accept(item) {
  const input = field.input;
  const { start, end } = field.offer;
  if (item.chip) {
    input.value = (input.value.slice(0, start) + input.value.slice(end)).trim();
    field.offer = null;
    add(item.chip);
    return;
  }
  input.value = input.value.slice(0, start) + item.insert + input.value.slice(end);
  input.setSelectionRange(start + item.insert.length, start + item.insert.length);
  refresh(false);
}

function keydown(event) {
  const input = field.input;
  const offer = field.offer;
  if (event.key === "Backspace" && input.selectionStart === 0 && input.selectionEnd === 0 && field.chips.length) {
    event.preventDefault();
    write(field.chips.slice(0, -1));
    return;
  }
  if (offer) {
    const step = (n) => {
      offer.index = (offer.index + n + offer.items.length) % offer.items.length;
      draw();
    };
    if (event.key === "ArrowDown") step(1);
    else if (event.key === "ArrowUp") step(-1);
    else if (event.key === "Tab" || event.key === "Enter") accept(offer.items[offer.index]);
    else if (event.key === "Escape") {
      field.offer = null;
      draw();
    } else return;
    event.preventDefault();
    event.stopPropagation();
    return;
  }
  if (event.key === "Enter") {
    event.preventDefault();
    const word = input.value.trim();
    if (!word) return;
    const chip = parseWord(word) ?? field.more.parse?.(word);
    if (chip) {
      input.value = "";
      add(chip);
    } else {
      field.note.textContent = tr("„{word}“ ist kein Chip. Ein Filter kennt keinen freien Text.", { word });
      field.note.hidden = false;
    }
  }
}

function chipNode(chip, i) {
  const more = field.more;
  const text = more.text?.(chip) ?? chipText(chip);
  const own = more.click && more.text?.(chip);
  return el("span", { class: `chip ${chipClass(chip)}${chip.not ? " not" : ""}` }, [
    el("button", {
      type: "button",
      class: "word",
      text,
      title: own ? tr("wechseln") : chip.not ? tr("wieder einschließen") : tr("ausschließen"),
      onclick: () =>
        own ? more.click(chip, i) : write(field.chips.map((c, j) => (j === i ? { ...c, not: !c.not } : c))),
    }),
    more.fixed?.(chip) ? null : el("button", {
      type: "button",
      class: "remove",
      text: "×",
      "aria-label": tr("{name} entfernen", { name: text }),
      onclick: () => write(field.chips.filter((_, j) => j !== i)),
    }),
  ]);
}

function draw() {
  const f = field;
  replace(f.chipsNode, f.chips.map(chipNode));
  f.input.placeholder = f.chips.length ? "" : tr("fällig:7t, ist:markiert, -@Warten …");
  f.list.hidden = !f.offer;
  replace(
    f.list,
    f.offer
      ? f.offer.items.map((item, i) =>
          el("li", {
            class: [i === f.offer.index ? "current" : null, item.kind ? "kind" : null].filter(Boolean).join(" ") || null,
            text: item.label,
            onmousedown: (event) => {
              event.preventDefault();
              accept(item);
            },
          }),
        )
      : [],
  );
}
