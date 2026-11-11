// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The interaction log (terminum.md AC-175): what the human did in a page, as the page counted it by the
//! clock minute: a kind, a place, a control's id and how many times; never a key, a text or a position.
//! `POST /api/interactions {now, rows}` appends them to the tasks' store's `user_interaction`; the Dashboard
//! reads presence and engagement from it. The log never holds up a page: what is not taken is dropped.

#[cfg(test)]
mod tests;

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, DefaultBodyLimit, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::post;
use axum::{Extension, Json, Router};
use parking_lot::Mutex;
use rusqlite::Transaction;
use serde::Deserialize;
use serde_json::json;

use crate::app::Shared;
use crate::page::ws::FromWeb;
use crate::tasks::api::answer;
use crate::tasks::ops::sql;

/// What a page counts: a press, a key, the wheel's or a finger's scroll, a drag, the pointer's move, the
/// seconds it was in front.
pub const KINDS: [&str; 6] = ["click", "key", "scroll", "drag", "move", "front"];
/// Where: a page, or a frame of the sessions' page.
pub const SURFACES: [&str; 8] = ["sessions", "kanban", "history", "dashboard", "scheduled", "files", "view", "quick"];
/// Rows kept: those of the last KEEP_DAYS, at most KEEP_ROWS.
pub const KEEP_DAYS: i64 = 180;
pub const KEEP_ROWS: i64 = 1_000_000;
const MINUTE: i64 = 60_000;
/// A request's body and rows at most; rows taken in one minute at most, from every page together.
const BODY: usize = 64 << 10;
const ROWS: usize = 300;
const A_MINUTE: usize = 600;
/// A row older than this, or later than now, is not the page's clock's doing: refused.
const OLDEST: i64 = 86_400_000;
const TARGET: usize = 40;
const PANE: usize = 200;

#[derive(Deserialize)]
struct Body {
    /// the page's clock when it sent, unix ms: its rows are shifted to this Mac's
    now: i64,
    rows: Vec<Sent>,
}

#[derive(Deserialize)]
struct Sent {
    at: i64,
    ended: i64,
    kind: String,
    surface: String,
    #[serde(default)]
    target: String,
    n: i64,
    #[serde(default)]
    amount: i64,
    #[serde(default)]
    pane: Option<String>,
}

/// A row as kept.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub at: i64,
    pub ended: i64,
    pub kind: &'static str,
    pub surface: &'static str,
    pub target: String,
    pub n: i64,
    pub amount: i64,
    pub pane: Option<String>,
    pub project: Option<String>,
}

/// A control's id as kept: digits as N, a plain name of 40 characters at most; anything else is no name.
pub fn target(id: &str) -> String {
    let mut out = String::new();
    for c in id.chars() {
        if !c.is_ascii_digit() { out.push(c); } else if !out.ends_with('N') { out.push('N'); }
    }
    let plain = out.len() <= TARGET && id.starts_with(|c: char| c.is_ascii_alphabetic()) && out.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if plain { out } else { String::new() }
}

/// `sent` as a row at this Mac's time (`shift`: its clock less the page's), or none: an unknown kind or
/// place, a time outside the last day, counts that no page counts.
fn row(sent: Sent, shift: i64, now: i64) -> Option<Row> {
    let kind = *KINDS.iter().find(|k| **k == sent.kind)?;
    let surface = *SURFACES.iter().find(|s| **s == sent.surface)?;
    let (at, ended) = (sent.at + shift, sent.ended + shift);
    let whole = at >= now - OLDEST && ended <= now + MINUTE && at <= ended && ended - at < MINUTE;
    if !whole || !(1..=100_000).contains(&sent.n) || !(0..=100_000_000).contains(&sent.amount) {
        return None;
    }
    let named = matches!(kind, "click" | "drag");
    let pane = sent.pane.filter(|p| !p.is_empty() && p.len() <= PANE && !p.chars().any(char::is_control));
    Some(Row { at, ended, kind, surface, target: if named { target(&sent.target) } else { String::new() }, n: sent.n, amount: sent.amount, pane, project: None })
}

pub fn insert(tx: &Transaction, rows: &[Row], device: &str) -> Result<(), String> {
    let mut st = tx.prepare_cached("INSERT INTO user_interaction (at, ended, kind, surface, target, n, amount, pane, project, device) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)").map_err(sql)?;
    for r in rows {
        st.execute(rusqlite::params![r.at, r.ended, r.kind, r.surface, r.target, r.n, r.amount, r.pane, r.project, device]).map_err(sql)?;
    }
    Ok(())
}

/// Rows older than KEEP_DAYS, then the oldest beyond `rows`, go (the input log's writer, each hour).
pub fn trim(tx: &Transaction, now_ms: i64, rows: i64) -> Result<(), String> {
    tx.execute("DELETE FROM user_interaction WHERE at < ?1", [now_ms - KEEP_DAYS * 86_400_000]).map_err(sql)?;
    tx.execute("DELETE FROM user_interaction WHERE id <= (SELECT id FROM user_interaction ORDER BY id DESC LIMIT 1 OFFSET ?1)", [rows]).map_err(sql).map(drop)
}

/// How many of `wanted` rows this minute still takes: a page that loops never pushes the history out.
fn room(taken: &Mutex<(i64, usize)>, now: i64, wanted: usize) -> usize {
    let mut t = taken.lock();
    if t.0 != now / MINUTE {
        *t = (now / MINUTE, 0);
    }
    let n = wanted.min(A_MINUTE - t.1);
    t.1 += n;
    n
}

static TAKEN: Mutex<(i64, usize)> = Mutex::new((0, 0));

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/interactions", post(take).layer(DefaultBodyLimit::max(BODY)))
}

/// `mac`, `web` (another device through the tailnet) or `phone`.
fn device(headers: &HeaderMap, peer: SocketAddr, web: bool) -> &'static str {
    let browser = crate::webaccess::client(headers, peer, web).browser;
    if browser.contains("iPhone") || browser.contains("Android") { "phone" } else if web { "web" } else { "mac" }
}

async fn take(State(app): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, from: Option<Extension<FromWeb>>, Json(body): Json<Body>) -> Response {
    let now = crate::inputs::now_ms();
    let device = device(&headers, peer, from.is_some_and(|f| f.0.0));
    let sent = body.rows.len();
    let mut rows: Vec<Row> = body.rows.into_iter().take(ROWS).filter_map(|s| row(s, now - body.now, now)).collect();
    rows.truncate(room(&TAKEN, now, rows.len()));
    let kept = rows.len();
    if kept < sent {
        tracing::debug!(event = "interactions.dropped", sent, kept);
    }
    let a = app.clone();
    answer(crate::http::off(move || {
        // the projects' lock is let go before the store's is taken: a project's save holds both the other way
        {
            let projects = a.projects.lock();
            for r in &mut rows {
                r.project = r.pane.as_deref().map(|p| projects.of(p).map(|p| p.id.clone()).unwrap_or_default());
            }
        }
        a.tasks.with(|tx| insert(tx, &rows, device))
    }).await.map(|()| json!({"kept": kept})))
}
