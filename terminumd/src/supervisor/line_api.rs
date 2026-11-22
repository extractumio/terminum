// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Owner-wide settings and test sends on the signed-in page router only (AC-227–229).

use axum::{Json, Router, extract::{Path, State}, response::Response, routing::{get, post}};
use serde::Deserialize;

use crate::{app::Shared, http::off, tasks::{api::answer, ops::check_project}};
use super::{SCHEMA, line};

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/task-projects/{project}/supervisor/line", get(shown).put(save))
        .route("/api/task-projects/{project}/supervisor/line/test", post(test))
}

async fn shown(State(app): State<Shared>, Path(project): Path<String>) -> Response {
    answer(off(move || app.tasks.snapshot(SCHEMA, |tx| {
        check_project(tx, &project)?;
        line::read(tx)
    })).await)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save { revision: i64, settings: line::Settings }

async fn save(State(app): State<Shared>, Path(project): Path<String>, Json(body): Json<Save>) -> Response {
    answer(line::configure(&app, project, body.revision, body.settings).await)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Test { channel: usize, revision: i64 }

async fn test(State(app): State<Shared>, Path(project): Path<String>, Json(body): Json<Test>) -> Response {
    answer(line::test(&app, project, body.channel, body.revision).await
        .map(|_| serde_json::json!({ "sent": true })))
}
