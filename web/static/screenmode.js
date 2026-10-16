// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Sizing (AC-13), Screen mode (AC-05) and the mouse in it (AC-06). The page tells terminumd the
// grid that fits its terminal at the chosen text size: after opening a pane, after the view or
// the text size changed (300 ms after the last change), and with the first key typed after a
// pause (at most once a second), which under the "latest" policy makes this page size the pane.
// Screen mode draws a full-screen program (tmux's alternate screen, or a pane set to "always")
// as its exact grid: no history, no Lines buttons, "Screen · <program>" in the header, the pane
// sized to this view. A pane another screen sized is drawn as Fit until this page types.
// While a program asked for the mouse, clicks, drags, the wheel and taps go to it.

const $ = (id) => document.getElementById(id);
const MODES = ["auto", "always", "never"];
const KEPT = 100;               // panes whose Screen mode is remembered
const SETTLE = 300;             // ms after the last change of the view before its size is sent
const TYPED_EVERY = 1000;       // ms: a key sends the size at most this often
const MIN = { cols: 20, rows: 5 };

/** view: the TermView; current(): the shown pane's {group, item} or null; sid(): its id;
 *  refit(): the Lines mode may have changed (Fit/Grid); defaults(): {screenMode} of settings. */
export function screenMode({ view, send, store, current, sid, refit, defaults }) {
  const term = $("term");
  let modes = store.get("screenModes", {});
  let on = false, last = null, typedAt = -Infinity, settle = 0, elsewhere = null, small = false;

  const mode = () => modes[sid()] ?? defaults().screenMode ?? "auto";
  const want = () => !!sid() && (mode() === "always" || (mode() === "auto" && !!view.alt));
  /** The grid that fits the terminal at the chosen text size (not Fit's scaled one). */
  const grid = () => view.gridFit(view.chosenSize());

  function sendSize(typing) {
    if (!sid() || term.hidden || !term.clientWidth) return;
    const g = grid();
    if (!(g.cols > 0 && g.rows > 0)) return;
    small = on && (g.cols < MIN.cols || g.rows < MIN.rows);
    label();
    if (small) return;                            // the pane keeps the last size
    if (!typing && last && last.cols === g.cols && last.rows === g.rows && last.screen === on) return;
    last = { ...g, screen: on };
    send({ t: "size", cols: g.cols, rows: g.rows, typing, screen: on });
  }

  function label() {
    const el = $("scrlabel"), prog = current()?.item.sub?.[0] ?? "";
    el.hidden = !on;
    el.textContent = small ? "Too small for Screen mode" : prog ? `Screen · ${prog}` : "Screen";
    el.classList.toggle("warn", small);
  }

  /** Screen mode follows the frame (alt) and the pane's choice. */
  function sync() {
    const now = want();
    if (now !== on) {
      on = now;
      document.body.classList.toggle("screenmode", on);
      term.classList.toggle("screenmode", on);
      refit();
      sendSize(false);
    }
    label();
    syncButtons();
  }

  function syncButtons() {
    for (const b of document.querySelectorAll("[data-screen]")) b.setAttribute("aria-pressed", String(b.dataset.screen === mode()));
  }

  function setMode(m) {
    if (!MODES.includes(m) || !sid()) return;
    delete modes[sid()];                          // re-added last: the oldest choices go first
    modes[sid()] = m;
    const ids = Object.keys(modes);
    for (const id of ids.slice(0, Math.max(0, ids.length - KEPT))) delete modes[id];
    store.set("screenModes", modes);
    send({ t: "screenmode", mode: m });
    sync();
  }
  for (const b of document.querySelectorAll("[data-screen]")) b.addEventListener("click", () => setMode(b.dataset.screen));

  // the view's size: the window, the session list, docked Files, the keyboard, the hot keys
  const changed = () => { clearTimeout(settle); settle = setTimeout(() => sendSize(false), SETTLE); };
  new ResizeObserver(changed).observe(term);

  function showSized() {
    $("sized").hidden = !elsewhere;
    if (elsewhere) $("sized").textContent = `Sized for another screen (${elsewhere.cols}×${elsewhere.rows})` + (elsewhere.error ? `: ${elsewhere.error}` : "");
  }

  // ---------- the mouse (AC-06) ----------
  const mouseOn = () => on && !!view.mouse;
  /** {x, y} (0-based cells of the screen) at a viewport point, or null off the screen. */
  function cellAt(cx, cy) {
    const rows = view.screen.filter(Boolean);
    if (!rows.length) return null;
    let y = rows.findIndex((r) => r.el.getBoundingClientRect().bottom > cy);
    if (y < 0) y = rows.length - 1;
    const box = rows[y].el.getBoundingClientRect();
    const w = view.charRatio() * parseFloat(getComputedStyle(term).fontSize);
    const x = Math.max(0, Math.min((view.cols || 1) - 1, Math.floor((cx - box.left) / w)));
    return { x, y: view.screen.indexOf(rows[y]) };
  }
  const mods = (e) => ({ shift: !!e.shiftKey, alt: !!e.altKey, ctrl: !!e.ctrlKey });
  function report(action, at, button, e) {
    if (at) send({ t: "mouse", x: at.x, y: at.y, button, action, mods: mods(e ?? {}) });
  }
  const BUTTON = { 0: 0, 1: 1, 2: 2 };
  let held = null;                                // a mouse button down: {button, x, y}
  term.addEventListener("pointerdown", (e) => {
    if (e.pointerType !== "mouse" || !mouseOn() || !(e.button in BUTTON)) return;
    e.preventDefault();                           // no text selection: the drag is the program's
    const at = cellAt(e.clientX, e.clientY);
    held = { button: BUTTON[e.button], ...at };
    report("press", at, held.button, e);
  });
  term.addEventListener("pointermove", (e) => {
    if (e.pointerType !== "mouse" || !held || !mouseOn()) return;
    const at = cellAt(e.clientX, e.clientY);
    if (!at || (at.x === held.x && at.y === held.y)) return;
    Object.assign(held, at);
    report("move", at, held.button, e);
  });
  addEventListener("pointerup", (e) => {
    if (e.pointerType !== "mouse" || !held) return;
    const b = held.button; held = null;
    if (mouseOn()) report("release", cellAt(e.clientX, e.clientY), b, e);
  });
  term.addEventListener("contextmenu", (e) => { if (mouseOn()) e.preventDefault(); });
  term.addEventListener("wheel", (e) => {
    if (!mouseOn() || !e.deltaY) return;
    e.preventDefault();
    report(e.deltaY < 0 ? "wheelup" : "wheeldown", cellAt(e.clientX, e.clientY), 0, e);
  }, { passive: false });

  // Touch: a tap is a click, a two-finger tap a right click, a swipe the wheel; a long press
  // selects text as in the terminal and sends nothing.
  let touch = null;
  term.addEventListener("touchstart", (e) => {
    if (!mouseOn()) { touch = null; return; }
    const t = e.touches[0];
    if (e.touches.length === 1) touch = { x: t.clientX, y: t.clientY, at: Date.now(), fingers: 1, moved: false, rest: t.clientY };
    else if (touch) touch.fingers = Math.max(touch.fingers, e.touches.length);
  }, { passive: true });
  term.addEventListener("touchmove", (e) => {
    if (!touch || !mouseOn() || touch.fingers > 1) return;
    const t = e.touches[0];
    if (!touch.moved && Math.hypot(t.clientX - touch.x, t.clientY - touch.y) < 10) return;
    touch.moved = true;
    e.preventDefault();                           // the swipe is the program's: Screen mode has no history
    const step = parseFloat(getComputedStyle(term).lineHeight) || 16;
    const at = cellAt(touch.x, touch.y);
    while (Math.abs(t.clientY - touch.rest) >= step) {
      const up = t.clientY > touch.rest;          // a finger moving down shows what is above
      report(up ? "wheelup" : "wheeldown", at, 0, e);
      touch.rest += up ? step : -step;
    }
  }, { passive: false });
  term.addEventListener("touchend", (e) => {
    if (!touch || e.touches.length || !mouseOn()) return;
    const t = touch; touch = null;
    if (t.moved || Date.now() - t.at > 350) return;   // a swipe, or a long press (a selection)
    const at = cellAt(t.x, t.y), button = t.fingers > 1 ? 2 : 0;
    report("press", at, button, e);
    report("release", at, button, e);
  });

  return {
    on: () => on, mouseOn, mode, setMode, sync,
    /** Fit until this page types: another screen sized the pane. */
    elsewhere: () => !!elsewhere,
    /** A pane was opened: its Screen mode and this view's size go to terminumd. */
    opened() {
      last = null; elsewhere = null; showSized();
      sync();
      send({ t: "screenmode", mode: mode() });
      sendSize(false);
    },
    /** The view or the text size changed. */
    changed,
    /** A key was typed: this client acted (the policy "latest"); a size for another screen ends. */
    typed() {
      if (elsewhere) { elsewhere = null; showSized(); refit(); }
      if (performance.now() - typedAt < TYPED_EVERY) return;
      typedAt = performance.now();
      sendSize(true);
    },
    /** terminumd sized the pane: another screen's size (or a refused resize) is drawn as Fit and said. */
    onSized(m) {
      elsewhere = m.mine && !m.error ? null : { cols: m.cols, rows: m.rows, error: m.error };
      showSized();
      refit();
    },
  };
}
