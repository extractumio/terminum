// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Sent history from the page (AC-90): `sent {text, agent}` keeps what stood at the cursor of
//! the shown pane when the user pressed Enter (a coding agent whose hooks spoke gives its prompt
//! itself, sock.rs, and its pane's are dropped); `sentget {n}` answers `sentone {sid, n, total,
//! item, host, path}`, the n-th newest of the shown pane's project. Every page hears `sentnew
//! {host, path}` when a project gained one.

use std::sync::Arc;

use serde_json::{json, Value};

use super::Page;
use crate::app::Shared;

/// Keeps `text` sent in pane `key` of project (`host`, `path`) and tells the pages; a failure
/// is logged (the user's command went to the terminal all the same).
pub async fn keep(app: &Shared, host: &str, path: &str, key: &str, text: &str, mut info: Value) {
    if info["title"].is_null() {
        info["title"] = json!(title_of(app, key));
    }
    let (a, host, path, key, text) = (app.clone(), host.to_string(), path.to_string(), key.to_string(), text.to_string());
    let (h, p) = (host.clone(), path.clone());
    match tokio::task::spawn_blocking(move || a.sent.add(&h, &p, &key, &text, info)).await.map_err(|e| e.to_string()).and_then(|r| r) {
        Ok(true) => app.pages.tell(&json!({"t": "sentnew", "host": host, "path": path}).to_string()),
        Ok(false) => {}
        Err(e) => tracing::warn!(event = "sent.unsaved", error = %e),
    }
}

fn title_of(app: &Shared, key: &str) -> String {
    super::marks::find_item(&app.layout.read(), key).and_then(|it| it["title"].as_str().map(str::to_string)).unwrap_or_default()
}

pub async fn handle(app: &Shared, page: &Arc<Page>, t: &str, msg: &Value) {
    let agent = msg["agent"].as_str().filter(|a| !a.is_empty());
    let hooked = page.state.lock().shown.as_ref().is_some_and(|s| app.hooks.has(&s.key));
    if t == "sent" && agent.is_some() && hooked {
        return;                                         // its hooks give the prompt exactly
    }
    // the project: the pane's host ("local" for every server of this Mac) and folder
    let (key, host, path) = match super::client::shown_cwd(page).await {
        Ok((key, server, path)) => (key, server.name.clone(), path),
        Err((_, e)) => {
            page.send_json(&json!({"t": "sentone", "error": e}));
            return;
        }
    };
    if t == "sent" {
        let text = msg["text"].as_str().unwrap_or("");
        return keep(app, &host, &path, &key, text, json!({"agent": agent, "via": "page"})).await;
    }
    let n = msg["n"].as_u64().unwrap_or(0) as usize;
    let a = app.clone();
    let (h, p) = (host.clone(), path.clone());
    let (total, item) = tokio::task::spawn_blocking(move || a.sent.get(&h, &p, n)).await.unwrap_or((0, None));
    page.send_json(&json!({"t": "sentone", "sid": key, "n": n, "total": total, "item": item, "host": host, "path": path}));
}
