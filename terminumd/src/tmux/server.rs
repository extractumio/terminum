// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! One tmux server terminumd is the control client of: the private local one (`tmux -L
//! terminum`, started by terminumd with its own config, OQ-01) or a remote host's over ssh
//! (AC-81). Locally there is a control client per session (its panes' `%output` arrives only
//! there, AS-06) for notices, typing and screens, and a bulk client (`no-output`) for the
//! list, history pages and search, so a bulk read never queues ahead of an echo (OQ-02).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use serde::Serialize;
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot, Notify};

use super::control::{Answer, Control, Notice};

/// How a server is reached.
#[derive(Clone, Debug)]
pub enum Link {
    /// this Mac: `tmux -L <socket>`, started with `conf` when it is not running
    Local { tmux: PathBuf, socket: String, conf: PathBuf },
    /// a host: `ssh <args> -- <remote tmux command>` (args checked by hosts::recipe)
    Ssh { ssh: PathBuf, args: Vec<String> },
}

/// The problem said when tmux is not installed.
pub const NO_TMUX: &str = "tmux is needed for terminals: brew install tmux";

/// The session a host's control client holds (OQ-81).
pub const HOST_SESSION: &str = "terminum";

/// What the list says about a server.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Status {
    Attached,
    Connecting,
    Reconnecting { why: String, retry_s: u64 },
    Failed { why: String },
    Off,
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<String, Arc<Control>>,
    bulk: Option<Arc<Control>>,
    /// pane id -> its session, from the last list
    panes: HashMap<String, String>,
}

pub struct Server {
    /// "local" or the host's name
    pub name: String,
    /// the socket part of a pane key: the -L name locally, "default" on a host
    pub socket: String,
    pub link: Link,
    inner: Mutex<Inner>,
    wakes: Mutex<HashMap<String, Arc<Notify>>>,
    /// the list must be read again (a window, pane, session or layout changed)
    pub dirty: Arc<Notify>,
    /// a control client ended (the manager reconnects or reports)
    pub ended: Arc<Notify>,
    pub status: Mutex<Status>,
    /// bumped whenever control clients are replaced: shown panes capture anew
    pub generation: AtomicU64,
    /// `extended-keys` is on, as asked once per generation (AS-10)
    extended: Mutex<Option<(u64, bool)>>,
    /// a host's `HOME`, as read once per generation
    home: Mutex<Option<(u64, String)>>,
}

impl Server {
    /// `dirty` is shared by every server: one list refresh reads them all.
    pub fn new(name: &str, socket: &str, link: Link, dirty: Arc<Notify>) -> Arc<Server> {
        Arc::new(Server {
            name: name.to_string(),
            socket: socket.to_string(),
            link,
            inner: Mutex::default(),
            wakes: Mutex::default(),
            dirty,
            ended: Arc::new(Notify::new()),
            status: Mutex::new(Status::Connecting),
            generation: AtomicU64::new(0),
            extended: Mutex::new(None),
            home: Mutex::new(None),
        })
    }

    pub fn is_local(&self) -> bool {
        matches!(self.link, Link::Local { .. })
    }

    /// The key of pane `%N` here: `tmux:<host>:<socket>:%N`.
    pub fn key(&self, pane: &str) -> String {
        format!("tmux:{}:{}:{}", self.name, self.socket, pane)
    }

    /// The control client that holds `pane`'s session (typing, screens), else the bulk one.
    pub fn client(&self, pane: &str) -> Option<Arc<Control>> {
        let inner = self.inner.lock();
        inner.panes.get(pane).and_then(|s| inner.sessions.get(s)).filter(|c| c.alive()).cloned().or_else(|| inner.bulk.clone().filter(|c| c.alive()))
    }

    /// The client for the list, history pages and search.
    pub fn bulk(&self) -> Option<Arc<Control>> {
        let inner = self.inner.lock();
        inner.bulk.clone().filter(|c| c.alive()).or_else(|| inner.sessions.values().find(|c| c.alive()).cloned())
    }

    /// Runs one command on the bulk client.
    pub async fn run(&self, line: impl Into<String>) -> Answer {
        match self.bulk() {
            Some(c) => c.run(line).await,
            None => Err(format!("tmux on {} is not connected", self.label())),
        }
    }

    /// Commands on the control client of `pane`'s session, written at once, answered in order.
    pub async fn run_all_on(&self, pane: &str, lines: Vec<String>) -> Vec<Answer> {
        match self.client(pane) {
            Some(c) => c.run_all(lines).await,
            None => lines.iter().map(|_| Err(format!("tmux on {} is not connected", self.label()))).collect(),
        }
    }

    pub fn label(&self) -> &str {
        if self.is_local() { "this Mac" } else { &self.name }
    }

    /// The pane -> session map of the last list.
    pub fn set_panes(&self, panes: HashMap<String, String>) {
        self.inner.lock().panes = panes;
    }

    /// The pane `%N`'s wake-up: notified on each of its `%output` (a permit is kept when
    /// nobody waits, so an output between two captures is never missed).
    pub fn wake(&self, pane: &str) -> Arc<Notify> {
        self.wakes.lock().entry(pane.to_string()).or_insert_with(|| Arc::new(Notify::new())).clone()
    }

    /// Forgets wake-ups nobody holds any more.
    pub fn prune_wakes(&self) {
        self.wakes.lock().retain(|_, n| Arc::strong_count(n) > 1);
    }

    /// Whether any control client is alive.
    pub fn connected(&self) -> bool {
        let inner = self.inner.lock();
        inner.bulk.as_ref().is_some_and(|c| c.alive()) || inner.sessions.values().any(|c| c.alive())
    }

    /// Whether a control client got tmux's first answer (a host's ssh reached tmux).
    /// Whether the server has `extended-keys` on (terminum's tmux.conf sets it; a host's tmux
    /// is off by default): asked once per connection, an error counts as off.
    pub async fn extended_keys(&self, client: &Control) -> bool {
        let generation = self.generation.load(Ordering::Relaxed);
        if let Some((g, on)) = *self.extended.lock() && g == generation {
            return on;
        }
        let answer = client.run_all(vec!["show-options -sv extended-keys".into()]).await.pop();
        let on = answer.and_then(|a| a.ok()).and_then(|l| l.into_iter().next()).is_some_and(|v| v.trim() != "off");
        *self.extended.lock() = Some((generation, on));
        on
    }

    /// The server's `HOME` (for "~" in a host's paths), read once per connection.
    pub async fn home(&self) -> Option<String> {
        let generation = self.generation.load(Ordering::Relaxed);
        if let Some((g, h)) = self.home.lock().as_ref() && *g == generation {
            return Some(h.clone());
        }
        let lines = self.run("show-environment -g HOME").await.ok()?;
        let home = lines.into_iter().find_map(|l| l.strip_prefix("HOME=").map(str::to_string))?;
        *self.home.lock() = Some((generation, home.clone()));
        Some(home)
    }

    pub fn attached(&self) -> bool {
        let inner = self.inner.lock();
        inner.bulk.as_ref().is_some_and(|c| c.alive() && c.attached.load(Ordering::Relaxed))
    }

    /// Why the last control client ended (ssh's or tmux's last words).
    pub fn last_words(&self) -> Option<String> {
        let inner = self.inner.lock();
        inner.bulk.as_ref().and_then(|c| c.ended.lock().clone())
    }

    /// Signals every control client to end now, also those others still hold (terminumd
    /// stops: `exit` runs no destructors).
    pub fn kill_clients(&self) {
        let inner = self.inner.lock();
        for c in inner.sessions.values().chain(inner.bulk.iter()) {
            c.kill();
        }
    }

    /// Kills every control client (the host was switched off, or terminumd stops).
    pub fn disconnect(&self) {
        let mut inner = self.inner.lock();
        inner.sessions.clear();
        inner.bulk = None;
    }

    /// The sessions of a local server, asked without a control client: `None` when no
    /// server runs on the socket.
    pub async fn local_sessions(&self) -> Result<Option<Vec<String>>, String> {
        let Link::Local { tmux, socket, .. } = &self.link else { return Ok(Some(vec![HOST_SESSION.into()])) };
        let out = plain(tmux).args(["-L", socket, "list-sessions", "-F", "#{session_name}"]).output().await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound { NO_TMUX.to_string() } else { format!("tmux: {}: {e}", tmux.display()) }
        })?;
        if out.status.success() {
            return Ok(Some(String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).filter(|s| !s.is_empty()).collect()));
        }
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("no server running") || err.contains("error connecting") || err.contains("No such file") {
            return Ok(None);
        }
        Err(format!("tmux: {}", err.trim()))
    }

    /// Starts the local server with its config and one session `main` in the home folder
    /// (AC-01 first start); the user's default server is never touched.
    pub async fn start_local(&self, session: &str, dir: &str) -> Result<(), String> {
        let Link::Local { tmux, socket, conf } = &self.link else { return Err("not a local server".into()) };
        let out = plain(tmux)
            .args(["-L", socket, "-f"])
            .arg(conf)
            .args(["new-session", "-d", "-s", session, "-c", dir])
            .output()
            .await
            .map_err(|e| e.to_string())?;
        if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
    }

    /// Brings the control clients in line with `sessions`: one for each, the bulk one
    /// attached to the first; clients of sessions that are gone end.
    pub fn sync_clients(self: &Arc<Self>, sessions: &[String]) -> Result<(), String> {
        let mut spawned = false;
        let mut inner = self.inner.lock();
        inner.sessions.retain(|s, c| sessions.contains(s) && c.alive());
        if !self.is_local() {
            if inner.bulk.as_ref().is_some_and(|c| c.alive()) {
                return Ok(());
            }
            let c = Arc::new(self.spawn(HOST_SESSION, false)?);
            inner.sessions.insert(HOST_SESSION.into(), c.clone());
            inner.bulk = Some(c);
            drop(inner);
            self.generation.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        for s in sessions {
            if !inner.sessions.contains_key(s) {
                let c = self.spawn(s, false)?;
                inner.sessions.insert(s.clone(), Arc::new(c));
                spawned = true;
            }
        }
        if !inner.bulk.as_ref().is_some_and(|c| c.alive()) {
            inner.bulk = match sessions.first() {
                Some(first) => Some(Arc::new(self.spawn(first, true)?)),
                None => None,
            };
            spawned = true;
        }
        drop(inner);
        if spawned {
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// A control client for `session`; `bulk`: it takes no pane output.
    fn spawn(self: &Arc<Self>, session: &str, bulk: bool) -> Result<Control, String> {
        let flags = if bulk { "ignore-size,no-output" } else { "ignore-size" };
        let cmd = match &self.link {
            Link::Local { tmux, socket, .. } => {
                let mut c = plain(tmux);
                c.args(["-C", "-u", "-L", socket, "attach-session", "-f", flags, "-t"]).arg(format!("={session}"));
                c
            }
            Link::Ssh { ssh, args } => {
                let mut c = Command::new(ssh);
                c.args(["-T", "-o", "BatchMode=yes", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=2"]);
                c.args(crate::hosts::recipe::argv(args)).arg(host_command());
                c.env_remove("TMUX").env_remove("TMUX_PANE");
                c
            }
        };
        let (ntx, nrx) = mpsc::unbounded_channel();
        let (dtx, drx) = oneshot::channel();
        let control = Control::spawn(cmd, ntx, dtx).map_err(|e| format!("cannot start tmux: {e}"))?;
        let me = Arc::downgrade(self);
        tokio::spawn(notices(me.clone(), nrx));
        let session = session.to_string();
        tokio::spawn(async move {
            let why = drx.await.unwrap_or_default();
            if let Some(s) = me.upgrade() {
                tracing::info!(event = "client.end", server = %s.name, session, why);
                s.dirty.notify_one();
                s.ended.notify_one();
            }
        });
        if !self.is_local() {
            // a slow link must not back up behind output nobody reads (AS-81)
            drop(control.send("refresh-client -f pause-after=2".into()));
        }
        Ok(control)
    }
}

/// The remote command of a host's control client, quoted for the host's shell (AC-81):
/// the session `terminum`, created when missing; `-A` without `-D` leaves other clients.
pub fn host_command() -> String {
    format!("tmux -u -C new-session -A -f ignore-size -s {HOST_SESSION}")
}

/// A tmux command with a UTF-8 locale (launchd gives none: tmux would draw `_` for every
/// wide character) and without the TMUX of a terminal terminumd may run in (tests).
pub fn plain(tmux: &std::path::Path) -> Command {
    let mut c = Command::new(tmux);
    c.env_remove("TMUX").env_remove("TMUX_PANE");
    let utf8 = ["LC_ALL", "LC_CTYPE", "LANG"].iter().any(|k| std::env::var(k).is_ok_and(|v| v.to_uppercase().contains("UTF-8") || v.to_uppercase().contains("UTF8")));
    if !utf8 {
        c.env("LANG", "en_US.UTF-8");
    }
    c
}

async fn notices(server: std::sync::Weak<Server>, mut rx: mpsc::UnboundedReceiver<Notice>) {
    while let Some(n) = rx.recv().await {
        let Some(s) = server.upgrade() else { return };
        match n {
            Notice::Output(pane) => {
                let wake = s.wakes.lock().get(&pane).cloned();
                if let Some(w) = wake {
                    w.notify_one();
                }
            }
            Notice::Changed => {
                s.dirty.notify_one();
                // a resize or a zoom redraws shown panes: capture them now
                for w in s.wakes.lock().values() {
                    w.notify_one();
                }
            }
            Notice::Pause(pane) => {
                // flow control on a slow link: shown panes are captured anyway, so go on
                if let Some(c) = s.client(&pane) {
                    drop(c.send(format!("refresh-client -A '{pane}:continue'")));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_name_host_socket_and_pane() {
        let n = Arc::new(Notify::new());
        let s = Server::new("local", "terminum", Link::Local { tmux: "tmux".into(), socket: "terminum".into(), conf: "/dev/null".into() }, n.clone());
        assert_eq!(s.key("%3"), "tmux:local:terminum:%3");
        let h = Server::new("devbox", "default", Link::Ssh { ssh: "ssh".into(), args: vec!["devbox.example".into()] }, n);
        assert_eq!(h.key("%7"), "tmux:devbox:default:%7");
        assert!(!h.is_local());
    }

    #[test]
    fn the_host_command_creates_or_attaches_terminum() {
        assert_eq!(host_command(), "tmux -u -C new-session -A -f ignore-size -s terminum");
    }
}
