// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A report as kept (terminum-reports.md AC-265): its ID in its project's count, its texts within their limits, its
//! author and source; its comments; its line (AC-271). Never a task: no status, no run of its own.

use rusqlite::{OptionalExtension, Transaction};

use crate::tasks::ops::{self, rows, sql};

pub const TITLE_MOST: usize = 200;
pub const SUMMARY_MOST: usize = 500;
pub const BODY_MOST: usize = 1 << 20;
pub const NOTE_MOST: usize = ops::COMMENT_MOST;
pub const FILES_MOST: i64 = 50;
/// A worker's session makes at most this many reports an hour, and raises at most one critical line an hour (AC-266).
pub const SESSION_HOURLY: i64 = 10;
const HOUR: i64 = 3600;

/// Who made a report, as its row keeps it: the role of the caller at the call (terminum-agents.md AC-174).
pub mod role {
    pub const WORKER: &str = "worker";
    pub const MANAGER: &str = "manager";
    pub const SUPERVISOR: &str = "supervisor";
    /// terminumd's own: a routine's result, a digest, a moved report
    pub const TERMINUM: &str = "terminum";
}

/// What a new report holds; `task`, `follows`: store keys.
#[derive(Default)]
pub struct New {
    pub project: String,
    pub title: String,
    pub summary: String,
    pub body: String,
    pub critical: bool,
    pub author: String,
    pub role: &'static str,
    pub session: Option<String>,
    pub task: Option<String>,
    pub schedule: Option<String>,
    pub run: Option<String>,
    pub incident: Option<i64>,
    pub follows: Option<String>,
}

/// A text's control characters but tab and newline dropped, as a task's are; a title on one line.
fn clean(text: &str, one_line: bool) -> String {
    let kept: String = text.chars().filter(|c| !crate::termtext::control(*c) || (!one_line && matches!(c, '\n' | '\t'))).collect();
    if one_line { kept.split_whitespace().collect::<Vec<_>>().join(" ") } else { kept }
}

/// The title, summary and body as kept, or the first limit passed, in words (AC-265).
pub fn checked(title: &str, summary: &str, body: &str) -> Result<(String, String, String), String> {
    let (title, summary, body) = (clean(title, true), clean(summary, true), clean(body, false));
    if title.is_empty() {
        return Err("Title: 1 to 200 characters".into());
    }
    if title.chars().count() > TITLE_MOST {
        return Err(format!("Title: at most {TITLE_MOST} characters"));
    }
    if summary.chars().count() > SUMMARY_MOST {
        return Err(format!("Summary: at most {SUMMARY_MOST} characters"));
    }
    if body.len() > BODY_MOST {
        return Err("Body: at most 1 MiB".into());
    }
    Ok((title, summary, body))
}

pub fn check_note(text: &str) -> Result<String, String> {
    let text = clean(text, false);
    if text.trim().is_empty() {
        return Err("A comment needs words".into());
    }
    if text.len() > NOTE_MOST { Err("Comment: at most 256 KiB".into()) } else { Ok(text) }
}

/// Whether a new critical report of `session` raises its line: one an hour per worker's session (AC-266, AC-271).
fn line_due(tx: &Transaction, new: &New, at: i64) -> Result<bool, String> {
    if !new.critical || new.incident.is_some() {
        return Ok(false);                              // an incident alerts through itself (AC-271)
    }
    let Some(session) = new.session.as_deref().filter(|_| new.role == role::WORKER) else { return Ok(true) };
    let had: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM reports WHERE session_key = ?1 AND line = 1 AND created_at > ?2)",
        (session, at - HOUR), |r| r.get(0)).map_err(sql)?;
    Ok(!had)
}

/// A worker's session at its hourly count of reports (AC-266).
pub fn session_full(tx: &Transaction, session: &str, at: i64) -> Result<bool, String> {
    let made: i64 = tx.query_row("SELECT count(*) FROM reports WHERE session_key = ?1 AND role = 'worker' AND created_at > ?2",
        (session, at - HOUR), |r| r.get(0)).map_err(sql)?;
    Ok(made >= SESSION_HOURLY)
}

/// Keeps a new report: its key and its ID.
pub fn create(tx: &Transaction, new: New, at: i64) -> Result<(String, String), String> {
    let (title, summary, body) = checked(&new.title, &new.summary, &new.body)?;
    let (slug, number) = crate::tasks::slugs::take_report(tx, &new.project)?;
    let key = crate::auth::crypto::random_hex::<16>();
    let line = line_due(tx, &new, at)?;
    tx.execute("INSERT INTO reports (id, project, number, slug, title, summary, body, critical, author, role, session_key, task, schedule, run,
            incident, follows, line, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?18)",
        rusqlite::params![key, new.project, number, slug, title, summary, body, new.critical, new.author, new.role, new.session, new.task,
            new.schedule, new.run, new.incident, new.follows, line, at]).map_err(sql)?;
    Ok((key, slug))
}

/// A report's key, from its ID; a deleted one too.
pub fn key_of(tx: &Transaction, id: &str) -> Result<String, String> {
    tx.query_row("SELECT id FROM reports WHERE slug = upper(trim(?1))", [id], |r| r.get(0)).optional().map_err(sql)?
        .ok_or_else(|| format!("No report {}.", id.trim()))
}

/// What the rules read of a report.
pub struct Head {
    pub key: String,
    pub id: String,
    pub project: String,
    pub task: Option<String>,
    pub session: Option<String>,
    pub deleted: bool,
}

pub fn head(tx: &Transaction, id: &str) -> Result<Head, String> {
    let key = key_of(tx, id)?;
    tx.query_row("SELECT id, slug, project, task, session_key, deleted_at IS NOT NULL FROM reports WHERE id = ?1", [&key], |r| Ok(Head {
        key: r.get(0)?, id: r.get(1)?, project: r.get(2)?, task: r.get(3)?, session: r.get(4)?, deleted: r.get(5)?,
    })).map_err(sql)
}

/// A comment, a reply or a change's line below report `key`, by `author`: its id. The report's changed time moves.
pub fn note(tx: &Transaction, key: &str, author: &str, kind: &str, text: &str, at: i64) -> Result<String, String> {
    let id = crate::auth::crypto::random_hex::<16>();
    tx.execute("INSERT INTO report_notes (id, report, at, author, kind, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", (&id, key, at, author, kind, text)).map_err(sql)?;
    tx.execute("UPDATE reports SET updated_at = ?1 WHERE id = ?2", (at, key)).map_err(sql)?;
    Ok(id)
}

/// Read or unread again (AC-269): whether that changed it. Read closes its line too.
pub fn set_read(tx: &Transaction, key: &str, read: bool, at: i64) -> Result<bool, String> {
    let q = if read { "UPDATE reports SET read_at = ?1 WHERE id = ?2 AND read_at IS NULL" } else { "UPDATE reports SET read_at = NULL WHERE id = ?2 AND read_at IS NOT NULL AND ?1 = ?1" };
    tx.execute(q, (at, key)).map(|n| n > 0).map_err(sql)
}

/// Deleted, or back (AC-269): whether that changed it.
pub fn set_deleted(tx: &Transaction, key: &str, deleted: bool, at: i64) -> Result<bool, String> {
    let q = if deleted { "UPDATE reports SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL" } else { "UPDATE reports SET deleted_at = NULL WHERE id = ?2 AND deleted_at IS NOT NULL AND ?1 = ?1" };
    tx.execute(q, (at, key)).map(|n| n > 0).map_err(sql)
}

/// The human closed the report's line (AC-271).
pub fn line_closed(tx: &Transaction, key: &str, at: i64) -> Result<bool, String> {
    tx.execute("UPDATE reports SET line_seen = ?1 WHERE id = ?2 AND line = 1 AND line_seen IS NULL", (at, key)).map(|n| n > 0).map_err(sql)
}

/// Deleted reports kept this long, then purged with their notes and files (AC-269).
pub const KEPT_DELETED: i64 = 30 * 24 * 3600;

/// Reports deleted `KEPT_DELETED` ago go, their notes and file rows with them; the file ids, whose bytes go next.
pub fn purge(tx: &Transaction, at: i64) -> Result<Vec<String>, String> {
    let gone: Vec<String> = rows(tx, "SELECT id FROM reports WHERE deleted_at < ?1", [at - KEPT_DELETED], |r| r.get(0))?;
    let mut files = Vec::new();
    for key in &gone {
        files.extend(rows(tx, "SELECT id FROM files WHERE report = ?1", [key], |r| r.get::<_, String>(0))?);
        tx.execute("DELETE FROM files WHERE report = ?1", [key]).map_err(sql)?;
        tx.execute("UPDATE reports SET follows = NULL WHERE follows = ?1", [key]).map_err(sql)?;
        tx.execute("DELETE FROM reports WHERE id = ?1", [key]).map_err(sql)?;
    }
    Ok(files)
}
