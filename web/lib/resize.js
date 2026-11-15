// Drag the sidebar and the inspector wider or narrower. The widths are
// this browser's own, like the sidebar's step: two CSS variables on #app,
// kept in localStorage. A double click goes back to the stylesheet's.

import { $ } from "./dom.js";

const KEY = "samu.widths";
// What each side may take, and what the list keeps between them.
const LIMITS = { side: [180, 480], detail: [280, 900] };
const MIDDLE = 360;
const STEP = 16;

function load() {
  try {
    const widths = JSON.parse(localStorage.getItem(KEY));
    return widths && typeof widths === "object" ? widths : {};
  } catch {
    return {};
  }
}

let widths = load();

function save() {
  try {
    if (Object.keys(widths).length) localStorage.setItem(KEY, JSON.stringify(widths));
    else localStorage.removeItem(KEY);
  } catch {
    // Private window: the widths last as long as the page.
  }
}

const pane = (which) => $(which === "side" ? "sidebar" : "detail");
const other = (which) => (which === "side" ? "detail" : "side");

// A width within its limits that leaves the list its room.
function clamp(which, width) {
  const [min, max] = LIMITS[which];
  const room = window.innerWidth - pane(other(which)).getBoundingClientRect().width - MIDDLE;
  return Math.round(Math.max(min, Math.min(max, room, width)));
}

function apply() {
  const app = $("app");
  for (const which of ["side", "detail"]) {
    const width = widths[which];
    if (typeof width === "number") app.style.setProperty(`--${which}-w`, `${clamp(which, width)}px`);
    else app.style.removeProperty(`--${which}-w`);
  }
}

function set(which, width) {
  if (width === null) delete widths[which];
  else widths[which] = clamp(which, width);
  apply();
}

function wire(which) {
  const handle = $(`resize-${which}`);
  // The sidebar grows to the right, the inspector to the left.
  const sign = which === "side" ? 1 : -1;
  let start = null;
  handle.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    event.preventDefault();
    handle.setPointerCapture(event.pointerId);
    start = { x: event.clientX, width: pane(which).getBoundingClientRect().width };
    handle.classList.add("dragging");
    document.body.classList.add("resizing");
  });
  handle.addEventListener("pointermove", (event) => {
    if (!start) return;
    set(which, start.width + sign * (event.clientX - start.x));
  });
  const end = () => {
    if (!start) return;
    start = null;
    handle.classList.remove("dragging");
    document.body.classList.remove("resizing");
    save();
  };
  handle.addEventListener("pointerup", end);
  handle.addEventListener("pointercancel", end);
  handle.addEventListener("dblclick", () => {
    set(which, null);
    save();
  });
  // With the keyboard: arrows move by a step, with Shift by four.
  handle.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    event.stopPropagation();
    const step = (event.shiftKey ? 4 : 1) * STEP * (event.key === "ArrowRight" ? 1 : -1);
    set(which, pane(which).getBoundingClientRect().width + sign * step);
    save();
  });
}

export function wireResize() {
  wire("side");
  wire("detail");
  apply();
  // A smaller window narrows what was chosen; the choice itself stays.
  window.addEventListener("resize", apply);
}
