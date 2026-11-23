// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The Inbox's routes (terminum-reports.md §9), behind the page's sign-in and the exact Host, Origin and Content-Type
//! checks of every write (routes.rs `guard`): list, one report, read and unread, delete and restore, a line closed.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use serde_json::json;

use super::inbox::{self, Filter};
use super::record;
use crate::app::Shared;
use crate::tasks::api::{answer, store};
use crate::tasks::ops;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/reports", get(list))
        .route("/api/reports/{id}", get(one).delete(remove))
        .route("/api/reports/{id}/{act}", post(act))
}

async fn list(State(app): State<Shared>, Query(q): Query<HashMap<String, String>>) -> Response {
    let f = Filter {
        project: q.get("project").filter(|p| !p.is_empty()).cloned(),
        deleted: q.get("deleted").is_some_and(|d| d == "1"),
        before: q.get("from").and_then(|n| n.parse().ok()),
        session: q.get("session").filter(|s| !s.is_empty()).cloned(),
        limit: q.get("limit").and_then(|n| n.parse().ok()),
        unread_only: false,
    };
    answer(store(&app, move |tx| inbox::list(tx, &f)).await)
}

async fn one(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    answer(store(&app, move |tx| inbox::detail(tx, &record::key_of(tx, &id)?)).await)
}

async fn remove(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    changed(&app, store(&app, move |tx| record::set_deleted(tx, &record::key_of(tx, &id)?, true, ops::now())).await)
}

/// read, unread, restore, seen: the human's acts on a report (AC-269, AC-271).
async fn act(State(app): State<Shared>, Path((id, act)): Path<(String, String)>) -> Response {
    let done = store(&app, move |tx| {
        let (key, at) = (record::key_of(tx, &id)?, ops::now());
        match act.as_str() {
            "read" => record::set_read(tx, &key, true, at),
            "unread" => record::set_read(tx, &key, false, at),
            "restore" => record::set_deleted(tx, &key, false, at),
            "seen" => record::line_closed(tx, &key, at),
            other => Err(format!("No act {other} on a report")),
        }
    }).await;
    changed(&app, done)
}

/// Every page hears of a change that took; the answer says whether it changed anything.
fn changed(app: &Shared, done: Result<bool, String>) -> Response {
    if matches!(done, Ok(true)) {
        super::changed(app);
    }
    answer(done.map(|changed| json!({"ok": true, "changed": changed})))
}
