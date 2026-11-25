// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// samu in the browser: lists, inspector and capture over the HTTP API and
// nothing else (ADR 0025).
//
// No framework and no build step, as in catastr: the server carries these
// files in its binary, so the page and the API it talks to are always the
// same version. What replaces a framework is one rule: all state lives in
// `lib/state.js`, and a view reads from it and draws.

import { keepAddress, resolveAddress } from "./lib/address.js";
import { applyTheme, hideGate, renderAccount, showGate, wireSignIn } from "./lib/account.js";
import { api, holding, onWriting, onWritten } from "./lib/api.js";
import { applyLocal, drawsAtOnce } from "./lib/local.js";
import { refreshAlarms, stopAlarms } from "./lib/alarms.js";
import { renderBulk } from "./lib/bulk.js";
import { renderCapture, wireCapture } from "./lib/capture.js";
import { renderEdit, wireEdit } from "./lib/edit.js";
import { renderStatusbar } from "./lib/clock.js";
import { drawDemoBanner, showDemoBanner } from "./lib/demo.js";
import { wireRegister } from "./lib/register.js";
import { renderDetail } from "./lib/detail.js";
import { $, el, replace } from "./lib/dom.js";
import { historyFrom } from "./lib/habits.js";
import { today } from "./lib/words.js";
import { wireKeyCapture } from "./lib/keycapture.js";
import { helpRows, keyOf, wireKeys } from "./lib/keys.js";
import { pretty } from "./lib/keymap.js";
import { chooseBlockFilter, renderList, showsBlocks, wireOrder } from "./lib/list.js";
import { listen, stopListening, watchConnection } from "./lib/live.js";
import { renderPalette, wirePalette } from "./lib/palette.js";
import { wireResize } from "./lib/resize.js";
import { wireMenu } from "./lib/menu.js";
import { refreshSchedule, renderScope, wireScope } from "./lib/scope.js";
import { renderSearch, wireSearch } from "./lib/searchbar.js";
import { renderSettings, wireSettings } from "./lib/settings.js";
import { chooseFilterOf, chooseOrderOf, current as currentPerspective } from "./lib/perspectives.js";
import { renderSidebar, wireSidebar } from "./lib/sidebar.js";
import { messageParts } from "./lib/message.js";
import { chooseScope, onChange, routePerspectiveOrder, state, update } from "./lib/state.js";
import { wireVoice } from "./lib/voice.js";
import { tr, translatePage } from "./lib/i18n.js";

// Titles that name a key follow the bindings (ADR 0079 §1).
function keyTitle(id, text, action) {
  const key = pretty(keyOf(action));
  const title = key ? `${text} (${key})` : text;
  $(id).title = title;
  $(id).setAttribute("aria-label", title);
}

function renderHelp() {
  keyTitle("sidebar-show", tr("Seitenleiste zeigen"), "sidebar");
  keyTitle("sidebar-toggle", tr("Seitenleiste umschalten: voll, schmal, weg"), "sidebar");
  for (const hint of document.querySelectorAll("[data-key]")) {
    const key = pretty(keyOf(hint.dataset.key));
    hint.textContent = key;
    hint.hidden = !key;
  }
  $("help").hidden = !state.help;
  replace(
    $("help-keys"),
    helpRows().map(([key, what]) => el("tr", {}, [el("th", { text: key }), el("td", { text: what })])),
  );
}

function render() {
  renderAccount();
  drawDemoBanner();
  renderScope();
  renderSearch();
  renderSidebar();
  renderList();
  renderBulk();
  renderDetail();
  renderHelp();
  renderPalette();
  renderCapture();
  renderEdit();
  renderSettings();
  renderStatusbar();
  document.body.dataset.pane = state.pane;
  document.body.dataset.full = state.full ? "on" : "off";
  showMessage();
  keepAddress();
  // The logbook of a newly selected task, once.
  if (state.selected && state.logbook.id !== state.selected) loadLogbook(state.selected);
}

// The message line goes away by itself after a while, longer for a longer
// text; a click takes it away at once. A new message starts the time anew.
let shownMessage = null;
let messageTimer = null;
function showMessage() {
  const box = $("message");
  box.hidden = !state.message;
  if (state.message === shownMessage) return;
  shownMessage = state.message;
  // What a message holds: lib/message.js. A click anywhere takes it away.
  const message = state.message;
  const { text, actions, kind } = messageParts(message);
  box.dataset.kind = kind;
  replace(box, text, ...actions.map(([label, act]) => el("button", { type: "button", class: "link", text: label, onclick: act })));
  clearTimeout(messageTimer);
  if (!message) return;
  messageTimer = setTimeout(() => {
    if (state.message === message) update({ message: null });
  }, Math.min(12000, 4000 + 60 * text.length + (actions.length ? 4000 : 0)));
}

// What arrives draws the inspector alone, not every view once more; until
// then the entries shown for this task stay.
async function loadLogbook(id) {
  const shown = state.logbook.of === id ? state.logbook.entries : [];
  state.logbook = { id, of: id, entries: shown };
  try {
    const entries = await api.logbook(id);
    if (state.selected !== id) return;
    state.logbook = { id, of: id, entries };
    renderDetail();
  } catch {
    // A task gone in the meantime has no logbook to show.
  }
}

// The change number the views were last loaded at, and the load on the way.
// A write loads at once (`onWritten`), and the event for that same write
// follows a moment later: it must not load everything a second time.
let loadedSeq = 0;
let loading = null;

// Every live object by id, as `GET /changes` gave it: all of them once, then
// only what changed since (ADR 0004 §5), in the order of the last change, as
// one answer from 0 would have them.
const objects = new Map();

// Takes a page of `/changes` asked after `loadedSeq`. One that is not newer
// than what is here was overtaken by a later answer and would put older
// values back.
function merge(page) {
  if (page.seq <= loadedSeq && objects.size) return;
  for (const id of page.gone) objects.delete(id);
  for (const entry of page.objects) {
    objects.delete(entry.object.id);
    objects.set(entry.object.id, entry);
  }
  loadedSeq = page.seq;
}

// What the server computed at the last load, kept so a write can be drawn
// over it before the next load (web/lib/local.js).
let served = null;

// All of them from the start: signed in anew, or the server said `reset`.
// An answer still on the way from before belongs to nothing here any more.
let epoch = 0;
function forget() {
  objects.clear();
  served = null;
  loadedSeq = 0;
  epoch += 1;
}

async function changes() {
  let asked = epoch;
  let page = await api.objects(loadedSeq);
  if (asked !== epoch) return;
  if (page.reset && loadedSeq) {
    forget();
    asked = epoch;
    page = await api.objects(0);
    if (asked !== epoch) return;
  }
  merge(page);
}

function load() {
  const run = loadNow().finally(() => {
    if (loading === run) loading = null;
  });
  loading = run;
  return run;
}

// From `/events`: load unless the views already show `seq` or later.
async function reloadFor(seq) {
  await loading;
  // Inside a bulk action the reload comes once, at its end (`together`).
  if (!holding() && (seq === null || seq > loadedSeq)) load();
  // The replica on this computer also reports when only its state moved
  // (ADR 0066 §6): online or not, the outbox.
  if (state.session?.replica) {
    try {
      update({ session: await api.session() });
    } catch {
      // The next notice tries again.
    }
  }
}

async function loadNow() {
  // A change may have moved a window of the schedule (ADR 0043 §7).
  refreshSchedule();
  const scope = state.scope;
  // The project each task has as this load sets out: what the statuses on
  // the way are computed with, unless a write drawn later moves it.
  const projectThen = new Map();
  for (const [id, entry] of objects) if (entry.kind === "task") projectThen.set(id, entry.object.project ?? null);
  try {
    const [, bare, projectStatuses, shownFolders, forecast, habits] = await Promise.all([
      changes(),
      api.statuses(scope),
      api.projectStatuses(scope),
      api.folders(scope),
      api.forecast(scope),
      api.habits(scope, historyFrom(today())),
    ]);
    // A change may have added to the open task's logbook: asked again.
    state.logbook = { ...state.logbook, id: null };
    // The scope changed while this was on the way: the next load counts.
    if (state.scope !== scope) return;
    // The project each status was computed with, to see a move drawn
    // before the next load; a task new to this load as it came.
    const projectAt = new Map(
      bare.map((s) => [s.id, projectThen.has(s.id) ? projectThen.get(s.id) : (objects.get(s.id)?.object.project ?? null)]),
    );
    served = { scope, bare, projectAt, projectStatuses, shownFolders, forecast, habits };
    derive(served, true);
  } catch (error) {
    if (error.status === 404 && scope) {
      // The scope chosen here is gone: it counts as none (ADR 0035 §4).
      chooseScope(null);
    } else if (error.status === 401) {
      stopListening();
      stopAlarms();
      showGate(tr("Bitte anmelden."));
    } else update({ message: error.message });
  }
}

// The views from the objects here and what the server computed for them.
// `loaded` after a load; a drawn write leaves the alarms to the reload.
function derive({ bare, projectAt, projectStatuses, shownFolders, forecast, habits }, loaded = false) {
  const live = [...objects.values()];
  const of = (kind) => live.filter((o) => o.kind === kind).map((o) => o.object);
  // A status comes without its task (ADR 0065) and gets it from here. One
  // whose task is not here yet was computed a change later; the notice
  // of that change loads again.
  const all = bare.flatMap(({ id, ...computed }) => {
    const task = objects.get(id)?.object;
    if (!task) return [];
    // Moved here before the server computed it anew: in the inbox as the
    // status defines it, without a project or with one that is not there
    // (ADR 0036 §6); everything else stays as computed until the reload.
    const project = task.project ?? null;
    if (project !== projectAt.get(id)) computed.inbox = !project || objects.get(project)?.kind !== "project";
    return [{ task, ...computed }];
  });
  update({
    folders: of("folder"),
    projects: of("project"),
    tasks: of("task"),
    tags: of("tag"),
    links: of("link"),
    clocks: of("clock"),
    reminders: of("reminder"),
    comments: of("comment"),
    checklist: of("checklist_item"),
    attachments: of("attachment"),
    scopes: of("scope"),
    templates: of("template"),
    perspectives: of("perspective"),
    calendars: of("calendar"),
    appointments: of("appointment"),
    accountSettings: of("settings")[0] ?? null,
    forecast,
    // Due outside the scope is counted for the header, never listed
    // (ADR 0035 §6).
    // What another own scope alone shows: only for the search's hint and
    // the project choice (ADR 0124 §3, §4).
    statuses: all.filter((s) => !s.outside && !s.elsewhere),
    outside: all.filter((s) => s.outside),
    elsewhere: all.filter((s) => s.elsewhere),
    projectStatuses: projectStatuses.filter((p) => !p.elsewhere),
    elsewhereProjects: projectStatuses.filter((p) => p.elsewhere),
    habits,
    shownFolders: shownFolders.map((f) => f.id),
  });
  // The perspective on screen was deleted elsewhere: back to the inbox.
  if (state.view.kind === "perspective" && !currentPerspective()) {
    $("search-input").value = "";
    update({ view: { kind: "inbox", project: null }, search: "", searchChips: [], selected: null });
  }
  resolveAddress();
  // What goes off may have changed with it (ADR 0101 §8).
  if (loaded) refreshAlarms();
}

// A write drawn before it goes out: on the objects here, over what the
// server last computed. Nothing drawn before the first load, or for what
// needs the server first; that waits for the reload.
function drawWrite(changes) {
  if (!served || served.scope !== state.scope || !drawsAtOnce(objects, changes)) return null;
  const undo = applyLocal(objects, changes);
  derive(served);
  return () => {
    undo();
    derive(served);
  };
}

function start() {
  forget();
  load();
  listen(reloadFor);
  onWritten(load);
  onWriting(drawWrite);
}

// A new scope draws the views through it at once.
let drawnScope = state.scope;
onChange(() => {
  render();
  if (state.scope !== drawnScope) {
    drawnScope = state.scope;
    if (state.session) load();
  }
});

translatePage();
applyTheme();
showDemoBanner();
watchConnection(renderStatusbar);
wireSignIn(start);
wireRegister();
wireScope();
wireSettings();
wireCapture();
wireEdit();
$("message").addEventListener("click", () => update({ message: null }));
wireVoice();
wireKeyCapture();
wireKeys();
wirePalette();
wireMenu();
wireSidebar();
wireResize();
for (const button of document.querySelectorAll("#filter button")) {
  // In a perspective the filter is the perspective's (ADR 0090 §6); in a
  // block perspective one for every block, for the moment (ADR 0116 §9).
  button.addEventListener("click", () =>
    showsBlocks()
      ? chooseBlockFilter(button.dataset.filter)
      : state.view.kind === "perspective"
        ? chooseFilterOf(state.view.perspective, button.dataset.filter)
        : update({ filter: button.dataset.filter }),
  );
}
routePerspectiveOrder(chooseOrderOf);
wireSearch();
wireOrder();
$("help-button").addEventListener("click", () => update({ help: !state.help }));
$("help-close").addEventListener("click", () => update({ help: false }));

api
  .session()
  .then((session) => {
    update({ session });
    hideGate();
    start();
  })
  .catch(() => showGate());
