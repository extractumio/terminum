// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! One page's state and its messages (spec §9 WebSocket table): which pane it shows, its
//! view's grid, and the dispatch of what it sends. Errors are said in words on the page
//! and in the log (fail loud).

use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};
use tokio::task::JoinHandle;

use super::stream::Ctl;
use super::{Page, CLOSED};
use crate::app::Shared;
use crate::tmux::Server;

/// The pane a page shows.
pub struct Shown {
    pub key: String,
    pub server: Arc<Server>,
    pub pane: String,
    pub ctl: Arc<Ctl>,
    pub task: JoinHandle<()>,
}

impl Drop for Shown {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Default)]
pub struct State {
    pub shown: Option<Shown>,
    /// the view's grid at the chosen text size (`size`)
    pub view: Option<(u16, u16)>,
    /// the page is hidden: no stream, no list refresh for it
    pub paused: bool,
    /// when this page last made a window, session or split (one a second, AC-07)
    pub last_new: Option<Instant>,
    /// when this page last typed (sizing policy `latest`, AC-13)
    pub typed_at: Option<Instant>,
    /// Screen mode asked for this pane
    pub screen: bool,
    /// per pane: auto | always | never (AC-05)
    pub screenmode: std::collections::HashMap<String, String>,
    /// typed, not yet handed to tmux
    pub queue: super::typing::Queue,
    /// when this page last resized a window (at most once a second, AC-13)
    pub resized_at: Option<Instant>,
}

/// What the page sent cannot be done: say why, on the page and in the log.
pub fn error(page: &Page, msg: impl Into<String>, sid: Option<&str>) {
    let msg = msg.into();
    tracing::warn!(event = "page.error", page = page.id, msg);
    let mut v = json!({"t": "error", "msg": msg});
    if let Some(s) = sid {
        v["sid"] = json!(s);
    }
    page.send_json(&v);
}

/// Handles one message from a page.
pub async fn handle(app: &Shared, page: &Arc<Page>, msg: Value) {
    let t = msg["t"].as_str().unwrap_or("");
    let s = |k: &str| msg[k].as_str().unwrap_or("").to_string();
    match t {
        "sub" => subscribe(app, page, &s("id")).await,
        "in" => super::typing::typed(app, page, s("data"), msg["k"].as_u64()),
        "paste" => super::typing::paste(app, page, s("data"), msg["bracket"].as_bool().unwrap_or(true)),
        "more" => super::hist::older(app, page, msg["before"].as_i64().unwrap_or(0), msg["to"].as_i64()).await,
        "unfold" => super::hist::unfold(app, page, msg["n"].as_i64().unwrap_or(0), msg["to"].as_i64().unwrap_or(-1)).await,
        "find" => super::find::start(app, page, &msg),
        "mark" | "unmark" | "locate" => super::marks::handle(app, page, t, &msg).await,
        "files" => files(app, page).await,
        "size" | "fit" | "unfit" | "screenmode" | "mouse" => super::sizing::handle(app, page, t, &msg).await,
        "new" | "newsession" | "split" | "rename" | "reorder" | "move" | "kill" => super::sessions::handle(app, page, t, &msg).await,
        "addhost" | "hostaction" | "restore" => super::sessions::admin(app, page, t, &msg).await,
        "updatechoice" if !page.web => {
            if let Err(e) = crate::release::choose(app, &s("action"), msg["tag"].as_str()) {
                error(page, e, None);
            }
        }
        "trace" => super::timing::handle(app, page, &msg),
        "pause" => pause(page, true),
        "resume" => resume(app, page),
        _ => tracing::debug!(event = "page.unknown", t),
    }
}

/// Shows pane `key`: its theme, history and screen, then every change (AC-02).
pub async fn subscribe(app: &Shared, page: &Arc<Page>, key: &str) {
    let old = page.state.lock().shown.take();
    if let Some(old) = old {
        super::sizing::left(app, page, &old).await;
        drop(old);
    }
    let Some((server, pane)) = app.tmux.resolve(key) else {
        error(page, CLOSED, Some(key));
        return;
    };
    let ctl = Arc::new(Ctl::default());
    let task = tokio::spawn(super::stream::run(app.clone(), page.clone(), server.clone(), pane.clone(), key.to_string(), ctl.clone()));
    page.state.lock().shown = Some(Shown { key: key.to_string(), server, pane, ctl, task });
    super::sizing::opened(app, page).await;
}

fn pause(page: &Page, on: bool) {
    let mut st = page.state.lock();
    st.paused = on;
    if on
        && let Some(s) = &st.shown {
            s.ctl.pause(true);
        }
}

/// Back from hidden: the stream goes on, sending only what changed meanwhile.
fn resume(app: &Shared, page: &Arc<Page>) {
    pause(page, false);
    let st = page.state.lock();
    if let Some(s) = &st.shown {
        s.ctl.pause(false);
    }
    drop(st);
    page.send_json(&json!({"t": "layout", "groups": app.layout.read().clone()}));
}

/// The shown pane's folder, home and host, for the Files view and printed paths (AC-51).
async fn files(app: &Shared, page: &Arc<Page>) {
    let (key, server, pane) = {
        let st = page.state.lock();
        let Some(s) = &st.shown else {
            page.send_json(&json!({"t": "files", "error": "No pane is shown."}));
            return;
        };
        (s.key.clone(), s.server.clone(), s.pane.clone())
    };
    let cwd = match server.run(format!("display -p -t {pane} '#{{pane_current_path}}'")).await {
        Ok(lines) => lines.into_iter().next().unwrap_or_default(),
        Err(e) => {
            page.send_json(&json!({"t": "files", "sid": key, "error": format!("The pane's folder is not known: {e}")}));
            return;
        }
    };
    page.send_json(&files_message(&server, &key, &cwd).await);
    let _ = app;
}

/// The `files` message: the shown pane's key, folder, home and host (for the Files panel).
pub async fn files_message(server: &crate::tmux::Server, key: &str, cwd: &str) -> Value {
    let host = (!server.is_local()).then(|| server.name.clone());
    let home = if server.is_local() { crate::config::home().display().to_string() } else { server.home().await.unwrap_or_default() };
    json!({"t": "files", "sid": key, "key": key, "cwd": cwd, "home": home, "host": host})
}
