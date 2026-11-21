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
    dir: PathBuf,
    head: String,
    files: HashSet<String>,
    name: String,
}

/// git's answers kept by the repository and the commits asked about: a pair's conflicting files (empty: it merges), the files a HEAD
/// changed since a commit; asked again only when one of them changes.
static ASKED: Mutex<Option<HashMap<String, Vec<String>>>> = Mutex::new(None);

/// Semantic overlaps already checked successfully in a store; a restart checks the persisted events again.
static TOLD: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// The IDs of the tasks a task's commits overlap, as a walk found them at a time.
type Found = (i64, Vec<String>);

/// Each task's overlapping tasks as the last walk found them, by task key (AC-193).
static OVERLAPS: Mutex<Option<HashMap<String, Found>>> = Mutex::new(None);

/// git's answer under `key`, asked the first time only, outside the lock; git failing
/// is said and asked again at the next walk.
fn asked(key: String, ask: impl FnOnce() -> Result<Vec<String>, String>) -> Option<Vec<String>> {
    if let Some(v) = ASKED.lock().as_ref().and_then(|m| m.get(&key)).cloned() {
        return Some(v);
    }
    let v = ask().map_err(|e| tracing::warn!(event = "integration.walk", error = %e, "git could not answer {key}")).ok()?;
    let mut kept = ASKED.lock();
    let map = kept.get_or_insert_with(HashMap::new);
    if map.len() > 10_000 {
        map.clear();
    }
    map.insert(key, v.clone());
    Some(v)
}

/// Avoids repeated store turns for unchanged advice; a failed transaction is retried on the next walk.
fn told(scope: &str, key: serde_json::Value, say: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let key = json!([scope, key]).to_string();
    if TOLD.lock().as_ref().is_some_and(|s| s.contains(&key)) {
        return Ok(());
    }
    say()?;
    let mut kept = TOLD.lock();
    let set = kept.get_or_insert_with(HashSet::new);
    if set.len() > 10_000 {
        set.clear();
    }
    set.insert(key);
    Ok(())
}

/// The IDs of the tasks whose commits overlap task `key`'s, as a walk found them in the last few minutes.
pub fn overlaps_of(key: &str, now: i64) -> Vec<String> {
    OVERLAPS.lock().as_ref().and_then(|m| m.get(key)).filter(|(at, _)| now - at <= OVERLAPS_FOR).map(|(_, ids)| ids.clone()).unwrap_or_default()
}

/// The walk over projects with integration on: each pair of tasks whose commits do not merge is told once for that
/// other task or tree, conflicting files and working/queued state; each task whose files the target changed since
/// it branched is told for new files or relevant target changes. `sessions`: each session's folder on this Mac and its name.
pub fn walk(store: &Store, home: &Path, trees: &[Tree], sessions: &[(PathBuf, String)], now: i64) -> Result<(), String> {
    let projects: HashSet<&str> = trees.iter().map(|t| t.project.as_str()).collect();
    for project in projects {
        let p = place(store, home, project)?;
        let Some((remote, branch)) = integration::target_parts(&p.s.target) else { continue };
        let Ok(target) = git::run(&p.repo, &["rev-parse", &format!("refs/remotes/{remote}/{branch}")]) else { continue };
        let common = git::run(&p.repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).unwrap_or_default();
        let ours: Vec<&Tree> = trees.iter().filter(|t| t.project == project).collect();
        let mut heads: Vec<Head> = ours.iter().filter(|t| t.working).take(TREES_MOST).filter_map(|t| head_of(t, &p.repo, &common, &target)).collect();
        let (entries, scope) = store.with(|tx| Ok((queue_of(tx, project)?, tx.path().unwrap_or_default().to_string())))?;
        heads.extend(entries.into_iter().enumerate().filter_map(|(i, e)| {
            Some(Head { files: changed(&p.repo, &target, &e.commit)?, task: e.task, id: e.id, head: e.commit, queued: Some(ordinal(i + 1)) })
        }));
        let loose = if heads.iter().any(|h| h.queued.is_none()) { loose_trees(&p.repo, &home.join("integration"), &target, &ours, &heads, sessions) } else { Vec::new() };
        let mut found: HashMap<&str, Vec<String>> = HashMap::new();
        for (i, a) in heads.iter().enumerate().filter(|(_, h)| h.queued.is_none()) {
            moved(store, a, &p.repo, &p.s.target, &target, now)?;
            for b in heads.iter().skip(i + 1).chain(heads.iter().take(i).filter(|h| h.queued.is_some())) {
                let Some(files) = conflict(&p.repo, (&a.head, &a.files), (&b.head, &b.files), &target) else { continue };
                let key = |other: &Head| json!(["task", other.task, files, other.queued.is_some(), p.s.target]).to_string();
                told(&scope, json!([a.task, key(b)]), || {
                    store.with(|tx| tell(tx, a, &key(b), &overlap_words(a, b, &files), now)
                        .and_then(|()| tell(tx, b, &key(a), &overlap_words(b, a, &files), now)))
                })?;
                found.entry(a.task.as_str()).or_default().push(b.id.clone());
                found.entry(b.task.as_str()).or_default().push(a.id.clone());
            }
            for l in &loose {
                if let Some(files) = conflict(&p.repo, (&a.head, &a.files), (&l.head, &l.files), &target) {
                    let key = json!(["tree", l.dir, files, p.s.target]).to_string();
                    told(&scope, json!([a.task, key]), || {
                        let words = format!("{}: its commits and {}, a tree with no task, change the same files: {}. Do not wait for it: integrate when your work is ready; if it lands first, the queue returns yours with the files to merge.",
                                            a.id, l.name, files.join(", "));
                        store.with(|tx| tell(tx, a, &key, &words, now))   // the tree gets nothing: there is no task to tell
                    })?;
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

/// The sorted files two HEADs, each with the files it changed, both changed and cannot merge in:
/// once for those commits and the target; none when they change no file in common or merge.
fn conflict(repo: &Path, (a, a_files): (&str, &HashSet<String>), (b, b_files): (&str, &HashSet<String>), target: &str) -> Option<Vec<String>> {
    if a_files.is_disjoint(b_files) {
        return None;
    }
    let files = asked(format!("{}:{a}-{b}@{target}", repo.display()), || merge(repo, a, b).map(|r| {
        let mut files = r.err().unwrap_or_default();
        files.sort();
        files
    }))?;
    (!files.is_empty()).then_some(files)
}

/// A worktree of the project's checkout as git lists it.
struct WorkTree {
    dir: PathBuf,
    head: String,
    branch: Option<String>,
}

/// The project's worktrees with commits of their own and no task, newest first, at most `TREES_MOST`, none of
/// terminum's own under `own`; each named by its branch and the session in it, else its folder. A tree with a task:
/// one holding the folder of a task's session (`ours`), or at a HEAD a task or the target holds.
fn loose_trees(repo: &Path, own: &Path, target: &str, ours: &[&Tree], heads: &[Head], sessions: &[(PathBuf, String)]) -> Vec<Loose> {
    let Ok(listed) = git::run(repo, &["worktree", "list", "--porcelain"]) else { return Vec::new() };
    let real = |d: &Path| std::fs::canonicalize(d).unwrap_or_else(|_| d.to_path_buf());
    let own = real(own);
    let all: Vec<WorkTree> = listed.split("\n\n").filter_map(|block| {
        let field = |k: &str| block.lines().find_map(|l| l.strip_prefix(k)).map(str::to_string);
        Some(WorkTree { dir: real(Path::new(&field("worktree ")?)), head: field("HEAD ")?, branch: field("branch ").map(|b| b.trim_start_matches("refs/heads/").to_string()) })
    }).collect();
    // the tree a folder is in: the deepest holding it (a primary checkout holds its .worktrees)
    let tree_of = |d: &Path| { let d = real(d); all.iter().filter(|t| d.starts_with(&t.dir)).max_by_key(|t| t.dir.as_os_str().len()).map(|t| t.dir.clone()) };
    let held: HashSet<PathBuf> = ours.iter().filter_map(|t| tree_of(&t.dir)).collect();
    let taken: HashSet<&str> = heads.iter().map(|h| h.head.as_str()).chain([target]).collect();
    let free: Vec<&WorkTree> = all.iter().filter(|t| !t.dir.starts_with(&own) && !held.contains(&t.dir) && !taken.contains(t.head.as_str())).collect();
    if free.is_empty() {
        return Vec::new();
    }
    // when each HEAD was made, in one git call
    let mut args = vec!["log", "--no-walk=unsorted", "--format=%H %ct"];
    args.extend(free.iter().map(|t| t.head.as_str()));
    let made: HashMap<String, i64> = git::run(repo, &args).unwrap_or_default().lines()
        .filter_map(|l| l.split_once(' ').and_then(|(h, t)| Some((h.to_string(), t.parse().ok()?)))).collect();
    let mut free: Vec<(i64, &WorkTree)> = free.into_iter().filter_map(|t| Some((*made.get(&t.head)?, t))).collect();
    free.sort_by_key(|(made, _)| std::cmp::Reverse(*made));
    let sessions: Vec<(Option<PathBuf>, &String)> = sessions.iter().map(|(d, n)| (tree_of(d), n)).collect();
    free.into_iter().take(TREES_MOST).filter_map(|(_, t)| {
        let files = changed(repo, target, &t.head).filter(|f| !f.is_empty())?;
        let place = match sessions.iter().find(|(d, _)| d.as_ref() == Some(&t.dir)) {
            Some((_, n)) => format!("session \"{}\"", plain(n, 60)),
            None => format!("folder {}", plain(&t.dir.display().to_string(), 300)),
        };
        let branch = t.branch.clone().unwrap_or_else(|| format!("detached {}", short(&t.head)));
        Some(Loose { dir: t.dir.clone(), name: format!("{branch} ({place})"), head: t.head.clone(), files })
    }).collect()
}

/// A name or folder as a notice says it: nothing stray (control, invisible), no double quote, `most` characters.
fn plain(name: &str, most: usize) -> String {
    crate::termtext::clean(name, false).chars().filter(|c| *c != '"').take(most).collect()
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
    asked(format!("{}:files {target}...{head}", repo.display()), || git::changed(repo, target, head)).map(|f| f.into_iter().collect())
}

/// `a`'s agent told `words` once for the advice's semantic key; a queued task learns of it when its batch is built
/// (AC-189).
fn tell(tx: &Transaction, a: &Head, key: &str, words: &str, now: i64) -> Result<(), String> {
    if a.queued.is_some() || ops::noticed(tx, DISPATCHER, &a.task, "overlap", key)? {
        return Ok(());
    }
    ops::notice(tx, DISPATCHER, &a.id, words, json!({"overlap": key}), now).map(drop)
}

/// What `a`'s agent is told of `b`'s commits that do not merge with its own: working, or queued at its place.
fn overlap_words(a: &Head, b: &Head, files: &[String]) -> String {
    let files = files.join(", ");
    match &b.queued {
        Some(place) => format!("{}: its commits and {}'s change the same files: {files}. {} is queued for integration ({place}). Do not wait for it: integrate when your work is ready; the queue holds yours until {}'s batch ends and returns it with the files to merge if it no longer merges.",
                               a.id, b.id, b.id, b.id),
        None => format!("{}: its commits and {}'s (IN PROGRESS) change the same files: {files}. They cannot be in one batch. Do not wait for {}: integrate as soon as your work is ready. The first queued lands first; the queue returns the other with the files to merge.",
                        a.id, b.id, b.id),
    }
}

/// `a`'s agent told for new affected files or a target change in those files; git runs
/// before the store's turn, which only checks and writes.
fn moved(store: &Store, a: &Head, repo: &Path, target: &str, at_target: &str, now: i64) -> Result<(), String> {
    let theirs = changed(repo, &a.head, at_target).unwrap_or_default();
    let mut both: Vec<&str> = a.files.intersection(&theirs).map(String::as_str).collect();
    if both.is_empty() {
        return Ok(());   // it holds the target, or the target moved elsewhere
    }
    both.sort();
    let previous = store.with(|tx| ops::rows(tx, "SELECT json_extract(data, '$.moved_target'), json_extract(data, '$.moved_files') FROM events WHERE task = ?1 AND kind = 'comment' AND author = ?2
        AND json_extract(data, '$.notice') IS NOT NULL AND json_extract(data, '$.moved') = ?3 ORDER BY rowid DESC LIMIT 1",
        (&a.task, DISPATCHER, target), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))))?.pop();
    if let Some((previous, files)) = previous {
        let files: Vec<String> = serde_json::from_str(&files).map_err(|e| e.to_string())?;
        if both.iter().all(|f| files.iter().any(|p| p.as_str() == *f)) {
            if previous == at_target {
                return Ok(());
            }
            let Some(files) = asked(format!("{}:target {previous}..{at_target}", repo.display()), || {
                git::run(repo, &["-c", "core.quotePath=true", "diff", "--name-only", "--no-renames", &previous, at_target])
                    .map(|s| s.lines().map(str::to_string).collect())
            }) else { return Ok(()) };
            if !files.iter().any(|f| both.contains(&f.as_str())) {
                return Ok(());
            }
        }
    }
    let conflicts = conflict(repo, (&a.head, &a.files), (at_target, &theirs), at_target)
        .map(|files| format!(" It conflicts in: {}.", files.join(", "))).unwrap_or_default();
    let words = format!("{}: {target} moved to {} in files you changed: {}. Merge {target} now, before you queue.{conflicts}", a.id, short(at_target), both.join(", "));
    let key = json!([target, both, at_target]).to_string();
    store.with(|tx| if ops::noticed(tx, DISPATCHER, &a.task, "moved_at", &key)? { Ok(()) } else {
        ops::notice(tx, DISPATCHER, &a.id, &words, json!({"moved": target, "moved_files": both, "moved_target": at_target, "moved_at": key}), now).map(drop)
    })
}

#[cfg(test)]
#[path = "notices_tests.rs"]
mod tests;
