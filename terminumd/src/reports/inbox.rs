// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What the Inbox shows (terminum-reports.md AC-267, AC-268): reports newest first, critical unread ones first, paged
//! by 100; the unread counts by project, schedule and session in the same answer; one report whole.

use rusqlite::{Row, Transaction};
use serde_json::{json, Value};

use crate::tasks::ops::{rows, sql};

pub const PAGE: i64 = 100;

/// Which reports a page asks for.
#[derive(Default)]
pub struct Filter {
    pub project: Option<String>,
    pub deleted: bool,
    /// below this number of the order: the next page
    pub before: Option<i64>,
    pub session: Option<String>,
    pub limit: Option<i64>,
    pub unread_only: bool,
}

/// A report's row, as every list shows it: columns `r.` of `reports r`.
const ROW: &str = "SELECT r.slug, r.project, r.title, r.summary, r.critical, r.read_at IS NOT NULL, (SELECT t.slug FROM tasks t WHERE t.id = r.task),
    (SELECT 'SCH-' || s.number FROM schedules s WHERE s.id = r.schedule), (SELECT s.title FROM schedules s WHERE s.id = r.schedule),
    (SELECT u.name FROM users u WHERE u.id = r.author), r.role, r.session_key, r.created_at, r.updated_at, r.deleted_at, r.incident, r.number,
    p.name, p.color
    FROM reports r JOIN projects p ON p.id = r.project";

fn row_of(r: &Row) -> rusqlite::Result<Value> {
    let (task, schedule, routine, author): (Option<String>, Option<String>, Option<String>, String) = (r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?);
    let incident: Option<i64> = r.get(15)?;
    // what the row names as its source: a routine by its name, the Supervisor, a task, else its author (AC-267)
    let name = routine.clone().or_else(|| incident.map(|_| "Supervisor".to_string())).or_else(|| task.clone()).unwrap_or_else(|| author.clone());
    Ok(json!({
        "id": r.get::<_, String>(0)?, "project": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?, "summary": r.get::<_, String>(3)?,
        "critical": r.get::<_, bool>(4)?, "read": r.get::<_, bool>(5)?,
        "source": {"task": task, "schedule": schedule, "incident": incident, "name": name},
        "author_name": author, "role": r.get::<_, String>(10)?, "session": r.get::<_, Option<String>>(11)?,
        "created_at": r.get::<_, i64>(12)?, "updated_at": r.get::<_, i64>(13)?, "deleted_at": r.get::<_, Option<i64>>(14)?,
        "number": r.get::<_, i64>(16)?, "project_name": r.get::<_, String>(17)?, "project_color": r.get::<_, i64>(18)?,
    }))
}

/// The order of the Inbox: critical unread first, then newest made; `rowid` breaks ties.
const ORDER: &str = "ORDER BY (r.critical = 1 AND r.read_at IS NULL) DESC, r.created_at DESC, r.rowid DESC";

/// The reports a page asks for, and the unread counts it shows beside them.
pub fn list(tx: &Transaction, f: &Filter) -> Result<Value, String> {
    let limit = f.limit.unwrap_or(PAGE).clamp(1, PAGE);
    let deleted = if f.deleted { "r.deleted_at IS NOT NULL" } else { "r.deleted_at IS NULL" };
    let order = if f.deleted { "ORDER BY r.deleted_at DESC, r.rowid DESC" } else { ORDER };
    let query = format!("{ROW} WHERE {deleted} AND (?1 IS NULL OR r.project = ?1) AND (?2 IS NULL OR r.session_key = ?2)
        AND (?3 = 0 OR r.read_at IS NULL) {order} LIMIT ?4 OFFSET ?5");
    let offset = f.before.unwrap_or(0).max(0);
    let mut reports = rows(tx, &query, (f.project.as_deref(), f.session.as_deref(), f.unread_only, limit + 1, offset), row_of)?;
    let more = reports.len() as i64 > limit;
    reports.truncate(limit as usize);
    Ok(json!({"reports": reports, "more": more, "next": offset + limit, "unread": unread(tx)?}))
}

/// Unread reports not deleted, counted by project, by schedule (`SCH-n`) and by session (AC-267, AC-272, AC-273).
pub fn unread(tx: &Transaction) -> Result<Value, String> {
    let count = |q: &str| -> Result<serde_json::Map<String, Value>, String> {
        Ok(rows(tx, q, [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.into_iter().map(|(k, n)| (k, json!(n))).collect())
    };
    const UNREAD: &str = "r.deleted_at IS NULL AND r.read_at IS NULL";
    Ok(json!({
        "projects": count(&format!("SELECT r.project, count(*) FROM reports r WHERE {UNREAD} GROUP BY r.project"))?,
        "schedules": count(&format!("SELECT 'SCH-' || s.number, count(*) FROM reports r JOIN schedules s ON s.id = r.schedule WHERE {UNREAD} GROUP BY s.number"))?,
        "sessions": count(&format!("SELECT r.session_key, count(*) FROM reports r WHERE {UNREAD} AND r.session_key IS NOT NULL GROUP BY r.session_key"))?,
    }))
}

/// What every page hears with the tasks' marks: the unread counts, and a revision that moves with any change of a
/// report (made, changed, read, deleted, a line closed), so an open Inbox reads its list again.
pub fn marks(tx: &Transaction) -> Result<Value, String> {
    let rev: String = tx.query_row("SELECT count(*) || ':' || coalesce(max(updated_at), 0) || ':' || total(coalesce(read_at, 0)) || ':' || total(coalesce(deleted_at, 0))
        || ':' || total(coalesce(line_seen, 0)) FROM reports", [], |r| r.get(0)).map_err(sql)?;
    Ok(json!({"rev": rev, "unread": unread(tx)?}))
}

/// One report whole (AC-268): its row, its body, comments oldest first, files and links.
pub fn detail(tx: &Transaction, key: &str) -> Result<Value, String> {
    let mut v = tx.query_row(&format!("{ROW} WHERE r.id = ?1"), [key], row_of).map_err(sql)?;
    let (body, follows): (String, Option<String>) = tx.query_row("SELECT r.body, (SELECT f.slug FROM reports f WHERE f.id = r.follows) FROM reports r WHERE r.id = ?1",
        [key], |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
    v["body"] = json!(body);
    v["follows"] = json!(follows);
    v["follow_ups"] = json!(rows(tx, "SELECT slug FROM reports WHERE follows = ?1 AND deleted_at IS NULL ORDER BY created_at", [key], |r| r.get::<_, String>(0))?);
    v["notes"] = json!(rows(tx, "SELECT n.id, n.at, u.name, u.kind, n.kind, n.text FROM report_notes n JOIN users u ON u.id = n.author WHERE n.report = ?1 ORDER BY n.at, n.rowid",
        [key], |r| Ok(json!({"id": r.get::<_, String>(0)?, "at": r.get::<_, i64>(1)?, "author_name": r.get::<_, String>(2)?, "author_kind": r.get::<_, String>(3)?,
                              "kind": r.get::<_, String>(4)?, "text": r.get::<_, String>(5)?})))?);
    v["files"] = json!(crate::tasks::files::of_report(tx, key)?);
    Ok(v)
}

/// A task's reports not deleted, oldest first: their IDs (terminum-reports.md AC-273).
pub fn of_task(tx: &Transaction, task_key: &str) -> Result<Vec<String>, String> {
    rows(tx, "SELECT slug FROM reports WHERE task = ?1 AND deleted_at IS NULL ORDER BY created_at, rowid", [task_key], |r| r.get(0))
}

/// The newest report of a schedule not deleted, its ID and when made (AC-272).
pub fn last_of_schedule(tx: &Transaction, schedule: &str) -> Result<Option<(String, i64)>, String> {
    Ok(rows(tx, "SELECT slug, created_at FROM reports WHERE schedule = ?1 AND deleted_at IS NULL ORDER BY created_at DESC, rowid DESC LIMIT 1",
        [schedule], |r| Ok((r.get(0)?, r.get(1)?)))?.pop())
}
