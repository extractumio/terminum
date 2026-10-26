// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A host's agents reach terminumd through a link: `<app>/links/<hostid>.sock`, one listener per
//! host with agents on, that ssh forwards to the host. It serves only POST /hook and POST /agent,
//! for that host's server only: the host is the listener's, never the body's, and the body's
//! `socket_path` must be that server's own (a `TMUX_TMPDIR` server is also named "default").

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::StatusCode;
use axum::middleware::{from_fn, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::MethodRouter;
use axum::{Json, Router};
use parking_lot::Mutex;
use serde_json::Value;

use crate::app::Shared;
use crate::tmux::server::Server;

/// A link's body limit.
pub const BODY_MOST: usize = 64 * 1024;
/// A link's rate limit: a burst of 50 requests, then 20 a second; more is refused (429).
const BURST: f64 = 50.0;
const PER_S: f64 = 20.0;

/// Where an event came in: the CLI socket (this Mac's servers) or a host's link.
pub enum Via {
    Local,
    Link(Arc<Server>),
}

/// The server an event or tool call is for, None when it names none terminumd serves there.
/// Locally by `socket_path` when the body has it, else by the socket's name (hooks before it);
/// over a link only the link's host server, and only when `socket_path` is that server's.
pub async fn resolve(app: &Shared, via: Via, body: &Value) -> Option<Arc<Server>> {
    match via {
        Via::Link(server) => of_link(server, body).await,
        Via::Local => of_local(app.tmux.all().into_iter().filter(|s| s.is_local()).collect(), body).await,
    }
}

fn socket_path(body: &Value) -> Option<&str> {
    body["socket_path"].as_str().filter(|p| !p.is_empty())
}

/// The link's server when the body's `socket_path` is its own.
pub async fn of_link(server: Arc<Server>, body: &Value) -> Option<Arc<Server>> {
    let path = socket_path(body)?;
    (server.socket_path().await.as_deref() == Some(path)).then_some(server)
}

/// The local server with the body's `socket_path`, or without one its `socket` name.
pub async fn of_local(locals: Vec<Arc<Server>>, body: &Value) -> Option<Arc<Server>> {
    let Some(path) = socket_path(body) else { return locals.into_iter().find(|s| body["socket"].as_str() == Some(s.socket.as_str())) };
    for s in locals {
        if s.socket_path().await.as_deref() == Some(path) {
            return Some(s);
        }
    }
    None
}

/// Drops the hook state of `host`'s panes: their screens are read again.
pub fn forget(hooks: &crate::tmux::hookstate::Hooks, host: &str) {
    hooks.retain(&format!("tmux:{host}:"), &[]);
}

/// A short stable name for a host's link socket.
pub fn host_id(host: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(host.as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// A token bucket: `take` says whether one more request may go now.
pub struct Bucket(Mutex<(f64, Instant)>);

impl Bucket {
    pub fn new() -> Bucket {
        Bucket(Mutex::new((BURST, Instant::now())))
    }

    pub fn take(&self, now: Instant) -> bool {
        let mut b = self.0.lock();
        b.0 = (b.0 + now.saturating_duration_since(b.1).as_secs_f64() * PER_S).min(BURST);
        b.1 = now;
        let ok = b.0 >= 1.0;
        if ok {
            b.0 -= 1.0;
        }
        ok
    }
}

/// A link's routes: only these two, the body limit and the rate limit around them.
pub fn routes<S: Clone + Send + Sync + 'static>(hook: MethodRouter<S>, agent: MethodRouter<S>, bucket: Arc<Bucket>) -> Router<S> {
    Router::new()
        .route("/hook", hook)
        .route("/agent", agent)
        .layer(DefaultBodyLimit::max(BODY_MOST))
        .layer(from_fn(move |req: Request, next: Next| {
            let ok = bucket.take(Instant::now());
            async move { if ok { next.run(req).await } else { StatusCode::TOO_MANY_REQUESTS.into_response() } }
        }))
}

#[derive(Clone)]
struct Link {
    app: Shared,
    host: String,
}

impl Link {
    /// The link's host server (None once the host is gone).
    fn server(&self) -> Option<Arc<Server>> {
        self.app.tmux.all().into_iter().find(|s| !s.is_local() && s.name == self.host)
    }

    async fn resolved(&self, body: &Value) -> Option<Arc<Server>> {
        let server = self.server()?;
        resolve(&self.app, Via::Link(server), body).await
    }
}

async fn hook(State(l): State<Link>, Json(ev): Json<Value>) -> Response {
    let server = l.resolved(&ev).await;
    crate::sock::hook_on(l.app, server, ev).await
}

async fn agent(State(l): State<Link>, Json(body): Json<Value>) -> Json<Value> {
    let server = l.resolved(&body).await;
    crate::agentapi::agent_on(l.app, server, body).await
}

/// Each host's link: its socket path and the task serving it.
#[derive(Default)]
pub struct Links(Mutex<HashMap<String, (PathBuf, tokio::task::JoinHandle<()>)>>);

/// Unix socket paths end at 104 bytes on macOS, the terminating NUL included.
const SUN_PATH_MAX: usize = 103;

/// `<dir>/links/<hostid>.sock`.
pub fn path_in(dir: &Path, host: &str) -> PathBuf {
    dir.join("links").join(format!("{}.sock", host_id(host)))
}

// started and stopped by the host tunnel (the next round); until then only tests use it
#[allow(dead_code)]
impl Links {
    /// Listens on `host`'s link (again, when it was), in `dir` (the app folder).
    pub fn start(&self, app: &Shared, dir: &Path, host: &str) -> Result<PathBuf, String> {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let p = path_in(dir, host);
        if p.as_os_str().len() > SUN_PATH_MAX {
            return Err(format!("{} is too long for a socket (set TM_APP_DIR to a shorter folder)", p.display()));
        }
        let folder = dir.join("links");
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", folder.display()))?;
        self.stop_listening(host);
        let _ = std::fs::remove_file(&p);
        let l = tokio::net::UnixListener::bind(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).map_err(|e| format!("{}: {e}", p.display()))?;
        let link = Link { app: app.clone(), host: host.to_string() };
        let router = routes(axum::routing::post(hook), axum::routing::post(agent), Arc::new(Bucket::new())).with_state(link);
        let task = tokio::spawn(async move { drop(axum::serve(l, router).await) });
        self.0.lock().insert(host.to_string(), (p.clone(), task));
        Ok(p)
    }

    /// Ends `host`'s link and drops its panes' hook state: their screens are read again.
    pub fn stop(&self, app: &Shared, host: &str) {
        self.stop_listening(host);
        forget(&app.hooks, host);
        app.dirty.notify_one();
    }

    fn stop_listening(&self, host: &str) {
        if let Some((p, task)) = self.0.lock().remove(host) {
            task.abort();
            let _ = std::fs::remove_file(p);
        }
    }

    /// `host`'s link socket while it listens.
    pub fn path(&self, host: &str) -> Option<PathBuf> {
        self.0.lock().get(host).map(|(p, _)| p.clone())
    }
}

#[cfg(test)]
#[path = "links_tests.rs"]
mod tests;
