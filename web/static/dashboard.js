// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Dashboard at /dashboard/ (terminum.md AC-152, AC-170, AC-171): three tabs over one period and one
// project: You (how much the user works here and how loaded), System (Dispatcher's sessions, failures, the
// agents' load), Costs (tokens and dollars). The tab, the period and the project are the address's; terminumd
// counts, a tab draws (dashyou.js, dashsystem.js, dashcosts.js).
import { bucketWords } from "./dashkit.js";
import { costs } from "./dashcosts.js";
import { system } from "./dashsystem.js";
import { you } from "./dashyou.js";
import { showError } from "./kanbanutil.js";
import { $, NONE, api, dayLabel, inkOf, loadMeta, menu, projectLabel, readQuery, writeQuery } from "./msgpage.js";

const KEYS = ["tab", "period", "project"];
const TABS = { you, system, costs }, PERIODS = ["day", "2w", "month", "quarter", "year"];
let state = readQuery(KEYS), meta = { projects: [] }, data = null, shown = [], loading = 0;
const tab = () => (TABS[state.tab] ? state.tab : "you");
const period = () => (PERIODS.includes(state.period) ? state.period : "month");

/** This page's address with `next` over the tab, period and project in view. */
function href(next) {
  const q = new URLSearchParams(Object.entries({ ...state, ...next }).filter(([k, v]) => KEYS.includes(k) && v));
  return `${location.pathname}${q.size ? `?${q}` : ""}`;
}

/** The Message history of `from` … `to` (local days), the human's, of the project in view or `project` ("-": none). */
function historyUrl(from, to, project = state.project) {
  const q = new URLSearchParams({ from, to, sender: "human" });
  if (project) q.set("project", project);
  return `/history/?${q}`;
}

/** The tab anew from the numbers last read. */
function draw() {
  if (!data) return;
  const ctx = { data, period: period(), w: bucketWords(data), historyUrl, projectHref: (id) => href({ project: id }) };
  const t = TABS[data.tab](ctx, draw);
  shown = t.cards;
  $("tiles").replaceChildren(...t.tiles);
  $("cards").replaceChildren(...shown.map((c) => c.box));
  $("dnote").textContent = `${t.note}${data.kept_from ? ` Counted from ${dayLabel(data.kept_from)}: nothing earlier is kept.` : ""}`;
  for (const c of shown) c.draw();
}

function drawKeys() {
  for (const b of $("dtabs").querySelectorAll("button")) b.setAttribute("aria-selected", String(b.dataset.tab === tab()));
  for (const b of $("dperiod").querySelectorAll("button")) b.setAttribute("aria-pressed", String(b.dataset.period === period()));
  const p = meta.projects.find((x) => (x.id || NONE) === state.project);
  $("dproject").querySelector("summary").textContent = state.project ? projectLabel(p ?? { id: state.project === NONE ? "" : state.project }) : "All projects";
  $("dproject").classList.toggle("on", !!state.project);
  const row = (label, value, color) => ({ label, color, on: value === (state.project ?? ""), pick: () => change({ project: value }) });
  menu($("dproject"), [row("All projects", ""), ...meta.projects.map((x) => row(projectLabel(x), x.id || NONE, inkOf(x)))]);
}

function change(next) {
  state = { ...state, ...next };
  writeQuery(state, KEYS, true);
  drawKeys();
  load();
}

async function load() {
  const mine = ++loading;
  $("cards").classList.add("busy");
  try {
    const asked = tab(), q = new URLSearchParams({ tab: asked, period: period() });
    if (state.project) q.set("project", state.project);
    const got = await api(`/api/dashboard?${q}`);
    if (mine !== loading) return;
    data = { ...got, tab: asked };
    showError($("derr"), "");
    draw();
  } catch (e) {
    if (mine !== loading) return;
    showError($("derr"), `The numbers could not be read: ${e.message}`);
  } finally {
    if (mine === loading) $("cards").classList.remove("busy");
  }
}

$("dtabs").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) change({ tab: b.dataset.tab === "you" ? "" : b.dataset.tab }); });
$("dperiod").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) change({ period: b.dataset.period === "month" ? "" : b.dataset.period }); });
addEventListener("popstate", () => { state = readQuery(KEYS); drawKeys(); load(); });
document.addEventListener("visibilitychange", () => { if (document.visibilityState === "visible") load(); });
// the charts fit their width: drawn again when it changes, not when a card's height does
let wait = 0, width = 0;
new ResizeObserver(([e]) => {
  if (Math.round(e.contentRect.width) === width) return;
  width = Math.round(e.contentRect.width);
  clearTimeout(wait);
  wait = setTimeout(() => { for (const c of shown) c.draw(); }, 120);
}).observe($("cards"));

try { meta = await loadMeta(); } catch (e) { showError($("derr"), `The projects could not be read: ${e.message}`); }
drawKeys();
load();
