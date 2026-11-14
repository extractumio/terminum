// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Dashboard at /dashboard/ (terminum.md AC-152, AC-171, AC-172, AC-176): four tabs over one period and one
// project: You (how much the user works here, how engaged and how loaded), Tasks (who does the work on tasks),
// System (Dispatcher's sessions, failures, the agents' load), Costs (tokens and dollars). The tab, the period and the project are the address's; terminumd
// counts, a tab draws (dashyou.js, dashtasks.js, dashsystem.js, dashcosts.js).
import { bucketWords } from "./dashkit.js";
import { costs } from "./dashcosts.js";
import { system } from "./dashsystem.js";
import { tasks } from "./dashtasks.js";
import { you } from "./dashyou.js";
import { showError } from "./kanbanutil.js";
import { $, NONE, api, dayLabel, inkOf, loadMeta, menu, projectLabel, queryUrl, readQuery, writeQuery } from "./msgpage.js";

const KEYS = ["tab", "period", "project"];
const TABS = { you, tasks, system, costs }, PERIODS = ["day", "2w", "month", "quarter", "year"];
/** What the address leaves unsaid. */
const FIRST = { tab: "you", period: "month" };
let state = readQuery(KEYS), meta = { projects: [] }, data = null, shown = [], loading = 0;
/** Why the projects could not be read; null before the first read, "" once one worked. */
let metaError = null;
const tab = () => (TABS[state.tab] ? state.tab : FIRST.tab);
const period = () => (PERIODS.includes(state.period) ? state.period : FIRST.period);

/** The Message history of `from` … `to` (local days), the human's, of the project in view or `project` ("-": none). */
function historyUrl(from, to, project = state.project) {
  const q = new URLSearchParams({ from, to, sender: "human" });
  if (project) q.set("project", project);
  return `/history/?${q}`;
}

/** The tab anew from the numbers last read; a card's key that held the focus holds it again. */
function draw() {
  if (!data) return;
  const held = document.activeElement?.closest?.("#cards .card [data-mode]");
  const at = held && `#${held.closest(".card").id} [data-mode="${held.dataset.mode}"]`;
  const ctx = { data, period: period(), w: bucketWords(data), historyUrl, projectHref: (id) => queryUrl({ ...state, project: id }, KEYS) };
  const t = TABS[data.tab](ctx, draw);
  shown = t.cards;
  $("tiles").replaceChildren(...t.tiles);
  $("cards").replaceChildren(...shown.map((c) => c.box));
  $("dnote").textContent = `${t.note}${data.kept_from ? ` Nothing earlier is kept: ${t.kept} ${dayLabel(data.kept_from)}.` : ""}`;
  for (const c of shown) c.draw();
  if (at) document.querySelector(at)?.focus();
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

/** The projects' menu from terminumd; a failed read is said and tried again with the next numbers. */
async function readMeta() {
  try {
    const got = await loadMeta();
    if (!Array.isArray(got.projects)) throw new Error("terminumd's answer holds no projects");
    meta = got;
    metaError = "";
  } catch (e) { metaError = `The projects could not be read: ${e.message}`; }
  drawKeys();
}

async function load() {
  const mine = ++loading;
  $("cards").classList.add("busy");
  if (metaError !== "") await readMeta();
  try {
    const asked = tab(), q = new URLSearchParams({ tab: asked, period: period() });
    if (state.project) q.set("project", state.project);
    const got = await api(`/api/dashboard?${q}`);
    if (mine !== loading) return;
    data = { ...got, tab: asked };
    showError($("derr"), metaError);
    draw();
  } catch (e) {
    if (mine !== loading) return;
    showError($("derr"), `The numbers could not be read: ${e.message}`);
  } finally {
    if (mine === loading) $("cards").classList.remove("busy");
  }
}

// a tab's or a period's key: the first of each is not said in the address
for (const key of ["tab", "period"]) {
  $(`d${key}${key === "tab" ? "s" : ""}`).addEventListener("click", (e) => {
    const to = e.target.closest("button")?.dataset[key];
    if (to) change({ [key]: to === FIRST[key] ? "" : to });
  });
}
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

drawKeys();
load();
