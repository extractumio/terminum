// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A shown pane's history on the page (AC-02, AC-32): the rows sent when it opens (reset),
//! the rows that scroll off (append), older pages on scroll (`more`), and folded rows the
//! page asks to see (`unfold`). Rows come from `capture-pane -S … -E …` ranges, numbered
//! absolutely (screen::history); folded rows are sent as null until asked for.

use std::sync::Arc;

use serde_json::{json, Value};

use super::stream::{Ctl, FIRST_HISTORY};
use super::find::shown;
use super::Page;
use crate::app::Shared;
use crate::screen::frame::Fields;
use crate::screen::rows;
use crate::tmux::control::Control;

/// Rows per `more` (AC-02).
const PAGE: i64 = 1000;
/// Rows one `unfold` may ask for (AC-32).
const UNFOLD_MAX: i64 = 20_000;

/// Rows `from..to` (absolute) of `pane` whose screen top is `top`, in the page format;
/// exactly `to - from` of them (tmux joined rows of a range never reach past its end).
pub async fn capture(client: &Control, pane: &str, top: i64, from: i64, to: i64, width: usize) -> Result<Vec<Value>, String> {
    if from >= to {
        return Ok(Vec::new());
    }
    let (s, e) = (from - top, to - top - 1);
    let lines = client.run(format!("capture-pane -p -e -J -t {pane} -S {s} -E {e}")).await?;
    let mut out: Vec<Value> = rows::capture_rows(&lines.join("\n"), width).iter().map(|r| rows::encode(r, None)).collect();
    let want = (to - from) as usize;
    out.truncate(want);
    while out.len() < want {
        out.push(json!({"r": [], "e": true}));
    }
    Ok(out)
}

/// Seconds since terminumd started: the folds' clock.
fn now() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}

fn keep(app: &Shared) -> i64 {
    app.cfg.history
}

/// The page's history anew: the last rows before the screen, then (the caller) a full frame.
pub async fn reset(app: &Shared, page: &Page, client: &Control, ctl: &Ctl, key: &str, pane: &str, f: &Fields) {
    let began = std::time::Instant::now();
    let (top, oldest, truncated) = {
        let n = ctl.numbering.lock();
        (n.top().unwrap_or(0), n.oldest(keep(app)), n.truncated(keep(app)))
    };
    let first = oldest.max(top - FIRST_HISTORY);
    let lines = capture(client, pane, top, first, top, f.width).await.unwrap_or_default();
    // folds are terminumd's (AC-32) unless settings.json says "page": {"folds": false}
    let (lines, items) = if app.looks.settings.read().page.folds {
        let mut folds = super::folds::History::new(keep(app) as usize);
        let taken = folds.take(first, lines, true);
        *ctl.folds.lock() = Some(folds);
        taken
    } else {
        *ctl.folds.lock() = None;
        (lines, Vec::new())
    };
    let mut m = json!({"t": "hist", "mode": "reset", "sid": key, "first": first, "oldest": oldest, "truncated": truncated, "lines": lines});
    if !items.is_empty() {
        m["folds"] = json!(items);
    }
    page.tracer.lock().hist("reset", m["lines"].as_array().map_or(0, Vec::len), began);
    page.send_json(&m);
}

/// Rows `from..to` scrolled off the screen: appended to the page's history.
#[allow(clippy::too_many_arguments)]
pub async fn append(app: &Shared, page: &Page, client: &Control, ctl: &Ctl, key: &str, pane: &str, f: &Fields, from: i64, to: i64) {
    let (top, oldest, truncated) = {
        let n = ctl.numbering.lock();
        (n.top().unwrap_or(0), n.oldest(keep(app)), n.truncated(keep(app)))
    };
    let from = from.max(oldest);
    let began = std::time::Instant::now();
    let lines = match capture(client, pane, top, from, to, f.width).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(event = "tmux.error", cmd = "capture-pane", msg = e);
            return;
        }
    };
    let lines = match ctl.folds.lock().as_mut() {
        Some(h) => h.take(from, lines, false).0,
        None => lines,
    };
    page.tracer.lock().hist("append", lines.len(), began);
    page.send_json(&json!({"t": "hist", "mode": "append", "sid": key, "first": from, "oldest": oldest, "truncated": truncated, "lines": lines}));
}

/// Folds among the rows that scrolled off, at most every second (AC-32).
pub fn scan_folds(page: &Page, ctl: &Ctl, key: &str) {
    let found = {
        let mut g = ctl.folds.lock();
        let Some(h) = g.as_mut() else { return };
        let now = now();
        if !h.due(now) {
            return;
        }
        h.scan(now)
    };
    if !found.is_empty() {
        page.send_json(&json!({"t": "folds", "sid": key, "items": found}));
    }
}

/// Older rows before `before` (down to `to`, a bookmark's line), never past the oldest
/// the page may load (AC-02, AC-33).
pub async fn older(app: &Shared, page: &Arc<Page>, before: i64, to: Option<i64>) {
    let Some((key, server, pane, ctl)) = shown(page) else { return };
    let Some(client) = server.bulk() else { return };
    let (top, oldest, truncated, width) = {
        let n = ctl.numbering.lock();
        let Some(top) = n.top() else { return };
        (top, n.oldest(keep(app)), n.truncated(keep(app)), ctl.fields.lock().as_ref().map_or(80, |f| f.width))
    };
    let before = before.clamp(oldest, top);
    let first = to.map_or(before - PAGE, |t| t.min(before - PAGE)).max(oldest);
    match capture(&client, &pane, top, first, before, width).await {
        Ok(lines) => {
            let (lines, folds) = match ctl.folds.lock().as_mut() {
                Some(h) => h.take(first, lines, true),
                None => (lines, Vec::new()),
            };
            let mut m = json!({"t": "hist", "mode": "prepend", "sid": key, "first": first, "oldest": oldest, "truncated": truncated, "lines": lines});
            if !folds.is_empty() {
                m["folds"] = json!(folds);
            }
            page.send_json(&m);
        }
        Err(e) => super::client::error(page, format!("Could not load older lines: {e}"), Some(&key)),
    }
}

/// A fold's rows, when the page shows it: only rows of a fold terminumd told of, still in
/// tmux's history, at most 20,000 (AC-32).
pub async fn unfold(app: &Shared, page: &Arc<Page>, a: i64, b: i64) {
    let Some((key, server, pane, ctl)) = shown(page) else { return };
    let Some(client) = server.bulk() else { return };
    let (top, oldest, width) = {
        let n = ctl.numbering.lock();
        (n.top().unwrap_or(0), n.oldest(keep(app)), ctl.fields.lock().as_ref().map_or(80, |f| f.width))
    };
    let a = a.max(oldest);
    let told = ctl.folds.lock().as_ref().is_some_and(|h| h.told(a, b));
    let why = if a > b || b >= top {
        Some("These lines are no longer in this session's history.".to_string())
    } else if !told {
        Some("Not a fold terminumd told of".to_string())
    } else if b - a >= UNFOLD_MAX {
        Some(format!("More than {UNFOLD_MAX} lines: open them with tmux."))
    } else {
        None
    };
    if let Some(why) = why {
        page.send_json(&json!({"t": "lines", "sid": key, "first": a, "lines": [], "error": why}));
        return;
    }
    match capture(&client, &pane, top, a, b + 1, width).await {
        Ok(lines) => {
            page.send_json(&json!({"t": "lines", "sid": key, "first": a, "lines": lines}));
        }
        Err(e) => {
            page.send_json(&json!({"t": "lines", "sid": key, "first": a, "lines": [], "error": e}));
        }
    }
}
