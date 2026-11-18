// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The pane's size follows the active page that shows it (AC-13, AC-06): a page sizes the
//! window on opening the pane (before its first frame, at the grid sent with `sub`: AC-04), on
//! becoming active (in sight and focused: `resume`) and on typing; a change of its view resizes only a window it sizes or nobody does; a hidden page
//! sizes nothing and hands its windows over. Where a coding agent draws on the normal screen only typing sizes the window (`redraws`). The user's own tmux too (AC-89), marked. A
//! window of one pane, or whose shown pane is zoomed, is resized with
//! `window-size manual` + `resize-window`. In a window of several panes terminumd zooms the
//! shown pane first, and only typing makes the window smaller: its hidden panes share what is
//! left; the user's own tmux or a client attached by hand keeps its layout, unzoomed and unsized.
//! A window no page shows goes back to tmux's own policy. At most one resize a second a page.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::client::Shown;
use super::zoom::{may_take, zoom_line, zoomable};
use super::Page;
use crate::app::Shared;
use crate::screen::frame::Fields;
use crate::tmux::Server;

const EVERY: Duration = Duration::from_secs(1);
/// The grids a pane is sized to, whatever the screen (AC-13): the smallest fits an iPhone in
/// portrait at the default 13 px (a smaller view re-flows it, AC-04), the largest a
/// big desktop window, about 1920×1200 (a larger view leaves the rest blank).
const MIN: (u16, u16) = (45, 14);
const MAX: (u16, u16) = (240, 70);

/// A page's grid within MIN and MAX.
fn bounded(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(MIN.0, MAX.0), rows.clamp(MIN.1, MAX.1))
}

/// Who sized which window: (server id, @window) -> its record.
#[derive(Default)]
pub struct Sizes(Mutex<HashMap<(String, String), Win>>);

#[derive(Default, Clone, Debug, PartialEq)]
struct Win {
    /// the page whose grid the window has
    by: Option<u64>,
    /// the page whose shown pane terminumd zoomed
    zoom_by: Option<u64>,
    /// window-size manual was set by terminumd
    manual: bool,
    /// the grid terminumd last asked tmux for (`holds`)
    asked: Option<(usize, usize)>,
    /// the claim that wrote this record (`resize`): a refused one is undone only while it is the last
    claim: u64,
}

impl Win {
    /// Where the window ends: the grid terminumd last asked for, else tmux's last-read size (`now`, the
    /// pane's fields), which a resize still on its way to tmux leaves behind (AC-13).
    fn ends_at(&self, now: (usize, usize)) -> (usize, usize) {
        self.asked.unwrap_or(now)
    }

    /// A looking page left the window as it is (`claim`): once `page`'s view is no longer the
    /// grid the window ends at, the window is not that page's, and its next key claims at once.
    fn left_as_is(&mut self, page: u64, view: Option<(usize, usize)>, now: (usize, usize)) {
        if self.by == Some(page) && view != Some(self.ends_at(now)) {
            self.by = None;
        }
    }

    /// Whether the window is at `grid` as terminumd sized it: tmux's last-read size and the grid
    /// terminumd last asked for agree (asked only with `window-size manual` set).
    fn holds(&self, now: (usize, usize), grid: (usize, usize)) -> bool {
        now == grid && self.asked == Some(grid)
    }
}

impl Sizes {
    /// Records `win` for `key` as a new claim: (the record before, the claim).
    fn record(g: &mut HashMap<(String, String), Win>, key: &(String, String), mut win: Win) -> (Option<Win>, u64) {
        static CLAIMS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        win.claim = CLAIMS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let claim = win.claim;
        (g.insert(key.clone(), win), claim)
    }

    /// tmux refused `claim` on `key`: the record is what it was before, unless a later claim
    /// replaced it meanwhile; a `window-size manual` tmux applied (`manual`) stays known, so the
    /// window still gets tmux's own size back (`left`, `release`).
    fn undo(&self, key: &(String, String), claim: u64, before: Option<Win>, manual: bool) {
        let mut g = self.0.lock();
        if g.get(key).map(|w| w.claim) != Some(claim) {
            return;
        }
        let back = before.map(|w| Win { manual: w.manual || manual, ..w }).or_else(|| manual.then(|| Win { manual: true, ..Win::default() }));
        match back {
            Some(w) => g.insert(key.clone(), w),
            None => g.remove(key),
        };
    }
}

/// Why a page claims its pane's window.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Why {
    /// its view changed size: only a window it sizes or nobody does
    View,
    /// it became the active page
    Active,
    /// it opened the pane: before its first frame, never kept waiting for the window's second
    Open,
    /// it typed
    Typed,
}

/// The shown pane's window and its last fields.
fn shown_window(page: &Page) -> Option<(Arc<Server>, String, Fields)> {
    let st = page.state.lock();
    let s = st.shown.as_ref()?;
    let f = s.ctl.fields.lock().clone()?;
    Some((s.server.clone(), s.pane.clone(), f))
}

/// Whether `pane` is in a session of the user's own tmux, attached by name (AC-89).
pub(super) fn users_pane(server: &Server, pane: &str) -> bool {
    server.is_users() || server.session_of(pane).is_some_and(|s| server.users(&s))
}

/// Pages showing `window` of `server` now.
fn viewers(app: &Shared, server: &Server, window: &str) -> Vec<Arc<Page>> {
    app.pages.all().into_iter().filter(|p| {
        let st = p.state.lock();
        st.shown.as_ref().is_some_and(|s| s.server.id() == server.id() && s.ctl.fields.lock().as_ref().is_some_and(|f| f.window == window))
    }).collect()
}

/// The pages in sight (active) other than `except` showing `window` of `server` now.
fn in_sight(app: &Shared, server: &Server, window: &str, except: u64) -> Vec<Arc<Page>> {
    viewers(app, server, window).into_iter().filter(|v| v.id != except && v.state.lock().active()).collect()
}

/// Resizes the shown pane's window to `page`'s grid, as `page`; tells the pages showing it.
/// A window at most once a second a page: a later resize of it waits for the next second, and
/// while one waits the others leave it to that one, which reads the window and the page's grid
/// anew; a dragged window's passing sizes never reach tmux (each narrowing reflows the pane and
/// leaves a redrawing program's old lines in history). Another window at once, and a pane's
/// opening at once: switching panes waits for nothing (AC-04).
async fn resize(app: &Shared, page: &Arc<Page>, why: Why) -> bool {
    let Some(window) = shown_window(page).map(|(s, _, f)| (s.id(), f.window)) else { return false };
    let wait = {
        let mut st = page.state.lock();
        let now = Instant::now();
        let waiting = st.resized_at.as_mut().filter(|(w, _, _)| why != Why::Open && *w == window);
        let last = waiting.as_ref().map(|(_, t, _)| *t);
        if last.is_some_and(|t| t > now) {
            if let Some((_, _, typed)) = waiting {
                *typed |= why == Why::Typed;
            }
            return false; // one is waiting: it will do what this one would
        }
        let wait = last.map(|t| EVERY.saturating_sub(t.elapsed())).filter(|w| !w.is_zero());
        st.resized_at = Some((window, now + wait.unwrap_or_default(), false));
        wait
    };
    if let Some(w) = wait {
        tokio::time::sleep(w).await;
    }
    let (view, inactive, why) = {
        let st = page.state.lock();
        let typed = st.resized_at.as_ref().is_some_and(|(_, _, typed)| *typed);
        (st.view, !st.active(), if typed { Why::Typed } else { why })
    };
    if inactive {
        return false; // hidden while it waited: a hidden page sizes nothing
    }
    let (Some((server, pane, f)), Some(grid)) = (shown_window(page), view) else { return false };
    let grid = (grid.0 as usize, grid.1 as usize);
    let key = (server.id(), f.window.clone());
    let mut cmds = Vec::new();
    if !f.whole() {
        if !zoomable(&server, &pane, &f.window).await {
            return false; // its layout is the user's; the page re-flows its pane (AC-04, AC-13)
        }
        cmds.push(zoom_line(&pane, f.zoomed));
    }
    let users = users_pane(&server, &pane);
    let redraws = redraws(app, &server.key(&pane), &f);
    // read and written under one lock: a claim of another page meanwhile is never overwritten by a stale copy
    let (before, claim, manual_at, (cols, rows)) = {
        let mut g = app.sizes.0.lock();
        let mut win = g.get(&key).cloned().unwrap_or_default();
        let (cols, rows) = target(grid, &f, win.ends_at(f.window_size), why, redraws);
        let mut manual_at = None;
        if !win.holds(f.window_size, (cols, rows)) {
            // also at its size already: held by tmux's own policy, another client would take it
            manual_at = Some(cmds.len());
            cmds.push(format!("set-option -w -t {} window-size manual", f.window));
            cmds.push(format!("resize-window -t {} -x {cols} -y {rows}", f.window));
            if users {
                // marked: a crash or a dropped link still gives it its size back (`reclaim`)
                cmds.push(format!("set-option -w -t {} {SIZED} 1", f.window));
            }
            win.manual = true;
        }
        win.asked = Some((cols, rows));
        if (cols, rows) == grid {
            win.by = Some(page.id);
        }
        if !f.whole() {
            win.zoom_by = Some(page.id);
        }
        if !page.state.lock().active() {
            return false; // hidden meanwhile (pause is set before handover): its handover saw nothing to give
        }
        let (before, claim) = Sizes::record(&mut g, &key, win);
        (before, claim, manual_at, (cols, rows))
    };
    if cmds.is_empty() {
        return false;
    }
    let resized = server.key(&pane);   // the user may show another pane by the time tmux answers
    let answers = server.run_all_on(&pane, cmds).await;
    if let Some(e) = answers.iter().find_map(|a| a.as_ref().err()) {
        // the claim is undone: the window is not this page's size (AC-13 failure)
        let manual = manual_at.is_some_and(|i| answers.get(i).is_some_and(Result::is_ok));
        app.sizes.undo(&key, claim, before, manual);
        refused(page, &resized, &f, "resize-window", e.clone());
        return false;
    }
    if (cols, rows) != f.window_size {
        // who changed which window: a program that redraws shows each change in its history
        tracing::info!(event = "sizing.resize", page = page.id, web = page.web, pane = %resized, from = %format!("{}x{}", f.window_size.0, f.window_size.1), to = %format!("{cols}x{rows}"), why = ?why);
    }
    for p in viewers(app, &server, &f.window) {
        p.send_json(&json!({"t": "sized", "sid": shown_key(&p), "cols": cols, "rows": rows, "mine": p.id == page.id}));
        if let Some(s) = p.state.lock().shown.as_ref() {
            s.ctl.wake.notify_one();
        }
    }
    if !page.state.lock().active() {
        handover(app, page);                   // hidden while it resized: its handover came first and saw nothing to give
    }
    true
}

/// tmux refused to size or zoom the pane `sid`: said where it shows, and in the log.
pub(super) fn refused(page: &Page, sid: &str, f: &Fields, cmd: &str, e: String) {
    tracing::warn!(event = "tmux.error", cmd, msg = e);
    page.send_json(&json!({"t": "sized", "sid": sid, "cols": f.width, "rows": f.height, "mine": false, "error": e}));
}

/// `page`'s stream takes `window`'s zoom, unless another page in sight keeps it (AC-13).
pub(super) fn take_zoom(app: &Shared, page: &Page, server: &Server, window: &str) -> bool {
    let mut g = app.sizes.0.lock();
    let look = page.state.lock().look;   // under the lock: pause is set before a handover (`resize`)
    let win = g.entry((server.id(), window.to_string())).or_default();
    let holder = win.zoom_by.filter(|&id| id != page.id).and_then(|id| viewers(app, server, window).into_iter().find(|v| v.id == id)).map(|v| v.state.lock().look);
    let yes = may_take(look, holder);
    if yes {
        win.zoom_by = Some(page.id);
    }
    yes
}

/// The window option marking a window of the user's that terminumd sized (AC-89).
const SIZED: &str = "@tm-sized";

fn shown_key(page: &Page) -> String {
    page.state.lock().shown.as_ref().map(|s| s.key.clone()).unwrap_or_default()
}

/// The pane's window gets `page`'s grid when `why` allows (AC-13): a hidden page claims
/// nothing; a change of the view only a window this page sizes or nobody (looking) does; the
/// user's own tmux as any (AC-89).
async fn claim(app: &Shared, page: &Arc<Page>, why: Why) -> bool {
    let Some((server, pane, f)) = shown_window(page) else { return false };
    let (view, inactive) = {
        let st = page.state.lock();
        (st.view.map(|(c, r)| (c as usize, r as usize)), !st.active())
    };
    if view.is_none() || inactive {
        return false;
    }
    let key = (server.id(), f.window.clone());
    if why != Why::Typed && f.whole() && redraws(app, &server.key(&pane), &f) {
        // left as it is: nothing sent, and the page's once-a-second slot stays free for its next key
        if let Some(w) = app.sizes.0.lock().get_mut(&key) {
            w.left_as_is(page.id, view, f.window_size);
        }
        return false;
    }
    let win = app.sizes.0.lock().get(&key).cloned().unwrap_or_default();
    let mine = win.by == Some(page.id);
    // free: nobody sizes it, or the page that does no longer shows it or is hidden (a locked phone, a tab in the background)
    let free = win.by.is_none_or(|id| !viewers(app, &server, &f.window).iter().any(|v| v.id == id && v.state.lock().active()));
    if mine && f.whole() && view.is_some_and(|v| win.holds(f.window_size, v)) {
        return false;                                         // it has this page's grid already: the once-a-second slot stays free
    }
    (why != Why::View || free || mine) && resize(app, page, why).await
}

/// Whether a coding agent (the list's, for the pane `key`) draws on the pane's normal screen. On a
/// new size it prints its transcript again from the top of the screen and leaves the scrollback
/// as it is, while tmux's re-wrap has moved rows of the old picture there: every change of the
/// window's size leaves copies, or loses rows, in the pane's history (AC-13).
fn redraws(app: &Shared, key: &str, f: &Fields) -> bool {
    !f.alt && super::marks::find_item(&app.layout.read(), key).is_some_and(|it| it["agent"].is_string())
}

/// The grid the window gets for `grid`; `now`: where the window ends (`Win::ends_at`). Typing
/// gives it the page's grid. A page that only looks leaves a window whose pane a coding agent
/// redraws (`redraws`) as it is, and the page re-flows the pane (AC-04); a window of several
/// panes grows for it and never shrinks: a phone that just looks would squeeze the panes the
/// zoom hides (AC-13).
fn target(grid: (usize, usize), f: &Fields, now: (usize, usize), why: Why, redraws: bool) -> (usize, usize) {
    if why == Why::Typed {
        grid
    } else if redraws {
        now
    } else if f.panes > 1 {
        (grid.0.max(now.0), grid.1.max(now.1))
    } else {
        grid
    }
}

/// A claim on its own task: a resize that waits for its second never holds up the page's
/// next messages (its keys).
fn later(app: &Shared, page: &Arc<Page>, why: Why) {
    let (app, page) = (app.clone(), page.clone());
    tokio::spawn(async move { claim(&app, &page, why).await });
}

/// `page` was hidden: a window it sized goes to the pages still looking at it (AC-13), so a
/// locked phone never keeps a computer's view at the phone's size; one of the user's own tmux
/// that no page looks at gets tmux's own size back (AC-89), whoever sized it: the page that
/// did may have opened another pane while this one was still in sight (`left` kept it then).
pub fn handover(app: &Shared, page: &Arc<Page>) {
    let Some((server, pane, f)) = shown_window(page) else { return };
    let key = (server.id(), f.window.clone());
    // the record and the pages in sight read under one lock, as `left` does: of a hide and
    // another pane opened at once, the later sees what the earlier did
    let (mine, others, release) = {
        let mut g = app.sizes.0.lock();
        let Some(w) = g.get(&key) else { return };
        let mine = w.by == Some(page.id);
        let others = in_sight(app, &server, &f.window, page.id);
        let mut release = false;
        if others.is_empty() && users_pane(&server, &pane) {
            release = g.remove(&key).is_some_and(|w| w.manual);
        }
        (mine, others, release)
    };
    if release {
        // sent now, in order: the page's next message (its resume) comes after it, and waits for nothing
        server.send_on(&pane, own_size(&f.window));
    }
    if mine {
        for v in others {
            later(app, &v, Why::Active);
        }
    }
}

/// `page` became the active page (in sight and focused), or showed the first frame of a pane it
/// opened without sizing it first (no grid with "sub"): it sizes its pane's window (AC-13, the
/// user's own tmux too, AC-89); with the grid it sizes it before that frame (`opening`).
pub fn activated(app: &Shared, page: &Arc<Page>) {
    later(app, page, Why::Active);
}

/// The page opened a pane (its fields just read, no frame sent yet): it sizes the pane's window
/// first, so the first frame comes at its grid (AC-04, AC-13); true when the window was resized.
pub async fn opening(app: &Shared, page: &Arc<Page>) -> bool {
    claim(app, page, Why::Open).await
}

/// The page's grid sent with a message (`size`, `sub`): its view within MIN and MAX.
pub fn view_of(page: &Page, msg: &Value) -> bool {
    let (c, r) = (msg["cols"].as_u64().unwrap_or(0).min(1000) as u16, msg["rows"].as_u64().unwrap_or(0).min(1000) as u16);
    if c == 0 || r == 0 {
        return false;
    }
    page.state.lock().view = Some(bounded(c, r));
    true
}

/// The page typed: its grid wins (AC-13).
pub async fn typed(app: &Shared, page: &Arc<Page>) {
    let Some((server, _, f)) = shown_window(page) else { return };
    let by = app.sizes.0.lock().get(&(server.id(), f.window.clone())).and_then(|w| w.by);
    if by != Some(page.id) {
        later(app, page, Why::Typed);
    }
}

/// The page stopped showing `old` (another pane, or it closed): a window no page in sight shows
/// goes back to tmux's policy (a hidden page showing it sizes it again when back, AC-89).
pub fn left(app: &Shared, page: &Arc<Page>, old: &Shown) {
    let Some(f) = old.ctl.fields.lock().clone() else { return };
    let key = (old.server.id(), f.window.clone());
    let release = {
        let mut g = app.sizes.0.lock();
        let Some(w) = g.get_mut(&key) else { return };
        if w.by == Some(page.id) {
            w.by = None;
        }
        // read under the lock, as `handover` does: a page hidden meanwhile sees this record after
        if !in_sight(app, &old.server, &f.window, page.id).is_empty() {
            return;
        }
        g.remove(&key).is_some_and(|w| w.manual)
    };
    if release {
        old.server.send_on(&old.pane, own_size(&f.window));   // sent now, in order; nothing waits for that server (AC-132)
    }
}

/// The commands that give `window` tmux's own size back.
fn own_size(window: &str) -> Vec<String> {
    vec![format!("set-option -w -u -t {window} window-size"), format!("set-option -w -u -t {window} {SIZED}")]
}

/// `size` and `mouse` from the page.
pub fn handle(app: &Shared, page: &Arc<Page>, t: &str, msg: &Value) {
    match t {
        "size" => {
            if view_of(page, msg) {
                later(app, page, if msg["typing"].as_bool().unwrap_or(false) { Why::Typed } else { Why::View });
            }
        }
        _ => super::pointer::event(app, page, msg),
    }
}

/// terminumd lets go of windows of the user's own tmux (a session detached, the server dropped,
/// terminumd stopping, AC-89): those it gave `window-size manual` go back to tmux's own size,
/// so the user's terminal is never left at a browser's size. `session`: only its windows.
pub async fn release(app: &Shared, server: &Server, session: Option<&str>) {
    let windows: Option<Vec<String>> = match session {
        Some(s) => match server.run(format!("list-windows -t {} -F '#{{window_id}}'", crate::tmux::args::quote(&format!("={s}")))).await {
            Ok(w) => Some(w),
            Err(_) => return,
        },
        None => None,
    };
    let id = server.id();
    let mine: Vec<String> = {
        let mut g = app.sizes.0.lock();
        let keys: Vec<(String, String)> = g.keys().filter(|(s, w)| *s == id && windows.as_ref().is_none_or(|ws| ws.contains(w))).cloned().collect();
        keys.into_iter().filter_map(|k| g.remove(&k).filter(|w| w.manual).map(|_| k.1)).collect()
    };
    for w in mine {
        let _ = server.run_all(own_size(&w)).await;
    }
}

/// `reclaim` once per connection (generation) of a server holding the user's sessions.
pub fn reclaim_once(app: &Shared, server: &Arc<Server>) {
    let has = server.is_users() || server.only.lock().as_ref().is_some_and(|o| !o.is_empty());
    if !has || !server.connected() || !crate::tmux::first_on_link("reclaim", server) {
        return;
    }
    let (app, server) = (app.clone(), server.clone());
    tokio::spawn(async move { reclaim(&app, &server).await });
}

/// The user's windows terminumd sized (marked `@tm-sized`) that no page sizes now get tmux's
/// own size back: after terminumd or a host's link ended without `release` (AC-89). Once per
/// connection of a server that has the user's sessions.
pub async fn reclaim(app: &Shared, server: &Server) {
    let Ok(lines) = server.run(format!("list-windows -a -F '#{{window_id}}\t#{{session_name}}\t#{{{SIZED}}}'")).await else { return };
    let id = server.id();
    let held: Vec<String> = app.sizes.0.lock().keys().filter(|(s, _)| *s == id).map(|(_, w)| w.clone()).collect();
    let mut cmds = Vec::new();
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        if let [w, session, "1"] = f[..] && (server.is_users() || server.users(session)) && !held.iter().any(|h| h == w) {
            cmds.extend(own_size(w));
        }
    }
    if !cmds.is_empty() {
        tracing::info!(event = "sizing.reclaim", server = %id, windows = cmds.len() / 2);
        let _ = server.run_all(cmds).await;
    }
}

#[cfg(test)]
#[path = "sizing_tests.rs"]
mod tests;
