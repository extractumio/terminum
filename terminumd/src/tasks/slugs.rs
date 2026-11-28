// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A task's ID as people see it (terminum-kanban.md AC-148): its project's prefix and the project's own
//! count, TERM-7; a schedule's task says so before them, S_TERM-8, in the same count. The row's key stays the task's for good; every ID a task had stays found in
//! `task_slugs` ("T-12" of before, one of a project it left, one of an older prefix), and the key's
//! first 8 hex find it too.

use rusqlite::{OptionalExtension, Transaction};

use super::ops::{rows, sql};
use super::store::DEFAULT_PROJECT;

pub const PREFIX_MOST: usize = 8;
/// The first characters of the key that name a task, as shown.
pub const UID_LEN: usize = 8;
/// What stands before the ID of a task a schedule made (terminum-schedules.md AC-162); no prefix holds "_".
pub(super) const SCHEDULED: &str = "S_";
/// A batch's integration task (terminum-integration.md AC-194).
pub(super) const INTEGRATING: &str = "I_";
/// A report (terminum-reports.md AC-265): in the same count, never a task.
pub const REPORT: &str = "R_";

/// A batch's integration task, by its ID's mark (`terminum-integration.md` AC-194).
pub fn is_integration(id: &str) -> bool {
    id.starts_with(INTEGRATING)
}

/// A prefix as kept: 1–8 of A–Z and 0–9, a letter first; lower case taken as upper.
pub fn check_prefix(prefix: &str) -> Result<String, String> {
    let p = prefix.trim().to_ascii_uppercase();
    let ok = (1..=PREFIX_MOST).contains(&p.len()) && p.starts_with(|c: char| c.is_ascii_uppercase()) && p.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if ok { Ok(p) } else { Err(format!("ID letters: 1–{PREFIX_MOST} of A–Z and 0–9, a letter first")) }
}

/// The first 4 letters of the folder's name, else of the project's name, upper case; PROJ with none.
pub fn default_prefix(dir: &str, name: &str) -> String {
    let letters = |s: &str| s.chars().filter(char::is_ascii_alphabetic).take(4).collect::<String>().to_ascii_uppercase();
    let folder = dir.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    [letters(folder), letters(name)].into_iter().find(|l| !l.is_empty()).unwrap_or_else(|| "PROJ".into())
}

fn taken(tx: &Transaction, prefix: &str, but: &str) -> Result<bool, String> {
    tx.query_row("SELECT count(*) FROM projects WHERE prefix = ?1 AND id <> ?2", (prefix, but), |r| r.get::<_, i64>(0)).map(|n| n > 0).map_err(sql)
}

/// `base` if no other project has it, else its last letter changed (A–Z, then 2–9), else a number added.
fn free_prefix(tx: &Transaction, base: &str, project: &str) -> Result<String, String> {
    let stem = if base.len() >= 4 { &base[..base.len() - 1] } else { base };
    let tries = std::iter::once(base.to_string())
        .chain(('A'..='Z').chain('2'..='9').map(|c| format!("{stem}{c}")))
        .chain((2..1000).map(|n| { let n = n.to_string(); format!("{}{n}", &base[..base.len().min(PREFIX_MOST - n.len())]) }));
    for p in tries {
        if !taken(tx, &p, project)? {
            return Ok(p);
        }
    }
    Err("No free ID letters: set them in the project's window.".into())
}

/// The count a project's next task takes: above every ID found with its prefix, so an ID a task left
/// behind never names another.
fn bump_above_found(tx: &Transaction, project: &str, prefix: &str) -> Result<(), String> {
    // "TERM-" up to "TERM." ('.' follows '-'): a range the key's index walks, where a LIKE would scan; a schedule's
    // tasks count with the others
    for mark in ["", SCHEDULED, INTEGRATING, REPORT] {
        // a report's ID is in the same count (terminum-reports.md AC-265), kept in its own table
        let table = if mark == REPORT { "reports" } else { "task_slugs" };
        let top: Option<i64> = tx.query_row(&format!("SELECT max(CAST(substr(slug, ?3) AS INTEGER)) FROM {table} WHERE slug >= ?1 AND slug < ?2"),
                                            (format!("{mark}{prefix}-"), format!("{mark}{prefix}."), (mark.len() + prefix.len()) as i64 + 2), |r| r.get(0)).map_err(sql)?;
        if let Some(top) = top {
            tx.execute("UPDATE projects SET next_seq = max(next_seq, ?1) WHERE id = ?2", (top + 1, project)).map_err(sql)?;
        }
    }
    Ok(())
}

/// `project`'s prefix and the next number of its count, taken: a task's or a report's.
fn next_number(tx: &Transaction, project: &str) -> Result<(String, i64), String> {
    let (prefix, seq, dir, name): (Option<String>, i64, String, String) = tx.query_row("SELECT prefix, next_seq, dir, name FROM projects WHERE id = ?1", [project],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(sql)?;
    let Some(prefix) = prefix else {
        fill_project(tx, project, &dir, &name)?;       // a project written since the last fill
        return next_number(tx, project);
    };
    tx.execute("UPDATE projects SET next_seq = ?1 WHERE id = ?2", (seq + 1, project)).map_err(sql)?;
    Ok((prefix, seq))
}

/// The next ID of `project`'s count for a report: R_TERM-7, a number no task's ID holds (terminum-reports.md AC-265).
pub fn take_report(tx: &Transaction, project: &str) -> Result<(String, i64), String> {
    next_number(tx, project).map(|(prefix, seq)| (format!("{REPORT}{prefix}-{seq}"), seq))
}

/// What a task's lookup says of an ID that names none: a report's says where to read it (terminum-reports.md AC-265).
pub fn not_found(id: &str) -> String {
    let id = id.trim();
    if id.get(..REPORT.len()).is_some_and(|m| m.eq_ignore_ascii_case(REPORT)) { format!("{id} is a report: use get_report.") } else { format!("No task {id}") }
}

/// Every project gets a prefix and every task an ID, once: after a migration and when projects are written.
/// A task of before keeps its number under its project's letters, T-12 becoming TERM-12, so an old mention
/// and the new ID say the same number; the project's next counts on from there.
pub fn fill(tx: &Transaction) -> Result<(), String> {
    for (id, dir, name) in rows(tx, "SELECT id, dir, name FROM projects WHERE prefix IS NULL ORDER BY place", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
        fill_project(tx, &id, &dir, &name)?;
    }
    for (key, number, prefix) in rows(tx, "SELECT t.id, t.number, p.prefix FROM tasks t JOIN projects p ON p.id = t.project WHERE t.slug IS NULL", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))? {
        put_slug(tx, &key, &format!("{prefix}-{number}"))?;
        tx.execute("UPDATE projects SET next_seq = max(next_seq, ?1) WHERE prefix = ?2", (number + 1, &prefix)).map_err(sql)?;
    }
    Ok(())
}

/// Task `key`'s ID now, found from then on; one it had before keeps finding it too.
fn put_slug(tx: &Transaction, key: &str, slug: &str) -> Result<(), String> {
    tx.execute("UPDATE tasks SET slug = ?1 WHERE id = ?2", (slug, key)).map_err(sql)?;
    tx.execute("INSERT OR IGNORE INTO task_slugs (slug, task) VALUES (?1, ?2)", (slug, key)).map(|_| ()).map_err(sql)
}

fn fill_project(tx: &Transaction, project: &str, dir: &str, name: &str) -> Result<(), String> {
    let prefix = free_prefix(tx, &default_prefix(dir, name), project)?;
    tx.execute("UPDATE projects SET prefix = ?1 WHERE id = ?2", (&prefix, project)).map_err(sql)?;
    bump_above_found(tx, project, &prefix)
}

/// Task `key` takes the next ID of `project`; the one it had stays found. `scheduled`: a schedule made it; one
/// that said so keeps saying it in the project it moves to.
/// `mark`: "S_" for a schedule's task, "I_" for a batch's, "" else; a task that had one keeps it.
pub fn assign(tx: &Transaction, key: &str, project: &str, mark: &str) -> Result<String, String> {
    let (prefix, seq) = next_number(tx, project)?;
    let had = |tx: &Transaction| tx.query_row("SELECT slug FROM tasks WHERE id = ?1", [key], |r| r.get::<_, Option<String>>(0)).optional().map(Option::flatten).map_err(sql);
    let had = had(tx)?;
    let mark = [SCHEDULED, INTEGRATING].into_iter().find(|m| *m == mark || had.as_deref().is_some_and(|s| s.starts_with(m))).unwrap_or("");
    let slug = format!("{mark}{prefix}-{seq}");
    put_slug(tx, key, &slug)?;
    Ok(slug)
}

/// A project's ID letters changed: its tasks keep their numbers under the new ones, the old IDs stay found.
pub fn set_prefix(tx: &Transaction, project: &str, prefix: &str) -> Result<(), String> {
    if project == DEFAULT_PROJECT {
        return Err("Default's tasks keep TASK.".into());
    }
    let prefix = check_prefix(prefix)?;
    if taken(tx, &prefix, project)? {
        return Err(format!("{prefix} names another project's tasks."));
    }
    let old: Option<String> = tx.query_row("SELECT prefix FROM projects WHERE id = ?1", [project], |r| r.get(0)).optional().map_err(sql)?.ok_or("That project is gone.")?;
    if old.as_deref() == Some(prefix.as_str()) {
        return Ok(());
    }
    let tasks = rows(tx, "SELECT id, slug FROM tasks WHERE project = ?1 AND slug IS NOT NULL", [project], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let renamed: Vec<(String, String)> = tasks.into_iter().map(|(key, slug)| {
        let (mark, seq) = (if slug.starts_with(SCHEDULED) { SCHEDULED } else { "" }, slug.rsplit('-').next().unwrap_or("0"));
        (key, format!("{mark}{prefix}-{seq}"))
    }).collect();
    // an ID another task had or has keeps finding that one: such letters are refused, never taken over
    for (key, new) in &renamed {
        let other: Option<String> = tx.query_row("SELECT task FROM task_slugs WHERE slug = ?1 AND task <> ?2", (new, key), |r| r.get(0)).optional().map_err(sql)?;
        if other.is_some() {
            return Err(format!("{new} already names another task: choose other letters."));
        }
    }
    tx.execute("UPDATE projects SET prefix = ?1 WHERE id = ?2", (&prefix, project)).map_err(sql)?;
    for (key, new) in &renamed {
        put_slug(tx, key, new)?;
    }
    bump_above_found(tx, project, &prefix)
}

/// A task's number (the store's order) from what a person or an agent calls it: its ID now, one it had,
/// or its key's first 8 hex characters or the whole key. None when nothing, or more than one, answers.
pub fn resolve(tx: &Transaction, id: &str) -> Result<Option<i64>, String> {
    let query = format!("SELECT number FROM tasks WHERE id = {}", key_named("?1"));
    tx.prepare_cached(&query).map_err(sql)?.query_row([id], |r| r.get(0)).optional().map_err(sql)
}

/// The key of a task named by a SQL parameter or column. RUN and its saved presses resolve the same IDs.
pub(crate) fn key_named(id: &str) -> String {
    // The trim set is Unicode White_Space, as in str::trim. SQLite's upper and lower fold only ASCII.
    // The key's index walks the prefix range: the keys are lower-case hex, and 'g' follows them all.
    format!("(SELECT coalesce(
        (SELECT id FROM tasks WHERE slug = upper(named.value)),
        (SELECT task FROM task_slugs WHERE slug = upper(named.value)),
        (SELECT min(id) FROM (SELECT id FROM tasks
            WHERE length(named.value) IN ({UID_LEN}, 32) AND named.value NOT GLOB '*[^0-9A-Fa-f]*'
                AND id >= lower(named.value) AND id < lower(named.value) || 'g' LIMIT 2) HAVING count(*) = 1))
      FROM (SELECT trim({id}, char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288)) AS value) named)")
}

#[cfg(test)]
#[path = "slugs_tests.rs"]
mod tests;
