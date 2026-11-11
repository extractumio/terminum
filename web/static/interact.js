// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The interaction log's page side (terminum.md AC-175): what the user does here, counted by the clock
// minute and sent to terminumd once the minute is over: presses, keys, the wheel's and a finger's scrolls,
// drags, the pointer's moves and the seconds the page was in front. A count says how many and where (the
// page or frame, the id of a control of this page that was pressed), never which key, what text or what
// position; a frame's controls are not named: their ids can be a document's headings. Only the user's own
// events count (isTrusted): a screen that scrolls by itself is no reading. The Dashboard reads presence
// and engagement from these counts.
import "./signin.js";

const MINUTE = 60_000;
const SECTION = location.pathname.split("/")[1];
const HERE = ["kanban", "history", "dashboard", "scheduled"].includes(SECTION) ? SECTION : "sessions";
/** A frame of the sessions' page: its id says what it shows. */
const FRAMES = { files: "files", quickframe: "quick" };
const MOVE_EVERY = 1000;        // ms between two counted moves of the pointer
const DRAG_PX = 8;              // a press that moved this far is a drag
const BATCH = 300;              // rows a request takes (terminumd's interact ROWS)
const UNSENT = 600;             // rows kept while terminumd does not take them

/** The pane the sessions' page shows (app.js says it on the body): the counts are that pane's project's. */
const pane = () => document.body.dataset.sid || undefined;

let minute = Math.floor(Date.now() / MINUTE), rows = new Map(), unsent = [], flying = [];
let frontSince = 0, moved = 0, press = null;

/** One more of `kind` in `surface` at control `target`, from `at` to `t`, of pane `where`. */
function count(kind, surface, { target = "", amount = 0, at = Date.now(), t = at, where = pane() } = {}) {
  const key = `${kind}|${surface}|${target}|${where ?? ""}`;
  const row = rows.get(key) ?? { at, ended: t, kind, surface, target, n: 0, amount: 0, pane: where };
  row.n += 1; row.amount += Math.round(amount); row.ended = Math.max(row.ended, t);
  rows.set(key, row);
}

/** What the user just did: a minute that is over goes first; it also shows the page is in front. */
function did(kind, surface, more) {
  roll();
  if (!frontSince && inFront()) frontSince = Date.now();
  count(kind, surface, more);
}

/** The minute's counts become rows to send. */
function close() {
  unsent.push(...rows.values());
  rows = new Map();
}

/** A minute that is over goes to terminumd. */
function roll(t = Date.now()) {
  const m = Math.floor(t / MINUTE);
  if (m === minute) return;
  // the seconds in front belong to the minute they were in
  const still = inFront();
  front(Math.min(t, (minute + 1) * MINUTE));
  close();
  minute = m;
  if (still) frontSince = m * MINUTE;
  send();
}

const inFront = () => document.visibilityState === "visible" && document.hasFocus();

/** The whole seconds in front since they were last counted, up to `until`, as a count of this minute. */
function front(until = Date.now()) {
  const since = frontSince, s = Math.floor((until - since) / 1000);
  frontSince = 0;
  if (since && s > 0) count("front", HERE, { amount: s, at: since, t: until - 1, where: undefined });
}

function frontChanged() {
  roll();
  if (inFront()) frontSince ||= Date.now();
  else front();
}

/** Sends what no request carries yet; `leaving`: the page goes, the request outlives it. */
async function send(leaving = false) {
  unsent = unsent.slice(-UNSENT);
  const batch = unsent.filter((r) => !flying.includes(r)).slice(0, BATCH);
  if (!batch.length || (flying.length && !leaving)) return;
  flying = flying.concat(batch);
  let taken = false;
  try {
    const r = await fetch("/api/interactions", { method: "POST", headers: { "Content-Type": "application/json" }, keepalive: leaving, body: JSON.stringify({ now: Date.now(), rows: batch }) });
    // refused rows are not sent again: only a server that did not answer gets them once more
    taken = r.status < 500;
  } catch { /* terminumd is away: the next minute tries again */ }
  flying = flying.filter((r) => !batch.includes(r));
  if (taken) unsent = unsent.filter((r) => !batch.includes(r));
}

const watched = new WeakSet();
/** Counts what the user does in `doc` as `surface`'s; `named`: a pressed control's id is kept (its own or its
 *  nearest ancestor's). */
function watch(doc, surface, named) {
  if (watched.has(doc)) return;
  watched.add(doc);
  const on = (type, fn) => doc.addEventListener(type, (e) => { if (e.isTrusted) fn(e); }, { capture: true, passive: true });
  on("keydown", (e) => { if (!e.repeat) did("key", surface); });
  on("wheel", (e) => did("scroll", surface, { amount: Math.abs(e.deltaY) + Math.abs(e.deltaX) }));
  let touchY = null;
  on("touchstart", (e) => { touchY = e.touches[0]?.clientY ?? null; });
  on("touchmove", (e) => {
    const y = e.touches[0]?.clientY;
    if (touchY === null || y === undefined || Math.abs(y - touchY) < DRAG_PX) return;
    did("scroll", surface, { amount: Math.abs(y - touchY) });
    touchY = y;
  });
  on("pointerdown", (e) => { press = { x: e.clientX, y: e.clientY, far: 0, target: (named && e.target?.closest?.("[id]")?.id) || "", surface }; });
  on("pointermove", (e) => {
    if (press) press.far = Math.max(press.far, Math.hypot(e.clientX - press.x, e.clientY - press.y));
    const t = Date.now();
    // a pointer crossing a window that is not in front is no presence
    if (t - moved < MOVE_EVERY || !document.hasFocus()) return;
    moved = t;
    did("move", surface);
  });
  on("pointerup", () => {
    if (!press) return;
    if (press.far >= DRAG_PX) did("drag", press.surface, { target: press.target, amount: press.far });
    else did("click", press.surface, { target: press.target });
    press = null;
  });
  on("pointercancel", () => { press = null; });
}

/** A frame's document is another one after each load. */
function watchFrame(frame) {
  const attach = () => {
    try { if (frame.contentDocument && frame.contentWindow.location.href !== "about:blank") watch(frame.contentDocument, FRAMES[frame.id] ?? "view", false); } catch { /* not this origin's: not counted */ }
  };
  frame.addEventListener("load", attach);
  attach();
}

// a page shown in a frame (the quick shell) is counted by the page that holds it
if (window.top === window) {
  watch(document, HERE, true);
  for (const f of document.querySelectorAll("iframe")) watchFrame(f);
  // the views of files are frames made as they open (views.js)
  const views = document.getElementById("filewin");
  if (views) new MutationObserver((changes) => { for (const c of changes) for (const n of c.addedNodes) if (n.nodeName === "IFRAME") watchFrame(n); }).observe(views, { childList: true });
  document.addEventListener("visibilitychange", frontChanged);
  addEventListener("focus", frontChanged);
  addEventListener("blur", frontChanged);
  addEventListener("pagehide", () => { front(); close(); send(true); });
  setInterval(roll, 15_000);
  if (inFront()) frontSince = Date.now();
}
