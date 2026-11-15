// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The integration queue (`terminum-integration.md` AC-187): a task queues its commit with its result and stands in
//! INTEGRATION, its budget and idle clocks stopped (they count IN PROGRESS only); it leaves by being taken out, by
//! any move out of INTEGRATION, or by its batch's end. git runs outside the store's transaction.

use std::path::Path;

use rusqlite::{OptionalExtension, Transaction};
use serde_json::{json, Value};

use super::closeout;
use super::integration::{self, Settings};
use super::ops::{self, sql, Task, INTEGRATION};
use super::store::Store;
use super::tools::{arg, author, movable, visible, Who};

/// One task's place in its project's queue.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub task: String,
    pub id: String,
    pub commit: String,
    pub batch: Option<i64>,
}

/// Whatever moved it out of INTEGRATION, the task is no longer queued; a batch it was in goes on without it.
pub(crate) fn leave(tx: &Transaction, task_key: &str) -> Result<(), String> {
    let batch: Option<i64> = tx.query_row("SELECT batch FROM integration_entries WHERE task = ?1", [task_key], |r| r.get(0)).optional().map_err(sql)?.flatten();
    tx.execute("DELETE FROM integration_entries WHERE task = ?1", [task_key]).map_err(sql)?;
    match batch {
        Some(b) => crate::integrate::store::left(tx, task_key, b, ops::now()),
        None => Ok(()),
    }
}

/// A project's queue in its order: first queued first, then by task number.
pub fn queue_of(tx: &Transaction, project: &str) -> Result<Vec<Entry>, String> {
    ops::rows(tx, "SELECT e.task, t.slug, e.commit_sha, e.batch FROM integration_entries e JOIN tasks t ON t.id = e.task
                   WHERE e.project = ?1 ORDER BY e.queued_at, t.number", [project],
        |r| Ok(Entry { task: r.get(0)?, id: r.get(1)?, commit: r.get(2)?, batch: r.get(3)? }))
}

fn entry_of(tx: &Transaction, task_key: &str) -> Result<Option<Entry>, String> {
    tx.query_row("SELECT e.task, t.slug, e.commit_sha, e.batch FROM integration_entries e JOIN tasks t ON t.id = e.task WHERE e.task = ?1", [task_key],
        |r| Ok(Entry { task: r.get(0)?, id: r.get(1)?, commit: r.get(2)?, batch: r.get(3)? })).optional().map_err(sql)
}

/// "1st", "2nd", "3rd", "4th"…
pub fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) { (1, x) if x != 11 => "st", (2, x) if x != 12 => "nd", (3, x) if x != 13 => "rd", _ => "th" };
    format!("{n}{suffix}")
}

use crate::integrate::git::{output as git_output, short};

fn git(dir: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    git_output(dir, args)
}

/// The commit to queue: `given`, else the caller's folder's HEAD with nothing uncommitted; one of the project's
/// repository, not yet in its target.
fn commit_to_queue(repo: &Path, s: &Settings, given: Option<&str>, folder: Option<&str>) -> Result<String, String> {
    let sha = match given {
        Some(c) => c.to_string(),
        None => {
            let folder = Path::new(folder.ok_or("This session has no folder on this Mac: name the commit.")?);
            let changed = String::from_utf8_lossy(&git(folder, &["status", "--porcelain"])?.stdout).lines().count();
            if changed > 0 {
                return Err(format!("Commit your work first: {changed} file{} changed in {}.", if changed == 1 { " is" } else { "s are" }, folder.display()));
            }
            String::from_utf8_lossy(&git(folder, &["rev-parse", "HEAD"])?.stdout).trim().to_string()
        }
    };
    let found = git(repo, &["rev-parse", "--verify", "--quiet", &format!("{sha}^{{commit}}")])?;
    if !found.status.success() || sha.is_empty() || sha.starts_with('-') {
        return Err(format!("{sha} is not a commit of this project's repository."));
    }
    let full = String::from_utf8_lossy(&found.stdout).trim().to_string();
    let (remote, branch) = integration::target_parts(&s.target).ok_or_else(|| integration::not_a_target(&s.target))?;
    if git(repo, &["merge-base", "--is-ancestor", &full, &format!("refs/remotes/{remote}/{branch}")])?.status.success() {
        return Err(format!("{} is already in {}: finish the task with finish_task.", short(&full), s.target));
    }
    Ok(full)
}

/// What the caller may queue: the task, its project's settings and folder, refused before git runs.
fn ready(tx: &Transaction, who: &Who, id: &str, result: &str) -> Result<(Task, Settings, String), String> {
    let t = movable(tx, who, id)?;
    let s = integration::read(tx, &t.project)?;
    let (name, host, dir) = ops::project_place(tx, &t.project)?;
    if !s.on || host.is_some() {
        return Err(format!("Integration is off for {name}: publish your work as its rules say."));
    }
    if let Some(e) = entry_of(tx, &t.key)? && let Some(b) = e.batch {
        return Err(format!("{id} is in batch {b}, under check: take it out first (withdraw_integration), which checks the batch again without it."));
    }
    // a result is there only in its sections, and not one whose Outcome says the work is not done (AC-123)
    let form = closeout::form_of(tx, &t.key)?;
    let lacks = closeout::missing(result, form);
    if !lacks.is_empty() {
        return Err(closeout::missing_words(id, &t.status, &lacks));
    }
    if form == closeout::SECTIONS.as_slice() && closeout::not_done(result) {
        return Err(closeout::not_done_words(id, &t.status));
    }
    Ok((t, s, dir))
}

/// Queues `args.id`'s commit with its result (AC-187): answered its place and the batch it is expected in.
pub fn integrate(store: &Store, who: &Who, args: &Value) -> Result<(String, Vec<String>), String> {
    let id = arg(args, "id")?;
    let result = crate::plans::rules::clean(arg(args, "result")?);
    let (_, s, dir) = store.with(|tx| ready(tx, who, id, &result))?;
    let repo = integration::local_dir(&dir);
    let commit = commit_to_queue(&repo, &s, args["commit"].as_str().filter(|c| !c.is_empty()), who.dir.as_deref())?;
    let at = ops::now();
    store.with(|tx| {
        // what changed while git ran: the same checks again, in the transaction that queues
        let (t, s, _) = ready(tx, who, id, &result)?;
        let returned: Option<String> = tx.query_row("SELECT why FROM integration_returns WHERE task = ?1 AND commit_sha = ?2", (&t.key, &commit), |r| r.get(0)).optional().map_err(sql)?;
        if returned.is_some() {
            return Err(format!("{} was returned for a red check: commit a fix first, or ask a human to queue it.", short(&commit)));
        }
        let me = author(tx, who)?;
        tx.execute("INSERT INTO integration_entries (task, project, commit_sha, result, author, queued_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                    ON CONFLICT (task) DO UPDATE SET commit_sha = ?3, result = ?4, author = ?5", (&t.key, &t.project, &commit, &result, &me, at)).map_err(sql)?;
        let short = short(&commit);
        if t.status == INTEGRATION {
            ops::log(tx, &t.key, &me, "queue", &format!("Queued {short} for integration into {} in its place.", s.target), None, at)?;
        } else {
            ops::place(tx, &me, &t, INTEGRATION, at)?;
            ops::log(tx, &t.key, &me, "queue", &format!("Queued {short} for integration into {}.", s.target), None, at)?;
        }
        Ok((queued_words(tx, &t, &s)?, vec![id.to_string()]))
    })
}

/// "API-12 is queued 3rd for integration into origin/main. Expected batch: with API-9 and API-10." (§9 Notices)
fn queued_words(tx: &Transaction, t: &Task, s: &Settings) -> Result<String, String> {
    let queue = queue_of(tx, &t.project)?;
    let place = queue.iter().position(|e| e.task == t.key).map_or(queue.len(), |i| i + 1);
    let with: Vec<&str> = queue.iter().filter(|e| e.batch.is_none() && e.task != t.key).take(s.size().saturating_sub(1)).map(|e| e.id.as_str()).collect();
    let batch = if with.is_empty() { "alone".to_string() } else { format!("with {}", and_list(&with)) };
    Ok(format!("{} is queued {} for integration into {}. Expected batch: {batch}.", t.id, ordinal(place), s.target))
}

/// "A", "A and B", "A, B and C".
pub fn and_list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Takes `args.id` out of the queue: IN PROGRESS, its budget running on.
pub fn withdraw(store: &Store, who: &Who, args: &Value) -> Result<(String, Vec<String>), String> {
    let id = arg(args, "id")?;
    let at = ops::now();
    store.with(|tx| {
        let t = movable(tx, who, id)?;
        if t.status != INTEGRATION {
            return Err(format!("{id} is not queued for integration."));
        }
        ops::place(tx, &author(tx, who)?, &t, "IN PROGRESS", at)?;
        Ok((format!("{id} was taken out of the queue: IN PROGRESS, its budget runs on."), vec![id.to_string()]))
    })
}

/// The project's settings and queue, or one task's place in it.
pub fn status(store: &Store, who: &Who, args: &Value) -> Result<String, String> {
    store.with(|tx| {
        let task = args["id"].as_str().filter(|i| !i.is_empty()).map(|id| visible(tx, who, id)).transpose()?;
        let project = match &task {
            Some(t) => t.project.clone(),
            None => who.project.clone().unwrap_or_else(|| super::store::DEFAULT_PROJECT.to_string()),
        };
        let s = integration::read(tx, &project)?;
        let queue: Vec<Value> = queue_of(tx, &project)?.iter().enumerate()
            .map(|(i, e)| json!({"task": e.id, "place": ordinal(i + 1), "commit": short(&e.commit), "batch": e.batch})).collect();
        let mut v = json!({"on": s.on, "target": s.target, "check": s.check, "publish": s.publish, "after": s.after, "batch_size": s.size(), "batch_most": s.batch_most, "queue": queue});
        if let Some(t) = task {
            v["task"] = of_task(tx, &t)?;
        }
        Ok(v.to_string())
    })
}

/// A task as `terminum integration wait` follows it (AC-187): its status, Dispatcher's newest words on it, and,
/// out of INTEGRATION, how it left (the mark its words carry): landed, red, conflict, or out (taken out, given up,
/// integration turned off).
fn of_task(tx: &Transaction, t: &Task) -> Result<Value, String> {
    let said: Vec<Value> = ops::newest_by(tx, &t.key, super::store::DISPATCHER, 20)?.into_iter().map(|(id, text)| json!({"id": id, "text": text})).collect();
    let left = if t.status == INTEGRATION { None } else { Some(ops::how_left(tx, &t.key)?.unwrap_or_else(|| "out".into())) };
    Ok(json!({"id": t.id, "status": t.status, "left": left, "said": said}))
}

#[cfg(test)]
#[path = "integration_queue_tests.rs"]
mod tests;
