// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The routes of each listener (spec §9 "What each listener allows"): the loopback one for
//! the Mac's browser, the web listener for other devices, an agent's Unix socket; and the
//! guard in front of them (auth::checks decides, AC-09, AC-10).

use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::app::Shared;
use crate::auth::checks::{self, Listener};
use crate::auth::signin::Redeem;
use crate::http::{self, err};
use crate::page::ws::FromWeb;

/// Which listener a router serves.
#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Loopback,
    Web,
}

/// The Files API as fbd had it (`/api/*`): under `/fb` for pages, at the root of an agent.
fn files_api(app: &Shared) -> Router<Shared> {
    use crate::api_fs::*;
    use crate::panel_api as p;
    Router::new()
        .route("/api/state", get(p::get_state))
        .route("/api/events", get(p::events))
        .route("/api/ws", get(crate::panel_ws::socket))
        .route("/api/ls", get(ls))
        .route("/api/file", get(file).put(save_file).layer(DefaultBodyLimit::max(save_body_limit(app.cfg.text_max))))
        .route("/api/raw", get(raw))
        .route("/api/fs/mkdir", post(fs_mkdir))
        .route("/api/fs/touch", post(fs_touch))
        .route("/api/fs/rename", post(fs_rename))
        .route("/api/fs/trash", post(fs_trash))
        .route("/api/os/open", post(os_open))
        .route("/api/os/reveal", post(os_reveal))
        .route("/api/terminal/{action}", post(p::terminal))
        .route("/api/workspace", get(p::get_workspace).put(p::put_workspace))
        .route("/api/prefs", get(p::get_prefs).put(p::put_prefs))
        .route("/api/remote/{action}", post(p::remote_action))
        .route("/api/watch", put(p::watch))
        .route("/api/health", get(p::health))
        .layer(middleware::from_fn_with_state(app.clone(), crate::remote::route))
}

/// The routes of a page listener (loopback or web).
pub fn pages(app: Shared, side: Side) -> Router {
    let mut r = Router::new()
        .route("/", get(index))
        .route("/ws", get(crate::page::ws::socket))
        .route("/api/hello", get(hello))
        .route("/api/settings", get(settings))
        .route("/custom.css", get(custom_css))
        .route("/auth/check", get(check))
        .route("/paste", post(crate::page::upload::paste).layer(DefaultBodyLimit::max(crate::page::upload::MAX_BYTES)))
        .route("/upload", post(crate::page::upload::upload).layer(DefaultBodyLimit::max(crate::page::upload::MAX_BYTES)))
        .route("/fb", get(|| async { axum::response::Redirect::permanent("/fb/") }))
        .route("/view", get(view))
        .route("/fb/", get(panel))
        .route("/fb/{*path}", get(panel_asset))
        .nest("/fb", files_api(&app));
    r = match side {
        Side::Loopback => r.route("/open/{code}", get(open)),
        Side::Web => r.route("/auth/login", post(crate::webaccess::login).layer(DefaultBodyLimit::max(crate::auth::web::SMALL_BODY))).route("/auth/logout", post(crate::webaccess::logout)),
    };
    r.fallback(static_file)
        .layer(middleware::from_fn_with_state((app.clone(), side), guard))
        .with_state(app)
}

/// An agent's routes on its Unix socket: the Files API at the root, the token decides.
pub fn agent(app: Shared) -> Router {
    files_api(&app)
        .route("/api/paste", post(crate::remote::agent_paste).layer(DefaultBodyLimit::max(crate::page::upload::MAX_BYTES)))
        .layer(middleware::from_fn_with_state(app.clone(), agent_guard))
        .with_state(app)
}

/// Whether a request carries a valid sign-in for its listener.
fn signed_in(app: &Shared, side: Side, req: &Request) -> bool {
    let h = req.headers();
    let header = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
    match side {
        Side::Loopback => checks::signin_secret(req.method().as_str(), req.uri().query(), header).is_some_and(|s| app.signins.check(&s)),
        Side::Web => crate::webaccess::cookie_ok(app, h),
    }
}

async fn guard(State((app, side)): State<(Shared, Side)>, mut req: Request, next: Next) -> Response {
    let ok = signed_in(&app, side, &req);
    let (allow, names) = crate::webaccess::host_lists(&app);
    let listener = match side {
        Side::Loopback => Listener::Loopback { port: app.cfg.port },
        Side::Web => Listener::Web { allow: &allow, own_names: &names },
    };
    let h = req.headers().clone();
    let header = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
    if let Err(d) = checks::decide(&listener, req.method().as_str(), req.uri().path(), header, ok) {
        app.denied.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tracing::warn!(event = "auth.denied", reason = d.code, path = %req.uri().path());
        let status = StatusCode::from_u16(d.status).unwrap_or(StatusCode::FORBIDDEN);
        return (status, Json(d.body())).into_response();
    }
    req.extensions_mut().insert(FromWeb(side == Side::Web));
    let mut res = next.run(req).await;
    let hs = res.headers_mut();
    hs.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    hs.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    hs.entry(header::CACHE_CONTROL).or_insert(HeaderValue::from_static("no-store"));
    res
}

/// An agent answers only the Mac's terminumd: its Host and the token it got on stdin.
async fn agent_guard(State(app): State<Shared>, req: Request, next: Next) -> Response {
    let h = req.headers();
    let host_ok = h.get(header::HOST).and_then(|v| v.to_str().ok()) == Some(crate::agent::HOST);
    let token_ok = h.get("x-tm-token").and_then(|v| v.to_str().ok()).is_some_and(|t| crate::auth::crypto::same(t, &app.cfg.agent_token));
    if !host_ok || !token_ok {
        return err(StatusCode::UNAUTHORIZED, "bad_token", "Missing or wrong token");
    }
    next.run(req).await
}

async fn index(headers: HeaderMap) -> Response {
    http::page_index(&headers, None)
}

async fn static_file(req: Request) -> Response {
    http::page_file(req.uri().path(), req.headers()).unwrap_or_else(|| err(StatusCode::NOT_FOUND, "not_found", "Not found"))
}

#[derive(Deserialize)]
struct ViewQuery {
    #[serde(default)]
    path: String,
    host: Option<String>,
}

/// `/view?path=&host=`: View alone in a second browser tab (AC-70), signed in by the page's
/// own storage (same origin), so no secret is in the address.
async fn view(Query(q): Query<ViewQuery>) -> Response {
    let enc = |s: &str| percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string();
    let mut to = format!("/fb/?view={}", enc(&q.path));
    if let Some(h) = q.host.filter(|h| !h.is_empty()) {
        to.push_str(&format!("&host={}", enc(&h)));
    }
    axum::response::Redirect::to(&to).into_response()
}

async fn panel(headers: HeaderMap) -> Response {
    http::panel_file("", &headers)
}

async fn panel_asset(UrlPath(path): UrlPath<String>, headers: HeaderMap) -> Response {
    http::panel_file(&path, &headers)
}

/// `GET /open/<code>`: the page with a new sign-in, once (AC-09).
async fn open(State(app): State<Shared>, UrlPath(code): UrlPath<String>, headers: HeaderMap) -> Response {
    if app.codes.redeem(&code) != Redeem::Ok {
        return http::code_spent_page();
    }
    let a = app.clone();
    match http::blocking(move || a.signins.create("terminum open")).await {
        Ok(Ok(secret)) => {
            tracing::info!(event = "signin.created", signins = app.signins.count());
            http::page_index(&headers, Some(&secret))
        }
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, "not_saved", format!("The sign-in could not be saved: {e}")),
        Err(r) => r,
    }
}

#[derive(Deserialize)]
struct Hello {
    #[serde(default)]
    n: String,
    #[serde(default)]
    id: String,
}

/// `GET /api/hello?n=&id=` (no sign-in): the server proves it holds the page's sign-in.
async fn hello(State(app): State<Shared>, Query(q): Query<Hello>) -> Response {
    match app.signins.hello(&q.id, &q.n) {
        Ok(proof) => Json(json!({"proof": proof})).into_response(),
        Err(d) => (StatusCode::from_u16(d.status).unwrap_or(StatusCode::BAD_REQUEST), Json(d.body())).into_response(),
    }
}

async fn settings(State(app): State<Shared>) -> Json<serde_json::Value> {
    Json(app.looks.message(false))
}

/// The user's stylesheet (AC-19), loaded after the page's own; empty when there is none.
async fn custom_css(State(app): State<Shared>) -> Response {
    let path = app.cfg.home.join("custom.css");
    let css = match std::fs::metadata(&path) {
        Ok(m) if m.len() <= 64 << 10 => std::fs::read(&path).unwrap_or_default(),
        Ok(_) => b"/* custom.css is larger than 64 KB: not loaded */".to_vec(),
        Err(_) => Vec::new(),
    };
    let mut res = css.into_response();
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/css; charset=utf-8"));
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    res
}

/// `GET /auth/check`: 204 when signed in; else 401 with how to sign in here.
async fn check(State(app): State<Shared>, Extension(FromWeb(web)): Extension<FromWeb>, req: Request) -> Response {
    let side = if web { Side::Web } else { Side::Loopback };
    if signed_in(&app, side, &req) {
        return StatusCode::NO_CONTENT.into_response();
    }
    let login = if web { "password" } else { "code" };
    (StatusCode::UNAUTHORIZED, Json(json!({"error": "bad_session", "login": login, "message": "Signed out: run terminum open"}))).into_response()
}
