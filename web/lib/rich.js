// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// Notes and comments rendered (ADR 0058): the tree comes from samu-core
// through POST /render and is built here with el(), never innerHTML, so
// nothing in a note runs (§5). Only `web` links and `task` links the server
// found are followed, and `app` links go to the system (ADR 0075 §5).
//
// Trees are kept per text: drawing the inspector again builds from what is
// known at once; a text not seen yet shows as plain text until its tree
// arrives, asked for together with every other text of the same drawing.

import { api } from "./api.js";
import { openObject } from "./address.js";
import { el } from "./dom.js";
import { update } from "./state.js";
import { tr } from "./i18n.js";

// text → document, the last few hundred texts.
const known = new Map();
const KEEP = 400;
// text → [container] waiting for their tree.
let waiting = new Map();
let asked = false;

function remember(text, document) {
  known.delete(text);
  known.set(text, document);
  while (known.size > KEEP) known.delete(known.keys().next().value);
}

async function ask() {
  asked = false;
  const batch = waiting;
  waiting = new Map();
  const texts = [...batch.keys()];
  try {
    const answer = await api.render(texts);
    texts.forEach((text, i) => {
      remember(text, answer.documents[i]);
      for (const { node, options } of batch.get(text)) {
        if (node.isConnected) node.replaceChildren(...build(answer.documents[i], text, options));
      }
    });
  } catch {
    // Without the server the text stays as it is: plain and complete.
  }
}

// A container with the text rendered. `options.onText(newText)` makes
// checkboxes clickable: it gets the text with the box switched.
export function richText(text, options = {}) {
  const node = el("div", { class: "rich" });
  const document = known.get(text);
  if (document) {
    node.append(...build(document, text, options));
  } else {
    node.append(el("p", { class: "plain", text }));
    if (!waiting.has(text)) waiting.set(text, []);
    waiting.get(text).push({ node, options });
    if (!asked) {
      asked = true;
      queueMicrotask(ask);
    }
  }
  return node;
}

// The text with the checkbox at `box` switched, in code points as the
// tree counts (ADR 0058 §6); null when the text there is no checkbox.
export function toggleBox(text, box) {
  const chars = Array.from(text);
  if (box.end !== box.start + 3 || box.end > chars.length) return null;
  const mark = chars.slice(box.start, box.end).join("");
  const next = { "[ ]": "x", "[-]": "x", "[x]": " ", "[X]": " " }[mark];
  if (next === undefined) return null;
  chars[box.start + 1] = next;
  return chars.join("");
}

function build(document, text, options) {
  const context = { text, options };
  return document.blocks.map((b) => block(b, context)).filter(Boolean);
}

function block(b, context) {
  const inl = (content) => content.map((i) => inline(i, context)).filter(Boolean);
  const blocks = (list) => list.map((x) => block(x, context)).filter(Boolean);
  switch (b.type) {
    case "heading": {
      const level = Math.min(b.level + 2, 6);
      return el(`h${level}`, { class: "rich-h" }, [
        b.keyword ? el("span", { class: b.done ? "kw done" : "kw", text: b.keyword }) : null,
        b.priority ? el("span", { class: "prio", text: `[#${b.priority}]` }) : null,
        ...inl(b.content),
        ...(b.tags ?? []).map((t) => el("span", { class: "tag", text: t })),
      ]);
    }
    case "paragraph":
      return el("p", {}, inl(b.content));
    case "list":
      return el(
        b.ordered ? "ol" : "ul",
        { start: b.start ?? null, class: b.items.some((i) => i.checked != null) ? "checks" : null },
        b.items.map((item) => listItem(item, context, inl, blocks)),
      );
    case "table":
      return el(
        "div",
        { class: "table" },
        el(
          "table",
          {},
          b.rows.map((row) =>
            el(
              "tr",
              {},
              row.cells.map((cell, i) =>
                el(row.header ? "th" : "td", { class: b.align?.[i] && b.align[i] !== "none" ? `al-${b.align[i]}` : null }, inl(cell)),
              ),
            ),
          ),
        ),
      );
    case "code":
      return el("pre", {}, el("code", { text: b.text }));
    case "quote":
      return el("blockquote", {}, blocks(b.blocks));
    case "center":
      return el("div", { class: "center" }, blocks(b.blocks));
    case "verse":
      return el("p", { class: "verse" }, inl(b.content));
    case "keyword":
      return el("p", { class: "keyword" }, [el("span", { class: "quiet", text: `${b.key}: ` }), b.value]);
    case "properties":
      return el("details", { class: "drawer" }, [
        el("summary", { text: "Properties" }),
        el(
          "dl",
          {},
          b.entries.flatMap((e) => [el("dt", { text: e.key }), el("dd", { text: e.value })]),
        ),
      ]);
    case "drawer":
      return el("details", { class: "drawer" }, [el("summary", { text: b.name }), el("pre", { text: b.text })]);
    case "comment":
      return null;
    case "html":
      return el("pre", { class: "html" }, el("code", { text: b.text }));
    case "rule":
      return el("hr");
    case "footnote":
      return el("div", { class: "footnote" }, [el("sup", { text: b.label }), ...blocks(b.blocks)]);
    default:
      return null;
  }
}

function listItem(item, context, inl, blocks) {
  let box = null;
  if (item.checked != null) {
    const onText = context.options.onText;
    box = el("input", {
      type: "checkbox",
      class: "box",
      checked: item.checked,
      disabled: !onText || !item.box,
      "aria-label": item.checked ? tr("erledigt") : tr("offen"),
      onclick: (event) => {
        event.preventDefault();
        const next = item.box && toggleBox(context.text, item.box);
        if (next !== null && next !== undefined) onText(next);
      },
    });
    if (item.partial) box.indeterminate = true;
  }
  return el("li", { class: item.checked ? "done" : null }, [
    box,
    item.term ? el("strong", { class: "term" }, [...inl(item.term), " – "]) : null,
    ...blocks(item.blocks),
  ]);
}

function inline(i, context) {
  const inl = (content) => content.map((x) => inline(x, context)).filter(Boolean);
  switch (i.type) {
    case "text":
      return i.text;
    case "strong":
      return el("strong", {}, inl(i.content));
    case "emphasis":
      return el("em", {}, inl(i.content));
    case "underline":
      return el("u", {}, inl(i.content));
    case "strike":
      return el("s", {}, inl(i.content));
    case "code":
      return el("code", { text: i.text });
    case "link":
      return link(i, inl(i.content));
    case "image":
      // Never loaded (ADR 0058 §3): a link to it, or its name.
      return link(i, [i.alt || i.target]);
    case "timestamp":
      return el("span", { class: "stamp", text: i.text });
    case "footnote_ref":
      return el("sup", { text: i.label });
    case "html":
      return el("code", { class: "html", text: i.text });
    case "break":
      return el("br");
    default:
      return null;
  }
}

function link(i, content) {
  const shown = content.length ? content : [i.target];
  if (i.class === "web") {
    return el("a", { href: i.target, target: "_blank", rel: "noopener noreferrer", title: i.target }, shown);
  }
  // A task, project, folder or tag the server found (ADR 0077); `task`
  // alone from a server before it.
  const object = i.object ?? (i.task ? { id: i.task, kind: "task" } : null);
  if (i.class === "task" && object) {
    return el(
      "a",
      {
        href: `/o/${object.id}`,
        class: "task-link",
        title: i.target,
        onclick: (event) => {
          event.preventDefault();
          if (object.kind === "task") update({ selected: object.id, appointment: null, pane: "detail" });
          else openObject(object.id);
        },
      },
      shown,
    );
  }
  // Another app of this device takes it (ADR 0075 §5): `catastr:` goes to
  // the system's handler; the browser asks first.
  if (i.class === "app") {
    return el("a", { href: i.target, class: "app-link", title: i.target }, shown);
  }
  return el("span", { class: "inert", title: i.target }, shown);
}
