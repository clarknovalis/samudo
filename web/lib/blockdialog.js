// The dialog „Blöcke …“ of a block perspective (ADR 0116 §6): its blocks
// in order, each another perspective or a fixed view with an optional
// heading of its own; adding, dragging or ↑ ↓ to order, removing; and the
// switch „Doppelte nur einmal“. Saving writes `blocks` and `dedupe`.

import { api, change } from "./api.js";
import { el, replace } from "./dom.js";
import { tr } from "./i18n.js";
import { isBlocks, knownView, VIEWS } from "./blocks.js";
import { perspectiveOf, perspectives } from "./perspectives.js";
import { state, update } from "./state.js";

const VIEW_TITLES = { inbox: () => "Inbox", today: () => tr("Heute"), flagged: () => tr("Markiert") };
const viewTitle = (view) => VIEW_TITLES[view]?.() ?? view;

// The name a block shows when it has no heading of its own.
function nameOf(block) {
  if (block.view) return knownView(block.view) ? viewTitle(block.view) : `${block.view} (${tr("fehlt")})`;
  const p = perspectiveOf(block.perspective);
  if (!p) return tr("fehlt");
  return [p.icon, p.title].filter(Boolean).join(" ");
}

// Whether another perspective shows `p` as a block: then it gets none.
export const isBlockOfAnother = (p) => state.perspectives.some((o) => o.id !== p.id && (o.blocks ?? []).some((b) => b.perspective === p.id));

export function openBlocks(p) {
  if (isBlockOfAnother(p)) {
    update({ message: tr("„{title}“ ist Block einer anderen Perspektive und bekommt keine Blöcke.", { title: p.title }) });
    return;
  }
  // A copy to change; only „Sichern“ writes it.
  let blocks = (p.blocks ?? []).map((b) => ({ ...b }));
  let dragged = null;
  const dedupe = el("input", { type: "checkbox", id: "blocks-dedupe" });
  dedupe.checked = p.dedupe !== false;
  const list = el("ul", { class: "block-list" });
  const choice = el("select", { "aria-label": tr("Block hinzufügen") });
  const close = () => {
    document.removeEventListener("keydown", onKey);
    overlay.remove();
  };
  const onKey = (event) => {
    if (event.key === "Escape") close();
  };

  const move = (from, to) => {
    if (to < 0 || to >= blocks.length || from === to) return;
    const [moved] = blocks.splice(from, 1);
    blocks.splice(to, 0, moved);
    draw();
  };

  function draw() {
    replace(
      list,
      blocks.length
        ? blocks.map((block, i) =>
            el(
              "li",
              {
                draggable: "true",
                ondragstart: (event) => {
                  dragged = i;
                  event.dataTransfer.effectAllowed = "move";
                  event.dataTransfer.setData("text/plain", String(i));
                },
                ondragover: (event) => {
                  if (dragged === null) return;
                  event.preventDefault();
                },
                ondrop: (event) => {
                  event.preventDefault();
                  if (dragged !== null) move(dragged, i);
                  dragged = null;
                },
                ondragend: () => {
                  dragged = null;
                },
              },
              [
                el("span", { class: "grip", "aria-hidden": "true", text: "⠿" }),
                el("span", { class: "block-name", text: nameOf(block) }),
                el("input", {
                  type: "text",
                  value: block.title ?? "",
                  placeholder: tr("eigene Überschrift"),
                  "aria-label": tr("Überschrift von {name}", { name: nameOf(block) }),
                  oninput: (event) => {
                    const title = event.target.value.trim();
                    if (title) block.title = title;
                    else delete block.title;
                  },
                }),
                el("button", { type: "button", text: "↑", "aria-label": tr("nach oben"), disabled: i === 0, onclick: () => move(i, i - 1) }),
                el("button", { type: "button", text: "↓", "aria-label": tr("nach unten"), disabled: i === blocks.length - 1, onclick: () => move(i, i + 1) }),
                el("button", {
                  type: "button",
                  text: "✕",
                  "aria-label": tr("{name} entfernen", { name: nameOf(block) }),
                  onclick: () => {
                    blocks.splice(i, 1);
                    draw();
                  },
                }),
              ],
            ),
          )
        : [el("li", { class: "quiet", text: tr("Noch kein Block: ohne Blöcke ist es eine einfache Perspektive.") })],
    );
    // What can be added: the fixed views and every plain perspective but
    // this one (§2).
    const plain = perspectives().filter((o) => o.id !== p.id && !isBlocks(o));
    replace(choice, [
      el("option", { value: "", text: tr("Block hinzufügen …") }),
      el("optgroup", { label: tr("Ansichten") }, VIEWS.map((v) => el("option", { value: `view:${v}`, text: viewTitle(v) }))),
      plain.length
        ? el("optgroup", { label: tr("Perspektiven") }, plain.map((o) => el("option", { value: `perspective:${o.id}`, text: [o.icon, o.title].filter(Boolean).join(" ") })))
        : null,
    ]);
  }

  choice.addEventListener("change", () => {
    const [kind, value] = choice.value.split(/:(.*)/);
    if (kind === "view") blocks.push({ view: value });
    if (kind === "perspective") blocks.push({ perspective: value });
    draw();
    choice.focus();
  });

  const save = async () => {
    const changes = [];
    const plainBlocks = blocks.map(({ perspective, view, title }) => ({
      ...(perspective ? { perspective } : {}),
      ...(view ? { view } : {}),
      ...(title ? { title } : {}),
    }));
    if (JSON.stringify(plainBlocks) !== JSON.stringify(p.blocks ?? [])) changes.push(change.set(p.id, "blocks", plainBlocks));
    if (dedupe.checked !== (p.dedupe !== false)) changes.push(change.set(p.id, "dedupe", dedupe.checked));
    if (changes.length) {
      try {
        await api.write(changes);
      } catch (error) {
        update({ message: error.message });
        return;
      }
      Object.assign(p, { blocks: plainBlocks, dedupe: dedupe.checked });
    }
    close();
    update({});
  };

  const box = el("div", { class: "box blocks", role: "dialog", "aria-modal": "true", "aria-label": tr("Blöcke") }, [
    el("h2", { text: tr("Blöcke von „{title}“", { title: p.title }) }),
    list,
    choice,
    el("label", { class: "check", for: "blocks-dedupe" }, [dedupe, el("span", { text: tr("Doppelte nur einmal: eine Aufgabe nur im ersten Block, der sie zeigt") })]),
    el("div", { class: "buttons" }, [
      el("button", { type: "button", text: tr("Abbrechen"), onclick: close }),
      el("button", { type: "button", class: "primary", text: tr("Sichern"), onclick: save }),
    ]),
  ]);
  const overlay = el("div", { class: "overlay" }, [box]);
  overlay.addEventListener("click", (event) => {
    if (event.target === overlay) close();
  });
  draw();
  // The menu that opened it closes.
  update({});
  document.addEventListener("keydown", onKey);
  document.body.append(overlay);
  choice.focus();
}
