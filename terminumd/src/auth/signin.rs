// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Local sign-ins (AC-09). Only the CLI, over the private Unix socket, gets a one-time code
//! (`Codes`); the browser opens `/open/<code>` and terminumd answers the page with a sign-in, a
//! 256-bit secret the page keeps in its origin's storage and sends as `X-TM-Session`.
//! `Signins` keeps at most 20 of them for 30 days in `local-sessions.json` (0600), never the
//! secret itself.
//!
//! The hello proof: another program may hold the port while terminumd is down, so before the
//! page sends its secret it asks `GET /api/hello?n=<nonce>&id=<id>` and checks the answer.
//! - `id` = the first 16 hex digits of SHA-256(secret): names the sign-in, reveals nothing of it.
//! - `proof_key` = HMAC-SHA256(secret, "tm-proof-key"), hex: derived by the page from its secret
//!   and kept by terminumd beside the sign-in's SHA-256 (terminumd never keeps the secret).
//! - the answer `{"proof"}` = hex(HMAC-SHA256(proof_key's 32 bytes, "tm-hello-v1:" + n)).
//!
//! The secret is 64 lowercase hex characters, keyed as their UTF-8 bytes. An unknown `id` is
//! 404 `unknown_signin`, with no proof.
//!
//! Only a terminumd that issued that sign-in holds its proof key, so another program cannot
//! answer, and the nonce (fresh per ask) keeps an old answer from being replayed. The fixed
//! prefix keeps the proof key from signing anything else. Reading the file gives neither the
//! secret (SHA-256 and HMAC do not invert) nor a sign-in: `check` wants the secret, whose
//! SHA-256 is stored; whoever reads the 0600 file is this user already.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::checks::Denial;
use super::crypto::{hex, hmac_sha256, random_hex, same, sha256_hex, unhex};
use super::store;

/// How long a code opens the page.
pub const CODE_TTL: Duration = Duration::from_secs(60);
/// Sign-ins kept; a new one beyond drops the oldest.
pub const MAX_SIGNINS: usize = 20;
/// How long a sign-in lasts, in seconds (30 days).
pub const SIGNIN_TTL_S: u64 = 30 * 86400;
pub const FILE: &str = "local-sessions.json";

/// What a code opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Redeem {
    Ok,
    /// Unknown, expired or used: the page answers 410 (`http::code_spent_page`).
    Spent,
}

/// One-time codes: 128 random bits, valid 60 s, spent on first use. Kept in memory only, as
/// their SHA-256, so a lookup's time says nothing of a code.
#[derive(Default)]
pub struct Codes {
    live: Mutex<HashMap<String, Instant>>,
}

impl Codes {
    pub fn new() -> Codes {
        Codes::default()
    }

    pub fn issue(&self) -> String {
        self.issue_at(Instant::now())
    }

    pub fn redeem(&self, code: &str) -> Redeem {
        self.redeem_at(code, Instant::now())
    }

    fn issue_at(&self, now: Instant) -> String {
        let code = random_hex::<16>();
        let mut live = self.live.lock();
        live.retain(|_, until| *until > now);
        live.insert(sha256_hex(code.as_bytes()), now + CODE_TTL);
        code
    }

    fn redeem_at(&self, code: &str, now: Instant) -> Redeem {
        match self.live.lock().remove(&sha256_hex(code.as_bytes())) {
            Some(until) if until > now => Redeem::Ok,
            _ => Redeem::Spent,
        }
    }
}

/// The sign-in's name in `/api/hello`: the first 16 hex digits of its SHA-256.
pub fn signin_id(secret: &str) -> String {
    sha256_hex(secret.as_bytes())[..16].to_string()
}

/// The key the hello proof is made with, derived from the secret (see the module doc).
pub fn proof_key(secret: &str) -> String {
    hex(&hmac_sha256(secret.as_bytes(), b"tm-proof-key"))
}

/// The hello proof for nonce `nonce` under `proof_key` (hex, as stored: keyed by its bytes).
pub fn proof(proof_key: &str, nonce: &str) -> String {
    let key = unhex(proof_key).unwrap_or_default();
    hex(&hmac_sha256(&key, format!("tm-hello-v1:{nonce}").as_bytes()))
}

/// A nonce is 16 to 128 hex digits.
pub fn valid_nonce(n: &str) -> bool {
    (16..=128).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Item {
    sha256: String,
    id: String,
    proof_key: String,
    /// Seconds since 1970.
    expires: u64,
    /// What made it (the CLI command, for `terminum signout` to list).
    by: String,
}

impl Item {
    fn well_formed(&self) -> bool {
        is_hex(&self.sha256, 64) && is_hex(&self.id, 16) && is_hex(&self.proof_key, 64)
    }
}

/// The local sign-ins, in memory and in `<app dir>/local-sessions.json`. `create` and
/// `revoke_all` write the file (blocking: an async caller runs them off its workers); `check`
/// and `hello` read memory only, never waiting for a write (`writer` serializes the writes;
/// `items` is held only to read or swap).
pub struct Signins {
    path: PathBuf,
    items: Mutex<Vec<Item>>,
    writer: Mutex<()>,
}

impl Signins {
    /// The sign-ins in `dir`'s file, the expired ones dropped. A file that cannot be read is
    /// set aside as `local-sessions.json.bak` and terminumd starts without sign-ins.
    pub fn load(dir: &Path) -> Signins {
        let path = dir.join(FILE);
        let items = store::read_or_set_aside(&path, |v| {
            serde_json::from_value::<Vec<Item>>(v.get("items").cloned().unwrap_or_default())
                .map_err(|e| format!("{FILE}: {e}"))
        });
        let now = store::unix_now();
        let mut items: Vec<Item> = items.unwrap_or_default().into_iter().filter(|i| i.well_formed() && i.expires > now).collect();
        keep_newest(&mut items, MAX_SIGNINS);
        Signins { path, items: Mutex::new(items), writer: Mutex::new(()) }
    }

    /// A new sign-in made by `by`: its secret (256 random bits, hex), which only the page keeps.
    /// The oldest go beyond 20. Not made when the file cannot be written.
    pub fn create(&self, by: &str) -> io::Result<String> {
        self.create_at(by, store::unix_now())
    }

    fn create_at(&self, by: &str, now: u64) -> io::Result<String> {
        let secret = random_hex::<32>();
        let item = Item {
            sha256: sha256_hex(secret.as_bytes()),
            id: signin_id(&secret),
            proof_key: proof_key(&secret),
            expires: now + SIGNIN_TTL_S,
            by: by.chars().filter(|c| !c.is_control()).take(100).collect(),
        };
        let _writing = self.writer.lock();
        let mut next: Vec<Item> = self.items.lock().iter().filter(|i| i.expires > now).cloned().collect();
        next.push(item);
        keep_newest(&mut next, MAX_SIGNINS);
        self.save(&next)?;
        *self.items.lock() = next;
        Ok(secret)
    }

    /// The secret is a live sign-in's.
    pub fn check(&self, secret: &str) -> bool {
        self.check_at(secret, store::unix_now())
    }

    fn check_at(&self, secret: &str, now: u64) -> bool {
        if secret.is_empty() {
            return false;
        }
        let digest = sha256_hex(secret.as_bytes());
        // every item compared, so the time says nothing of which one matched
        self.items.lock().iter().fold(false, |found, i| found | (same(&i.sha256, &digest) && i.expires > now))
    }

    /// `terminum signout`: every page signs in again. Err: the file still holds them, and so
    /// does memory (a restart would bring them back otherwise).
    pub fn revoke_all(&self) -> io::Result<()> {
        let _writing = self.writer.lock();
        self.save(&[])?;
        self.items.lock().clear();
        Ok(())
    }

    pub fn count(&self) -> usize {
        let now = store::unix_now();
        self.items.lock().iter().filter(|i| i.expires > now).count()
    }

    /// `GET /api/hello?n=<nonce>&id=<id>` (no sign-in): the proof for the sign-in named `id`.
    pub fn hello(&self, id: &str, nonce: &str) -> Result<String, Denial> {
        if !valid_nonce(nonce) {
            return Err(Denial::new(400, "bad_nonce", "n must be 16 to 128 hex digits"));
        }
        if !is_hex(id, 16) {
            return Err(Denial::new(400, "bad_id", "id must be 16 hex digits"));
        }
        let now = store::unix_now();
        let items = self.items.lock();
        let key = items.iter().find(|i| same(&i.id, &id.to_ascii_lowercase()) && i.expires > now).map(|i| i.proof_key.clone());
        key.map(|k| proof(&k, nonce))
            .ok_or_else(|| Denial::new(404, "unknown_signin", "Signed out: run terminum open"))
    }

    fn save(&self, items: &[Item]) -> io::Result<()> {
        store::write_private(&self.path, &json!({ "items": items }))
    }
}

/// Keeps the `max` sign-ins that expire last; of those expiring in the same second, the ones
/// added last (the sort is stable).
fn keep_newest(items: &mut Vec<Item>, max: usize) {
    items.sort_by_key(|i| i.expires);
    items.drain(..items.len().saturating_sub(max));
}

#[cfg(test)]
#[path = "signin_tests.rs"]
mod tests;
