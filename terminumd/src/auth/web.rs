// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The password (AC-10): `web.json` in the app folder (0600) holds it as a salted
//! PBKDF2-SHA256 hash in iterm-enhancer's format (`config.py`), and the names a proxy may
//! forward besides `*.ts.net`. Until a password is set no browser signs in. Checking a password
//! (brakes) is `websignin.rs`; the sign-ins it makes are `signin.rs`'s.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::crypto::{hex, pbkdf2_sha256, same_bytes, unhex};
use super::store;

pub const FILE: &str = "web.json";
/// OWASP's count for PBKDF2-HMAC-SHA256; a hash of fewer (200,000 before 0.18.9) is made again
/// at the next right password (`WebConfig::strengthen`).
pub const ITERATIONS: u32 = 600_000;
pub const MIN_PASSWORD: usize = 8;
/// What a sign-in request may carry.
pub const SMALL_BODY: usize = 4096;
/// Iterations a stored hash may ask for: more is a broken (or hostile) file, not a password.
const MAX_ITERATIONS: u32 = 10_000_000;
/// Keys of the second listener of 0.5.0 and before (`terminum web on`): dropped when read.
const OBSOLETE: [&str; 4] = ["enabled", "host", "port", "history"];

/// web.json. Keys terminumd does not know are kept as they are.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    /// Names served besides 127.0.0.1, localhost and `*.ts.net` (a name another proxy forwards).
    pub allow_hosts: Vec<String>,
    pub password: Option<PasswordHash>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl WebConfig {
    /// The settings in `dir`, defaults for what is not set. Err: why the file cannot be read;
    /// no browser signs in and the page says so (the file is left for the user to fix).
    pub fn load(dir: &Path) -> Result<WebConfig, String> {
        match store::read(&dir.join(FILE))? {
            None => Ok(WebConfig::default()),
            Some(v) if v.is_object() => {
                let mut cfg: WebConfig = serde_json::from_value(v).map_err(|e| format!("{FILE} cannot be read: {e}"))?;
                for k in OBSOLETE {
                    cfg.other.remove(k);
                }
                Ok(cfg)
            }
            Some(_) => Err(format!("{FILE} is not a JSON object")),
        }
    }

    /// Atomically, readable by the user only.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        store::write_private(&dir.join(FILE), &serde_json::to_value(self).map_err(std::io::Error::other)?)
    }

    /// `password`, just found right against `old`, hashed again with ITERATIONS if `old` has
    /// fewer and web.json still holds it (not a password set meanwhile); whether it changed.
    pub fn strengthen(&mut self, old: &PasswordHash, password: &str) -> bool {
        if old.iterations >= ITERATIONS || self.password.as_ref() != Some(old) {
            return false;
        }
        // a password of 0.5.0 may be shorter than the minimum: it is kept, hashed as strong
        self.password = Some(PasswordHash::with_salt(password, &rand::random::<[u8; 16]>(), ITERATIONS));
        true
    }

    /// The password browsers sign in with: None until one is set, Err when the stored one is
    /// broken (no browser signs in then either).
    pub fn usable_password(&self) -> Result<Option<PasswordHash>, String> {
        match &self.password {
            None => Ok(None),
            Some(p) => p.check().map(|_| Some(p.clone())).map_err(|e| format!("{FILE} holds a broken password; set it again: terminum password ({e})")),
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

    /// The password is this one (a broken hash matches nothing). Takes about 0.3 s at 600,000
    /// iterations: an async caller runs it off its workers.
    pub fn verify(&self, password: &str) -> bool {
        self.parts().is_ok_and(|(salt, rounds, want)| same_bytes(&pbkdf2_sha256(password.as_bytes(), &salt, rounds, want.len()), &want))
    }
}

#[cfg(test)]
#[path = "web_tests.rs"]
mod tests;
