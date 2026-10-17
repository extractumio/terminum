// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The CLI's way in (AC-08, AC-09): `~/.terminum/state/terminumd.sock`, a Unix socket in the
//! private state folder, never a sign-in over TCP. `terminum status`, `open`, `web`,
//! `signout` and `hosts` talk to it; another program cannot, nor can it hold the TCP port
//! unnoticed: a port another program held means codes may have reached it.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::Shared;
use crate::http::err;

/// Unix socket paths end at 104 bytes on macOS, the terminating NUL included.
const SUN_PATH_MAX: usize = 103;

pub fn path(dir: &Path) -> PathBuf {
    dir.join("terminumd.sock")
}

fn fail(msg: String) -> ! {
    eprintln!("error: {msg}");
    tracing::error!(event = "start.failed", msg);
    std::process::exit(2);
}

/// The CLI socket; refuses to start when another terminumd answers on it, replaces a
/// socket left by one that is gone.
pub fn bind(dir: &Path) -> tokio::net::UnixListener {
    let p = path(dir);
    if p.as_os_str().len() > SUN_PATH_MAX {
        fail(format!("{} is too long for a socket (set TM_APP_DIR to a shorter folder)", p.display()));
    }
    if std::os::unix::net::UnixStream::connect(&p).is_ok() {
        fail(format!("another terminumd uses {}", dir.display()));
    }
    let _ = std::fs::remove_file(&p);
    let l = tokio::net::UnixListener::bind(&p).unwrap_or_else(|e| fail(format!("{}: {e}", p.display())));
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    l
}

/// The loopback port when launchd does not hold it (activation.rs); a previous terminumd
/// still shutting down is waited for (3 s). A port another program holds ends terminumd with
/// the message the log and `terminum status` show (AC-08).
pub async fn bind_port(dir: &Path, port: u16) -> tokio::net::TcpListener {
    for attempt in 0.. {
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => return l,
            Err(e) => {
                let ours = std::os::unix::net::UnixStream::connect(path(dir)).is_ok();
                if attempt == 15 {
                    if ours {
                        fail(format!("port {port}: another terminumd of this install runs ({e})"));
                    }
                    no_port(format!("port {port} is in use (set `port` in ~/.terminum/settings.json)"));
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
    unreachable!()
}

/// launchd's socket is unbound (another program held the port at bootstrap): says why once and
/// waits for `terminum restart`; exiting would only have launchd start it again on the same
/// unbound socket every 10 s.
pub fn unbound(msg: String) -> ! {
    eprintln!("error: {msg}");
    tracing::error!(event = "start.failed", msg);
    loop {
        std::thread::park();
    }
}

/// No port to serve on: says why in the log (`terminum status` reads it there), waits so
/// launchd, which starts terminumd again at once, tries every 10 s (AC-08), and exits.
pub fn no_port(msg: String) -> ! {
    eprintln!("error: {msg}");
    tracing::error!(event = "start.failed", msg);
    let wait = std::env::var("TM_PORT_WAIT_S").ok().and_then(|s| s.parse().ok()).unwrap_or(9);
    std::thread::sleep(std::time::Duration::from_secs(wait));
    std::process::exit(2);
}

pub fn routes(app: Shared) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/code", post(code))
        .route("/web/{action}", post(web))
        .route("/signout", post(signout))
        .route("/hosts", get(hosts))
        .route("/hosts/{action}", post(host_action))
        .route("/tmux", get(tmux_list))
        .route("/tmux/{action}", post(tmux_action))
        .route("/restore", post(restore))
        .with_state(app)
}

fn rss_mb() -> f64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: getrusage writes into a valid rusage
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    // ru_maxrss is in bytes on macOS, in KB on Linux
    let bytes = if cfg!(target_os = "macos") { usage.ru_maxrss as f64 } else { usage.ru_maxrss as f64 * 1024.0 };
    (bytes / 1048576.0 * 10.0).round() / 10.0
}

/// `terminum status`: what runs (AC-08).
async fn health(State(app): State<Shared>) -> Json<Value> {
    let servers: Vec<Value> = app.tmux.all().iter().map(|s| {
        let groups = app.layout.read().as_array().cloned().unwrap_or_default();
        let mine: Vec<&Value> = groups.iter().filter(|g| g["server"] == s.id().as_str()).collect();
        let name = if s.is_users() { s.label() } else { s.name.clone() };
        json!({"name": name, "socket": s.socket, "state": *s.status.lock(), "connected": s.connected(),
               "sessions": mine.iter().filter_map(|g| g["wid"].as_str()).collect::<Vec<_>>(),
               "panes": mine.iter().map(|g| g["items"].as_array().map_or(0, Vec::len)).sum::<usize>()})
    }).collect();
    let problem = app.tmux_problem.lock().clone();
    Json(json!({
        "build": crate::config::build_id(), "pid": std::process::id(), "uptime_s": app.started.elapsed().as_secs(),
        "port": app.cfg.port, "web": crate::webaccess::status(&app), "servers": servers,
        "hosts": crate::hosts::list(&app), "pages": app.pages.count(),
        "tmux": {"ok": problem.is_none(), "message": problem},
        "signins": app.signins.count(), "workspaces": app.store.len(),
        "denied_requests": app.denied.load(Ordering::Relaxed), "rss_mb": rss_mb(),
    }))
}

/// `terminum open`: a one-time code and the address that spends it (AC-09).
async fn code(State(app): State<Shared>) -> Json<Value> {
    let code = app.codes.issue();
    Json(json!({"code": code, "url": format!("http://127.0.0.1:{}/open/{code}", app.cfg.port)}))
}

#[derive(Deserialize, Default)]
struct Password {
    #[serde(default)]
    password: String,
}

/// `terminum web on|off|password` (AC-10).
async fn web(State(app): State<Shared>, UrlPath(action): UrlPath<String>, body: Option<Json<Password>>) -> Response {
    let result = match action.as_str() {
        "on" => crate::webaccess::switch(&app, true).await,
        "off" => crate::webaccess::switch(&app, false).await,
        "password" => crate::webaccess::set_password(&app, &body.map(|b| b.0.password).unwrap_or_default()).await,
        "status" => Ok(crate::webaccess::status(&app)),
        _ => return err(StatusCode::NOT_FOUND, "not_found", "web on, off, password or status"),
    };
    match result {
        Ok(v) => Json(json!({"ok": true, "web": v})).into_response(),
        Err(e) => err(StatusCode::CONFLICT, "web", e),
    }
}

/// `terminum signout`: every local page signs in again (AC-09).
async fn signout(State(app): State<Shared>) -> Response {
    let a = app.clone();
    match crate::http::blocking(move || a.signins.revoke_all()).await {
        Ok(Ok(())) => {
            for p in app.pages.all().into_iter().filter(|p| !p.web) {
                p.close();
            }
            Json(json!({"ok": true})).into_response()
        }
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, "not_saved", e.to_string()),
        Err(r) => r,
    }
}

async fn hosts(State(app): State<Shared>) -> Json<Value> {
    Json(json!({"hosts": crate::hosts::list(&app)}))
}

#[derive(Deserialize, Default)]
struct HostBody {
    #[serde(default)]
    name: String,
    #[serde(default)]
    recipe: String,
    #[serde(default)]
    files: bool,
}

/// `terminum hosts add|remove|enable|files-remove|off|on|retry` (AC-87).
async fn host_action(State(app): State<Shared>, UrlPath(action): UrlPath<String>, Json(b): Json<HostBody>) -> Response {
    let result = if action == "add" {
        crate::hosts::add(&app, &b.name, &b.recipe, b.files).await
    } else {
        crate::hosts::action(&app, &b.name, &action).await.map(|()| b.name)
    };
    match result {
        Ok(name) => {
            crate::manager::refresh(&app).await;
            Json(json!({"ok": true, "name": name, "hosts": crate::hosts::list(&app)})).into_response()
        }
        Err(e) => err(StatusCode::CONFLICT, "host", e),
    }
}

/// `terminum tmux`: each attached session, here and on hosts, and whether it runs (AC-89).
async fn tmux_list(State(app): State<Shared>) -> Json<Value> {
    use crate::attach::Place;
    let groups = app.layout.read().as_array().cloned().unwrap_or_default();
    let attached: Vec<Value> = crate::attach::every(&app).into_iter().map(|(place, s)| {
        let g = groups.iter().find(|g| !g["attached"].is_null() && g["server"] == place.id().as_str() && g["label"] == s.as_str());
        let state = match g {
            Some(g) if g["state"] == "absent" => g["why"].as_str().unwrap_or("not running").to_string(),
            Some(g) => format!("attached, {} panes", g["items"].as_array().map_or(0, Vec::len)),
            None => "not read yet".into(),
        };
        let (socket, host) = match &place {
            Place::Local(socket) => (Some(socket.clone()), None),
            Place::Host(h) => (None, Some(h.clone())),
        };
        json!({"session": s, "socket": socket, "host": host, "server": place.id(), "state": state})
    }).collect();
    Json(json!({"attached": attached}))
}

#[derive(Deserialize, Default)]
struct TmuxBody {
    #[serde(default)]
    session: String,
    #[serde(default)]
    socket: String,
    #[serde(default)]
    host: String,
}

/// `terminum tmux attach|detach <session> [--socket S | --host H]` (AC-89).
async fn tmux_action(State(app): State<Shared>, UrlPath(action): UrlPath<String>, Json(b): Json<TmuxBody>) -> Response {
    let id = match (b.host.as_str(), b.socket.as_str()) {
        ("", "") => String::new(),
        ("", socket) => format!("local:{socket}"),
        (host, _) => format!("{host}:default"),
    };
    let place = crate::attach::Place::parse(&app, &id);
    let result = match (action.as_str(), place) {
        (_, Err(e)) => Err(e),
        ("attach", Ok(p)) => crate::attach::add(&app, &p, std::slice::from_ref(&b.session)),
        ("detach", Ok(p)) => crate::attach::detach(&app, &p, &b.session).await,
        _ => Err(format!("unknown tmux action: {action}")),
    };
    match result {
        Ok(()) => {
            crate::manager::refresh(&app).await;
            Json(json!({"ok": true})).into_response()
        }
        Err(e) => err(StatusCode::CONFLICT, "tmux", e),
    }
}

/// "Restore again" from the CLI (AC-15).
async fn restore(State(app): State<Shared>) -> Response {
    match crate::restore::again(&app).await {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => err(StatusCode::CONFLICT, "restore", e),
    }
}
