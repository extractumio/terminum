// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A task's files' routes (`terminum-kanban.md` AC-109), behind the page's sign-in: kept, read with the header
//! sign-in only, deleted.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query as Params, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use serde_json::json;

use super::api::{answer, told};
use super::files;
use super::ops;
use super::store::HUMAN;
use crate::app::Shared;
use crate::http::{off, refuse};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/task-files", post(upload).layer(DefaultBodyLimit::max(files::MOST_BYTES + 1024)))
        .route("/api/task-files/{id}", get(download).delete(remove_file))
}

async fn upload(State(app): State<Shared>, Params(q): Params<HashMap<String, String>>, data: Bytes) -> Response {
    let name = q.get("name").cloned().unwrap_or_default();
    let a = app.clone();
    answer(off(move || files::keep(&a.tasks, &a.cfg.dir, &name, &data, ops::now())).await)
}

/// Bytes with the header sign-in only: a raster picture shows, all else downloads (AC-109).
async fn download(State(app): State<Shared>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    let a = app.clone();
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).map(str::to_string);
    let done = off(move || {
        let (file, bytes, inline) = files::open(&a.tasks, &a.cfg.dir, &id)?;
        let len = bytes.metadata().map_err(|e| format!("{}: {e}", file.name))?.len();
        Ok((file, inline, crate::raw_body::response(bytes, len, range.as_deref())))
    }).await;
    let (file, inline, mut res) = match done {
        Ok(d) => d,
        Err(e) => return refuse(StatusCode::NOT_FOUND, e),
    };
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(inline.unwrap_or("application/octet-stream")));
    let safe: String = file.name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
    let how = if inline.is_some() { "inline" } else { "attachment" };
    if let Ok(v) = HeaderValue::from_str(&format!("{how}; filename=\"{safe}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    res
}

async fn remove_file(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    let a = app.clone();
    let done = off(move || files::delete(&a.tasks, &a.cfg.dir, &id, HUMAN, ops::now())).await;
    if let Ok(Some(task)) = &done {
        told(&app, task);
    }
    answer(done.map(|_| json!({"ok": true})))
}
