// Settings › Kalender (ADR 0047 §2, §3, §7–§9): the calendars of the
// account. Make one from an ICS address or a CalDAV collection, find the
// collections on a CalDAV server or next to a calendar already there with
// its stored user and password (ADR 0100 §1–3), each with a colour, set
// colour, refresh and the link into the original, fetch now, delete. What a calendar is comes over POST /changes
// like any object; where it comes from goes to PUT /calendars/{id}/source and
// never comes back.
//
// The page is rebuilt on every update; the fields are kept across redraws
// (by key), so an event from /events takes nothing from under the cursor,
// and a password typed is still there.

import { api, change } from "./api.js";
import { el, uuidv7 } from "./dom.js";
import { EXAMPLES, LINK_TEMPLATES } from "./addresses.js";
import { byId, state, update } from "./state.js";
import { locale, tr } from "./i18n.js";

const fields = new Map();
let chosen = null;
let confirming = null;
let busy = null;
// What the last import came to, by calendar, until another is chosen.
let imported = null;

// What was found, until something is added from it: by the discover form
// (`foundFrom` null) or next to the calendar `foundFrom`.
let found = null;
let foundFrom = null;
let picked = new Set();
// The colour chosen for each found address.
let colors = new Map();

// Colours for a calendar that brings none, the first one not in use.
const PALETTE = ["#3b82f6", "#16a34a", "#dc2626", "#d97706", "#7c3aed", "#0891b2", "#db2777", "#65a30d"];

function nextColor(taken) {
  return PALETTE.find((c) => !taken.has(c)) ?? PALETTE[taken.size % PALETTE.length];
}

// Takes what was found: those not here yet are ticked, each with the
// server's colour or a free one.
function take(list, from) {
  found = list;
  foundFrom = from;
  picked = new Set(found.filter((f) => !known(f)).map((f) => f.url));
  const taken = new Set(state.calendars.map((c) => c.color?.toLowerCase()).filter(Boolean));
  colors = new Map();
  for (const f of found) {
    const color = f.color?.toLowerCase() ?? nextColor(taken);
    taken.add(color);
    colors.set(f.url, color);
  }
}

function forgetFound() {
  found = null;
  foundFrom = null;
  picked = new Set();
  colors = new Map();
}

// Adds the ticked ones, each with its colour and a source made by
// `sourceOf(url)`, and fetches them.
async function addPicked(sourceOf) {
  const failed = [];
  let last = null;
  for (const one of found.filter((f) => picked.has(f.url))) {
    const id = await create(one.title, "caldav", colors.get(one.url) ?? null);
    const after = await connect(id, sourceOf(one.url));
    if (after?.error) failed.push(one.title);
    last = id;
  }
  forgetFound();
  if (last) chosen = last;
  if (failed.length) update({ message: `${tr("Nicht abgerufen:")} ${failed.join(", ")}` });
}

// The list of what was found, with a box and a colour each.
function foundBlock(what, add) {
  if (!found) return null;
  if (!found.length) {
    return el("p", { class: "empty", text: tr("Dort liegt kein Kalender mit Terminen. Aufgabenlisten (VTODO) bleiben draußen.") });
  }
  const rows = found.map((f) => {
    const here = known(f);
    return el("li", {}, el("label", {}, [
      el("input", {
        type: "checkbox",
        checked: picked.has(f.url),
        onchange: (e) => {
          if (e.target.checked) picked.add(f.url);
          else picked.delete(f.url);
          update({});
        },
      }),
      el("input", {
        type: "color",
        class: "found-color",
        value: colors.get(f.url) ?? PALETTE[0],
        title: tr("Farbe"),
        "aria-label": tr("Farbe für {name}", { name: f.title }),
        onchange: (e) => colors.set(f.url, e.target.value),
      }),
      el("span", { text: f.title }),
      here ? el("span", { class: "quiet", text: here.title ? ` – ${tr("schon da als „{name}“", { name: here.title })}` : ` – ${tr("schon abonniert")}` }) : null,
    ]));
  });
  return el("div", { class: "found" }, [
    el("h4", { text: tr("Gefunden") }),
    el("ul", { class: "found-list" }, rows),
    el("button", {
      type: "button",
      class: "primary",
      disabled: !picked.size || busy === what,
      text: busy === what ? tr("Lege an …") : tr("{n} hinzufügen und abrufen", { n: picked.size }),
      onclick: add,
    }),
  ]);
}

// One input per key, made once and kept.
function field(key, attrs) {
  let node = fields.get(key);
  if (!node) {
    node = el("input", { autocomplete: "off", ...attrs });
    fields.set(key, node);
  }
  return node;
}

const valueOf = (key) => fields.get(key)?.value.trim() ?? "";

function clear(...keys) {
  for (const key of keys) {
    const node = fields.get(key);
    if (node) node.value = "";
  }
}

async function attempt(what, work) {
  busy = what;
  update({ message: null });
  try {
    await work();
  } catch (error) {
    update({ message: error.message });
  } finally {
    busy = null;
    update({});
  }
}

const byPosition = (a, b) => a.position - b.position || a.title.localeCompare(b.title, "de");

// The source from three fields; user and password only when given.
function source(prefix, url) {
  const username = valueOf(`${prefix}:user`);
  const password = fields.get(`${prefix}:password`)?.value ?? "";
  return {
    url,
    ...(username ? { username } : {}),
    ...(password ? { password } : {}),
  };
}

async function create(title, kind, color) {
  const id = uuidv7();
  const now = new Date().toISOString();
  await api.write([
    change.create("calendar", {
      id,
      created: now,
      modified: now,
      title,
      kind,
      ...(color ? { color } : {}),
      enabled: true,
      refresh: 15,
      position: state.calendars.length,
    }),
  ]);
  return id;
}

// Sets the source and fetches at once; what went wrong stays on the
// calendar under `error` and is shown there.
async function connect(id, body) {
  await api.calendarSource(id, body);
  return api.refreshCalendar(id);
}

function list() {
  const calendars = [...state.calendars].sort(byPosition);
  return el("div", { class: "scope-list calendar-list" }, [
    el(
      "ul",
      {},
      calendars.map((c) =>
        el(
          "li",
          { class: chosen === c.id ? "current" : null },
          el("button", {
            type: "button",
            onclick: () => {
              confirming = null;
              chosen = c.id;
              update({});
            },
          }, [
            el("span", { class: "calendar-name" }, [
              el("span", { class: "swatch", style: c.color ? `background: ${c.color}` : null }),
              el("span", { text: c.title }),
            ]),
            c.error ? el("span", { class: "calendar-error-mark", title: c.error, text: "!" }) : !c.enabled ? el("span", { class: "quiet", text: "aus" }) : null,
          ]),
        ),
      ),
    ),
    el("button", { type: "button", class: chosen === "new" ? "current" : null, text: tr("+ Kalender"), onclick: () => { chosen = "new"; update({}); } }),
    el("button", { type: "button", class: chosen === "discover" ? "current" : null, text: tr("Auf CalDAV-Server finden"), onclick: () => { chosen = "discover"; update({}); } }),
    defaultChoice(),
  ]);
}

// Where a new appointment goes without asking (ADR 0068 §10): one of the
// calendars samu may write into, for the account on every device.
function defaultChoice() {
  const s = state.accountSettings;
  const writable = [...state.calendars].filter((c) => c.writable && c.kind === "caldav").sort(byPosition);
  if (!s || !writable.length) return null;
  const current = writable.some((c) => c.id === s.default_calendar) ? s.default_calendar : "";
  return el("div", { class: "default-calendar" }, [
    el("label", { class: "field", for: "default-calendar", text: tr("Neue Termine in") }),
    el(
      "select",
      {
        id: "default-calendar",
        onchange: async (event) => {
          const value = event.target.value || null;
          try {
            await api.write([change.set(s.id, "default_calendar", value)]);
            update({ message: null });
          } catch (error) {
            update({ message: error.message });
          }
        },
      },
      [
        el("option", { value: "", text: tr("jedes Mal fragen"), selected: !current }),
        ...writable.map((c) => el("option", { value: c.id, text: c.title, selected: c.id === current })),
      ],
    ),
  ]);
}

function credentials(prefix, ics = false) {
  return [
    el("label", { class: "field", for: `${prefix}-user`, text: ics ? tr("Benutzer (leer bei einem Abo ohne Anmeldung)") : tr("Benutzer") }),
    field(`${prefix}:user`, { id: `${prefix}-user` }),
    el("label", { class: "field", for: `${prefix}-password`, text: tr("Passwort") }),
    field(`${prefix}:password`, { id: `${prefix}-password`, type: "password", autocomplete: "new-password" }),
  ];
}

const SECRET = tr("Adresse, Benutzer und Passwort liegen verschlüsselt auf dem Server und kommen nie zurück, auch nicht in diese Seite.");

function newForm() {
  const kind = fields.get("new:kind")?.value ?? "ics";
  const kindSelect =
    fields.get("new:kind") ??
    (() => {
      const node = el("select", { id: "new-kind", onchange: () => update({}) }, [
        el("option", { value: "ics", text: tr("ICS-Abo (Google, Exchange, jede .ics-Adresse)") }),
        el("option", { value: "caldav", text: tr("CalDAV-Sammlung") }),
      ]);
      fields.set("new:kind", node);
      return node;
    })();
  const add = () =>
    attempt("new", async () => {
      const title = valueOf("new:title");
      const url = valueOf("new:url");
      if (!title || !url) throw new Error(tr("Name und Adresse fehlen."));
      const id = await create(title, kind, null);
      await connect(id, source("new", url));
      clear("new:title", "new:url", "new:user", "new:password");
      chosen = id;
    });
  return el("div", { class: "scope-edit calendar-edit" }, [
    el("label", { class: "field", for: "new-title", text: "Name" }),
    field("new:title", { id: "new-title", placeholder: tr("Familie") }),
    el("label", { class: "field", for: "new-kind", text: tr("Art") }),
    kindSelect,
    el("label", { class: "field", for: "new-url", text: kind === "ics" ? tr("Adresse (https:// oder webcal://)") : tr("Adresse der Sammlung") }),
    field("new:url", { id: "new-url", type: "url", placeholder: kind === "ics" ? EXAMPLES.ics : EXAMPLES.caldav }),
    ...credentials("new", kind === "ics"),
    el("p", { class: "quiet", text: kind === "caldav" ? `${tr("Die Adresse einer Sammlung kennt man selten; „Auf CalDAV-Server finden“ sucht sie.")} ${SECRET}` : SECRET }),
    el("div", { class: "actions" }, [
      el("button", { type: "button", class: "primary", disabled: busy === "new", text: busy === "new" ? tr("Hole …") : tr("Anlegen und abrufen"), onclick: add }),
    ]),
  ]);
}

function discoverForm() {
  const search = () =>
    attempt("discover", async () => {
      const url = valueOf("discover:url");
      if (!url) throw new Error(tr("Die Adresse fehlt."));
      take(await api.discoverCalendars(source("discover", url)), null);
    });
  const add = () =>
    attempt("discover-add", async () => {
      await addPicked((url) => source("discover", url));
      clear("discover:password");
    });
  return el("div", { class: "scope-edit calendar-edit" }, [
    el("label", { class: "field", for: "discover-url", text: tr("Server: der Rechnername reicht (über .well-known/caldav), sonst Principal oder Sammlung") }),
    field("discover:url", { id: "discover-url", type: "url", placeholder: EXAMPLES.server }),
    ...credentials("discover"),
    el("div", { class: "actions" }, [
      el("button", { type: "button", class: found ? null : "primary", disabled: busy === "discover", text: busy === "discover" ? tr("Suche …") : tr("Finden"), onclick: search }),
    ]),
    foundFrom === null ? foundBlock("discover-add", add) : null,
    el("p", { class: "quiet", text: `${tr("Finden legt nichts an und speichert nichts; erst „hinzufügen“ tut es.")} ${SECRET}` }),
  ]);
}

// A calendar here already fetched from the same collection: the server says
// so (ADR 0100 §1); else the host matches and the title too (the address
// itself never comes back).
function known(found) {
  const same = sameCollection(found);
  if (same) return same;
  return found.known ? {} : null;
}

function sameCollection(found) {
  const host = (() => {
    try {
      return new URL(found.url).host;
    } catch {
      return null;
    }
  })();
  return state.calendars.find((c) => c.kind === "caldav" && c.host === host && c.title === found.title) ?? null;
}

// What an import came to: counts, then each part left out or failed.
function importReport({ name, answer }) {
  const issues = [
    ...(answer.failed ?? []).map((i) => [tr("Fehlgeschlagen"), i]),
    ...(answer.skipped ?? []).map((i) => [tr("Ausgelassen"), i]),
  ];
  return el("div", { class: "import-report", role: "status" }, [
    el("p", {
      text: tr("{name}: {created} neu, {replaced} ersetzt, {skipped} ausgelassen, {failed} fehlgeschlagen.", { name, created: answer.created, replaced: answer.replaced, skipped: answer.skipped?.length ?? 0, failed: answer.failed?.length ?? 0 }),
    }),
    issues.length
      ? el(
          "ul",
          { class: "quiet" },
          issues.map(([what, i]) => el("li", { text: `${what}: ${i.uid ?? tr("ohne UID")} – ${i.reason}` })),
        )
      : null,
  ]);
}

function when(stamp) {
  if (!stamp) return tr("noch nie");
  return new Date(stamp).toLocaleString(locale(), { dateStyle: "medium", timeStyle: "short" });
}

// The template with today's date, to try it out; {uid} stays as it is.
// The link an appointment carries is samu-core's (§9), this is only a probe.
function probe(template) {
  const now = new Date();
  const p = (n) => String(n).padStart(2, "0");
  const [y, m, d] = [String(now.getFullYear()), p(now.getMonth() + 1), p(now.getDate())];
  return template.replaceAll("{date}", `${y}-${m}-${d}`).replaceAll("{y}", y).replaceAll("{m}", m).replaceAll("{d}", d);
}

function editor(c) {
  const set = (name, value) => {
    c[name] = value;
    return api.write([change.set(c.id, name, value)]).catch((error) => update({ message: error.message }));
  };
  const keep = (key, value, attrs) => {
    const node = field(`${c.id}:${key}`, attrs);
    if (document.activeElement !== node) node.value = value ?? "";
    return node;
  };
  const title = keep("title", c.title, {
    id: "calendar-title",
    onchange: (e) => {
      const t = e.target.value.trim();
      if (t && t !== c.title) set("title", t);
    },
  });
  const color = keep("color", c.color ?? "#3b82f6", {
    id: "calendar-color",
    type: "color",
    onchange: (e) => set("color", e.target.value),
  });
  const refresh = keep("refresh", String(c.refresh), {
    id: "calendar-refresh",
    type: "number",
    min: "5",
    onchange: (e) => {
      const n = Number(e.target.value);
      if (Number.isInteger(n) && n >= 5) set("refresh", n);
      else e.target.value = String(c.refresh);
    },
  });
  const template = keep("link", c.link_template, {
    id: "calendar-link",
    placeholder: EXAMPLES.link,
    onchange: (e) => set("link_template", e.target.value.trim() || null),
  });
  const deleting = confirming === c.id;
  const newSource = () =>
    attempt(`source:${c.id}`, async () => {
      const url = valueOf(`${c.id}:url`);
      if (!url) throw new Error(tr("Die Adresse fehlt."));
      await connect(c.id, source(c.id, url));
      clear(`${c.id}:url`, `${c.id}:user`, `${c.id}:password`);
    });
  const more = () =>
    attempt(`more:${c.id}`, async () => {
      take(await api.discoverFrom(c.id), c.id);
    });
  const addMore = () => attempt(`more-add:${c.id}`, () => addPicked((url) => ({ url, credentials_from: c.id })));
  const nextTo =
    c.kind === "caldav" && c.host
      ? el("div", { class: "calendar-more" }, [
          el("button", {
            type: "button",
            disabled: busy === `more:${c.id}`,
            text: busy === `more:${c.id}` ? tr("Suche …") : tr("Weitere Kalender von {host}", { host: c.host }),
            onclick: more,
          }),
          foundFrom === c.id ? foundBlock(`more-add:${c.id}`, addMore) : null,
          foundFrom === c.id
            ? el("p", { class: "quiet", text: tr("Benutzer und Passwort dieses Kalenders werden auf dem Server übernommen; sie kommen nicht in diese Seite.") })
            : null,
        ])
      : null;
  const importFile = (file) =>
    attempt(`import:${c.id}`, async () => {
      if (file.size > 10 * 1024 * 1024) throw new Error(tr("Die Datei ist größer als 10 MB."));
      imported = { calendar: c.id, name: file.name, answer: await api.importCalendar(c.id, file) };
    });
  const picker =
    c.kind === "caldav" && c.writable
      ? el("input", {
          type: "file",
          accept: ".ics,text/calendar",
          hidden: true,
          onchange: (e) => {
            const file = e.target.files?.[0];
            e.target.value = "";
            if (file) importFile(file);
          },
        })
      : null;
  const importer = picker
    ? el("div", { class: "calendar-import" }, [
        picker,
        el("button", {
          type: "button",
          disabled: busy === `import:${c.id}`,
          text: busy === `import:${c.id}` ? tr("Importiere …") : tr("ICS-Datei importieren …"),
          onclick: () => picker.click(),
        }),
        imported?.calendar === c.id ? importReport(imported) : null,
      ])
      : null;
  return el("div", { class: "scope-edit calendar-edit" }, [
    el("dl", { class: "calendar-facts" }, [
      el("dt", { text: tr("Art") }),
      el("dd", { text: c.kind === "caldav" ? "CalDAV" : c.kind === "ics" ? "ICS-Abo" : c.kind }),
      el("dt", { text: tr("Quelle") }),
      el("dd", { text: c.host ?? tr("keine gesetzt") }),
      el("dt", { text: tr("Zuletzt geändert") }),
      el("dd", { text: when(c.fetched_at) }),
    ]),
    c.error ? el("p", { class: "calendar-error", role: "status", text: c.error }) : null,
    nextTo,
    importer,
    el("label", { class: "field", for: "calendar-title", text: "Name" }),
    title,
    el("div", { class: "calendar-row" }, [
      el("label", { class: "field", for: "calendar-color", text: tr("Farbe") }),
      color,
      el("label", { class: "field", for: "calendar-refresh", text: tr("Abruf alle (Minuten)") }),
      refresh,
    ]),
    el("label", { class: "toggle" }, [
      el("input", { type: "checkbox", checked: c.enabled, onchange: (e) => set("enabled", e.target.checked) }),
      el("span", {}, [tr("Eingeschaltet"), el("span", { class: "quiet", text: ` – ${tr("ausgeschaltet wird er weder geholt noch gezeigt")}` })]),
    ]),
    el("label", { class: "field", for: "calendar-link", text: tr("Link ins Original, wenn ein Termin keinen eigenen hat: {date}, {y}, {m}, {d}, {uid}") }),
    template,
    el("div", { class: "template-row" }, [
      el("span", { class: "quiet", text: tr("Vorlagen:") }),
      ...LINK_TEMPLATES.map(([name, make]) =>
        el("button", {
          type: "button",
          class: "chip",
          text: name,
          onclick: () => {
            const value = make(c);
            template.value = value;
            set("link_template", value);
          },
        }),
      ),
      c.link_template
        ? el("a", { href: probe(c.link_template), target: "_blank", rel: "noopener noreferrer", text: tr("Probe mit heute ↗") })
        : null,
    ]),
    el("details", { class: "calendar-source" }, [
      el("summary", { text: c.host ? tr("Quelle ersetzen") : tr("Quelle setzen") }),
      el("label", { class: "field", for: `${c.id}-url`, text: c.kind === "caldav" ? tr("Adresse der Sammlung") : tr("Adresse (https:// oder webcal://)") }),
      field(`${c.id}:url`, { id: `${c.id}-url`, type: "url" }),
      ...credentials(c.id, c.kind === "ics"),
      el("p", { class: "quiet", text: SECRET }),
      el("button", { type: "button", disabled: busy === `source:${c.id}`, text: busy === `source:${c.id}` ? tr("Hole …") : tr("Setzen und abrufen"), onclick: newSource }),
    ]),
    el("div", { class: "actions" }, [
      el("button", {
        type: "button",
        class: "primary",
        disabled: busy === `refresh:${c.id}` || !c.host,
        text: busy === `refresh:${c.id}` ? tr("Hole …") : tr("Jetzt abrufen"),
        onclick: () => attempt(`refresh:${c.id}`, () => api.refreshCalendar(c.id)),
      }),
      el("button", {
        type: "button",
        class: deleting ? "danger" : null,
        text: deleting ? tr("Wirklich löschen? Die Termine gehen mit") : tr("Kalender löschen"),
        onclick: () => {
          if (!deleting) {
            confirming = c.id;
            update({});
            return;
          }
          confirming = null;
          attempt(`delete:${c.id}`, async () => {
            await api.deleteObject(c.id, {});
            chosen = null;
          });
        },
      }),
    ]),
  ]);
}

export function calendarsPane() {
  const current = chosen && chosen !== "new" && chosen !== "discover" ? byId(state.calendars, chosen) : null;
  if (!current && chosen !== "new" && chosen !== "discover") {
    chosen = [...state.calendars].sort(byPosition)[0]?.id ?? "new";
  }
  const c = chosen === "new" || chosen === "discover" ? null : byId(state.calendars, chosen);
  const main =
    chosen === "new"
      ? newForm()
      : chosen === "discover"
        ? discoverForm()
        : editor(c);
  return el("div", { class: "panes scopes" }, [list(), main]);
}

// When the settings close, typed passwords go with them.
export function forgetCalendarForms() {
  for (const [key, node] of fields) {
    if (key.endsWith(":password")) node.value = "";
  }
  forgetFound();
  imported = null;
  confirming = null;
}
