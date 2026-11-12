// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Scheduled page at /scheduled/ (terminum-schedules.md AC-164): every firing of the schedules, newest first,
// 100 at a time: when, the schedule, its task and agent, the task's status, when it went IN PROGRESS and ended;
// a row opens to what went in and what came out, or says why nothing ran. terminumd reads; the page draws.
import { el, plural } from "./listparts.js";
import { expandable } from "./expand.js";
import { statusBox } from "./kanbanutil.js";
import { taskUrl } from "./tasklink.js";
import { $, api, loadMeta, menu, readQuery, writeQuery } from "./msgpage.js";

const KEYS = ["schedule"];
let state = readQuery(KEYS), shown = [], schedules = [], loading = 0;

const at = (s) => (s ? new Date(s * 1000).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }) : "");
const span = (a, b) => { const m = Math.round((b - a) / 60); return m >= 60 ? `${Math.floor(m / 60)} h ${m % 60} min` : `${m} min`; };
const STATES = { skipped: "skipped", "called off": "called off", failed: "did not start", made: "starting", started: "" };

/** One firing: its line, and folded under it what went in and came out. */
function row(f) {
  const r = el("details", "fire");
  r.setAttribute("role", "listitem");
  const line = el("summary", "fireline");
  line.append(el("span", "fwhen", at(f.at)), el("span", "fsched", `${f.schedule} · ${f.title}`));
  if (f.task) {
    line.append(statusBox({ id: f.task, status: f.status ?? "" }, (id) => { location.href = taskUrl(id); }), el("span", "fstatus", f.status ?? ""));
  }
  if (f.agent) line.append(el("span", "fagent", f.agent));
  const times = [f.started_at && `in progress ${at(f.started_at)}`, f.started_at && f.ended_at && `ended after ${span(f.started_at, f.ended_at)}`].filter(Boolean).join(" · ");
  if (times) line.append(el("span", "ftimes", times));
  const said = [STATES[f.state], f.missed ? (f.task ? `${f.missed} missed before it` : `${f.missed + 1} firings`) : ""].filter(Boolean).join(" · ");
  if (said) line.append(el("span", `fstate ${f.state.replace(" ", "-")}`, said));
  r.append(line);
  const body = el("div", "firebody");
  if (f.why) body.append(el("p", "fwhy", f.why));
  if (f.prompt) body.append(el("h3", null, "In"), expandable(el("pre", "fio", f.prompt)));
  if (f.result) body.append(el("h3", null, "Out"), expandable(el("pre", "fio", f.result)));
  if (!body.children.length) body.append(el("p", "msay", "Nothing went in: no task was made."));
  r.append(body);
  return r;
}

function draw() {
  $("fires").replaceChildren(...shown.map(row));
  if (!shown.length) $("fires").append(el("div", "mempty", "No firings yet. A schedule's runs come here; + Schedule on the board makes one."));
  $("ssum").textContent = shown.length ? `${plural(shown.length, "firing")}${state.schedule ? ` of ${state.schedule}` : ""}` : "";
  const sum = $("fschedule").querySelector("summary"), on = schedules.find((s) => s.key === state.schedule);
  sum.textContent = on ? `${on.key} · ${on.title}` : state.schedule || "All schedules";
  menu($("fschedule"), [{ label: "All schedules", on: !state.schedule, pick: () => choose("") },
    ...schedules.map((s) => ({ label: `${s.key} · ${s.title}`, on: s.key === state.schedule, pick: () => choose(s.key) }))]);
}

function choose(key) {
  state.schedule = key;
  writeQuery(state, KEYS, true);
  load();
}

/** The firings anew, or the next page after what is shown; a failure keeps what is shown and says so. */
async function load(more = false) {
  const mine = ++loading, q = new URLSearchParams();
  if (state.schedule) q.set("schedule", state.schedule);
  if (more && shown.length) q.set("before", String(shown[shown.length - 1].id));
  try {
    const got = await api(`/api/schedule-fires${q.size ? `?${q}` : ""}`);
    if (mine !== loading) return;
    shown = more ? [...shown, ...got.fires] : got.fires;
    $("smore").hidden = !got.more;
    $("serr").hidden = true;
    draw();
  } catch (e) {
    if (mine !== loading) return;
    $("serr").textContent = `The firings could not be read: ${e.message}`;
    $("serr").hidden = false;
  }
}

$("smore").addEventListener("click", () => load(true));
addEventListener("popstate", () => { state = readQuery(KEYS); load(); });
try {
  await loadMeta();
  schedules = (await api("/api/schedules")).schedules;
} catch (e) { $("serr").textContent = `The schedules could not be read: ${e.message}`; $("serr").hidden = false; }
await load();
