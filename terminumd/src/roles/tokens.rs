// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Tokens for callers outside a pane (`terminum-agents.md` AC-181): `terminum tokens add` prints one once, and
//! terminumd keeps its SHA-256 alone. A token says which role and project a program's `terminum tool` calls have
//! and names it in what it writes; it is no secret from a process of the same user, which can make one.

use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{OptionalExtension, Transaction};
use serde_json::{json, Value};

use super::peer::{self, Peer};
use crate::app::Shared;

use crate::auth::crypto::{random_hex, sha256_hex};
use crate::tasks::ops::{rows, sql};

/// What every token begins with, so a scan knows one.
pub const PREFIX: &str = "tmt_";
const NAME_MOST: usize = 40;
const MOST: i64 = 50;
const DAYS_MOST: i64 = 3650;

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub id: String,
    pub name: String,
    pub manager: bool,
    pub project: String,
}

impl Token {
    /// What stands for its pane where a caller's key is kept.
    pub fn key(&self) -> String {
        format!("token:{}", self.id)
    }
}

/// Makes token `name` of `role` for `project`, good for `days` (none: until removed): the token itself, said once.
pub fn add(tx: &Transaction, name: &str, role: &str, project: &str, days: Option<i64>, now: i64) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > NAME_MOST || name.chars().any(crate::termtext::stray) {
        return Err(format!("A token's name is 1 to {NAME_MOST} characters on one line."));
    }
    if !["manager", "worker"].contains(&role) {
        return Err("A token's role is manager or worker.".into());
    }
    if days.is_some_and(|d| !(1..=DAYS_MOST).contains(&d)) {
        return Err(format!("A token lasts 1 to {DAYS_MOST} days, or until removed."));
    }
    crate::tasks::ops::check_project(tx, project)?;
    let (count, taken): (i64, bool) = tx.query_row("SELECT count(*), coalesce(max(name = ?1), 0) FROM tokens", [name], |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
    if taken {
        return Err(format!("A token named {name} exists: remove it first."));
    }
    if count >= MOST {
        return Err(format!("{MOST} tokens at most: remove one first."));
    }
    let token = format!("{PREFIX}{}", random_hex::<24>());
    tx.execute("INSERT INTO tokens (id, name, hash, role, project, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
               rusqlite::params![random_hex::<8>(), name, sha256_hex(token.as_bytes()), role, project, now, days.map(|d| now + d * 86400)]).map_err(sql)?;
    Ok(token)
}

/// The tokens, never themselves: name, role, project, when made, when they end, when last used.
pub fn list(tx: &Transaction) -> Result<Vec<Value>, String> {
    rows(tx, "SELECT t.name, t.role, coalesce(p.name, t.project), t.created_at, t.expires_at, t.last_used_at FROM tokens t LEFT JOIN projects p ON p.id = t.project ORDER BY t.created_at, t.name", [],
         |r| Ok(json!({"name": r.get::<_, String>(0)?, "role": r.get::<_, String>(1)?, "project": r.get::<_, String>(2)?, "created_at": r.get::<_, i64>(3)?, "expires_at": r.get::<_, Option<i64>>(4)?, "last_used_at": r.get::<_, Option<i64>>(5)?})))
}

pub fn remove(tx: &Transaction, name: &str) -> Result<(), String> {
    match tx.execute("DELETE FROM tokens WHERE name = ?1", [name]).map_err(sql)? {
        0 => Err(format!("No token is named {name}.")),
        _ => Ok(()),
    }
}

/// Whose `token` is: found by its hash, in its time, its project still there; its use is written.
pub fn of(tx: &Transaction, token: &str, now: i64) -> Result<Token, String> {
    const NOT: &str = "terminum does not know this token: it was removed, has ended, or was never made (terminum tokens).";
    if !token.starts_with(PREFIX) || token.len() > 200 {
        return Err(NOT.into());
    }
    let found = tx.query_row("SELECT id, name, role, project FROM tokens WHERE hash = ?1 AND (expires_at IS NULL OR expires_at > ?2)", (sha256_hex(token.as_bytes()), now),
                             |r| Ok(Token { id: r.get(0)?, name: r.get(1)?, manager: r.get::<_, String>(2)? == "manager", project: r.get(3)? })).optional().map_err(sql)?.ok_or(NOT)?;
    crate::tasks::ops::check_project(tx, &found.project).map_err(|_| format!("Token {}'s project is gone: make a new one.", found.name))?;
    tx.execute("UPDATE tokens SET last_used_at = ?2 WHERE id = ?1", (&found.id, now)).map_err(sql)?;
    Ok(found)
}

/// `terminum tokens` on the private socket: the list, a new one, one removed. A token is made and removed from
/// outside every pane of terminum (AC-181): a session there, a worker or a manager, gives itself no other role.
pub async fn route(State(app): State<Shared>, seen: axum::http::Extensions, Json(body): Json<Value>) -> Response {
    let peer = seen.get::<ConnectInfo<Peer>>().map_or(Peer(None), |c| c.0);
    let action = body["action"].as_str().unwrap_or("").to_string();
    if action != "list" && peer::in_any_pane(&app, peer).await {
        tracing::warn!(event = "tokens.from_pane", action);
        return crate::http::err(StatusCode::FORBIDDEN, "in_pane", "Tokens are made and removed in a terminal outside terminum: a session in terminum cannot.");
    }
    // a project by its name, as the board shows it, else its id
    let named = body["project"].as_str().unwrap_or("").to_string();
    let project = app.projects.lock().list().iter().find(|p| p.name == named || p.id == named).map_or(named, |p| p.id.clone());
    let done = crate::tasks::api::store(&app, move |tx| {
        let text = |k: &str| body[k].as_str().unwrap_or("");
        match action.as_str() {
            "list" => Ok(json!({"tokens": list(tx)?})),
            "add" => Ok(json!({"token": add(tx, text("name"), text("role"), &project, body["days"].as_i64(), crate::tasks::ops::now())?})),
            "remove" => remove(tx, text("name")).map(|()| json!({"ok": true})),
            other => Err(format!("unknown action {other}")),
        }
    }).await;
    match done {
        Ok(v) => Json(v).into_response(),
        Err(e) => crate::http::err(StatusCode::CONFLICT, "tokens", e),
    }
}
