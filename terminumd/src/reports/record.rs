// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A report as kept (terminum-reports.md AC-265): its ID in its project's count, its texts within their limits, its
//! author and source; its comments; its line (AC-271). Never a task: no status, no run of its own.

use rusqlite::{OptionalExtension, Transaction};

use crate::tasks::ops::{self, rows, sql};

pub const TITLE_MOST: usize = ops::TITLE_MOST;
pub const SUMMARY_MOST: usize = 500;
pub const BODY_MOST: usize = 1 << 20;
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

/// A report's type (AC-265), shown beside it in the Inbox: terminumd sets it from where the report came from and what
/// it holds, never from an agent's word (an agent's report is a report).
pub mod kind {
    pub const REPORT: &str = "report";
    pub const OK: &str = "ok";
    pub const ATTENTION: &str = "attention";
    pub const FAILED: &str = "failed";
    pub const INCIDENT: &str = "incident";
    pub const DIGEST: &str = "digest";
}

/// What a new report holds; `task`, `follows`: store keys; `kind`: empty for a report.
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
    pub kind: &'static str,
}

/// A summary on one line, within its limit (AC-265).
pub fn check_summary(summary: &str) -> Result<String, String> {
    let one = crate::termtext::clean(summary, false).split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > SUMMARY_MOST { Err(format!("Summary: at most {SUMMARY_MOST} characters")) } else { Ok(one) }
}

pub fn check_body(body: &str) -> Result<String, String> {
    let body = crate::termtext::clean(body, true);
    if body.len() > BODY_MOST { Err("Body: at most 1 MiB".into()) } else { Ok(body) }
}

/// The title, summary and body as kept, or the first limit passed, in words (AC-265): a title as a task's.
pub fn checked(title: &str, summary: &str, body: &str) -> Result<(String, String, String), String> {
    Ok((ops::check_title(title)?, check_summary(summary)?, check_body(body)?))
}

pub fn check_note(text: &str) -> Result<String, String> {
    let text = crate::termtext::clean(text, true);
    if text.trim().is_empty() {
        return Err("A comment needs words".into());
    }
    ops::check_comment(&text).map(|()| text)
}

/// Every write of a report moves the Inbox's revision, which every page hears with the tasks' marks (AC-267): one
/// row read, never a scan of the reports.
pub fn bump(tx: &Transaction) -> Result<(), String> {
    tx.execute("INSERT INTO meta (key, value) VALUES ('reports_rev', '1') ON CONFLICT (key) DO UPDATE SET value = CAST(value AS INTEGER) + 1", [])
        .map(drop).map_err(sql)
}

pub fn revision(tx: &Transaction) -> Result<String, String> {
    Ok(tx.query_row("SELECT value FROM meta WHERE key = 'reports_rev'", [], |r| r.get::<_, String>(0)).optional().map_err(sql)?.unwrap_or_default())
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

/// Keeps a new report: its key, its ID and its title as kept; the task it is for says so in a comment by its author
/// (AC-266).
pub fn create(tx: &Transaction, new: New, at: i64) -> Result<(String, String, String), String> {
    let (title, summary, body) = checked(&new.title, &new.summary, &new.body)?;
    let (slug, number) = crate::tasks::slugs::take_report(tx, &new.project)?;
    let key = crate::auth::crypto::random_hex::<16>();
    let line = line_due(tx, &new, at)?;
    // a critical report alerts once: by the incident it opens or joins, else by its own line (AC-276)
    let supervised = new.critical && new.incident.is_none();
    let kind = if new.kind.is_empty() { kind::REPORT } else { new.kind };
    tx.execute("INSERT INTO reports (id, project, number, slug, title, summary, body, critical, author, role, session_key, task, schedule, run,
            incident, follows, line, created_at, updated_at, kind) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?18, ?19)",
        rusqlite::params![key, new.project, number, slug, title, summary, body, new.critical, new.author, new.role, new.session, new.task,
            new.schedule, new.run, new.incident, new.follows, line && !supervised, at, kind]).map_err(sql)?;
    // a line of its task's history, never a comment: the task's result stays its agent's last comment
    if let Some(task) = &new.task {
        ops::log(tx, task, &new.author, "report", &format!("Report {slug}: {title}"), Some(&serde_json::json!({"report": slug})), at)?;
    }
    if supervised {
        alert(tx, &key, at, line)?;
    }
    bump(tx)?;
    Ok((key, slug, title))
}

/// Critical report `key`, its line down, alerts once (AC-276): by the incident its project's Supervisor opens or
/// joins for it; with none to alert (the Supervisor off, its detector ruled wrong, a breaker open), by its own line
/// when `line` says it is due.
pub fn alert(tx: &Transaction, key: &str, at: i64, line: bool) -> Result<(), String> {
    if crate::supervisor::report_rules::supervise(tx, key, at)?.is_none() && line {
        tx.execute("UPDATE reports SET line = 1 WHERE id = ?1", [key]).map_err(sql)?;
    }
    Ok(())
}

/// Incident `id` was acknowledged: the reports it alerts for, and its own, are read (AC-276).
pub fn read_of_incident(tx: &Transaction, id: i64, at: i64) -> Result<(), String> {
    if tx.execute("UPDATE reports SET read_at = ?2 WHERE (alerted_by = ?1 OR incident = ?1) AND read_at IS NULL AND deleted_at IS NULL",
        (id, at)).map_err(sql)? > 0
    {
        bump(tx)?;
    }
    Ok(())
}

/// The owner read report `key`: the incident alerting for it, or whose own report it is, is acknowledged by them;
/// its other reports stay as they are (AC-276).
pub fn acknowledge_by_reading(tx: &Transaction, key: &str, at: i64) -> Result<bool, String> {
    use crate::supervisor::incidents::{Action, act};
    let incident = tx.query_row("SELECT coalesce(alerted_by, incident) FROM reports WHERE id = ?1", [key],
        |r| r.get::<_, Option<i64>>(0)).map_err(sql)?;
    incident.map_or(Ok(false), |id| act(tx, id, Action::Acknowledge, at))
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
    bump(tx)?;
    Ok(id)
}

/// Read or unread again, deleted or back (AC-269), a line closed (AC-271): `column` set to `at`, or cleared with
/// none; whether that changed it.
pub fn stamp(tx: &Transaction, key: &str, column: Stamp, at: Option<i64>) -> Result<bool, String> {
    let col = match column { Stamp::Read => "read_at", Stamp::Deleted => "deleted_at", Stamp::LineSeen => "line_seen" };
    let changed = tx.execute(&format!("UPDATE reports SET {col} = ?1 WHERE id = ?2 AND {col} IS NOT ?1 AND (?1 IS NULL OR {col} IS NULL)"), (at, key))
        .map_err(sql)? > 0;
    if changed {
        bump(tx)?;
    }
    Ok(changed)
}

#[derive(Clone, Copy)]
pub enum Stamp { Read, Deleted, LineSeen }

/// The critical reports whose line stands (AC-271): unread, not closed, not deleted, newest first; ID, project, words.
pub fn lines(tx: &Transaction) -> Result<Vec<(String, String, String)>, String> {
    rows(tx, "SELECT slug, project, title FROM reports WHERE line = 1 AND read_at IS NULL AND line_seen IS NULL AND deleted_at IS NULL
        ORDER BY created_at DESC, rowid DESC", [], |r| {
        let id: String = r.get(0)?;
        let words = format!("{id} critical: {}", r.get::<_, String>(2)?);
        Ok((id, r.get(1)?, words))
    })
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
    if !gone.is_empty() {
        bump(tx)?;
    }
    Ok(files)
}
