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
    let query = format!("{ROW} WHERE {deleted} AND (?1 IS NULL OR r.project = ?1) AND (?2 = 0 OR r.read_at IS NULL) {order} LIMIT ?3 OFFSET ?4");
    let offset = f.before.unwrap_or(0).max(0);
    let mut reports = rows(tx, &query, (f.project.as_deref(), f.unread_only, limit + 1, offset), row_of)?;
    let more = reports.len() as i64 > limit;
    reports.truncate(limit as usize);
    Ok(json!({"reports": reports, "more": more, "next": offset + limit}))
}

/// Unread reports not deleted, counted by project (AC-267), by the partial index `reports_unread`.
pub fn unread(tx: &Transaction) -> Result<Value, String> {
    let counts: serde_json::Map<String, Value> = rows(tx, "SELECT project, count(*) FROM reports WHERE deleted_at IS NULL AND read_at IS NULL GROUP BY project",
        [], |r| Ok((r.get::<_, String>(0)?, json!(r.get::<_, i64>(1)?))))?.into_iter().collect();
    Ok(json!({"projects": counts}))
}

/// What every page hears with the tasks' marks: the unread counts, and the revision every write of a report moves
/// (record.rs `bump`), so an open Inbox reads its list again.
pub fn marks(tx: &Transaction) -> Result<Value, String> {
    Ok(json!({"rev": super::record::revision(tx)?, "unread": unread(tx)?}))
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

/// Each schedule's newest report not deleted (its ID, when made) and its unread count, in one read (AC-272).
pub fn by_schedule(tx: &Transaction) -> Result<std::collections::HashMap<String, (String, i64, i64)>, String> {
    Ok(rows(tx, "SELECT r.schedule, (SELECT n.slug FROM reports n WHERE n.schedule = r.schedule AND n.deleted_at IS NULL ORDER BY n.created_at DESC, n.rowid DESC LIMIT 1),
            max(r.created_at), sum(r.read_at IS NULL) FROM reports r WHERE r.schedule IS NOT NULL AND r.deleted_at IS NULL GROUP BY r.schedule", [],
        |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))?.into_iter().collect())
}
