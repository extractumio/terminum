// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A task's files' routes (`terminum-kanban.md` AC-109), behind the page's sign-in: kept, read with the header
//! sign-in only, deleted.

use std::collections::HashMap;
use std::io;

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
use crate::http::{blocking, off, refuse, ApiResult};

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
async fn download(State(app): State<Shared>, Path(id): Path<String>, headers: HeaderMap) -> ApiResult {
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).map(str::to_string);
    let (name, inline, mut res) = blocking(move || {
        let files::Opened { name, bytes, inline, len } = files::open(&app.tasks, &app.cfg.dir, &id)?;
        Ok((name, inline, crate::raw_body::response(bytes, len, range.as_deref())))
    }).await?.map_err(|e: io::Error| {
        let status = if e.kind() == io::ErrorKind::NotFound { StatusCode::NOT_FOUND } else { StatusCode::INTERNAL_SERVER_ERROR };
        refuse(status, e.to_string())
    })?;
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(inline.unwrap_or("application/octet-stream")));
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
    let how = if inline.is_some() { "inline" } else { "attachment" };
    if let Ok(v) = HeaderValue::from_str(&format!("{how}; filename=\"{safe}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(res)
}

async fn remove_file(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    let a = app.clone();
    let done = off(move || files::delete(&a.tasks, &a.cfg.dir, &id, HUMAN, ops::now())).await;
    if let Ok(Some(task)) = &done {
        told(&app, task);
    }
    answer(done.map(|_| json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Method, Request};
    use std::{fs, os::unix::fs::symlink};
    use tower::ServiceExt;

    async fn fetch(router: &Router, method: Method, id: &str) -> (StatusCode, Bytes) {
        let request = Request::builder().method(method).uri(format!("/api/task-files/{id}")).body(Body::empty()).unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        (response.status(), to_bytes(response.into_body(), 1024).await.unwrap())
    }

    #[tokio::test]
    async fn ac109_download_distinguishes_missing_files_from_read_failures() {
        let (root, app) = crate::testutil::app_with_tasks("task-file-download");
        let keep = |name| files::keep(&app.tasks, &app.cfg.dir, name, b"kept", ops::now()).unwrap().id;
        let missing = keep("missing.txt");
        let unreadable = keep("unreadable.txt");
        let folder = files::folder(&app.cfg.dir);
        fs::remove_file(folder.join(&missing)).unwrap();
        let path = folder.join(&unreadable);
        fs::rename(&path, path.with_extension("kept")).unwrap();
        symlink(&unreadable, &path).unwrap(); // opening a loop fails even when run as root
        let router = routes().with_state(app);

        let mut responses = Vec::new();
        for (id, expected) in [("a".repeat(32), StatusCode::NOT_FOUND), (missing, StatusCode::NOT_FOUND), (unreadable, StatusCode::INTERNAL_SERVER_ERROR)] {
            responses.push((expected, fetch(&router, Method::GET, &id).await, fetch(&router, Method::HEAD, &id).await));
        }
        drop(router);
        fs::remove_dir_all(root).unwrap();

        for (expected, (get, body), (head, head_body)) in responses {
            assert_eq!(get, expected, "GET: {}", String::from_utf8_lossy(&body));
            assert_eq!(head, expected, "HEAD");
            assert!(head_body.is_empty());
            if expected == StatusCode::INTERNAL_SERVER_ERROR {
                let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert!(error["error"].as_str().unwrap().starts_with("unreadable.txt: "), "{error}");
            }
        }
    }
}
