// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What agents are told (`terminum-integration.md` AC-192): the queue's notices handed to each task's session, and a
//! walk over the trees still in work that tells two tasks whose commits overlap, and a task whose files the target
//! changed. Every notice is a comment by Dispatcher marked `notice` in its data; what it was said for is kept with it,
//! so it is said once. Only commits are compared: uncommitted work is not seen.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::Transaction;
use serde_json::json;

use super::batch::merge;
use super::git::{self, short};
use crate::tasks::integration;
use crate::tasks::ops::{self, sql};
use crate::tasks::store::{Store, DISPATCHER};

/// The most trees of a project the walk compares: the newest.
pub const TREES_MOST: usize = 30;

/// A notice waiting to reach its task's session.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub rowid: i64,
    pub id: String,
    pub session: Option<String>,
    pub text: String,
    /// for a task that left the queue: red, conflict, out or landed
    pub left: Option<String>,
}

/// The newest event's row: notices after it are new.
pub fn last(tx: &Transaction) -> Result<i64, String> {
    tx.query_row("SELECT coalesce(max(rowid), 0) FROM events", [], |r| r.get(0)).map_err(sql)
}

/// The notices written after row `after`, oldest first.
pub fn since(tx: &Transaction, after: i64) -> Result<Vec<Notice>, String> {
    ops::rows(tx, "SELECT e.rowid, t.slug, t.session_key, coalesce(e.text, ''), json_extract(e.data, '$.left') FROM events e JOIN tasks t ON t.id = e.task
                   WHERE e.rowid > ?1 AND e.author = ?2 AND e.kind = 'comment' AND json_extract(e.data, '$.notice') IS NOT NULL ORDER BY e.rowid",
        (after, DISPATCHER), |r| Ok(Notice { rowid: r.get(0)?, id: r.get(1)?, session: r.get(2)?, text: r.get(3)?, left: r.get(4)? }))
}

/// The line a notice is in a session's next hook: its words, terminum's.
pub fn hook_line(n: &Notice) -> String {
    format!("[terminum] {}", n.text)
}

/// What the gate may type for a notice into an idle agent of Dispatcher's (`terminum-dispatcher.md` §9): fixed words
/// with the task's ID alone, never a file, a commit's message or a task's text; only a task returned to it.
pub fn gate_words(n: &Notice) -> Option<String> {
    let why = match n.left.as_deref()? {
        "conflict" => "conflict",
        "red" => "red check",
        _ => return None,
    };
    Some(format!("[terminum Dispatcher] {} came back from integration: {why}. Read why by execute_tool(\"get_task\", {{\"id\": \"{}\"}}), fix it and integrate again.", n.id, n.id))
}

/// A task's tree still in work, as the walk compares it.
#[derive(Debug, Clone)]
pub struct Tree {
    pub task: String,
    pub id: String,
    pub project: String,
    pub dir: PathBuf,
    /// the session's name, to name it to the other task
    pub session: String,
}

/// One task's commits as compared: its HEAD, and the files it changed since the target.
struct Head {
    tree: Tree,
    head: String,
    files: HashSet<String>,
    queued: Option<String>,
}

/// The walk over projects with integration on: each pair of tasks whose commits do not merge is told once for that
/// pair of commits, and each task whose files the target changed since it branched, once for that target.
pub fn walk(store: &Store, trees: &[Tree], now: i64) -> Result<(), String> {
    let projects: Vec<(String, String)> = store.with(|tx| ops::rows(tx, "SELECT i.project, p.dir FROM project_integration i JOIN projects p ON p.id = i.project
        WHERE i.is_on = 1 AND p.host IS NULL", [], |r| Ok((r.get(0)?, r.get(1)?))))?;
    for (project, dir) in projects {
        let s = store.with(|tx| integration::read(tx, &project))?;
        let repo = integration::local_dir(&dir);
        let Ok(target) = git::run(&repo, &["rev-parse", "--verify", "--quiet", &s.target]) else { continue };
        let mut heads: Vec<Head> = trees.iter().filter(|t| t.project == project).take(TREES_MOST).filter_map(|t| head_of(t, &repo, &s.target)).collect();
        heads.extend(queued(store, &project, &repo, &s.target)?);
        for (i, a) in heads.iter().enumerate() {
            if a.queued.is_none() {
                store.with(|tx| moved(tx, a, &repo, &s.target, &target, now))?;
            }
            for b in &heads[i + 1..] {
                if a.files.is_disjoint(&b.files) {
                    continue;   // no file in common: they merge
                }
                if let Ok(Err(files)) = merge(&repo, &a.head, &b.head) {
                    store.with(|tx| overlap(tx, a, b, &files, &s.target, now).and_then(|()| overlap(tx, b, a, &files, &s.target, now)))?;
                }
            }
        }
    }
    Ok(())
}

/// A tree's HEAD and the files it changed since the target; none when it is no checkout of the project's repository
/// or has nothing of its own.
fn head_of(t: &Tree, repo: &Path, target: &str) -> Option<Head> {
    let head = git::run(&t.dir, &["rev-parse", "--verify", "--quiet", "HEAD"]).ok()?;
    let same_repo = git::run(&t.dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).ok()? == git::run(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).ok()?;
    if !same_repo || git::ok(repo, &["merge-base", "--is-ancestor", &head, target]).ok()? {
        return None;
    }
    Some(Head { files: changed(repo, target, &head)?, tree: t.clone(), head, queued: None })
}

/// The tasks queued in the project, as heads, each with its place.
fn queued(store: &Store, project: &str, repo: &Path, target: &str) -> Result<Vec<Head>, String> {
    let entries: Vec<(String, String, String)> = store.with(|tx| ops::rows(tx, "SELECT e.task, t.slug, e.commit_sha FROM integration_entries e JOIN tasks t ON t.id = e.task
        WHERE e.project = ?1 ORDER BY e.queued_at, t.number", [project], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))))?;
    Ok(entries.into_iter().enumerate().filter_map(|(i, (task, id, commit))| {
        let tree = Tree { task, id, project: project.to_string(), dir: repo.to_path_buf(), session: String::new() };
        Some(Head { files: changed(repo, target, &commit)?, tree, head: commit, queued: Some(crate::tasks::integration_queue::ordinal(i + 1)) })
    }).collect())
}

/// The files `head` changed since it left `target`.
fn changed(repo: &Path, target: &str, head: &str) -> Option<HashSet<String>> {
    Some(git::run(repo, &["diff", "--name-only", &format!("{target}...{head}")]).ok()?.lines().map(str::to_string).collect())
}

/// `a`'s agent told, once for that pair of commits, that `b`'s commits do not merge with its own.
fn overlap(tx: &Transaction, a: &Head, b: &Head, files: &[String], target: &str, now: i64) -> Result<(), String> {
    if a.queued.is_some() {
        return Ok(());   // a queued task learns of a conflict when its batch is built (AC-189)
    }
    let pair = format!("{}-{}", short(&a.head), short(&b.head));
    if ops::noticed(tx, &a.tree.task, "overlap", &pair)? {
        return Ok(());
    }
    let files = files.join(", ");
    let words = match &b.queued {
        Some(place) => format!("{}: its commits and {}'s change the same files: {files}. {} is queued for integration ({place}). Go on; merge {target} once {} lands, before you queue.",
                               a.tree.id, b.tree.id, b.tree.id, b.tree.id),
        None => format!("{}: its commits and {}'s (session {}, IN PROGRESS) change the same files: {files}. They cannot be in one batch. Agree with that session who goes first; the other merges {target} after the first lands, then queues.",
                        a.tree.id, b.tree.id, if b.tree.session.is_empty() { "?" } else { &b.tree.session }),
    };
    ops::notice(tx, DISPATCHER, &a.tree.id, &words, json!({"overlap": pair}), now).map(drop)
}

/// `a`'s agent told, once for that target, that the target moved in files it changed since it branched.
fn moved(tx: &Transaction, a: &Head, repo: &Path, target: &str, at_target: &str, now: i64) -> Result<(), String> {
    let Ok(base) = git::run(repo, &["merge-base", &a.head, at_target]) else { return Ok(()) };
    if base == at_target {
        return Ok(());   // it holds the target
    }
    let theirs: HashSet<String> = git::run(repo, &["diff", "--name-only", &base, at_target]).unwrap_or_default().lines().map(str::to_string).collect();
    let mut both: Vec<&String> = a.files.intersection(&theirs).collect();
    if both.is_empty() {
        return Ok(());
    }
    both.sort();
    let mark = format!("{}@{}", short(&a.head), short(at_target));
    if ops::noticed(tx, &a.tree.task, "moved", &mark)? {
        return Ok(());
    }
    let files = both.iter().map(|f| f.as_str()).collect::<Vec<_>>().join(", ");
    let conflicts = match merge(repo, &a.head, at_target) {
        Ok(Err(files)) => format!(" It conflicts in: {}.", files.join(", ")),
        _ => String::new(),
    };
    let words = format!("{}: {target} moved to {} in files you changed: {files}. Merge {target} now, before you queue.{conflicts}", a.tree.id, short(at_target));
    ops::notice(tx, DISPATCHER, &a.tree.id, &words, json!({"moved": mark}), now).map(drop)
}

#[cfg(test)]
#[path = "notices_tests.rs"]
mod tests;
