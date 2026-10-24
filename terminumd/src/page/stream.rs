// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A shown pane's frames (AC-02): `%output` wakes the task, which captures the screen at
//! once when the pane was idle, else at the next 33 ms tick, and sends only the rows that
//! changed; rows that scrolled off are appended to the page's history with their numbers.
//! A pane nobody shows is never captured.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::json;
use tokio::sync::Notify;

use super::{Page, CLOSED};
use crate::app::Shared;
use crate::screen::frame::{Fields, Shown, FIELDS};
use crate::screen::history::{Numbering, Step};
use crate::screen::rows;
use crate::tmux::Server;

/// Frames at most 30 a second (AC-02).
pub const FRAME_GAP: Duration = Duration::from_millis(33);
/// A shown pane is also looked at this often: a safety net, %output wakes it (AS-06 holds).
const POLL: Duration = Duration::from_secs(1);
/// History rows sent when a pane opens; older ones load on scroll (AC-02).
pub const FIRST_HISTORY: i64 = 1000;

/// What the page's other messages share with its stream.
#[derive(Default)]
pub struct Ctl {
    /// capture now (a key was typed, a resize was made)
    pub wake: Notify,
    paused: AtomicBool,
    resumed: Notify,
    /// the numbering and the last fields, for `more`, `files` and sizing
    pub numbering: Mutex<Numbering>,
    pub fields: Mutex<Option<Fields>>,
    /// the page's history as folds see it (AC-32)
    pub folds: Mutex<Option<super::folds::History>>,
}

impl Ctl {
    pub fn pause(&self, on: bool) {
        self.paused.store(on, Ordering::Relaxed);
        if !on {
            self.resumed.notify_one();
        }
    }
}

pub async fn run(app: Shared, page: Arc<Page>, server: Arc<Server>, pane: String, key: String, ctl: Arc<Ctl>) {
    let (name, theme) = crate::looks::theme_of(&app, &server, &pane).await;
    page.tracer.lock().opening(&key, !server.is_local());
    page.send_json(&json!({"t": "theme", "sid": key, "theme": theme, "name": name}));
    let wake = server.wake(&pane);
    let mut shown = Shown::default();
    // the last frame's rows, cursor columns and encodings: an unchanged row is not encoded again
    let mut encodings: Vec<(rows::Row, Option<usize>, String)> = Vec::new();
    let mut last_folds: Option<(Vec<String>, Option<usize>, bool, serde_json::Value)> = None;
    let mut agent_dir = app.hooks.cwd(&key);   // the folder its coding agent works in, as last told (AC-51)
    let mut generation = u64::MAX;
    let mut said_lost = false;
    let mut server_pid: Option<u32> = None;
    loop {
        if ctl.paused.load(Ordering::Relaxed) {
            ctl.resumed.notified().await;
            continue;
        }
        let Some(client) = server.client(&pane) else {
            if !said_lost {
                let msg = if server.is_local() { "tmux on this Mac is not connected".to_string() } else { format!("Connection to {} lost", server.name) };
                page.send_json(&json!({"t": "error", "msg": msg, "sid": key}));
                said_lost = true;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        };
        if said_lost {
            page.send_json(&json!({"t": "error", "msg": "", "sid": key}));
            said_lost = false;
        }
        let g = server.generation.load(Ordering::Relaxed);
        if g != generation {
            // new control clients (a restart, a reconnect): everything is sent anew
            generation = g;
            ctl.numbering.lock().forget();
            shown.clear();
        }
        let started = Instant::now();
        let prev_cursor = ctl.fields.lock().as_ref().map(|f| (f.cx, f.cy));
        let answers = client.run_all(vec![format!("display -p -t {pane} '{FIELDS}'"), format!("capture-pane -p -e -J -t {pane}")]).await;
        let (display, capture) = (&answers[0], &answers[1]);
        let (Ok(display), Ok(capture)) = (display, capture) else {
            let e = display.as_ref().err().or(capture.as_ref().err()).cloned().unwrap_or_default();
            if e.contains("can't find") || e.contains("no such") {
                page.send_json(&json!({"t": "error", "msg": CLOSED, "sid": key}));
                tracing::info!(event = "pane.closed", key, error = e);
                return;
            }
            tracing::warn!(event = "tmux.error", server = %server.name, cmd = "capture-pane", msg = e);
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        };
        let Some(f) = display.first().and_then(|l| Fields::parse(l)) else {
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        };
        if *server_pid.get_or_insert(f.server_pid) != f.server_pid {
            // the server was started anew: this pane id names another pane now (AC-14)
            page.send_json(&json!({"t": "error", "msg": CLOSED, "sid": key}));
            tracing::info!(event = "pane.closed", key, why = "tmux server restarted");
            return;
        }
        let opened = ctl.fields.lock().is_none();
        if opened {
            // a pane just opened: the active page's grid first, then its first frame at it, never
            // one at the window's old size (AC-04, AC-13)
            *ctl.fields.lock() = Some(f.clone());
            if super::sizing::opening(&app, &page).await {
                continue;
            }
        }
        super::timing::captured(&page, started);
        let screen = rows::screen_rows(&capture.join("\n"), f.width, f.height);
        let texts: Vec<String> = screen.iter().map(rows::row_text).collect();
        let step = ctl.numbering.lock().step(f.history, f.width, &texts, f.alt, f.limit);
        match step {
            Step::Reset => {
                shown.clear();
                super::hist::reset(&app, &page, &client, &ctl, &key, &pane, &f).await;
            }
            Step::Append { from, to } => super::hist::append(&app, &page, &client, &ctl, &key, &pane, &f, from, to).await,
            Step::Same => {}
        }
        let cursor = (f.cursor_on && f.cy < screen.len()).then_some(f.cy);
        // a full-screen program's rows never fold (a coding agent's own alternate screen is an
        // agent's); worked out again only when the screen changed
        let hard: Vec<bool> = screen.iter().map(|r| r.hard).collect();
        let full_screen = f.alt && crate::tmux::titles::agent_of(&f.command).is_none();
        if last_folds.as_ref().is_none_or(|(t, c, a, _)| *t != texts || *c != cursor || *a != full_screen) {
            last_folds = Some((texts.clone(), cursor, full_screen, super::folds::on_screen(&texts, &hard, cursor, full_screen)));
        }
        let folds = last_folds.as_ref().map(|l| l.3.clone()).unwrap_or_default();
        let cols: Vec<Option<usize>> = (0..screen.len()).map(|i| (cursor == Some(i)).then_some(f.cx)).collect();
        let encoded: Vec<String> = screen.iter().zip(&cols).enumerate().map(|(i, (r, &c))| match encodings.get(i) {
            Some((was, at, e)) if was == r && *at == c => e.clone(),
            _ => rows::encode_str(r, c),
        }).collect();
        encodings = screen.into_iter().zip(cols).zip(encoded.iter().cloned()).map(|((r, c), e)| (r, c, e)).collect();
        let moved = prev_cursor != Some((f.cx, f.cy)) || shown.changed(f.cy, &encoded);
        let extra = super::timing::frame(&page, moved);
        if let Some(msg) = shown.frame(&key, &f, encoded, folds, extra)
            && !page.send(msg) {
                return;
            }
        // the pane's folder changed (a cd, by Enter, a hot key or a script), or its agent's: Files follows it
        let agent_now = app.hooks.cwd(&key);
        let agent_moved = agent_now != agent_dir;
        agent_dir = agent_now;
        let moved_to = ctl.fields.lock().as_ref().filter(|old| old.path != f.path || agent_moved).map(|_| f.path.clone());
        *ctl.fields.lock() = Some(f);
        if let Some(cwd) = moved_to {
            page.send_json(&super::client::files_message(&app, &server, &key, &cwd).await);
        }
        if opened {
            super::sizing::activated(&app, &page);       // its grid came after "sub" (or not yet): sized as it comes
        }
        let below: Vec<(&str, bool)> = if full_screen { Vec::new() } else { texts.iter().map(String::as_str).zip(hard.iter().copied()).collect() };
        super::hist::scan_folds(&page, &ctl, &key, &below);
        let last = Instant::now();
        // wait for output, a key, or the poll; then keep frames at most 30 a second
        let how = tokio::select! {
            _ = wake.notified() => "output",
            _ = ctl.wake.notified() => "key",
            _ = tokio::time::sleep(POLL) => "poll",
        };
        super::timing::woken(&page, how);
        let since = last.elapsed();
        if since < FRAME_GAP {
            tokio::time::sleep(FRAME_GAP - since).await;
        }
    }
}
