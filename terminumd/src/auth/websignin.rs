// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Checking the password (AC-09), as iterm-enhancer's `auth.py` did: brakes on guessing per
//! address and for all addresses together (an attacker on IPv6 can change address at will),
//! one check at a time. A right password gets a sign-in from `signin.rs`.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::web::PasswordHash;

/// Wrong passwords from one address before it has to wait.
pub const FREE_ATTEMPTS: u32 = 5;
/// Wrong passwords from all addresses before every address waits.
pub const FREE_ATTEMPTS_ALL: u32 = 20;
/// The longest wait, in seconds.
pub const MAX_LOCK_S: u64 = 300;
/// How long a wrong password holds the next check.
pub const WRONG_HOLD: Duration = Duration::from_secs(1);

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

/// Wrong passwords per address and in all, and until when they wait. "In all" is counted
/// apart for this Mac's own requests (a loopback address) and for forwarded ones, so guesses
/// from the tailnet never keep the Mac's own browser waiting, nor the reverse.
#[derive(Default)]
pub struct Brakes {
    failures: HashMap<String, (u32, Instant)>,
    /// [forwarded, this Mac's own]
    all: [Option<(u32, Instant)>; 2],
}

/// Which "in all" an address counts in: 1 for this Mac's own (loopback), 0 for the rest.
fn pool(address: &str) -> usize {
    usize::from(address.parse::<IpAddr>().is_ok_and(|ip| ip.to_canonical().is_loopback()))
}

impl Brakes {
    /// Seconds `address` waits before its next password is checked (rounded up).
    pub fn locked_for(&self, address: &str, now: Instant) -> u64 {
        let until = [self.failures.get(address).map(|f| f.1), self.all[pool(address)].map(|a| a.1)].into_iter().flatten().max();
        until.map_or(0, |u| u.saturating_duration_since(now).as_secs_f64().ceil() as u64)
    }

    pub fn failed(&mut self, address: &str, now: Instant) {
        if self.failures.len() > 1000 {
            // addresses come and go: keep the ones still waiting
            self.failures.retain(|_, f| f.1 > now);
        }
        let count = self.failures.get(address).map_or(0, |f| f.0) + 1;
        self.failures.insert(address.to_string(), (count, now + Duration::from_secs(lock_for(count, FREE_ATTEMPTS))));
        let all = &mut self.all[pool(address)];
        let total = all.map_or(0, |a| a.0) + 1;
        *all = Some((total, now + Duration::from_secs(lock_for(total, FREE_ATTEMPTS_ALL))));
    }

    pub fn passed(&mut self, address: &str) {
        self.failures.remove(address);
        self.all[pool(address)] = None;
    }
}

/// What a password got.
#[derive(Debug, PartialEq, Eq)]
pub enum Login {
    /// The right password: the caller makes the sign-in.
    Right,
    /// 401 `{"reason": "wrong", "retry": s}`.
    Wrong { retry: u64 },
    /// 429 `{"reason": "locked", "retry": s}`: not checked.
    Locked { retry: u64 },
    /// 503: the check itself failed.
    Failed(String),
}

/// The password check. Passwords are checked one at a time on a blocking
/// thread, and a wrong one holds the next for a second: at most one guess a second in all, so
/// a flood of sign-ins can neither guess in parallel past the brakes nor take the workers.
pub struct WebAuth {
    hash: PasswordHash,
    brakes: tokio::sync::Mutex<Brakes>,
    hold: Duration,
}

impl WebAuth {
    pub fn new(hash: PasswordHash) -> WebAuth {
        WebAuth { hash, brakes: tokio::sync::Mutex::new(Brakes::default()), hold: WRONG_HOLD }
    }

    /// `POST /auth/login {"password"}` from `address` (`client_address`). The check runs as a task
    /// of its own: a client that hangs up mid-check (its request's future dropped) neither
    /// escapes the brakes nor the hold, nor lets the next guess start before them.
    pub async fn login(self: &Arc<Self>, password: String, address: String) -> Login {
        let me = self.clone();
        match tokio::spawn(async move { me.check(password, &address).await }).await {
            Ok(login) => login,
            Err(e) => Login::Failed(format!("The password check failed: {e}")),
        }
    }

    async fn check(&self, password: String, address: &str) -> Login {
        let mut brakes = self.brakes.lock().await;
        let wait = brakes.locked_for(address, Instant::now());
        if wait > 0 {
            return Login::Locked { retry: wait };
        }
        let hash = self.hash.clone();
        match tokio::task::spawn_blocking(move || hash.verify(&password)).await {
            Ok(true) => {
                brakes.passed(address);
                Login::Right
            }
            Ok(false) => {
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
