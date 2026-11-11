// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The interaction log's page side (terminum.md AC-175): what the user does here, counted by the clock
// minute and sent to terminumd once the minute is over: presses, keys, the wheel's and a finger's scrolls,
// drags, the pointer's moves and the seconds the page was in front. A count says how many and where (the
// page or frame, a pressed control's id), never which key, what text or what position. Only the user's
// own events count (isTrusted): a screen that scrolls by itself is no reading. The Dashboard reads
// presence and engagement from these counts.
import "./signin.js";

const MINUTE = 60_000;
const SURFACES = { sessions: "sessions", kanban: "kanban", history: "history", dashboard: "dashboard", scheduled: "scheduled" };
const HERE = SURFACES[location.pathname.split("/")[1]] ?? "sessions";
/** A frame of the sessions' page: its id says what it shows. */
const FRAMES = { files: "files", quickframe: "quick" };
const MOVE_EVERY = 1000;        // ms between two counted moves of the pointer
const DRAG_PX = 8;              // a press that moved this far is a drag
const UNSENT = 600;             // rows kept while terminumd does not take them

/** The pane the sessions' page shows, as its list marks it: the counts are that pane's project's. */
const pane = () => (HERE === "sessions" ? document.querySelector('#sessions [aria-current="true"]')?.dataset.id : null);

let minute = 0, rows = new Map(), unsent = [], sending = false;
let frontSince = 0, moved = 0, press = null;

/** One more of `kind` in `surface` at control `target`, now. */
function count(kind, surface, target = "", amount = 0, n = 1) {
  const t = Date.now();
  roll(t);
  const p = pane() || undefined, key = `${kind}|${surface}|${target}|${p ?? ""}`;
  const row = rows.get(key) ?? { at: t, ended: t, kind, surface, target, n: 0, amount: 0, pane: p };
  row.n += n; row.amount += Math.round(amount); row.ended = t;
  rows.set(key, row);
}

/** A minute that is over goes to terminumd. */
function roll(t = Date.now()) {
  const m = Math.floor(t / MINUTE);
  if (m === minute) return;
  // the seconds in front belong to the minute they were in
  if (minute) front(Math.min(t, (minute + 1) * MINUTE));
  if (rows.size) unsent.push(...rows.values());
  rows = new Map();
  minute = m;
  if (inFront()) frontSince = Math.max(frontSince, m * MINUTE);
  send();
}

const inFront = () => document.visibilityState === "visible" && document.hasFocus();

/** The time in front since it was last counted, up to `until`, as a count of its own. */
function front(until = Date.now()) {
  if (!frontSince) return;
  const s = Math.round((until - frontSince) / 1000);
  frontSince = 0;
  if (s <= 0) return;
  const key = `front|${HERE}||`, at = until - s * 1000;
  const row = rows.get(key) ?? { at, ended: at, kind: "front", surface: HERE, target: "", n: 0, amount: 0 };
  row.n += 1; row.amount += s; row.ended = Math.max(row.ended, until - 1);
  rows.set(key, row);
}

function frontChanged() {
  roll();
  if (inFront()) frontSince ||= Date.now();
  else front();
}

async function send(leaving = false) {
  if (sending && !leaving) return;
  if (!unsent.length) return;
  unsent = unsent.slice(-UNSENT);
  const batch = unsent.slice(0, 300);
  sending = true;
  try {
    const r = await fetch("/api/interactions", { method: "POST", headers: { "Content-Type": "application/json" }, keepalive: leaving, body: JSON.stringify({ now: Date.now(), rows: batch }) });
    // refused rows are not sent again: only a server that did not answer gets them once more
    if (r.status < 500) unsent = unsent.filter((x) => !batch.includes(x));
  } catch { /* terminumd is away: the next minute tries again */ }
  finally { sending = false; }
}

/** The id of the control an event is on: its own or its nearest ancestor's. */
const targetOf = (e) => e.target?.closest?.("[id]")?.id ?? "";

const watched = new WeakSet();
/** Counts what the user does in `doc` as `surface`'s. */
function watch(doc, surface) {
  if (watched.has(doc)) return;
  watched.add(doc);
  const on = (type, fn) => doc.addEventListener(type, (e) => { if (e.isTrusted) fn(e); }, { capture: true, passive: true });
  on("keydown", (e) => { if (!e.repeat) count("key", surface); });
  on("wheel", (e) => count("scroll", surface, "", Math.abs(e.deltaY) + Math.abs(e.deltaX)));
  let touchY = null;
  on("touchstart", (e) => { touchY = e.touches[0]?.clientY ?? null; });
  on("touchmove", (e) => {
    const y = e.touches[0]?.clientY;
    if (touchY === null || y === undefined || Math.abs(y - touchY) < DRAG_PX) return;
    count("scroll", surface, "", Math.abs(y - touchY));
    touchY = y;
  });
  on("pointerdown", (e) => { press = { x: e.clientX, y: e.clientY, far: 0, target: targetOf(e), surface }; });
  on("pointermove", (e) => {
    if (press) press.far = Math.max(press.far, Math.hypot(e.clientX - press.x, e.clientY - press.y));
    const t = Date.now();
    // a pointer crossing a window that is not in front is no presence
    if (t - moved < MOVE_EVERY || !document.hasFocus()) return;
    moved = t;
    count("move", surface);
  });
  const up = () => {
    if (!press) return;
    if (press.far >= DRAG_PX) count("drag", press.surface, press.target, press.far);
    else count("click", press.surface, press.target);
    press = null;
  };
  on("pointerup", up);
  on("pointercancel", () => { press = null; });
}

/** A frame's document is another one after each load. */
function watchFrame(frame) {
  const surface = FRAMES[frame.id] ?? "view";
  const attach = () => {
    try { if (frame.contentDocument && frame.contentWindow.location.href !== "about:blank") watch(frame.contentDocument, surface); } catch { /* not this origin's: not counted */ }
  };
  frame.addEventListener("load", attach);
  attach();
}

// a page shown in a frame (the quick shell) is counted by the page that holds it
const framed = window.top !== window;
if (!framed) start();

function start() {
  watch(document, HERE);
  for (const f of document.querySelectorAll("iframe")) watchFrame(f);
  new MutationObserver((changes) => {
    for (const c of changes) for (const n of c.addedNodes) if (n.nodeName === "IFRAME") watchFrame(n);
  }).observe(document.documentElement, { childList: true, subtree: true });
  document.addEventListener("visibilitychange", frontChanged);
  addEventListener("focus", frontChanged);
  addEventListener("blur", frontChanged);
  addEventListener("pagehide", () => { front(); unsent.push(...rows.values()); rows = new Map(); send(true); });
  setInterval(roll, 15_000);
  minute = Math.floor(Date.now() / MINUTE);
  if (inFront()) frontSince = Date.now();
}
