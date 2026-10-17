// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { signedUrl } from "./signin.js";              // first: it signs every request the page makes
import { connection } from "./conn.js";
import { Input } from "./input.js";
import { findItem, metaLine, profileBar, renderNav, stateIcon } from "./nav.js";
import { textForm } from "./render.js";
import { arrowsTo, cellAt, cursorCell, typedEnd } from "./cursor.js";
import { BoxMoves } from "./boxnav.js";
import { askConfirm, navTools } from "./navtools.js";
import { embedded, quickShell } from "./quick.js";
import { paneMenu } from "./menu.js";
import { screenMode } from "./screenmode.js";
import { pageSettings } from "./settings.js";
import { bindReorder, ordered } from "./reorder.js";
import { linkMarks } from "./linkmarks.js";
import { paneFiles } from "./panefiles.js";
import { bindLinks } from "./termlinks.js";
import { copyButton } from "./touchcopy.js";
import { TermView } from "./term.js";
import { Views } from "./views.js";
import { uploads } from "./upload.js";
import { waitingCards } from "./waiting.js";
import { Widgets } from "./widgets.js";
import { Marks } from "./marks.js";
import * as trace from "./trace.js";

const $ = (id) => document.getElementById(id);
const narrow = matchMedia("(max-width: 860px)");
const touch = matchMedia("(hover: none)");
const LINES = ["wrap", "grid", "fit"];   // the Lines modes (view modes below)

// a quick shell's page (embed mode, AC-88) reads this browser's choices and keeps none of its own
const store = {
  get(k, d) { try { const v = localStorage.getItem(k); return v === null ? d : JSON.parse(v); } catch { return d; } },
  set(k, v) { if (embedded) return; try { localStorage.setItem(k, JSON.stringify(v)); } catch { /* private mode */ } },
};
if (embedded) document.body.classList.add("embed");

const st = {
  groups: [], sid: embedded ?? store.get("sid", null),
  mode: store.get("mode", {}), size: store.get("size", null), resized: false,
  collapsed: new Set(store.get("collapsed", [])),   // session groups closed in the list
  appCursor: false, bracketed: store.get("bracketed", true),
};

// ---------- connection (conn.js) ----------
const conn = connection({ parse: (data) => trace.parse(data), onMessage: (m) => onMessage(m), status,
  busy: () => ["files", "file"].some((f) => { try { return $(f).contentWindow?.fbBusy?.(); } catch { return false; } }),
  onOpen: (ws) => {
    trace.connected(send, ws);
    if (st.sid) select(st.sid, true);
    if (!embedded || focusAsked) focusTerminal();  // a computer types into the chat at once; a phone opens no keyboard
    active = true;                                 // a new connection streams until told otherwise
    syncActive();
  } });
// Nothing typed waits for a connection: it would arrive late, all at once (AC-14).
function send(m) {
  trace.sent(m);
  if (!conn.send(m) && (m.t === "in" || m.t === "paste" || m.t === "mouse")) toast("Not connected");
}
function status(text) { $("status").hidden = !text; $("status").textContent = text; }

// Pasted images, uploaded and dropped files: saved for the shown pane, their paths pasted.
const files = uploads({ sid: () => st.sid, paste: (text) => input.paste(text), toast, hideToast: () => { $("toast").hidden = true; } });

// A phone keeps fewer lines on the page: each one costs on every touch (AC-52).
const view = new TermView({ term: $("term"), hist: $("hist"), screen: $("screen"), note: $("note"),
  bottomBtn: $("bottom"), paused: $("paused"), send, keep: touch.matches ? 2000 : Infinity });
// Screen mode and sizing (AC-05, AC-13), settings.json's page defaults (AC-19): before anything draws.
const screen = screenMode({ view, send, store, current: () => (st.sid && findItem(st.groups, st.sid)) || null, sid: () => st.sid,
  refit: () => view.keepBottom(syncControls), defaults: () => settings.page() });
const settings = pageSettings({ toast, setLayout: (rows) => { input.setLayout(rows); syncControls(); }, onPage: (page) => {
  if (store.get("size", null) === null && page.textSize) setSize(narrow.matches ? Math.min(page.textSize, 13) : page.textSize, false);
  screen.sync(); view.keepBottom(syncControls);
} });

const input = new Input({ kbd: $("kbd"), kbdButton: $("kbdbtn"), term: $("term"), panel: $("hotkeys"), pasteDialog: $("pastebox"),
  send: (data, path, e) => { send({ t: "in", data, ...trace.keyFields(data, path, e) }); screen.typed(); if (data.includes("\r")) st.ran = true; },
  options: { appCursor: () => st.appCursor || view.alt, bracketed: () => st.bracketed, onTyped: (line) => { view.typed(); if (!line) boxMoves.forget(); }, status: toast, pasteImage: files.pasteImage, pickFile,
    tapLink: (x, y) => tapLink(x, y), tapMove: (x, y) => tapMove(x, y), cellWidth: () => view.charRatio() * parseFloat(getComputedStyle($("term")).fontSize),
    // Ctrl+W deletes a word: in an agent's input box; at a shell's prompt, not after an Enter until the list says the shell is back (nano: search)
    lineMove: (d, repeat) => (findItem(st.groups, st.sid)?.item.agent ? boxMoves.line(d, st.appCursor, repeat) : null),
    wordDeletes: () => { const it = st.sid && findItem(st.groups, st.sid)?.item; return !!(it?.agent ? view.box : it?.shell && !st.ran); } } });
copyButton($("copyfab"), $("term"), (text) => input.copySelection(text));

// The files the output names, on the pane's host: a path is a link, an image gets a button, only
// when its file is there (relative paths from the pane's folder, as a click takes them).
const shownFiles = paneFiles(() => views.paneInfo(), undefined, signedUrl);
// Addresses and paths get a dotted underline (linkmarks.js).
const marks = linkMarks($("term"), [$("hist"), $("screen")], shownFiles);
const views = new Views({ send, store, onShow: () => { view.keepBottom(applyFit); marks(); }, onBack: () => focusTerminal() });
// Addresses open in a new tab here, paths in File (on the pane's host).
// (a quick shell's page has no View: its pane's page opens it)
const openFile = (p, host) => (embedded ? parent.postMessage({ type: "tm-open", path: p, host }, location.origin) : views.open(p, host));
const openAt = bindLinks($("term"), { paneInfo: () => views.paneInfo(), openFile, toast, state: shownFiles.state });
// Where a tap moves the cursor (a shell or a coding agent), on the cursor's row it does so, also
// over a path typed there; elsewhere a tap there opens a link.
function tapLink(x, y) {
  if (screen.mouseOn()) return false;              // a tap is the program's click (AC-06)
  const it = st.sid && findItem(st.groups, st.sid)?.item;
  const cur = (it?.agent || it?.shell) && cursorCell($("screen")), at = cur && cellAt($("screen"), x, y);
  return !(at && (cur.row === at.row || (it.agent && boxMoves.has(x, y)))) && openAt(x, y);
}

// A tap or a click (input.js) on the input moves the cursor there with arrow keys: in a coding
// agent's input box by its text (boxnav.js: ← and → only, as the page draws its lines), on a
// shell's prompt along the row. Only where arrows edit a line (not htop, less, mc); never ↑ or ↓,
// which would recall history.
const boxMoves = new BoxMoves(view, $("screen"));
function tapMove(x, y) {
  const found = st.sid && findItem(st.groups, st.sid);
  if ((!found?.item.agent && !found?.item.shell) || screen.mouseOn()) return;
  const inBox = found.item.agent ? boxMoves.to(x, y, st.appCursor) : null;
  if (inBox !== null) { if (inBox) { send({ t: "in", data: inBox, ...trace.keyFields(inBox, "tap") }); view.typed(); } return; }
  const from = cursorCell($("screen")), to = cellAt($("screen"), x, y);
  // right of the cursor only as far as the typed text: → over a suggestion would take it
  if (from && to && to.row === from.row && to.col > from.col) to.col = Math.max(from.col, Math.min(to.col, typedEnd(view.screen[to.row]?.data)));
  const seq = arrowsTo(from, to, { appCursor: st.appCursor });
  if (seq) send({ t: "in", data: seq, ...trace.keyFields(seq, "tap") });
  if (seq) view.typed();
}

// On a touch screen the invisible keyboard field lies over the cursor's row, so a long press
// there gets iOS's own Paste (text, or an image); elsewhere it is out of the way.
function placeKbd() {
  // a coding agent's input box whole (where it draws its own cursor may not be tmux's), else the
  // cursor's row; in Screen mode with the mouse on, taps go to the program
  const k = $("kbd"), cur = touch.matches && !screen.mouseOn() ? $("screen").querySelector(".blk.inbox") ?? $("screen").querySelector(".cur") : null;
  const t = $("term").getBoundingClientRect(), r = cur?.getBoundingClientRect();
  if (!r || r.bottom < t.top || r.top > t.bottom) { k.style.cssText = ""; return; }
  k.style.cssText = `left:${t.left}px;top:${Math.max(r.top, t.top)}px;width:${t.width}px;height:${Math.min(r.bottom, t.bottom) - Math.max(r.top, t.top)}px`;
}
view.onDrawn = () => { placeKbd(); marks(); trace.drawn(); };

// Code, Markdown and diagrams in the output as widgets, images named in it behind a button
// (AC-56), read through the Files proxy on the pane's host.
const widgets = new Widgets({ view, term: $("term"), boxes: [$("hist"), $("screen")], files: shownFiles,
  copy: (text) => input.copySelection(text), send, notify: (text) => toast(text, 3000), onZoomClosed: () => focusTerminal() });
// none in Screen mode (a full-screen program's grid), nor when settings.json turned them off
const widgetsOn = () => !screen.on() && settings.page().widgets !== false;
view.onRows = (full, keep) => { if (widgetsOn()) (keep ? widgets.keepPlace(() => widgets.update(full)) : widgets.update(full)); };
view.onReset = () => { widgets.reset(); shownFiles.forget(); };
// Bookmarks of a selection, with a note, kept on the Mac (AC-57)
const bookmarks = new Marks({ view, widgets, term: $("term"), send, sid: () => st.sid, alive: (id) => findItem(st.groups, id)?.item.title ?? null,
  pick: (id) => pick(id), toast, onChange: () => drawNav(), copy: (text) => input.copySelection(text, true) });
trace.watch(view, { st, bookmarks, widgets, findItem, send, mode: () => shownMode(), copy: (text) => input.copySelection(text) });
let kbdFrame = 0;
$("term").addEventListener("scroll", () => { if (!kbdFrame) kbdFrame = requestAnimationFrame(() => { kbdFrame = 0; placeKbd(); }); }, { passive: true });

// On a phone an agent's status lines (its footer) stay behind a button, remembered per browser.
function showFooter(on) {
  $("term").classList.toggle("showfoot", on);
  $("footfab").setAttribute("aria-pressed", String(on));
  store.set("showFooter", on);
}
showFooter(store.get("showFooter", false));
$("footfab").addEventListener("click", () => view.keepBottom(() => showFooter(!$("term").classList.contains("showfoot"))));

let toastTimer = 0;
function toast(text, ms = 1800) {
  $("toast").textContent = text; $("toast").hidden = false;
  clearTimeout(toastTimer); toastTimer = setTimeout(() => { $("toast").hidden = true; }, ms);
}

function pickFile() { $("upload").value = ""; $("upload").click(); }
$("upload").addEventListener("change", () => { const f = $("upload").files[0]; if (f) files.uploadFile(f); });
files.bindDrop($("term"));


const tools = navTools({ send, store, current: () => (st.sid && findItem(st.groups, st.sid)) || null, groups: () => st.groups,
  onResize: () => view.keepBottom(applyFit), onRenamed: () => (computer.matches ? focusTerminal() : document.activeElement?.blur()) });
const menu = paneMenu({ send, current: () => (st.sid && findItem(st.groups, st.sid)) || null, groups: () => st.groups,
  pick: (id) => pick(id), signOut: () => conn.signOut(), copy: (text) => input.copySelection(text), quick: (side) => quick.open(side) });
const quick = quickShell({ send, store, current: () => (st.sid && findItem(st.groups, st.sid)) || null, item: (id) => findItem(st.groups, id)?.item,
  pick: (id) => pick(id), askConfirm, narrow });

// ---------- the keys go back to the terminal (a computer only) ----------
// With a mouse and a hardware keyboard, typing goes to the terminal again when the user comes
// back to this window or tab (if it had the keys before), picks a session, or returns to the
// Terminal view; never while a field, Files or File, the paste dialog, a name or a menu has
// them. A touch screen gets no focus() it did not tap for: on iOS that raises the keyboard.
const computer = matchMedia("(hover: hover) and (pointer: fine)");
let termHadKeys = false;
function focusTerminal() {
  if (!computer.matches || $("app").hidden || document.body.dataset.shown !== "term") return;
  if (!$("menu").hidden || $("pastebox").open || bookmarks.busy() || tools.renaming()) return;
  if (document.activeElement?.closest?.("input, textarea:not(#kbd), select, iframe, [contenteditable]")) return;
  input.focus();
}
addEventListener("blur", () => { termHadKeys = document.activeElement === $("kbd"); });   // leaving: who had the keys
addEventListener("focus", () => { if (termHadKeys) focusTerminal(); });
document.addEventListener("visibilitychange", () => { if (!document.hidden && termHadKeys) focusTerminal(); });
for (const b of [document.querySelector('button[data-view="term"]'), $("backterm"), $("fileclose")]) b.addEventListener("click", () => focusTerminal());

// A hidden page (another app, a locked phone) costs the Mac nothing: its stream pauses, and the
// panes' captures with it. On a computer so does a page out of focus (the user works in another
// window); focus in the Files frame is still this page's.
let active = true;                             // as terminumd takes a page that connects
let parentActive = true;                       // embed mode: its pane's page is looked at
let focusAsked = false;                        // embed mode: its pane's page gave it the keys
function syncActive() {
  const now = !document.hidden && (!computer.matches || document.hasFocus() || (!!embedded && parentActive));
  if (now !== active) send({ t: now ? "resume" : "pause" });
  active = now;
  quick.active(now);
}
addEventListener("message", (e) => {
  if (e.origin !== location.origin) return;
  if (!embedded && e.source === $("quickframe").contentWindow && e.data?.type === "tm-open") return views.open(String(e.data.path), e.data.host ?? null);
  if (!embedded || e.source !== parent) return;
  if (e.data?.type === "tm-focus") { focusAsked = true; focusTerminal(); }
  if (e.data?.type === "tm-active") { parentActive = !!e.data.on; syncActive(); }
});
document.addEventListener("visibilitychange", syncActive);
addEventListener("focus", syncActive);
addEventListener("blur", () => setTimeout(syncActive, 0));   // the focus may have gone into the Files frame

function onMessage(m) {
  if (m.sid && m.sid !== st.sid) return;            // a frame from the session we just left
  trace.received(m);
  bookmarks.seen(m);
  switch (m.t) {
    case "layout": st.groups = m.groups; st.ran = false; if (!embedded) reminders.layout(m.groups); drawNav(); tools.layoutCame(m.groups); break;
    case "theme": applyTheme(m.theme); break;
    case "hist":                                    // a reset: another pane, or its scrollback cleared
      if (m.mode === "reset") widgets.folds.reset();
      widgets.folds.add(m.folds); view.onHist(m); break;
    case "folds": widgets.folds.add(m.items); widgets.keepPlace(() => widgets.update(false, true)); break;   // lines that scrolled off
    case "lines":
      if (m.error) { widgets.folds.failed(m.first); toast(m.error, 5000); break; }
      view.refill(m.first, m.lines); widgets.keepPlace(() => widgets.folds.filled(m.first, m.lines.length)); break;
    case "screen": view.onScreen(m); screen.sync(); break;
    case "sized": screen.onSized(m); break;
    case "settings": settings.apply(m); break;
    case "restore": if (!embedded) menu.showRestore(m.report); break;
    case "fit": st.resized = m.on; syncControls(); break;
    case "files": if (embedded) views.answered(m); else views.onFiles(m); break;   // a quick shell has no Files
    case "quickmade": quick.made(m); break;
    case "hostadded": tools.hostAdded(m.name); break;
    case "tmuxadded": tools.tmuxAdded(); break;
    case "tmuxsessions": tools.tmuxChoices(m); break;
    case "error": if (!m.msg) status(""); else if (!tools.formError(m.msg)) status(m.msg); break;   // empty: tmux is back
    case "update": if (!embedded) menu.showUpdate(m.latest); break;
    case "note": toast(m.msg, 4000); break;
    case "stall": tools.showStall(m); break;
    case "build": conn.checkBuild(m.id); break;
    case "trace": trace.onMessage(m); break;
    case "created": pick(m.id); if (m.note) toast(m.note, 6000); break;
  }
}

// ---------- navigation ----------

// Rows reorder tmux's windows in their session; groups reorder this browser's list only.
const reorder = bindReorder($("sessions"), (kind, parent, ids) => {
  if (kind === "group") store.set("groupOrder", ids);
  else send({ t: "reorder", group: parent.dataset.wid, ids });
}, () => drawNav());                                // a layout may have come while dragging

function pick(id) {
  const of = !narrow.matches && findItem(st.groups, id)?.item.quickOf;   // a quick shell's row: its pane, with it
  select(of || id); closeDrawer();
  if (of) quick.focus(); else focusTerminal();
}
const reminders = waitingCards($("asks"), pick);
function drawNav() {
  if (embedded) return showCurrent(findItem(st.groups, st.sid) || null);
  reminders.show(st.sid);
  if (reorder.dragging()) return;                  // the next layout redraws it
  renderNav($("sessions"), ordered(st.groups, store.get("groupOrder", [])), st.sid, $("filter").value, pick, st.collapsed, toggleGroup,
    (wid) => send({ t: "new", group: wid }), bookmarks.sids(), tools.hostMenu, (text) => input.copySelection(text, true), tools.importTmux);
  const found = st.sid && findItem(st.groups, st.sid);
  if (!found) {
    const first = st.groups.flatMap((g) => g.items)[0];
    if (first) select(first.id);
    else showCurrent(null);
    if (!st.sid && narrow.matches) openDrawer();
    return quick.sync();
  }
  showCurrent(found);
  quick.sync();
}

function toggleGroup(key) {
  if (!st.collapsed.delete(key)) st.collapsed.add(key);
  store.set("collapsed", [...st.collapsed]);
  drawNav();
}

// The header: the profile's bar beside two lines: the agent's state, the title and the pencil;
// then one line that only shortens at its end: host, program, folder, profile, where the tab is, titles.
function showCurrent(found) {
  $("current").querySelector(".t").textContent = found ? textForm(found.item.title) : "Choose a session";
  $("curstate").replaceChildren(...[found && stateIcon(found.item.state)].filter(Boolean));
  view.setAgent(!!found?.item.agent);
  if (!found) { $("cursub").replaceChildren(); $("curbar").replaceChildren(); }
  else {
    const { group: g, item: it } = found;
    const where = (g.session ? `${g.label} ${g.session}` : g.label) + (it.index ? `, window ${it.index}` : "");
    // what runs there first (host, program, folder), then where the tab is, then its titles
    const sub = [...(it.host ? [it.host] : []), ...it.sub], dir = sub.findIndex((p) => /^[~/]/.test(p));
    const at = dir >= 0 ? dir + 1 : Math.min(sub.length, it.host ? 2 : 1);
    const line = metaLine([...sub.slice(0, at), it.profile || "", where, ...sub.slice(at)].filter(Boolean), !!it.host);
    $("cursub").replaceChildren(...line.childNodes);
    $("curbar").replaceChildren(profileBar(it.profile, true, it.state === "working"));
  }
  document.title = found ? `${found.item.title} – terminum` : "terminum";
  if (!tools.renaming()) $("rename").hidden = !found;
}

function select(id, force = false) {
  if (id === st.sid && !force && view.screen.length) return;
  st.sid = id; store.set("sid", id);
  status("");                                      // an error about the pane shown before
  if (tools.renaming()) tools.endRename();         // a name being typed was for that pane
  view.reset();
  trace.opening(id);
  send({ t: "sub", id });                          // first: the server answers "files" for the subscribed pane
  screen.opened();                                 // its Screen mode and this view's size
  views.sessionChanged();
  drawNav(); syncControls();
}

function openDrawer() {
  if (!narrow.matches) { $("app").classList.remove("nav-hidden"); store.set("navHidden", false); return; }
  $("app").classList.add("nav-open");
  $("navbtn").setAttribute("aria-expanded", "true");
  $("sessions").querySelector(".item.on")?.scrollIntoView({ block: "center" });
}
function closeDrawer() {
  $("app").classList.remove("nav-open");
  $("navbtn").setAttribute("aria-expanded", "false");
}
$("navbtn").onclick = openDrawer;
$("current").onclick = () => (narrow.matches ? openDrawer() : $("filter").focus());

$("scrim").onclick = closeDrawer;
$("drawerclose").onclick = closeDrawer;
// "/" filters the sessions, unless the keys go to a field or the terminal
document.addEventListener("keydown", (e) => {
  if (e.key !== "/" || e.metaKey || e.ctrlKey || e.altKey || e.target.closest?.("input, textarea, [contenteditable]")) return;
  e.preventDefault();
  openDrawer();
  $("filter").focus();
});
$("navclose").onclick = () => {
  if (narrow.matches) return closeDrawer();
  $("app").classList.add("nav-hidden"); store.set("navHidden", true);
};
$("filter").addEventListener("input", drawNav);
$("filter").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("sessions").querySelector(".item")?.click();
  if (e.key === "Escape") { $("filter").value = ""; drawNav(); input.focus(); }
});
if (store.get("navHidden", false)) $("app").classList.add("nav-hidden");

// ---------- theme ----------

let shownTheme = "", shownFont = "";
function applyTheme(theme) {
  // sent with every subscribe and resume; the same one changes nothing (and rebuilds no line)
  const key = JSON.stringify(theme);
  if (key === shownTheme) return;
  shownTheme = key;
  views.setTheme(theme);
  widgets.setTheme(theme);
  const r = document.documentElement.style;
  r.setProperty("--t-bg", theme.bg); r.setProperty("--t-fg", theme.fg);
  r.setProperty("--accent", theme.cursor || theme.fg);
  if (theme.sel) r.setProperty("--sel", theme.sel);
  if (theme.selfg) r.setProperty("--selfg", theme.selfg);
  document.querySelector('meta[name="theme-color"]').content = theme.bg;
  if (st.size === null && !settings.page().textSize && theme.size) setSize(narrow.matches ? Math.min(theme.size, 13) : theme.size, false);
  if (theme.font && document.fonts && theme.font !== shownFont) {
    shownFont = theme.font;
    const face = new FontFace("TermFont", `local("${theme.font.replace(/["\\]/g, "")}")`);
    face.load().then((f) => document.fonts.add(f)).catch(() => { /* not on this device: Menlo */ });
  }
  view.setTheme(theme);
}

function setSize(px, save = true) {
  st.size = Math.max(7, Math.min(28, Math.round(px)));
  document.documentElement.style.setProperty("--size", `${st.size}px`);
  $("sizeval").textContent = st.size;
  if (save) store.set("size", st.size);
  screen.changed();                                // the grid this view holds changed (AC-13)
}

// ---------- view modes ----------
// Wrap: lines re-flow to the screen. Grid: tmux's exact layout at the chosen text size.
// Fit: tmux's exact layout scaled down so a whole line fits the screen width; the pane is not
// touched. (Fitting the pane to the screen is a separate menu action, as it changes its size.)
// Screen mode (screenmode.js) shows a full-screen program as its grid, as Fit when the pane is
// wider than the view or sized for another screen; with Screen mode "never", Wrap shows it as
// Fit while it runs. The buttons keep the mode chosen.

// The mode is remembered per pane, and the one chosen last is the default for every other.
const MODES_KEPT = 100;
// Until the user picks one: settings.json's page.lines, else Wrap, on every screen.
function mode() { return st.mode[st.sid] ?? store.get("modeDefault", LINES.includes(settings.page().lines) ? settings.page().lines : "wrap"); }
function shownMode() {
  if (screen.on() || screen.elsewhere()) return screen.elsewhere() || view.tooWide() ? "fit" : "grid";
  return view.alt && view.tooWide() && mode() === "wrap" ? "fit" : mode();
}
function setMode(m, chosen = true) {
  delete st.mode[st.sid];                          // re-added last: the oldest choices go first
  st.mode[st.sid] = m;
  const ids = Object.keys(st.mode);
  for (const id of ids.slice(0, Math.max(0, ids.length - MODES_KEPT))) delete st.mode[id];
  store.set("mode", st.mode);
  if (chosen) store.set("modeDefault", m);
  view.keepBottom(syncControls);
}

function applyFit() {
  const term = $("term"), m = shownMode();
  term.classList.toggle("wrap", m === "wrap");
  term.classList.toggle("grid", m !== "wrap");
  if (m !== "fit" || !view.cols) { term.style.fontSize = ""; return; }
  const px = Math.floor(((view.inner().w - 1) / view.cols / view.charRatio()) * 10) / 10;
  term.style.fontSize = `${Math.min(px, view.chosenSize() + 4)}px`;
}
view.onCols = () => view.keepBottom(applyFit);
view.onAlt = () => { screen.sync(); view.keepBottom(applyFit); };
document.fonts?.addEventListener("loadingdone", () => { view.ratio = 0; view.keepBottom(applyFit); });

function syncControls() {
  const m = mode();
  for (const b of document.querySelectorAll("[data-mode]")) b.setAttribute("aria-pressed", String(b.dataset.mode === m));
  $("resize").querySelector(".label").textContent = st.resized ? "Restore size" : "Fit to this screen";
  $("appcur").checked = st.appCursor;
  $("bracket").checked = st.bracketed;
  const keysOn = !$("hotkeys").hidden;
  $("keysbtn").setAttribute("aria-pressed", String(keysOn));
  $("keysfab").setAttribute("aria-pressed", String(keysOn));
  document.documentElement.style.setProperty("--keys-h", `${$("hotkeys").offsetHeight}px`);
  applyFit();
}

for (const b of document.querySelectorAll("[data-mode]")) b.onclick = () => setMode(b.dataset.mode);
$("smaller").onclick = () => view.keepBottom(() => { setSize(st.size - 1); applyFit(); });
$("bigger").onclick = () => view.keepBottom(() => { setSize(st.size + 1); applyFit(); });
$("appcur").onchange = (e) => { st.appCursor = e.target.checked; };
$("bracket").onchange = (e) => { st.bracketed = e.target.checked; store.set("bracketed", st.bracketed); };
$("resize").onclick = () => {
  menu.close();
  if (st.resized) return send({ t: "unfit" });
  if (mode() === "wrap") setMode("grid", false);   // a resized session is shown as its exact grid
  $("term").style.fontSize = "";
  send({ t: "fit", ...view.gridFit(view.chosenSize()) });
};

// ---------- the on-screen keyboard ----------
// iOS Safari lays the keyboard over the page instead of shrinking it. Size the app to the
// visible part of the screen, so the terminal and the hot keys sit just above the keyboard,
// and give the space back when the keyboard closes.
const vv = window.visualViewport;
function fitViewport() {
  if (!vv) return;
  const stick = view.atBottom();
  document.documentElement.style.setProperty("--app-h", `${Math.round(vv.height)}px`);
  $("app").style.transform = vv.offsetTop ? `translateY(${Math.round(vv.offsetTop)}px)` : "";
  applyFit();
  if (stick) view.toBottom();
}
vv?.addEventListener("resize", fitViewport);
vv?.addEventListener("scroll", fitViewport);
addEventListener("resize", fitViewport);   // a window resize does not always reach visualViewport

function showKeys(on) {
  $("hotkeys").hidden = !on;
  store.set("keys", on);
  view.keepBottom(syncControls);
}
$("keysbtn").onclick = () => showKeys($("hotkeys").hidden);
$("keysfab").onclick = () => showKeys($("hotkeys").hidden);
$("keysfab").onmousedown = (e) => e.preventDefault();      // the iOS keyboard stays up
document.addEventListener("keydown", (e) => { if (e.key === "Escape") { menu.close(); closeDrawer(); } });

// Toolbar buttons never take focus from the terminal: Enter must keep going to the shell.
$("bar").addEventListener("mousedown", (e) => { if (e.target.closest("#tools button")) e.preventDefault(); });

$("hotkeys").hidden = !store.get("keys", false);
if (st.size) setSize(st.size, false);
syncControls();
conn.connect();
