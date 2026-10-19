// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Bookmarks from the page (AC-33): `mark` (new {ref, sid, n, text, note, remind}; changed
//! {ref, id, note, remind}), `unmark`, `locate`, `remind {id, action: close|resolve}` (AC-91); the list goes to a page with its hello. Kept on the Mac in web-bookmarks.json and
//! told to every signed-in page at once; a bookmark is found again by its text.

use std::sync::Arc;

use serde_json::{json, Value};

use super::bookmarks::{self as bm, NewMark};
use super::Page;
use crate::app::Shared;

fn all(app: &Shared) -> Value {
    let store = app.bookmarks.lock();
    json!({"t": "marks", "items": store.all().iter().map(bm::Item::wire).collect::<Vec<_>>(), "error": store.broken()})
}

/// What a page learns when it connects.
pub fn hello(app: &Shared, page: &Page) {
    page.send_json(&all(app));
}

pub async fn handle(app: &Shared, page: &Arc<Page>, t: &str, msg: &Value) {
    match t {
        "unmark" => {
            let id = msg["id"].as_str().unwrap_or("").to_string();
            match app.bookmarks.lock().remove(&id) {
                Ok(true) => app.pages.tell(&json!({"t": "unmark", "id": id}).to_string()),
                Ok(false) => {}
                Err(e) => super::client::error(page, e.to_string(), None),
            }
        }
        "remind" => {
            // a reminder closed or resolved (AC-91)
            let (id, resolve) = (msg["id"].as_str().unwrap_or(""), msg["action"] == "resolve");
            match app.bookmarks.lock().settle(id, resolve) {
                Ok(item) => tell_mark(app, &item),
                Err(e) => super::client::error(page, e.to_string(), None),
            }
        }
        "locate" => {
            let (app, page, id) = (app.clone(), page.clone(), msg["id"].as_str().unwrap_or("").to_string());
            tokio::spawn(async move { locate(&app, &page, &id).await });
        }
        _ => mark(app, page, msg),
    }
}

/// A new bookmark, or a changed note; a save sent again after no answer came is kept once.
fn mark(app: &Shared, page: &Page, msg: &Value) {
    let r: String = msg["ref"].as_str().map(|s| s.chars().take(40).collect()).unwrap_or_default();
    if let Some(id) = app.bookmarks.lock().ref_id(&r).map(str::to_string) {
        page.send_json(&json!({"t": "marked", "ref": r, "id": id}));
        return;
    }
    let result = if let Some(id) = msg["id"].as_str().filter(|s| !s.is_empty()) {
        bm::note_of(msg).and_then(|note| app.bookmarks.lock().set_note(id, note, msg["remind"].as_bool()))
    } else {
        new_mark(app, page, msg)
    };
    match result {
        Ok(item) => {
            app.bookmarks.lock().remember(&r, &item.id);
            page.send_json(&json!({"t": "marked", "ref": r, "id": item.id}));
            tell_mark(app, &item);
        }
        Err(e) => {
            tracing::warn!(event = "bookmark.refused", reason = %e);
            page.send_json(&json!({"t": "marked", "ref": r, "error": e.to_string()}));
        }
    }
}

fn new_mark(app: &Shared, page: &Page, msg: &Value) -> Result<bm::Item, bm::MarkError> {
    let key = page.state.lock().shown.as_ref().map(|s| s.key.clone());
    let Some(key) = key.filter(|k| msg["sid"].as_str() == Some(k.as_str())) else {
        return Err(bm::MarkError(bm::NOT_SHOWN.into()));
    };
    let (text, note, n) = bm::mark_of(msg)?;
    let item = find_item(&app.layout.read(), &key);
    let Some(item) = item else { return Err(bm::MarkError(bm::CLOSED.into())) };
    let new = NewMark {
        key: Some(key),
        created: None,
        n,
        text,
        note,
        title: item["title"].as_str().unwrap_or("").to_string(),
        host: project_of(&item).0,
        remind: msg["remind"] == true,
        dir: project_of(&item).1,
    };
    app.bookmarks.lock().add(new)
}

/// The list's row of pane `key`.
pub fn find_item(layout: &Value, key: &str) -> Option<Value> {
    layout_items(layout.as_array()?).find(|it| it["id"] == key).cloned()
}

/// Every row of the layout's groups.
pub fn layout_items(groups: &[Value]) -> impl Iterator<Item = &Value> {
    groups.iter().flat_map(|g| g["items"].as_array().into_iter().flatten())
}

/// A layout row's project (AC-91): its host (none on this Mac) and folder (none when unknown).
pub fn project_of(item: &Value) -> (Option<String>, Option<String>) {
    let s = |k: &str| item[k].as_str().filter(|v| !v.is_empty()).map(str::to_string);
    (s("host"), s("dir"))
}

/// Every page learns of a new or changed bookmark.
pub fn tell_mark(app: &Shared, item: &bm::Item) {
    app.pages.tell(&json!({"t": "mark", "item": item.wire()}).to_string());
}

/// Where a bookmark's text is now in the shown pane: near its line first, the whole history
/// only when not there (tmux renumbers when it drops lines or the scrollback is cleared).
async fn locate(app: &Shared, page: &Page, id: &str) {
    let item = app.bookmarks.lock().get(id).cloned();
    let Some(item) = item else {
        page.send_json(&bm::unlocated(id, bm::DELETED));
        return;
    };
    let Some((key, server, pane, ctl)) = super::find::shown(page).filter(|s| Some(&s.0) == item.key.as_ref()) else {
        page.send_json(&bm::unlocated(id, bm::ELSEWHERE));
        return;
    };
    let keep = app.cfg.history;
    // the rows read and searched, the search off the async workers (thousands of rows)
    let read = |around| {
        let (server, pane, ctl, text, n) = (server.clone(), pane.clone(), ctl.clone(), item.text.clone(), item.n);
        async move {
            let rows: Vec<(i64, String)> = super::find::rows_of(&server, &pane, &ctl, keep, around).await?.into_iter().map(|(n, t, _)| (n, t)).collect();
            tokio::task::spawn_blocking(move || bm::find(&rows, &text, n)).await.map_err(|e| e.to_string())
        }
    };
    let mut found = match read(Some(item.n)).await {
        Ok(at) => at,
        Err(e) => {
            page.send_json(&bm::unlocated(id, &format!("The history could not be read: {e}")));
            return;
        }
    };
    if bm::needs_whole(found.as_ref(), &item.text) {
        match read(None).await {
            Ok(at) => found = at.or(found),
            Err(e) => {
                page.send_json(&bm::unlocated(id, &format!("The history could not be read: {e}")));
                return;
            }
        }
    }
    page.send_json(&bm::located(id, &key, found.as_ref()));
    if let Some(f) = found.filter(|f| f.n != item.n) {
        // where to look first next time; the marks move there
        if let Ok(moved) = app.bookmarks.lock().set_n(id, f.n) {
            tell_mark(app, &moved);
        }
    }
}
