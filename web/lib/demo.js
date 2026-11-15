// The banner of a public demo (ADR 0098 §2, ADR 0099 §4, §9). The server
// puts <meta name="samu-demo" content="N"> into the page when the demo is
// reset every N minutes, counted from midnight; then the banner says so.
// Signed in to a demo account of one's own, it says instead on which day
// that account goes, with or without the meta. Otherwise nothing shows.

import { $ } from "./dom.js";
import { state } from "./state.js";
import { lang, locale } from "./i18n.js";

const WORDS = {
  de: {
    every: (n) => (n === 60 ? "jede volle Stunde" : `alle ${n} Minuten`),
    next: (m, at) => (m <= 1 ? "jetzt gleich" : `in ${m} min, um ${at}`),
  },
  en: {
    every: (n) => (n === 60 ? "every hour on the hour" : `every ${n} minutes`),
    next: (m, at) => (m <= 1 ? "any moment now" : `in ${m} min, at ${at}`),
  },
};

/// Minutes until the next reset and its time of day for the locale `lang`,
/// from `now`.
export function nextReset(every, lang, now = new Date()) {
  const minutes = now.getHours() * 60 + now.getMinutes() + now.getSeconds() / 60;
  const next = Math.min(Math.floor(minutes / every + 1) * every, 24 * 60);
  const at = new Date(now);
  at.setHours(0, next, 0, 0);
  const time = at.toLocaleTimeString(lang, { hour: "2-digit", minute: "2-digit" });
  return { minutes: Math.ceil(next - minutes), at: time };
}


let every = null;
let tryOwn = null;
let drawn = null;
let timer = null;

/// Reads the meta once, at the start.
export function showDemoBanner() {
  const minutes = Number(document.querySelector('meta[name="samu-demo"]')?.content);
  every = Number.isInteger(minutes) && minutes > 0 ? minutes : null;
  const banner = $("demo-banner");
  // The page below starts under the banner, however many lines it takes.
  const measure = () => document.body.style.setProperty("--demo-h", `${banner.offsetHeight}px`);
  new ResizeObserver(measure).observe(banner);
  for (const link of banner.querySelectorAll(".demo-try-link")) {
    link.addEventListener("click", () => tryOwn?.());
  }
  drawDemoBanner();
}

/// Before signing in, the reset banner offers an account of one's own;
/// `go` leads to it (ADR 0099 §9).
export function offerOwnAccount(go) {
  tryOwn = go;
  drawn = null;
  drawDemoBanner();
}

/// Which banner, if any: called with every redraw, does something only
/// when the answer changes.
export function drawDemoBanner() {
  const until = state.session?.demo_until ?? null;
  const kind = until ? "own" : every ? "reset" : null;
  const key = `${kind}|${until}|${Boolean(tryOwn && !state.session)}`;
  if (key === drawn) return;
  drawn = key;
  const banner = $("demo-banner");
  clearInterval(timer);
  for (const p of banner.querySelectorAll("p")) p.hidden = true;
  if (!kind) {
    banner.hidden = true;
    delete document.body.dataset.demo;
    return;
  }
  const text = banner.querySelector(`p[lang="${lang()}"][data-banner="${kind}"]`);
  text.hidden = false;
  if (kind === "own") {
    const [y, m, d] = until.split("-").map(Number);
    text.querySelector(".demo-until").textContent = new Date(y, m - 1, d).toLocaleDateString(
      locale(),
      { day: "numeric", month: "long", year: "numeric" },
    );
  } else {
    const words = WORDS[lang()];
    text.querySelector(".demo-every").textContent = words.every(every);
    text.querySelector(".demo-try").hidden = !(tryOwn && !state.session);
    const draw = () => {
      const { minutes, at } = nextReset(every, locale());
      text.querySelector(".demo-next").textContent = words.next(minutes, at);
    };
    draw();
    timer = setInterval(draw, 20_000);
  }
  banner.hidden = false;
  document.body.dataset.demo = "on";
}
