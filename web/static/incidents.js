// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-225–226: one signed-in incident feed and the same owner keys on every surface.
import { held, prove, protocols, socketUrl } from "./signin.js";
import { sessionUrl, taskUrl } from "./tasklink.js";
import { openReport, reportUrl } from "./reportlink.js";
import { oneAtATime } from "./dialogs.js";
import { api, button, placeChildren, showError, stamp } from "./kanbanutil.js";
import { el, part, textKey } from "./listparts.js";
import { levelText, levelTip } from "./supervisorfields.js";

export const INCIDENTS = "/api/supervisor/incidents";
const listeners = new Set();
const FILES = location.pathname.startsWith("/fb/");
const KEY = FILES ? "textkey btn" : "textkey";
let rows = [], published = "", reading = false, again = false, writes = 0, signedOut = false;

export function onIncidents(heard) {
  listeners.add(heard);
  heard(rows);
}

async function readIncidents() {
  if (signedOut) return;
  if (reading) { again = true; return; }
  if (!held()) { publish([]); return; }
  reading = true;
  const before = writes;
  try {
    const got = await api(INCIDENTS, { signal: AbortSignal.timeout(8000) });
    if (!Array.isArray(got)) throw new Error("The incident feed returned an invalid answer.");
    if (before === writes) publish(held() ? got : []);
  } catch (e) {
    if (FILES && e.status === 401) signedOut = true; // standalone Files signs in on its next load
    if (!held() || e.status === 401) publish([]);
    showError(feedError, rows.some((i) => !i.acknowledged_at) ? e.message : "");
  } finally {
    reading = false;
    if (again) { again = false; readIncidents(); }
  }
}

function publish(next) {
  showError(feedError, "");
  const now = Date.now() / 1000;
  const revision = JSON.stringify(next.map((i) => [i.id, i.revision, i.project_name, i.snoozed_until > now]));
  if (published === revision) return;
  rows = next;
  published = revision;
  for (const heard of listeners) heard(rows);
}

function incidentUrl(i) {
  return "/supervisor/?" + new URLSearchParams({ project: i.project, incident: i.id }) + "#incident-" + i.id;
}

export function incidentCard(initial, surface) {
  let i = initial;
  const title = el(surface === "bar" ? "span" : "h3", "incident-title");
  const happened = el("span");
  const brief = part("div", "incident-brief", title, happened);
  if (surface === "bar") brief.setAttribute("role", "alert");
  const state = el("p", "incident-state");
  const error = el("p", "incident-error");
  error.setAttribute("role", "alert");
  error.hidden = true;
  const card = part("article", "incident-card", brief, state, error);
  card.dataset.incident = i.id;
  const write = oneAtATime(card, async (action, detail = {}) => {
    showError(error, "");
    try {
      const saved = await api(`${INCIDENTS}/${i.id}`, { method: "POST", body: { action, ...detail } });
      update(saved);
      if (action === "release_hold" && !rows.some((row) => row.id === saved.id)) {
        document.dispatchEvent(new CustomEvent("supervisorholdchange", { detail: saved.project }));
      }
      if (action === "this_is_wrong") document.dispatchEvent(new CustomEvent("supervisorverdict", { detail: saved.project }));
      writes++;
      publish(rows.map((row) => row.id === saved.id ? saved : row).filter((row) => !row.closed_at || !row.acknowledged_at));
      readIncidents();
    } catch (e) { showError(error, e.message); }
  });
  const ack = button(KEY, "Acknowledge", () => write("acknowledge"));
  const snooze = button(KEY, "Snooze 1 hour", () => write("snooze"));
  const holdKeys = part("div", "incident-actions");
  holdKeys.hidden = true;
  const alertKeys = part("div", "incident-actions");
  alertKeys.hidden = true;
  const keys = part("div", "incident-actions", ack, snooze);
  card.append(holdKeys, alertKeys);
  card.append(keys);
  const thread = el("a", KEY, "Thread");
  textKey(thread);
  if (surface !== "bar") {
    const feedback = el("p", "incident-state");
    feedback.setAttribute("role", "status");
    const investigate = button(KEY, "Investigate now", oneAtATime(card, async () => {
      showError(error, "");
      try {
        const saved = await api(`${INCIDENTS}/${i.id}/investigate`, { method: "POST" });
        feedback.textContent = saved.message;
      } catch (e) { showError(error, e.message); }
    }));
    const talk = button(KEY, "Talk", oneAtATime(card, async () => {
      showError(error, "");
      feedback.textContent = "Opening the Supervisor session…";
      try {
        const saved = await api(`${INCIDENTS}/${i.id}/talk`, { method: "POST" });
        if (saved.typed) { location.href = sessionUrl(saved.key); return; }
        feedback.replaceChildren();
        const open = el("a", KEY, "Open session");
        open.href = sessionUrl(saved.key);
        feedback.append(open);
        showError(error, saved.why);
      } catch (e) {
        feedback.textContent = "";
        showError(error, e.message);
      }
    }));
    keys.append(investigate, talk, thread);
    card.append(feedback);
  }
  if (surface !== "screen") {
    const link = el("a", KEY, "Timeline");
    link.href = incidentUrl(i);
    textKey(link);
    keys.append(link);
  }
  // its reports, each opening in the board's reading pane (terminum-reports.md AC-276)
  const reportKeys = part("div", "incident-actions");
  card.append(reportKeys);
  const put = (node, words) => { if (node.textContent !== words) node.textContent = words; };
  let shownHolds = "", shownAlerts = "", shownReports = "";
  function update(next) {
    i = next;
    const snoozed = i.snoozed_until > Date.now() / 1000;
    put(title, `${i.project_name} · Incident ${i.id} · ${levelText(i)}`);
    title.title = levelTip(i);
    put(happened, i.happened);
    const holds = i.holds ?? [];
    // AC-225: an interim verification hold says by when it ends, in this device's time.
    const ends = holds.filter((h) => h.until).map((h) => `Hold ${h.id} ends by ${stamp(h.until)}.`);
    put(state, [i.held, ...ends, i.needed, snoozed && `Repeats snoozed until ${stamp(i.snoozed_until)}.`, i.no_answer].filter(Boolean).join(" "));
    const signature = holds.map((h) => h.id).join(",");
    if (signature !== shownHolds) {
      const hadFocus = holdKeys.contains(document.activeElement);
      holdKeys.replaceChildren(...holds.flatMap((hold) => [
        ["Keep the hold", "keep_hold"], ["Release the hold", "release_hold"],
      ].map(([label, action]) => {
        const key = button(KEY, label, () => write(action, { hold: hold.id }));
        if (holds.length > 1) key.setAttribute("aria-label", `${label} · ${hold.label}`);
        return key;
      })));
      holdKeys.hidden = !holds.length;
      shownHolds = signature;
      if (hadFocus) (holdKeys.querySelector("button") ?? keys.querySelector("button:not([hidden]),a"))?.focus();
    }
    const alerts = surface === "screen" ? i.alerts ?? [] : [];
    const alertSignature = alerts.map((a) => `${a.id}/${a.wrong}`).join(",");
    if (alertSignature !== shownAlerts) {
      const focused = alertKeys.contains(document.activeElement);
      alertKeys.replaceChildren(...alerts.map((a) => {
        const label = `${a.metric.replaceAll("_", " ")} · ${a.detector} / ${a.response}`;
        if (a.wrong) return el("span", "incident-state", `Marked wrong · ${label}`);
        const key = button(KEY, "This is wrong", () => write("this_is_wrong", { alert: a.id }));
        key.setAttribute("aria-label", `This is wrong · ${label}`);
        return part("div", "sv-actions", el("span", "incident-state", label), key);
      }));
      alertKeys.hidden = !alerts.length;
      shownAlerts = alertSignature;
      if (focused) (alertKeys.querySelector("button") ?? keys.querySelector("button:not([hidden]),a"))?.focus();
    }
    const reports = i.reports ?? [], reportSignature = reports.join(",");
    if (reportSignature !== shownReports) {
      reportKeys.replaceChildren(...reports.map((id) => {
        const link = el("a", KEY, id);
        link.href = reportUrl(id);
        textKey(link);
        link.addEventListener("click", (e) => { e.preventDefault(); openReport(id); });
        return link;
      }));
      reportKeys.hidden = !reports.length;
      shownReports = reportSignature;
    }
    thread.hidden = !i.task;
    if (i.task) thread.href = taskUrl(i.task);
    ack.hidden = Boolean(i.acknowledged_at);
    snooze.hidden = Boolean(i.acknowledged_at || i.closed_at || snoozed);
  }
  update(i);
  return { card, update };
}

export function incidentCards(host, surface, make = (i) => incidentCard(i, surface)) {
  const cards = new Map();
  return (shown) => {
    const ids = new Set(shown.map((i) => i.id));
    for (const id of cards.keys()) if (!ids.has(id)) cards.delete(id);
    placeChildren(host, shown.map((i) => {
      if (!cards.has(i.id)) cards.set(i.id, make(i));
      const { card, update } = cards.get(i.id);
      update(i);
      return card;
    }));
  };
}

// The signed-in wake-up socket carries no incident data. An open or a change
// fetches the feed through the usual sign-in guard.
async function watchIncidents() {
  if (signedOut) return;
  const retry = () => setTimeout(watchIncidents, 5000);
  if (!held()) { retry(); return; }
  try {
    if (await prove() === "ok") {
      const address = new URL(socketUrl());
      address.pathname = `${INCIDENTS}/watch`;
      const socket = new WebSocket(address, protocols());
      socket.addEventListener("open", readIncidents);
      socket.addEventListener("message", readIncidents);
      socket.addEventListener("close", () => { readIncidents(); retry(); });
      return;
    }
  } catch { /* the feed reports loss of access to an incident that was visible */ }
  readIncidents();
  retry();
}

const feedError = el("p", "incident-error");
feedError.setAttribute("role", "alert");

if (window.top === window) {
  const css = el("link");
  css.rel = "stylesheet";
  css.href = "/incidents.css";
  document.head.insertBefore(css, document.getElementById("customcss"));

  const list = el("div", "incident-list");
  const bar = part("aside", "incident-bar", feedError, list);
  bar.setAttribute("aria-label", "Critical Supervisor incidents");
  bar.setAttribute("popover", "manual");
  bar.hidden = true;
  document.body.prepend(bar);

  // A native modal makes outside controls inert, including a popover mounted outside it.
  // Put the same bar within the current modal's scope, leaving the existing dialog and its focus alone.
  const modals = new Set();
  let priorFocus;
  function position() {
    const modal = [...modals].findLast((d) => d.isConnected && d.matches(":modal"));
    const parent = modal ?? document.body;
    if (bar.parentNode !== parent) parent.prepend(bar);
    if (bar.hidden) {
      if (bar.matches(":popover-open")) bar.hidePopover();
      if (priorFocus && modal?.contains(priorFocus) && !modal.contains(document.activeElement)) {
        priorFocus.focus({ preventScroll: true });
      }
    } else if (!bar.matches(":popover-open")) bar.showPopover();
  }
  new MutationObserver((changes) => {
    let changed = false;
    for (const { target } of changes) {
      if (!(target instanceof HTMLDialogElement)) continue;
      modals.delete(target);
      if (target.matches(":modal")) modals.add(target);
      changed = true;
    }
    if (changed) position();
  }).observe(document.body, { subtree: true, attributes: true, attributeFilter: ["open"] });
  document.addEventListener("focusin", ({ target }) => {
    if (!bar.contains(target)) priorFocus = target;
  });

  const draw = incidentCards(list, "bar");
  onIncidents((items) => {
    const unread = items.filter((i) => !i.acknowledged_at);
    const hadFocus = bar.contains(document.activeElement);
    draw(unread);
    bar.hidden = !unread.length;
    position();
    if (hadFocus && !bar.contains(document.activeElement)) {
      priorFocus?.focus({ preventScroll: true });
    }
  });
  readIncidents();
  watchIncidents();
  setInterval(readIncidents, 20000);   // snooze expiry and changes made while disconnected
  document.addEventListener("visibilitychange", () => { if (!document.hidden) readIncidents(); });
  addEventListener("pageshow", readIncidents);
}
