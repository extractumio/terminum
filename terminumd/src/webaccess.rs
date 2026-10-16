// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Web access (AC-10): the second listener (off by default, `0.0.0.0:8765`) for a phone or
//! another computer, with a password (PBKDF2, brakes) and an HttpOnly cookie. Switched on and
//! off by the CLI over the socket; off signs every browser out. Before a sign-in a request
//! carries at most 4 KB, its head arrives within 10 s, at most 64 connections are open.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use hyper_util::rt::{TokioIo, TokioTimer};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::Shared;
use crate::auth::web::{PasswordHash, WebConfig, HEAD_TIMEOUT, MAX_CONNECTIONS};
use crate::auth::websignin::{self, cookie_header, cookie_value, Login, WebAuth, WebSessions, COOKIE};

#[derive(Default)]
pub struct Web {
    auth: Mutex<Option<Arc<WebAuth>>>,
    stop: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
    status: Mutex<Value>,
    allow: Mutex<Vec<String>>,
    names: Mutex<Vec<String>>,
}

/// web.json's allowed names and this Mac's own (the Host check, AC-10).
pub fn host_lists(app: &Shared) -> (Vec<String>, Vec<String>) {
    (app.web.allow.lock().clone(), app.web.names.lock().clone())
}

/// The web listener's cookie names a live sign-in.
pub fn cookie_ok(app: &Shared, headers: &HeaderMap) -> bool {
    let token = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()).and_then(|c| cookie_value(c, COOKIE).map(str::to_string));
    let auth = app.web.auth.lock().clone();
    matches!((token, auth), (Some(t), Some(a)) if a.sessions().check(&t))
}

/// `{"enabled", "urls", "error"}` for `terminum status` and the CLI.
pub fn status(app: &Shared) -> Value {
    app.web.status.lock().clone()
}

/// This Mac's names and addresses: the host name, `<name>.local`, its interfaces' addresses.
fn own_names() -> Vec<String> {
    let mut out = Vec::new();
    if let Some(name) = crate::config::host_name() {
        let short = name.split('.').next().unwrap_or("").to_string();
        out.extend([name.clone(), format!("{short}.local"), short]);
    }
    out.extend(addresses());
    out.sort();
    out.dedup();
    out.retain(|n| !n.is_empty());
    out
}

/// The IPv4 and IPv6 addresses of this Mac's interfaces.
fn addresses() -> Vec<String> {
    let mut out = Vec::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs allocates a list freed below with freeifaddrs; each entry is read only
    unsafe {
        if libc::getifaddrs(&mut ifap) != 0 {
            return out;
        }
        let mut p = ifap;
        while !p.is_null() {
            let a = (*p).ifa_addr;
            if !a.is_null() {
                match (*a).sa_family as i32 {
                    libc::AF_INET => {
                        let s = &*(a as *const libc::sockaddr_in);
                        out.push(std::net::Ipv4Addr::from(u32::from_be(s.sin_addr.s_addr)).to_string());
                    }
                    libc::AF_INET6 => {
                        let s = &*(a as *const libc::sockaddr_in6);
                        out.push(std::net::Ipv6Addr::from(s.sin6_addr.s6_addr).to_string());
                    }
                    _ => {}
                }
            }
            p = (*p).ifa_next;
        }
        libc::freeifaddrs(ifap);
    }
    out
}

/// Reads web.json and starts or stops the listener to match it.
pub async fn apply(app: &Shared) -> Result<Value, String> {
    let cfg = match WebConfig::load(&app.cfg.dir) {
        Ok(c) => c,
        Err(e) => {
            stop(app);
            *app.web.status.lock() = json!({"enabled": false, "urls": [], "error": e});
            return Err(e);
        }
    };
    *app.web.allow.lock() = cfg.allow_hosts.clone();
    *app.web.names.lock() = own_names();
    match cfg.ready() {
        Ok(true) => start(app, &cfg).await,
        Ok(false) => {
            stop(app);
            *app.web.status.lock() = json!({"enabled": false, "urls": [], "error": null});
            Ok(status(app))
        }
        Err(e) => {
            stop(app);
            *app.web.status.lock() = json!({"enabled": false, "urls": [], "error": e});
            Err(e)
        }
    }
}

async fn start(app: &Shared, cfg: &WebConfig) -> Result<Value, String> {
    let hash = cfg.password.clone().expect("ready() checked");
    let addr = format!("{}:{}", cfg.host, cfg.port);
    if app.web.stop.lock().is_some() {
        return Ok(status(app)); // running already (a password change restarts it: see set_password)
    }
    let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| {
        let msg = format!("port {} is in use or not allowed ({e}): set another \"port\" in web.json", cfg.port);
        *app.web.status.lock() = json!({"enabled": false, "urls": [], "error": msg});
        msg
    })?;
    let sessions = Arc::new(WebSessions::load(&app.cfg.dir, &hash.salt));
    *app.web.auth.lock() = Some(Arc::new(WebAuth::new(hash, sessions)));
    let (tx, rx) = tokio::sync::watch::channel(false);
    *app.web.stop.lock() = Some(tx);
    let urls: Vec<String> = addresses().into_iter().filter(|a| !a.contains(':') && !a.starts_with("127.")).map(|a| format!("http://{a}:{}/", cfg.port)).collect();
    *app.web.status.lock() = json!({"enabled": true, "urls": urls, "error": null});
    tracing::info!(event = "web.on", addr);
    tokio::spawn(serve(app.clone(), listener, rx));
    Ok(status(app))
}

/// Closes the listener and every connection; every browser is signed out (AC-10).
pub fn stop(app: &Shared) {
    if let Some(tx) = app.web.stop.lock().take() {
        let _ = tx.send(true);
        tracing::info!(event = "web.off");
    }
    if let Some(a) = app.web.auth.lock().take() {
        a.sessions().close();
    }
    app.pages.close_web();
}

async fn serve(app: Shared, listener: tokio::net::TcpListener, mut stop: tokio::sync::watch::Receiver<bool>) {
    let open = Arc::new(AtomicUsize::new(0));
    let router = crate::routes::pages(app.clone(), crate::routes::Side::Web);
    loop {
        let (stream, peer) = tokio::select! {
            a = listener.accept() => match a { Ok(a) => a, Err(_) => continue },
            _ = stop.changed() => return,
        };
        if open.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
            drop(stream);
            continue;
        }
        open.fetch_add(1, Ordering::Relaxed);
        let (open, router, mut stop) = (open.clone(), router.clone(), stop.clone());
        tokio::spawn(async move {
            let svc = hyper::service::service_fn(move |mut req: hyper::Request<hyper::body::Incoming>| {
                req.extensions_mut().insert(ConnectInfo(peer));
                let r = router.clone();
                async move { Ok::<_, std::convert::Infallible>(tower_call(r, req).await) }
            });
            let conn = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEAD_TIMEOUT)
                .serve_connection(TokioIo::new(stream), svc)
                .with_upgrades();
            tokio::pin!(conn);
            tokio::select! {
                _ = conn.as_mut() => {}
                _ = stop.changed() => conn.as_mut().graceful_shutdown(),
            }
            open.fetch_sub(1, Ordering::Relaxed);
        });
    }
}

async fn tower_call(router: axum::Router, req: hyper::Request<hyper::body::Incoming>) -> Response {
    use tower::ServiceExt;
    match router.oneshot(req.map(axum::body::Body::new)).await {
        Ok(r) => r,
        Err(e) => match e {},
    }
}

/// `terminum web password`: a new password (at least 8 characters); sign-ins of the old go.
pub async fn set_password(app: &Shared, password: &str) -> Result<Value, String> {
    let hash = PasswordHash::new(password)?;
    let dir = app.cfg.dir.clone();
    let mut cfg = WebConfig::load(&dir)?;
    cfg.password = Some(hash);
    cfg.save(&dir).map_err(|e| format!("web.json not saved: {e}"))?;
    websignin::forget(&dir);
    stop(app);
    apply(app).await
}

/// `terminum web on|off`.
pub async fn switch(app: &Shared, on: bool) -> Result<Value, String> {
    let dir = app.cfg.dir.clone();
    let mut cfg = WebConfig::load(&dir)?;
    cfg.enabled = on;
    if on {
        cfg.ready()?;
    }
    cfg.save(&dir).map_err(|e| format!("web.json not saved: {e}"))?;
    if !on {
        stop(app);
    }
    apply(app).await
}

#[derive(Deserialize)]
pub struct LoginBody {
    #[serde(default)]
    password: String,
}

/// The address a sign-in counts against: the peer, or behind `tailscale serve` (a loopback
/// peer) X-Forwarded-For's last entry.
fn address(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    let ip = peer.map_or(std::net::IpAddr::from([0, 0, 0, 0]), |p| p.ip());
    websignin::client_address(ip, headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()))
}

fn secure(headers: &HeaderMap) -> bool {
    headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()) == Some("https")
}

/// `POST /auth/login {"password"}` on the web listener.
pub async fn login(State(app): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(b): Json<LoginBody>) -> Response {
    let Some(auth) = app.web.auth.lock().clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"reason": "off"}))).into_response();
    };
    let sec = secure(&headers);
    match auth.login(b.password, address(&headers, Some(peer)), sec).await {
        Login::SignedIn(token) => {
            let mut res = StatusCode::NO_CONTENT.into_response();
            if let Ok(v) = HeaderValue::from_str(&cookie_header(&token, sec)) {
                res.headers_mut().insert(header::SET_COOKIE, v);
            }
            res
        }
        Login::Wrong { retry } => (StatusCode::UNAUTHORIZED, Json(json!({"reason": "wrong", "retry": retry}))).into_response(),
        Login::Locked { retry } => (StatusCode::TOO_MANY_REQUESTS, Json(json!({"reason": "locked", "retry": retry}))).into_response(),
        Login::Failed(why) => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"reason": "failed", "message": why}))).into_response(),
    }
}

/// `POST /auth/logout`: this browser's sign-in ends.
pub async fn logout(State(app): State<Shared>, headers: HeaderMap) -> Response {
    let token = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()).and_then(|c| cookie_value(c, COOKIE).map(str::to_string));
    if let (Some(t), Some(a)) = (token, app.web.auth.lock().clone()) {
        a.sessions().sign_out(&t);
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    if let Ok(v) = HeaderValue::from_str(&cookie_header("", secure(&headers))) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    res
}
