// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! An agent's queue as a human changes it (`terminum-dispatcher.md` AC-169): its order, and a task taken out. The
//! queue itself is the waits of `waits.rs`.

use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use super::{store, waits};
use crate::app::Shared;
use crate::tasks::api::{answer, told};

pub(super) fn routes() -> Router<Shared> {
    Router::new().route("/api/task-agents/{id}/queue", post(queue_order)).route("/api/tasks/{id}/leave-queue", post(leave_queue))
}

#[derive(Deserialize)]
struct QueueOrder {
    tasks: Vec<String>,
}

/// The human takes a task out of its agent's queue (AC-169): its wait alone, never a run that started meanwhile.
async fn leave_queue(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    let wanted = id.clone();
    let done = store(&app, move |tx| waits::take_out(tx, &wanted).map(|()| json!({"ok": true}))).await;
    if done.is_ok() {
        told(&app, &id);
    }
    answer(done)
}

/// The human's order of an agent's queue (AC-169); every page hears.
async fn queue_order(State(app): State<Shared>, Path(agent): Path<String>, Json(body): Json<QueueOrder>) -> Response {
    // any task's change has every board read its slots anew: the queue's first stands in for the queue
    let first = body.tasks.first().cloned();
    let done = store(&app, move |tx| waits::reorder(tx, &agent, &body.tasks).map(|()| json!({"ok": true}))).await;
    if let (Ok(_), Some(id)) = (&done, first) {
        told(&app, &id);
    }
    answer(done)
}
