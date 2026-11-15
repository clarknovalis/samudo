// The view „Bericht“ (ADR 0080 §5, ADR 0081 §1): three tabs, Erledigt,
// Bearbeitet and Zeit, over one span, one grouping and one filter. It took
// the place of the view „Zeit“; its bar is that view's, with the filter
// field and a menu of the columns shown (12.11.2026).
//
// Everything is computed by samu-core behind `GET /report`; this only
// draws it. The choice is this browser's (`samu.report`); a filter worth
// keeping is a scope.

import { api } from "./api.js";
import { el } from "./dom.js";
import { chipField } from "./chipfield.js";
import { chipText } from "./query.js";
import { byId, state, update } from "./state.js";
import { estimateText, today, addDays } from "./words.js";
import { dayWord, length } from "./clock.js";
import { openCapture } from "./capture.js";
import { openEdit } from "./edit.js";
import { locale, tr } from "./i18n.js";
import { AUTHORS, author, authorOffers, authorWord, knownAuthor, parseAuthor } from "./author.js";

// The account's name: the word for a person's work in `autor:`.
const me = () => state.session?.account ?? null;

// Monday of the week `day` is in.
function monday(day) {
  const d = new Date(`${day}T12:00:00`);
  return addDays(day, -((d.getDay() + 6) % 7));
}

function lastMonth() {
  const first = `${today().slice(0, 8)}01`;
  const end = addDays(first, -1);
  return [`${end.slice(0, 8)}01`, end];
}

const RANGES = {
  today: () => [today(), today()],
  yesterday: () => [addDays(today(), -1), addDays(today(), -1)],
  week: () => [monday(today()), addDays(monday(today()), 6)],
  lastweek: () => [addDays(monday(today()), -7), addDays(monday(today()), -1)],
  month: () => [`${today().slice(0, 8)}01`, today()],
  lastmonth: lastMonth,
  days: () => [addDays(today(), -(Math.max(1, choice.days) - 1)), today()],
};

const RANGE_TITLES = [
  ["today", tr("Heute")],
  ["yesterday", tr("Gestern")],
  ["week", tr("Diese Woche")],
  ["lastweek", tr("Letzte Woche")],
  ["month", tr("Dieser Monat")],
  ["lastmonth", tr("Letzter Monat")],
  ["days", tr("Letzte Tage")],
  ["free", tr("Frei")],
];

const BY_TITLES = [
  ["task", tr("Aufgabe")],
  ["project", tr("Projekt")],
  ["folder", tr("Ordner")],
  ["tag", "Tag"],
  ["day", tr("Tag im Kalender")],
];

const TABS = [
  ["done", tr("Erledigt")],
  ["worked", tr("Bearbeitet")],
  ["time", tr("Zeit")],
];

// The columns each tab can show, and those shown at first.
const COLUMNS = {
  done: [
    ["project", tr("Projekt")],
    ["folder", tr("Ordner")],
    ["date", tr("Datum")],
    ["time", tr("Zeit")],
    ["estimate", tr("Schätzung")],
  ],
  worked: [
    ["project", tr("Projekt")],
    ["folder", tr("Ordner")],
    ["date", tr("Datum")],
    ["reasons", tr("Gründe")],
    ["comments", tr("Kommentare")],
    ["time", tr("Zeit")],
    ["estimate", tr("Schätzung")],
  ],
  time: [
    ["bar", tr("Balken")],
    ["estimate", tr("Schätzung")],
  ],
};
const SHOWN = {
  done: ["project", "date", "time"],
  worked: ["project", "date", "reasons", "time"],
  time: ["bar", "estimate"],
};

// `autor:` is the report's own chip (ADR 0080 §2); without one the server
// counts a person's work, so the chip stands in the field all the same. Its
// words in both languages are in author.js.

// The choice of this browser; the first time from the view „Zeit“.
const read = (key) => {
  try {
    return JSON.parse(localStorage.getItem(key));
  } catch {
    return null;
  }
};
function initial() {
  const base = {
    tab: "done",
    range: "week",
    days: 7,
    by: "task",
    from: today(),
    to: today(),
    chips: [author("ich")],
    shown: SHOWN,
    dropped: false,
  };
  const stored = read("samu.report");
  if (stored) {
    const chips = (stored.chips ?? base.chips).map((c) => (c.kind === "author" ? knownAuthor(c.value) : c));
    return { ...base, ...stored, chips, shown: { ...SHOWN, ...(stored.shown ?? {}) } };
  }
  // ADR 0081 §1: what „Zeit“ had chosen.
  const time = read("samu.time");
  if (!time) return base;
  const range = time.range === "days30" ? "days" : time.range;
  return {
    ...base,
    tab: "time",
    range: RANGES[range] || range === "free" ? range : base.range,
    days: time.range === "days30" ? 30 : base.days,
    by: time.by ?? base.by,
    from: time.from ?? base.from,
    to: time.to ?? base.to,
  };
}
let choice = initial();

function choose(patch) {
  choice = { ...choice, ...patch };
  try {
    localStorage.setItem("samu.report", JSON.stringify(choice));
  } catch {
    // Private mode: the choice lasts as long as the page.
  }
  update({});
}

// `g z` opens the tab „Zeit“ (ADR 0081 §1).
export function reportTab(tab) {
  choice = { ...choice, tab };
}

const span = () =>
  choice.range === "free"
    ? [choice.from, choice.to <= choice.from ? choice.from : choice.to]
    : (RANGES[choice.range] ?? RANGES.week)();

// The filter as the search language: names, not ids, as samu-core reads it.
const filterText = () =>
  choice.chips.map((c) => (c.kind === "author" ? authorWord(c.value, null, "de") : chipText(c))).join(" ");

// The answer for one question, fetched once and kept until the data
// changes (an object written brings new tasks or clocks).
let report = { key: null, data: null, error: null, seen: null };
function fetchReport() {
  const [from, to] = span();
  const q = filterText();
  const key = JSON.stringify([from, to, choice.by, state.scope, q, choice.dropped]);
  const seen = [state.tasks, state.clocks, state.comments];
  if (report.key === key && report.seen?.every((s, i) => s === seen[i])) return report;
  report = { key, data: report.key === key ? report.data : null, error: null, seen };
  api
    .report(from, to, choice.by, state.scope, q, choice.dropped)
    .then((data) => {
      if (report.key === key) {
        report.data = data;
        update({});
      }
    })
    .catch((error) => {
      if (report.key === key) {
        report.error = error.message;
        update({});
      }
    });
  return report;
}

function noneTitle(by) {
  return { project: "Inbox", folder: tr("ohne Ordner"), tag: tr("ohne Tag") }[by] ?? "?";
}

function dayTitle(date) {
  return new Date(`${date}T12:00:00`).toLocaleDateString(locale(), { weekday: "short", day: "2-digit", month: "2-digit" });
}

function groupTitle(group) {
  if (group.date) return dayTitle(group.date);
  if (!group.id) return noneTitle(choice.by);
  return group.title || tr("(gelöscht)");
}

function open(id) {
  if (byId(state.tasks, id)) update({ selected: id, pane: "detail" });
  else if (byId(state.projects, id)) update({ view: { kind: "project", project: id }, selected: null });
}

// --- The bar ---------------------------------------------------------

function authorField() {
  const more = {
    parse: (word) => parseAuthor(word, me()),
    offers: (word) => authorOffers(word, me()),
    text: (chip) => (chip.kind === "author" ? authorWord(chip.value, me()) : null),
    // A click goes round: the account, agent, alle.
    click: (chip, i) => {
      const next = AUTHORS[(AUTHORS.indexOf(chip.value) + 1) % AUTHORS.length];
      choose({ chips: choice.chips.map((c, j) => (j === i ? author(next) : c)) });
    },
    fixed: (chip) => chip.kind === "author",
  };
  // One author chip, first; a second typed replaces it.
  const save = (chips) => {
    const authors = chips.filter((c) => c.kind === "author");
    const rest = chips.filter((c) => c.kind !== "author");
    const one = authors.length ? author(authors[authors.length - 1].value) : author("ich");
    choose({ chips: [one, ...rest] });
  };
  return chipField("report", choice.chips, save, more);
}

// Whether the menu of columns is open, across redraws: a tick redraws.
let columnsOpen = false;

function columnsMenu() {
  const tab = choice.tab;
  const shown = new Set(choice.shown[tab] ?? SHOWN[tab]);
  const menu = el("details", { class: "columns-menu", ontoggle: (e) => (columnsOpen = e.target.open) }, [
    el("summary", { text: tr("Spalten") }),
    el(
      "div",
      { class: "columns-list" },
      COLUMNS[tab].map(([key, title]) =>
        el("label", { class: "toggle" }, [
          el("input", {
            type: "checkbox",
            checked: shown.has(key),
            onchange: (e) => {
              const next = COLUMNS[tab].map(([k]) => k).filter((k) => (k === key ? e.target.checked : shown.has(k)));
              choose({ shown: { ...choice.shown, [tab]: next } });
            },
          }),
          el("span", { text: title }),
        ]),
      ),
    ),
  ]);
  menu.open = columnsOpen;
  return menu;
}

function controls(from, to) {
  return el("div", { class: "time-controls" }, [
    el(
      "div",
      { class: "ranges", role: "group", "aria-label": tr("Zeitraum") },
      RANGE_TITLES.map(([key, title]) =>
        el("button", {
          type: "button",
          class: choice.range === key ? "current" : null,
          text: title,
          onclick: () => choose({ range: key, ...(key === "free" ? { from, to } : {}) }),
        }),
      ),
    ),
    choice.range === "days"
      ? el("label", { class: "free" }, [
          el("input", {
            type: "number",
            min: "1",
            max: "3660",
            value: String(choice.days),
            "aria-label": tr("Tage"),
            onchange: (e) => {
              const n = Number(e.target.value);
              if (Number.isInteger(n) && n >= 1 && n <= 3660) choose({ days: n });
              else e.target.value = String(choice.days);
            },
          }),
          ` ${tr("Tage")}`,
        ])
      : null,
    choice.range === "free"
      ? el("span", { class: "free" }, [
          el("input", { type: "date", value: choice.from, "aria-label": tr("von"), onchange: (e) => choose({ from: e.target.value }) }),
          "–",
          el("input", { type: "date", value: choice.to, "aria-label": tr("bis"), onchange: (e) => choose({ to: e.target.value }) }),
        ])
      : null,
    el("label", {}, [
      `${tr("Nach")} `,
      el(
        "select",
        { "aria-label": tr("Gruppieren"), onchange: (e) => choose({ by: e.target.value }) },
        BY_TITLES.map(([key, title]) => el("option", { value: key, selected: choice.by === key, text: title })),
      ),
    ]),
    choice.tab === "done"
      ? el("label", { class: "toggle" }, [
          el("input", { type: "checkbox", checked: choice.dropped, onchange: (e) => choose({ dropped: e.target.checked }) }),
          el("span", { text: tr("mit Verworfenen") }),
        ])
      : null,
    columnsMenu(),
    el("button", { type: "button", class: "primary", text: tr("+ Zeit nachbuchen"), onclick: () => openCapture(null, "z:") }),
  ]);
}

function tabs(data) {
  const count = (part) => (data?.[part] ? String(data[part].rows.length) : "–");
  const label = {
    done: () => `${tr("Erledigt")} ${count("done")}`,
    worked: () => `${tr("Bearbeitet")} ${count("worked")}`,
    time: () => `${tr("Zeit")} ${data?.time ? length(data.time.seconds) : "–"}`,
  };
  return el(
    "div",
    { class: "report-tabs", role: "tablist", "aria-label": tr("Teile") },
    TABS.map(([key]) =>
      el("button", {
        type: "button",
        role: "tab",
        "aria-selected": String(choice.tab === key),
        class: choice.tab === key ? "current" : null,
        text: label[key](),
        onclick: () => choose({ tab: key }),
      }),
    ),
  );
}

// --- Done and worked ---------------------------------------------------

const REASONS = {
  state: "⇄",
  completed: "✓",
  reopened: "↺",
  dropped: "✗",
  commented: "💬",
  child_completed: "↳✓",
  changed: "✎",
};

function reasonsText(row) {
  return (row.reasons ?? [])
    .filter((r) => r.reason !== "clocked" && r.reason !== "commented")
    .map((r) => {
      const sign = REASONS[r.reason] ?? "?";
      return r.count > 1 ? `${sign} ${r.count}` : sign;
    })
    .join("  ");
}

const commentsOf = (row) => row.reasons?.find((r) => r.reason === "commented")?.count ?? 0;

function rowCells(row, shown) {
  const project = row.project ? byId(state.projects, row.project)?.title ?? "?" : row.kind === "project" ? "" : "Inbox";
  const folder = row.folder ? byId(state.folders, row.folder)?.title ?? "?" : "";
  const cells = {
    project: () => el("td", { class: "quiet", text: project }),
    folder: () => el("td", { class: "quiet", text: folder }),
    date: () => el("td", { class: "quiet when", text: `${dayWord(row.at)} ${new Date(row.at).toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" })}` }),
    reasons: () => el("td", { class: "reasons", text: reasonsText(row) }),
    comments: () => el("td", { class: "len", text: commentsOf(row) ? `💬 ${commentsOf(row)}` : "" }),
    time: () => el("td", { class: "len", text: row.seconds ? length(row.seconds) : "" }),
    estimate: () => el("td", { class: "est quiet", text: row.estimate ? estimateText(row.estimate) : "" }),
  };
  return shown.map((key) => cells[key]());
}

function listTable(part, list) {
  const shown = (choice.shown[part] ?? SHOWN[part]).filter((k) => COLUMNS[part].some(([c]) => c === k));
  const titles = new Map(COLUMNS[part]);
  const row = (r) =>
    el("tr", { class: "openable", onclick: () => open(r.id) }, [
      el("th", { scope: "row" }, [
        el("span", { text: r.title || tr("(ohne Titel)") }),
        r.dropped ? el("span", { class: "quiet", text: ` · ${tr("verworfen")}` }) : null,
      ]),
      ...rowCells(r, shown),
    ]);
  const byId_ = new Map(list.rows.map((r) => [r.id, r]));
  const body = list.groups?.length
    ? list.groups.flatMap((g) => [
        el("tr", { class: "group" }, [
          el("th", { scope: "rowgroup", colspan: String(shown.length + 1) }, [
            el("span", { text: groupTitle(g) }),
            el("span", { class: "quiet", text: ` · ${g.rows.length}${g.seconds ? ` · ${length(g.seconds)}` : ""}` }),
          ]),
        ]),
        ...g.rows.map((id) => byId_.get(id)).filter(Boolean).map(row),
      ])
    : list.rows.map(row);
  return el("table", { class: "report" }, [
    el("thead", {}, el("tr", {}, [el("th", { text: tr("Aufgabe") }), ...shown.map((k) => el("th", { text: titles.get(k) }))])),
    el("tbody", {}, body),
  ]);
}

// --- Time ------------------------------------------------------------

function rowTitle(row, by) {
  if (row.date) return dayTitle(row.date);
  if (!row.id) return noneTitle(by);
  return row.title || tr("(gelöscht)");
}

function timeTable(data) {
  const shown = new Set(choice.shown.time ?? SHOWN.time);
  const by = data.by;
  const most = Math.max(...data.rows.map((r) => r.seconds));
  const plus = (row) =>
    by === "task" && row.id && byId(state.tasks, row.id)
      ? el("button", {
          type: "button",
          class: "link plus",
          text: "+",
          title: tr("Zeit nachbuchen"),
          "aria-label": tr("Zeit nachbuchen auf {title}", { title: row.title }),
          onclick: (event) => {
            event.stopPropagation();
            openEdit(row.id, "z:");
          },
        })
      : null;
  return el("table", { class: "report" }, [
    el(
      "tbody",
      {},
      data.rows.map((row) =>
        el("tr", { class: row.id && by !== "day" ? "openable" : null, onclick: () => row.id && by !== "day" && open(row.id) }, [
          el("th", { scope: "row" }, [el("span", { text: rowTitle(row, by) }), plus(row)]),
          shown.has("bar") ? el("td", { class: "bar" }, el("span", { style: `width: ${Math.max(2, (row.seconds / most) * 100)}%` })) : null,
          el("td", { class: "len", text: length(row.seconds) }),
          shown.has("estimate") ? el("td", { class: "est quiet", text: row.estimate ? `${tr("Schätzung")} ${estimateText(row.estimate)}` : "" }) : null,
        ]),
      ),
    ),
    el("tfoot", {}, el("tr", {}, [
      el("th", { scope: "row", text: tr("Summe") }),
      shown.has("bar") ? el("td") : null,
      el("td", { class: "len", text: length(data.seconds) }),
      shown.has("estimate") ? el("td", { class: "quiet", text: by === "tag" ? tr("jede Zeit einmal") : "" }) : null,
    ])),
  ]);
}

// --- The view ----------------------------------------------------------

const EMPTY = {
  done: tr("In diesem Zeitraum ist nichts erledigt."),
  worked: tr("In diesem Zeitraum ist an nichts gearbeitet worden."),
  time: tr("In diesem Zeitraum ist nichts geclockt."),
};

export function reportView() {
  const [from, to] = span();
  const { data, error } = fetchReport();
  const part = data?.[choice.tab];
  let body;
  if (error) body = el("p", { class: "empty", text: error });
  else if (!data) body = el("p", { class: "empty quiet", text: "…" });
  else if (!part) {
    body = el("p", {
      class: "empty",
      text: choice.tab === "worked" ? tr("„Bearbeitet“ braucht das Logbook: nur mit Verbindung zum Server.") : tr("Nicht gefragt."),
    });
  } else if (!part.rows.length) body = el("p", { class: "empty", text: EMPTY[choice.tab] });
  else body = choice.tab === "time" ? timeTable(part) : listTable(choice.tab, part);
  return [
    controls(from, to),
    el("div", { class: "report-filter" }, authorField()),
    tabs(data),
    el("p", { class: "quiet span", text: from === to ? from : `${from} – ${to}` }),
    body,
  ];
}
