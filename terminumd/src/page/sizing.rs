// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The pane's size follows the page that shows it (AC-13, AC-05, AC-06). Policy `latest`:
//! the page that typed last (or opened it first) sizes the window; `largest`: the largest
//! page showing it; `manual`: only "Fit to this screen". A window of one pane is resized
//! with `window-size manual` + `resize-window`; a split window only in Screen mode, zoomed.
//! A window no page shows goes back to tmux's own policy. At most one resize a second a page.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::client::{error, Shown};
use super::Page;
use crate::app::Shared;
use crate::screen::frame::Fields;
use crate::tmux::{mouse, Server};

const EVERY: Duration = Duration::from_secs(1);
/// The smallest grid terminumd sizes a pane to (AC-05: smaller is refused by the page).
const MIN: (u16, u16) = (20, 5);

/// Who sized which window: (server, @window) -> its record.
#[derive(Default)]
pub struct Sizes(Mutex<HashMap<(String, String), Win>>);

#[derive(Default, Clone)]
struct Win {
    /// the page whose grid the window has
    by: Option<u64>,
    /// the window's size before "Fit to this screen", restored by "Restore size"
    fitted: Option<(usize, usize)>,
    /// zoomed by terminumd for Screen mode
    zoomed: bool,
    /// window-size manual was set by terminumd
    manual: bool,
    /// the grid terminumd last gave it
    grid: Option<(usize, usize)>,
}

fn policy(app: &Shared) -> String {
    app.looks.settings.read().size.as_str().to_string()
}

/// The shown pane's window and its last fields.
fn shown_window(page: &Page) -> Option<(Arc<Server>, String, Fields)> {
    let st = page.state.lock();
    let s = st.shown.as_ref()?;
    let f = s.ctl.fields.lock().clone()?;
    Some((s.server.clone(), s.pane.clone(), f))
}

/// Pages showing `window` of `server` now.
fn viewers(app: &Shared, server: &Server, window: &str) -> Vec<Arc<Page>> {
    app.pages.all().into_iter().filter(|p| {
        let st = p.state.lock();
        st.shown.as_ref().is_some_and(|s| s.server.name == server.name && s.ctl.fields.lock().as_ref().is_some_and(|f| f.window == window))
    }).collect()
}

/// Resizes the shown pane's window to `page`'s grid, as `page`; tells the pages showing it.
/// At most one resize a second a page: a later one waits for the next second, and while one
/// waits the others leave it to that one, which reads the window, the page's grid and Screen
/// mode anew; a dragged window's passing sizes never reach tmux (each narrowing reflows the
/// pane and leaves a redrawing program's old lines in history).
async fn resize(app: &Shared, page: &Arc<Page>) {
    let wait = {
        let mut st = page.state.lock();
        let now = Instant::now();
        if st.resized_at.is_some_and(|t| t > now) {
            return; // one is waiting: it will do what this one would
        }
        let wait = st.resized_at.map(|t| EVERY.saturating_sub(t.elapsed())).filter(|w| !w.is_zero());
        st.resized_at = Some(now + wait.unwrap_or_default());
        wait
    };
    if let Some(w) = wait {
        tokio::time::sleep(w).await;
    }
    let (view, screen) = {
        let st = page.state.lock();
        (st.view, st.screen)
    };
    let (Some((server, pane, f)), Some(grid)) = (shown_window(page), view) else { return };
    let (cols, rows) = (grid.0.max(MIN.0) as usize, grid.1.max(MIN.1) as usize);
    let key = (server.name.clone(), f.window.clone());
    let split = f.panes > 1;
    if split && !screen {
        return; // a split window keeps its layout; the page reads its pane in Wrap or Fit
    }
    let mut cmds = Vec::new();
    let mut win = app.sizes.0.lock().get(&key).cloned().unwrap_or_default();
    // terminumd's own record too: `resize-pane -Z` toggles, and two calls may run at once
    if split && screen && !f.zoomed && !win.zoomed {
        cmds.push(format!("resize-pane -Z -t {pane}"));
        win.zoomed = true;
    }
    let done = win.by == Some(page.id) && win.grid == Some((cols, rows));
    if (f.width, f.height) != (cols, rows) || (split && !done) {
        cmds.push(format!("set-option -w -t {} window-size manual", f.window));
        cmds.push(format!("resize-window -t {} -x {cols} -y {rows}", f.window));
        win.manual = true;
    }
    win.by = Some(page.id);
    win.grid = Some((cols, rows));
    app.sizes.0.lock().insert(key, win);
    if cmds.is_empty() {
        return;
    }
    for a in server.run_all_on(&pane, cmds).await {
        if let Err(e) = a {
            tracing::warn!(event = "tmux.error", cmd = "resize-window", msg = e);
            page.send_json(&json!({"t": "sized", "sid": shown_key(page), "cols": f.width, "rows": f.height, "mine": false, "error": e}));
            return;
        }
    }
    for p in viewers(app, &server, &f.window) {
        p.send_json(&json!({"t": "sized", "sid": shown_key(&p), "cols": cols, "rows": rows, "mine": p.id == page.id}));
        if let Some(s) = p.state.lock().shown.as_ref() {
            s.ctl.wake.notify_one();
        }
    }
}

fn shown_key(page: &Page) -> String {
    page.state.lock().shown.as_ref().map(|s| s.key.clone()).unwrap_or_default()
}

/// The pane's window should have `page`'s grid now (it opened it first, typed, or sizes it).
async fn claim(app: &Shared, page: &Arc<Page>, force: bool) {
    let Some((server, _, f)) = shown_window(page) else { return };
    let (has_view, screen) = {
        let st = page.state.lock();
        (st.view.is_some(), st.screen)
    };
    if !has_view {
        return;
    }
    let key = (server.name.clone(), f.window.clone());
    let by = app.sizes.0.lock().get(&key).and_then(|w| w.by);
    let p = policy(app);
    let mine = by == Some(page.id);
    let free = by.is_none_or(|id| !viewers(app, &server, &f.window).iter().any(|v| v.id == id));
    let take = screen
        || match p.as_str() {
            "manual" => false,
            "largest" => free || mine || largest(app, &server, &f.window) == Some(page.id),
            _ => force || free || mine,
        };
    if take {
        resize(app, page).await;
    }
}

/// A claim on its own task: a resize that waits for its second never holds up the page's
/// next messages (its keys).
fn later(app: &Shared, page: &Arc<Page>, force: bool) {
    let (app, page) = (app.clone(), page.clone());
    tokio::spawn(async move { claim(&app, &page, force).await });
}

/// The page whose grid is the largest among those showing `window`.
fn largest(app: &Shared, server: &Server, window: &str) -> Option<u64> {
    viewers(app, server, window).into_iter().filter_map(|p| p.state.lock().view.map(|v| (v.0 as u32 * v.1 as u32, p.id))).max().map(|(_, id)| id)
}

/// A page opened a pane: it sizes the window when nobody else does (AC-13).
pub async fn opened(_app: &Shared, page: &Arc<Page>) {
    page.state.lock().screen = false;
}

/// The shown pane's first frame (its fields stored): this page sizes the window when nobody does.
pub fn first_frame(app: &Shared, page: &Arc<Page>, server: &Server, window: &str) {
    if !app.sizes.0.lock().contains_key(&(server.name.clone(), window.to_string())) {
        later(app, page, false);
    }
}

/// The page typed: under `latest` its grid wins (AC-13).
pub async fn typed(app: &Shared, page: &Arc<Page>) {
    let Some((server, _, f)) = shown_window(page) else { return };
    let by = app.sizes.0.lock().get(&(server.name.clone(), f.window.clone())).and_then(|w| w.by);
    if by != Some(page.id) && policy(app) == "latest" {
        later(app, page, true);
    }
}

/// The page stopped showing `old` (another pane, or it closed): a window nobody shows goes
/// back to tmux's policy; a fitted one gets its size back; a zoom made for Screen mode ends.
pub async fn left(app: &Shared, page: &Arc<Page>, old: &Shown) {
    let Some(f) = old.ctl.fields.lock().clone() else { return };
    let key = (old.server.name.clone(), f.window.clone());
    let others = viewers(app, &old.server, &f.window).into_iter().any(|p| p.id != page.id);
    let win = {
        let mut g = app.sizes.0.lock();
        let Some(w) = g.get_mut(&key) else { return };
        if w.by == Some(page.id) {
            w.by = None;
        }
        let w = w.clone();
        if !others {
            g.remove(&key);
        }
        w
    };
    let mut cmds = Vec::new();
    if win.zoomed && !others {
        cmds.push(format!("resize-pane -Z -t {}", old.pane));
    }
    if let Some((c, r)) = win.fitted.filter(|_| !others) {
        cmds.push(format!("resize-window -t {} -x {c} -y {r}", f.window));
    }
    if win.manual && !others {
        cmds.push(format!("set-option -w -u -t {} window-size", f.window));
    }
    if !cmds.is_empty() {
        let _ = old.server.run_all_on(&old.pane, cmds).await;
    }
}

/// `size`, `fit`, `unfit`, `screenmode` and `mouse` from the page.
pub async fn handle(app: &Shared, page: &Arc<Page>, t: &str, msg: &Value) {
    let grid = || (msg["cols"].as_u64().unwrap_or(0).min(1000) as u16, msg["rows"].as_u64().unwrap_or(0).min(1000) as u16);
    match t {
        "size" => {
            let (c, r) = grid();
            if c == 0 || r == 0 {
                return;
            }
            let screen = msg["screen"].as_bool().unwrap_or(false);
            let was_screen = {
                let mut st = page.state.lock();
                st.view = Some((c, r));
                std::mem::replace(&mut st.screen, screen)
            };
            if was_screen && !screen {
                unzoom(app, page).await;
            }
            later(app, page, msg["typing"].as_bool().unwrap_or(false));
        }
        "fit" => {
            let (c, r) = grid();
            let Some((server, _, f)) = shown_window(page) else { return };
            app.sizes.0.lock().entry((server.name.clone(), f.window.clone())).or_default().fitted.get_or_insert((f.width, f.height));
            page.state.lock().view = Some((c, r));
            resize(app, page).await;
            page.send_json(&json!({"t": "fit", "on": true}));
        }
        "unfit" => {
            if let Some((server, pane, f)) = shown_window(page) {
                let fitted = app.sizes.0.lock().get_mut(&(server.name.clone(), f.window.clone())).and_then(|w| w.fitted.take());
                if let Some((c, r)) = fitted {
                    let _ = server.run_all_on(&pane, vec![format!("resize-window -t {} -x {c} -y {r}", f.window)]).await;
                }
            }
            page.send_json(&json!({"t": "fit", "on": false}));
        }
        "screenmode" => {
            let mode = msg["mode"].as_str().filter(|m| matches!(*m, "auto" | "always" | "never")).unwrap_or("auto").to_string();
            let mut st = page.state.lock();
            if let Some(key) = st.shown.as_ref().map(|s| s.key.clone()) {
                st.screenmode.insert(key, mode);
            }
        }
        "mouse" => mouse_event(page, msg).await,
        _ => {}
    }
}

/// Screen mode ended: a zoom terminumd made for it ends too.
async fn unzoom(app: &Shared, page: &Arc<Page>) {
    let Some((server, pane, f)) = shown_window(page) else { return };
    let zoomed = app.sizes.0.lock().get_mut(&(server.name.clone(), f.window.clone())).is_some_and(|w| std::mem::take(&mut w.zoomed));
    if zoomed && f.zoomed {
        let _ = server.run_all_on(&pane, vec![format!("resize-pane -Z -t {pane}")]).await;
    }
}

/// A click, drag or wheel in Screen mode, as the mouse report a terminal would send (AC-06).
async fn mouse_event(page: &Arc<Page>, msg: &Value) {
    let Some((server, pane, f)) = shown_window(page) else { return };
    if !f.mouse {
        return; // the program did not ask for the mouse: nothing is sent
    }
    let Some(bytes) = from_message(msg, f.mouse_sgr) else { return };
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let line = format!("send-keys -t {pane} -H {}", hex.join(" "));
    if let Some(Err(e)) = server.run_all_on(&pane, vec![line]).await.into_iter().next() {
        error(page, format!("Mouse event not sent: {e}"), None);
    }
}

/// The page's `mouse` message as the bytes of a report.
fn from_message(msg: &Value, sgr: bool) -> Option<Vec<u8>> {
    let (button, action) = mouse::from_page(msg["button"].as_u64().unwrap_or(0).min(255) as u8, msg["action"].as_str().unwrap_or(""))?;
    let m = &msg["mods"];
    let mods = mouse::Mods { shift: m["shift"].as_bool().unwrap_or(false), meta: m["alt"].as_bool().unwrap_or(false), ctrl: m["ctrl"].as_bool().unwrap_or(false) };
    let coord = |k: &str| msg[k].as_u64().filter(|v| *v < 10_000).map(|v| v as u16);
    mouse::report(coord("x")?, coord("y")?, button, action, mods, sgr)
}
