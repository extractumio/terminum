// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Signing in over the network (AC-10), as iterm-enhancer's `auth.py` and `site.py` did: a
//! password for a sign-in cookie; brakes on guessing per address and for all addresses
//! together (an attacker on IPv6 can change address at will); sign-ins that outlive a restart
//! in `web-sessions.json` (0600) as their SHA-256 only, made under the password's salt so a new
//! password signs everyone out, and all forgotten when web access goes off.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::json;

use super::crypto::{random_hex, sha256_hex};
use super::store;
use super::web::PasswordHash;

pub const FILE: &str = "web-sessions.json";
pub const COOKIE: &str = "tm";
/// Wrong passwords from one address before it has to wait.
pub const FREE_ATTEMPTS: u32 = 5;
/// Wrong passwords from all addresses before every address waits.
pub const FREE_ATTEMPTS_ALL: u32 = 20;
/// The longest wait, in seconds.
pub const MAX_LOCK_S: u64 = 300;
/// How long a wrong password holds the next check.
pub const WRONG_HOLD: Duration = Duration::from_secs(1);
/// Remembered sign-ins; beyond, the ones that expire first go.
pub const MAX_SESSIONS: usize = 100;

/// Days a sign-in lasts: 30 behind HTTPS, 7 over plain HTTP (a cookie that can be read in transit).
pub fn session_days(secure: bool) -> u64 {
    if secure { 30 } else { 7 }
}

/// The browser's address. Behind a proxy on this Mac (`tailscale serve`) every request comes
/// from loopback; only then is X-Forwarded-For believed, and only its last entry, which that
/// proxy added (earlier ones are whatever the client sent).
pub fn client_address(peer: IpAddr, forwarded_for: Option<&str>) -> String {
    let peer = peer.to_canonical();
    let forwarded = forwarded_for.and_then(|f| f.rsplit(',').next()).map(str::trim).filter(|a| !a.is_empty());
    match forwarded {
        Some(last) if peer.is_loopback() => last.to_string(),
        _ => peer.to_string(),
    }
}

/// Seconds to wait after `count` wrong passwords: none before the `free`th, then 15 s doubling
/// up to 5 minutes.
pub fn lock_for(count: u32, free: u32) -> u64 {
    if count < free { 0 } else { MAX_LOCK_S.min(15u64.saturating_mul(1u64 << (count - free).min(32))) }
}

/// Wrong passwords per address and in all, and until when they wait.
#[derive(Default)]
pub struct Brakes {
    failures: HashMap<String, (u32, Instant)>,
    all: Option<(u32, Instant)>,
}

impl Brakes {
    /// Seconds `address` waits before its next password is checked (rounded up).
    pub fn locked_for(&self, address: &str, now: Instant) -> u64 {
        let until = [self.failures.get(address).map(|f| f.1), self.all.map(|a| a.1)].into_iter().flatten().max();
        until.map_or(0, |u| u.saturating_duration_since(now).as_secs_f64().ceil() as u64)
    }

    pub fn failed(&mut self, address: &str, now: Instant) {
        if self.failures.len() > 1000 {
            // addresses come and go: keep the ones still waiting
            self.failures.retain(|_, f| f.1 > now);
        }
        let count = self.failures.get(address).map_or(0, |f| f.0) + 1;
        self.failures.insert(address.to_string(), (count, now + Duration::from_secs(lock_for(count, FREE_ATTEMPTS))));
        let total = self.all.map_or(0, |a| a.0) + 1;
        self.all = Some((total, now + Duration::from_secs(lock_for(total, FREE_ATTEMPTS_ALL))));
    }

    pub fn passed(&mut self, address: &str) {
        self.failures.remove(address);
        self.all = None;
    }
}

struct Sessions {
    /// SHA-256 of a token -> expiry, seconds since 1970.
    items: HashMap<String, u64>,
    closed: bool,
}

/// The web listener's sign-ins. `create`, `sign_out` and `close` write the file (blocking);
/// `check` reads memory only, never waiting for a write (`writer` serializes the writes).
pub struct WebSessions {
    path: PathBuf,
    salt: String,
    state: Mutex<Sessions>,
    writer: Mutex<()>,
}

impl WebSessions {
    /// The sign-ins kept in `dir` for the password whose salt is `salt`; a file of another
    /// password holds none, one that cannot be read is set aside.
    pub fn load(dir: &Path, salt: &str) -> WebSessions {
        let path = dir.join(FILE);
        let now = store::unix_now();
        let items = store::read_or_set_aside(&path, |v| {
            let sessions = v.get("sessions").and_then(|s| s.as_object()).ok_or(format!("{FILE}: no sessions"))?;
            let mut items = HashMap::new();
            for (k, e) in sessions {
                items.insert(k.clone(), e.as_f64().ok_or(format!("{FILE}: an expiry is not a number"))? as u64);
            }
            let same_password = v.get("salt").and_then(|s| s.as_str()) == Some(salt);
            Ok(if same_password { items } else { HashMap::new() })
        });
        let items = items.unwrap_or_default().into_iter().filter(|(_, e)| *e > now).collect();
        WebSessions { path, salt: salt.to_string(), state: Mutex::new(Sessions { items, closed: false }), writer: Mutex::new(()) }
    }

    /// A new sign-in's token (256 random bits, hex). Err: web access went off meanwhile, or the
    /// file cannot be written (then there is no sign-in).
    pub fn create(&self, secure: bool) -> Result<String, String> {
        self.create_at(secure, store::unix_now())
    }

    fn create_at(&self, secure: bool, now: u64) -> Result<String, String> {
        let _writing = self.writer.lock();
        let mut kept: Vec<(u64, String)> = {
            let s = self.state.lock();
            if s.closed {
                return Err("Web access is off".into());
            }
            s.items.iter().filter(|(_, e)| **e > now).map(|(k, e)| (*e, k.clone())).collect()
        };
        kept.sort_unstable_by(|a, b| b.cmp(a));
        kept.truncate(MAX_SESSIONS - 1);
        let token = random_hex::<32>();
        let mut items: HashMap<String, u64> = kept.into_iter().map(|(e, k)| (k, e)).collect();
        items.insert(sha256_hex(token.as_bytes()), now + session_days(secure) * 86400);
        self.save(&items).map_err(|e| format!("The sign-in was not saved: {e}"))?;
        let mut s = self.state.lock();
        if s.closed {
            // closed while writing: close() removes the file once this write is done
            return Err("Web access is off".into());
        }
        s.items = items;
        Ok(token)
    }

    pub fn check(&self, token: &str) -> bool {
        !token.is_empty() && self.state.lock().items.get(&sha256_hex(token.as_bytes())).is_some_and(|e| *e > store::unix_now())
    }

    /// Signed out at once in memory, then in the file.
    pub fn sign_out(&self, token: &str) {
        let _writing = self.writer.lock();
        let items = {
            let mut s = self.state.lock();
            if s.closed || s.items.remove(&sha256_hex(token.as_bytes())).is_none() {
                return;
            }
            s.items.clone()
        };
        if let Err(e) = self.save(&items) {
            tracing::warn!(event = "web.signout_unsaved", error = %e);
        }
    }

    /// Web access goes off: every browser is signed out (a lost phone) and nothing is written
    /// after, not even by a password check still running.
    pub fn close(&self) {
        {
            let mut s = self.state.lock();
            s.closed = true;
            s.items.clear();
        }
        let _writing = self.writer.lock();
        forget(self.path.parent().unwrap_or(Path::new(".")));
    }

    fn save(&self, items: &HashMap<String, u64>) -> std::io::Result<()> {
        store::write_private(&self.path, &json!({"salt": self.salt, "sessions": items}))
    }
}

/// Deletes the remembered sign-ins in `dir` (web access switched off while no listener runs).
pub fn forget(dir: &Path) {
    if let Err(e) = std::fs::remove_file(dir.join(FILE))
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(event = "web.sessions_not_removed", error = %e);
    }
}

/// The sign-in cookie; an empty token clears it. HttpOnly: no script reads it; Strict: no
/// other site's page sends it.
pub fn cookie_header(token: &str, secure: bool) -> String {
    let age = if token.is_empty() { 0 } else { session_days(secure) * 86400 };
    format!("{COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={age}{}", if secure { "; Secure" } else { "" })
}

/// The value of cookie `name` in a Cookie header.
pub fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').filter_map(|c| c.trim().split_once('=')).find(|(n, _)| *n == name).map(|(_, v)| v)
}

/// What a password got.
#[derive(Debug, PartialEq, Eq)]
pub enum Login {
    /// 204 with `Set-Cookie: cookie_header(token, secure)`.
    SignedIn(String),
    /// 401 `{"reason": "wrong", "retry": s}`.
    Wrong { retry: u64 },
    /// 429 `{"reason": "locked", "retry": s}`: not checked.
    Locked { retry: u64 },
    /// 503: web access went off, or the sign-in could not be saved.
    Failed(String),
}

/// The web listener's password check. Passwords are checked one at a time on a blocking
/// thread, and a wrong one holds the next for a second: at most one guess a second in all, so
/// a flood of sign-ins can neither guess in parallel past the brakes nor take the workers.
pub struct WebAuth {
    hash: PasswordHash,
    sessions: Arc<WebSessions>,
    brakes: tokio::sync::Mutex<Brakes>,
    hold: Duration,
}

impl WebAuth {
    pub fn new(hash: PasswordHash, sessions: Arc<WebSessions>) -> WebAuth {
        WebAuth { hash, sessions, brakes: tokio::sync::Mutex::new(Brakes::default()), hold: WRONG_HOLD }
    }

    pub fn sessions(&self) -> &WebSessions {
        &self.sessions
    }

    /// `POST /login {"password"}` from `address` (`client_address`). The check runs as a task
    /// of its own: a client that hangs up mid-check (its request's future dropped) neither
    /// escapes the brakes nor the hold, nor lets the next guess start before them.
    pub async fn login(self: &Arc<Self>, password: String, address: String, secure: bool) -> Login {
        let me = self.clone();
        match tokio::spawn(async move { me.check(password, &address, secure).await }).await {
            Ok(login) => login,
            Err(e) => Login::Failed(format!("The password check failed: {e}")),
        }
    }

    async fn check(&self, password: String, address: &str, secure: bool) -> Login {
        let mut brakes = self.brakes.lock().await;
        let wait = brakes.locked_for(address, Instant::now());
        if wait > 0 {
            return Login::Locked { retry: wait };
        }
        let (hash, sessions) = (self.hash.clone(), self.sessions.clone());
        let checked = tokio::task::spawn_blocking(move || hash.verify(&password).then(|| sessions.create(secure))).await;
        match checked {
            Ok(Some(made)) => {
                brakes.passed(address);
                made.map_or_else(Login::Failed, Login::SignedIn)
            }
            Ok(None) => {
                brakes.failed(address, Instant::now());
                tokio::time::sleep(self.hold).await;
                Login::Wrong { retry: brakes.locked_for(address, Instant::now()) }
            }
            Err(e) => Login::Failed(format!("The password check failed: {e}")),
        }
    }
}

#[cfg(test)]
#[path = "websignin_tests.rs"]
mod tests;
