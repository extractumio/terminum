// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What agents are told (`terminum-integration.md` AC-192): the queue's notices handed to each task's session, and a
//! walk over the trees still in work that tells two tasks whose commits overlap, and a task whose files the target
//! changed; a tree of the project with no task is only the other side of an overlap. Every notice is a comment by
//! Dispatcher marked `notice` in its data; what it was said for is kept with it, so it is said once. Only commits are
//! compared: uncommitted work is not seen. git runs outside the store's turn. Each walk keeps which tasks a task's
//! commits overlap, for its card (AC-193).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use rusqlite::Transaction;
use serde_json::json;

use super::batch::merge;
use super::engine::place;
use super::git::{self, short};
use crate::tasks::integration;
use crate::tasks::integration_queue::{ordinal, queue_of};
use crate::tasks::ops::{self, sql};
use crate::tasks::store::{Store, DISPATCHER};

/// The most trees of a project the walk compares, of tasks and of none each: the newest.
const TREES_MOST: usize = 30;
/// How long a walk's overlaps stand on a card when no later walk said otherwise: its task left the walk.
const OVERLAPS_FOR: i64 = 300;

/// A notice waiting to reach its task's session.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub task: String,
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

/// The notices written after row `after`, oldest first, and the newest row read, where the next look starts.
pub fn since(tx: &Transaction, after: i64) -> Result<(Vec<Notice>, i64), String> {
    let notices = ops::rows(tx, "SELECT e.task, t.slug, t.session_key, coalesce(e.text, ''), json_extract(e.data, '$.left') FROM events e JOIN tasks t ON t.id = e.task
                   WHERE e.rowid > ?1 AND e.author = ?2 AND e.kind = 'comment' AND json_extract(e.data, '$.notice') IS NOT NULL ORDER BY e.rowid",
        (after, DISPATCHER), |r| Ok(Notice { task: r.get(0)?, id: r.get(1)?, session: r.get(2)?, text: r.get(3)?, left: r.get(4)? }))?;
    Ok((notices, last(tx)?.max(after)))
}

/// The line a notice is in a session's next hook: its first line, terminum's words; what follows (an agent's
/// analysis) is read by get_task, so no line of it stands as terminum's own.
pub fn hook_line(n: &Notice) -> String {
    let mut lines = n.text.lines();
    let first = lines.next().unwrap_or_default();
    let more = if lines.next().is_some() { format!(" (the rest: execute_tool(\"get_task\", {{\"id\": \"{}\"}}))", n.id) } else { String::new() };
    format!("[terminum] {first}{more}")
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

/// A task's tree, as the walk compares it when its task is IN PROGRESS (`working`); any other task's only tells that
/// tree from one with no task.
#[derive(Debug, Clone)]
pub struct Tree {
    pub task: String,
    pub id: String,
    pub project: String,
    pub dir: PathBuf,
    pub working: bool,
}

/// The folder a session's task works in, from its layout row: the one its agent's hooks said (a worktree it went
/// into), else the pane's own; none on another host.
pub fn tree_dir(row: &serde_json::Value) -> Option<PathBuf> {
    if row["host"].as_str().is_some_and(|h| !h.is_empty()) {
        return None;
    }
    row["agentDir"].as_str().or(row["dir"].as_str()).filter(|d| !d.is_empty()).map(PathBuf::from)
}

/// One task's commits as compared: its HEAD and the files it changed since the target; working, or queued at a place.
struct Head {
    task: String,
    id: String,
    head: String,
    files: HashSet<String>,
    queued: Option<String>,
}

/// A tree of the project with no task: its HEAD, the files it changed, and how a notice names it.
struct Loose {
    head: String,
    files: HashSet<String>,
    name: String,
}

/// What was judged already, by commits and the target's: a pair's conflicting files (none: it merges), a move's mark;
/// judged again only when one of them changes.
static JUDGED: Mutex<Option<HashMap<String, Option<Vec<String>>>>> = Mutex::new(None);

/// Each task's overlapping tasks as the last walk found them, by task key, with when (AC-193).
static OVERLAPS: Mutex<Option<HashMap<String, (i64, Vec<String>)>>> = Mutex::new(None);

/// The judgement under `key`, made by `judge` the first time only, outside the lock; git failing is said and judged
/// again at the next walk.
fn judged(key: String, judge: impl FnOnce() -> Result<Option<Vec<String>>, String>) -> Option<Vec<String>> {
    if let Some(v) = JUDGED.lock().as_ref().and_then(|m| m.get(&key)) {
        return v.clone();
    }
    let v = match judge() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(event = "integration.walk", error = %e, "git could not judge {key}");
            return None;
        }
    };
    let mut j = JUDGED.lock();
    let map = j.get_or_insert_with(HashMap::new);
    if map.len() > 10_000 {
        map.clear();
    }
    map.insert(key, v.clone());
    v
}

/// Whether `key` is judged for the first time now.
fn first_time(key: String) -> bool {
    let mut fresh = false;
    judged(key, || { fresh = true; Ok(None) });
    fresh
}

/// The IDs of the tasks whose commits overlap task `key`'s, as a walk found them in the last few minutes.
pub fn overlaps_of(key: &str, now: i64) -> Vec<String> {
    OVERLAPS.lock().as_ref().and_then(|m| m.get(key)).filter(|(at, _)| now - at <= OVERLAPS_FOR).map(|(_, ids)| ids.clone()).unwrap_or_default()
}

/// The walk over projects with integration on: each pair of tasks whose commits do not merge is told once for that
/// pair of commits, a task overlapping a tree of the project with no task too, and each task whose files the target
/// changed since it branched, once for that target. `sessions`: each session's folder on this Mac and its name.
pub fn walk(store: &Store, home: &Path, trees: &[Tree], sessions: &[(PathBuf, String)], now: i64) -> Result<(), String> {
    let projects: HashSet<&str> = trees.iter().map(|t| t.project.as_str()).collect();
    for project in projects {
        let p = place(store, home, project)?;
        let Some((remote, branch)) = integration::target_parts(&p.s.target) else { continue };
        let Ok(target) = git::run(&p.repo, &["rev-parse", &format!("refs/remotes/{remote}/{branch}")]) else { continue };
        let common = git::run(&p.repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).unwrap_or_default();
        let ours: Vec<&Tree> = trees.iter().filter(|t| t.project == project).collect();
        let mut heads: Vec<Head> = ours.iter().filter(|t| t.working).take(TREES_MOST).filter_map(|t| head_of(t, &p.repo, &common, &target)).collect();
        let entries = store.with(|tx| queue_of(tx, project))?;
        heads.extend(entries.into_iter().enumerate().filter_map(|(i, e)| {
            Some(Head { files: changed(&p.repo, &target, &e.commit)?, task: e.task, id: e.id, head: e.commit, queued: Some(ordinal(i + 1)) })
        }));
        let taken: HashSet<&str> = heads.iter().map(|h| h.head.as_str()).chain([target.as_str()]).collect();
        let claimed: Vec<&Path> = ours.iter().map(|t| t.dir.as_path()).collect();
        let loose = loose_trees(&p.repo, &home.join("integration"), &target, (&taken, &claimed), sessions);
        let mut found: HashMap<&str, Vec<String>> = HashMap::new();
        for (i, a) in heads.iter().enumerate().filter(|(_, h)| h.queued.is_none()) {
            moved(store, a, &p.repo, &p.s.target, &target, now)?;
            for b in heads.iter().skip(i + 1).chain(heads.iter().take(i).filter(|h| h.queued.is_some())) {
                // no file in common merges; a pair judged for these commits is not merged again
                let Some(files) = conflict(&p.repo, &a.head, &b.head, &a.files, &b.files, &target) else { continue };
                store.with(|tx| overlap(tx, a, b, &files, &p.s.target, now).and_then(|()| overlap(tx, b, a, &files, &p.s.target, now)))?;
                found.entry(a.task.as_str()).or_default().push(b.id.clone());
                found.entry(b.task.as_str()).or_default().push(a.id.clone());
            }
            for l in &loose {
                if let Some(files) = conflict(&p.repo, &a.head, &l.head, &a.files, &l.files, &target) {
                    store.with(|tx| beside(tx, a, l, &files, &p.s.target, now))?;
                }
            }
        }
        let mut kept = OVERLAPS.lock();
        let kept = kept.get_or_insert_with(HashMap::new);
        kept.retain(|_, (at, _)| now - *at <= OVERLAPS_FOR);
        for t in ours {
            match found.remove(t.task.as_str()) {
                Some(ids) => kept.insert(t.task.clone(), (now, ids)),
                None => kept.remove(&t.task),
            };
        }
    }
    Ok(())
}

/// The files two HEADs both changed and cannot merge in, judged once for those commits and the target; none when
/// they change no file in common or merge.
fn conflict(repo: &Path, a: &str, b: &str, a_files: &HashSet<String>, b_files: &HashSet<String>, target: &str) -> Option<Vec<String>> {
    if a_files.is_disjoint(b_files) {
        return None;
    }
    judged(format!("{a}-{b}@{target}"), || merge(repo, a, b).map(Result::err))
}

/// The project's worktrees with commits of their own and no task, newest first, at most `TREES_MOST`, none of
/// terminum's own under `ours`; each named by its branch and the session in it, else its folder. A tree with a task:
/// one holding a task's folder (`claimed`), or at a HEAD a task or the target holds (`taken`).
fn loose_trees(repo: &Path, ours: &Path, target: &str, (taken, claimed): (&HashSet<&str>, &[&Path]), sessions: &[(PathBuf, String)]) -> Vec<Loose> {
    let Ok(listed) = git::run(repo, &["worktree", "list", "--porcelain"]) else { return Vec::new() };
    let real = |d: &Path| std::fs::canonicalize(d).unwrap_or_else(|_| d.to_path_buf());
    let ours = real(ours);
    let all: Vec<(PathBuf, String, Option<String>)> = listed.split("\n\n").filter_map(|block| {
        let field = |k: &str| block.lines().find_map(|l| l.strip_prefix(k)).map(str::to_string);
        Some((real(Path::new(&field("worktree ")?)), field("HEAD ")?, field("branch ").map(|b| b.trim_start_matches("refs/heads/").to_string())))
    }).collect();
    // the tree a folder is in: the deepest holding it (a primary checkout holds its .worktrees)
    let tree_of = |d: &Path| { let d = real(d); all.iter().filter(|(t, _, _)| d.starts_with(t)).max_by_key(|(t, _, _)| t.as_os_str().len()).map(|(t, _, _)| t.clone()) };
    let held: HashSet<PathBuf> = claimed.iter().filter_map(|d| tree_of(d)).collect();
    let free: Vec<&(PathBuf, String, Option<String>)> = all.iter().filter(|(dir, head, _)| !dir.starts_with(&ours) && !held.contains(dir) && !taken.contains(head.as_str())).collect();
    if free.is_empty() {
        return Vec::new();
    }
    // when each HEAD was made, in one git call
    let mut args = vec!["log", "--no-walk=unsorted", "--format=%H %ct"];
    args.extend(free.iter().map(|(_, head, _)| head.as_str()));
    let made: HashMap<String, i64> = git::run(repo, &args).unwrap_or_default().lines()
        .filter_map(|l| l.split_once(' ').and_then(|(h, t)| Some((h.to_string(), t.parse().ok()?)))).collect();
    let mut free: Vec<(i64, &(PathBuf, String, Option<String>))> = free.into_iter().filter_map(|t| Some((*made.get(&t.1)?, t))).collect();
    free.sort_by(|a, b| b.0.cmp(&a.0));
    let sessions: Vec<(Option<PathBuf>, &String)> = sessions.iter().map(|(d, n)| (tree_of(d), n)).collect();
    free.into_iter().take(TREES_MOST).filter_map(|(_, (dir, head, branch))| {
        let files = changed(repo, target, head).filter(|f| !f.is_empty())?;
        let place = match sessions.iter().find(|(t, _)| t.as_ref() == Some(dir)) {
            Some((_, n)) => format!("session \"{}\"", plain(n, 60)),
            None => format!("folder {}", plain(&dir.display().to_string(), 300)),
        };
        let branch = branch.clone().unwrap_or_else(|| format!("detached {}", short(head)));
        Some(Loose { name: format!("{branch} ({place})"), head: head.clone(), files })
    }).collect()
}

/// A name or folder as a notice says it: no control character, no double quote, `most` characters at most.
fn plain(name: &str, most: usize) -> String {
    name.chars().filter(|c| !c.is_control() && *c != '"').take(most).collect()
}

/// A tree's HEAD and the files it changed since the target; none when it is no checkout of the project's repository
/// (`common`, its common folder) or has nothing of its own.
fn head_of(t: &Tree, repo: &Path, common: &str, target: &str) -> Option<Head> {
    let said = git::run(&t.dir, &["rev-parse", "--path-format=absolute", "--git-common-dir", "HEAD"]).ok()?;
    let (its_common, head) = said.split_once('\n')?;
    if its_common != common {
        return None;
    }
    let files = changed(repo, target, head)?;
    (!files.is_empty()).then(|| Head { task: t.task.clone(), id: t.id.clone(), head: head.to_string(), files, queued: None })
}

/// The files `head` changed since it left `target`, asked of git once for those commits.
fn changed(repo: &Path, target: &str, head: &str) -> Option<HashSet<String>> {
    judged(format!("files {target}...{head}"), || git::changed(repo, target, head).map(Some)).map(|f| f.into_iter().collect())
}

/// `a`'s agent told, once for that pair of commits, that `b`'s commits do not merge with its own; a queued task learns
/// of it when its batch is built (AC-189).
fn overlap(tx: &Transaction, a: &Head, b: &Head, files: &[String], target: &str, now: i64) -> Result<(), String> {
    let pair = format!("{}-{}", short(&a.head), short(&b.head));
    if a.queued.is_some() || ops::noticed(tx, DISPATCHER, &a.task, "overlap", &pair)? {
        return Ok(());
    }
    let files = files.join(", ");
    let words = match &b.queued {
        Some(place) => format!("{}: its commits and {}'s change the same files: {files}. {} is queued for integration ({place}). Go on; merge {target} once {} lands, before you queue.",
                               a.id, b.id, b.id, b.id),
        None => format!("{}: its commits and {}'s (IN PROGRESS) change the same files: {files}. They cannot be in one batch. Agree with its session who goes first; the other merges {target} after the first lands, then queues.",
                        a.id, b.id),
    };
    ops::notice(tx, DISPATCHER, &a.id, &words, json!({"overlap": pair}), now).map(drop)
}

/// `a`'s agent told, once for that pair of commits, that a tree of the project with no task changes the same files;
/// the tree gets nothing: there is no task to tell.
fn beside(tx: &Transaction, a: &Head, l: &Loose, files: &[String], target: &str, now: i64) -> Result<(), String> {
    let pair = format!("{}-{}", short(&a.head), short(&l.head));
    if ops::noticed(tx, DISPATCHER, &a.task, "overlap", &pair)? {
        return Ok(());
    }
    let words = format!("{}: its commits and {}, a tree with no task, change the same files: {}. Whichever lands second merges {target} first: once it lands, merge {target} before you queue.",
                        a.id, l.name, files.join(", "));
    ops::notice(tx, DISPATCHER, &a.id, &words, json!({"overlap": pair}), now).map(drop)
}

/// `a`'s agent told, once for that target, that the target moved in files it changed since it branched; git runs
/// before the store's turn, which only checks and writes.
fn moved(store: &Store, a: &Head, repo: &Path, target: &str, at_target: &str, now: i64) -> Result<(), String> {
    let mark = format!("{}@{}", short(&a.head), short(at_target));
    if !first_time(format!("moved {mark}")) {
        return Ok(());
    }
    let theirs = changed(repo, &a.head, at_target).unwrap_or_default();
    let mut both: Vec<&str> = a.files.intersection(&theirs).map(String::as_str).collect();
    if both.is_empty() {
        return Ok(());   // it holds the target, or the target moved elsewhere
    }
    both.sort();
    let conflicts = match merge(repo, &a.head, at_target) {
        Ok(Err(files)) => format!(" It conflicts in: {}.", files.join(", ")),
        _ => String::new(),
    };
    let words = format!("{}: {target} moved to {} in files you changed: {}. Merge {target} now, before you queue.{conflicts}", a.id, short(at_target), both.join(", "));
    store.with(|tx| if ops::noticed(tx, DISPATCHER, &a.task, "moved", &mark)? { Ok(()) } else { ops::notice(tx, DISPATCHER, &a.id, &words, json!({"moved": mark}), now).map(drop) })
}

#[cfg(test)]
#[path = "notices_tests.rs"]
mod tests;
