// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-253: owner controls for reverting a change and promoting a shadow trial.

use axum::{Router, Json, extract::{Path, State}, response::Response, routing::post};
use serde::Deserialize;

use crate::{app::Shared, tasks::{api::{answer, store}, ops::{check_project, now}}};

use super::config_changes;

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/task-projects/{project}/supervisor/config/{change}/revert", post(revert))
        .route("/api/task-projects/{project}/supervisor/config/{change}/promote", post(promote))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Press {}

async fn apply(app: Shared, (project, id): (String, i64),
    action: fn(&rusqlite::Transaction, &str, i64, i64) -> Result<(), String>,
) -> Response {
    let result = store(&app, move |tx| {
        check_project(tx, &project)?;
        action(tx, &project, id, now())?;
        Ok(serde_json::json!({"change": id}))
    }).await;
    if result.is_ok() { app.supervisor.wake(); }
    answer(result)
}

async fn revert(State(app): State<Shared>, Path(key): Path<(String, i64)>, Json(_): Json<Press>) -> Response {
    apply(app, key, config_changes::revert).await
}

async fn promote(State(app): State<Shared>, Path(key): Path<(String, i64)>, Json(_): Json<Press>) -> Response {
    apply(app, key, config_changes::promote).await
}
