// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Web access (AC-10): `web.json` in the app folder (0600), off unless switched on, with the
//! password kept as a salted PBKDF2-SHA256 hash in iterm-enhancer's format (`config.py`), and
//! the limits a request has before it is signed in. The sign-in itself (brakes, sessions, the
//! cookie) is `websignin.rs`.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::crypto::{hex, pbkdf2_sha256, same_bytes, unhex};
use super::store;

pub const FILE: &str = "web.json";
pub const DEFAULT_PORT: u16 = 8765;
pub const ITERATIONS: u32 = 200_000;
pub const MIN_PASSWORD: usize = 8;
/// What a request may carry before it is signed in (a sign-in, a JSON action).
pub const SMALL_BODY: usize = 4096;
/// Time for a request's head (and a small body) to arrive.
pub const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
/// Connections open at once on the web listener; more are refused.
pub const MAX_CONNECTIONS: usize = 64;
/// Iterations a stored hash may ask for: more is a broken (or hostile) file, not a password.
const MAX_ITERATIONS: u32 = 10_000_000;

/// web.json. Keys terminumd does not know are kept as they are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    pub enabled: bool,
    pub port: u16,
    pub host: String,
    /// Lines a network page may load.
    pub history: u32,
    /// Names served besides this Mac's own (a name a proxy forwards, say).
    pub allow_hosts: Vec<String>,
    pub password: Option<PasswordHash>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl Default for WebConfig {
    fn default() -> WebConfig {
        WebConfig {
            enabled: false,
            port: DEFAULT_PORT,
            host: "0.0.0.0".into(),
            history: 10_000,
            allow_hosts: Vec::new(),
            password: None,
            other: Map::new(),
        }
    }
}

impl WebConfig {
    /// The settings in `dir`, defaults for what is not set. Err: why the file cannot be read;
    /// the caller keeps web access off and says so (the file is left for the user to fix).
    pub fn load(dir: &Path) -> Result<WebConfig, String> {
        match store::read(&dir.join(FILE))? {
            None => Ok(WebConfig::default()),
            Some(v) if v.is_object() => serde_json::from_value(v).map_err(|e| format!("{FILE} cannot be read: {e}")),
            Some(_) => Err(format!("{FILE} is not a JSON object")),
        }
    }

    /// Atomically, readable by the user only.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        store::write_private(&dir.join(FILE), &serde_json::to_value(self).map_err(std::io::Error::other)?)
    }

    /// Whether the listener should run, or why not: on, with a password that can be checked.
    pub fn ready(&self) -> Result<bool, String> {
        if !self.enabled {
            return Ok(false);
        }
        match &self.password {
            None => Err("Set a password first: terminum web password".into()),
            Some(p) => p.check().map(|_| true).map_err(|e| format!("{FILE} holds a broken password; set it again: terminum web password ({e})")),
        }
    }
}

/// A salted PBKDF2-SHA256 hash: `{"salt": hex, "iterations": n, "hash": hex}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordHash {
    pub salt: String,
    pub iterations: u32,
    pub hash: String,
}

impl PasswordHash {
    /// The hash of a new password (at least 8 characters) with a fresh 128-bit salt.
    pub fn new(password: &str) -> Result<PasswordHash, String> {
        if password.chars().count() < MIN_PASSWORD {
            return Err(format!("The password must be at least {MIN_PASSWORD} characters"));
        }
        Ok(PasswordHash::with_salt(password, &rand::random::<[u8; 16]>(), ITERATIONS))
    }

    fn with_salt(password: &str, salt: &[u8], iterations: u32) -> PasswordHash {
        PasswordHash { salt: hex(salt), iterations, hash: hex(&pbkdf2_sha256(password.as_bytes(), salt, iterations, 32)) }
    }

    /// The parts, or why they cannot be used.
    fn parts(&self) -> Result<(Vec<u8>, u32, Vec<u8>), String> {
        let salt = unhex(&self.salt).filter(|s| !s.is_empty()).ok_or("the salt is not hex")?;
        let want = unhex(&self.hash).filter(|h| !h.is_empty() && h.len() <= 64).ok_or("the hash is not hex")?;
        if !(1..=MAX_ITERATIONS).contains(&self.iterations) {
            return Err(format!("iterations must be 1 to {MAX_ITERATIONS}"));
        }
        Ok((salt, self.iterations, want))
    }

    pub fn check(&self) -> Result<(), String> {
        self.parts().map(|_| ())
    }

    /// The password is this one (a broken hash matches nothing). Takes about 0.1 s at 200,000
    /// iterations: an async caller runs it off its workers.
    pub fn verify(&self, password: &str) -> bool {
        self.parts().is_ok_and(|(salt, rounds, want)| same_bytes(&pbkdf2_sha256(password.as_bytes(), &salt, rounds, want.len()), &want))
    }
}

#[cfg(test)]
#[path = "web_tests.rs"]
mod tests;
