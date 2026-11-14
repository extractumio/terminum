// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! One step of the integration queue (`terminum-integration.md` AC-188…AC-191): with no batch in work, the next
//! project's batch is built; else the batch in work moves on: its check started, followed or ended, a green one
//! published only while its target has not moved (else built anew on it and checked again), its tasks DONE, the
//! project's folder fast-forwarded, After run once. Nothing but a candidate whose own check was green is published.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::batch::{choose, Queued};
use super::runner::Runner;
use super::store::{self as bs, Batch};
use crate::tasks::integration::{self, Settings};
use crate::tasks::integration_queue::and_list;
use crate::tasks::ops::{self, sql};
use crate::tasks::store::{Store, DISPATCHER};

fn git(dir: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git").arg("-C").arg(dir).args(args).output().map_err(|e| format!("git: {e}"))
}

fn said(o: &std::process::Output) -> String {
    let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
    if e.is_empty() { String::from_utf8_lossy(&o.stdout).trim().to_string() } else { e }
}

fn short(sha: &str) -> &str {
    &sha[..9.min(sha.len())]
}

/// A project's facts for a step: its settings, name and repository.
struct Place {
    s: Settings,
    name: String,
    repo: PathBuf,
    /// terminum's own folder (`TM_HOME`): the candidate's tree and logs are under it
    home: PathBuf,
}

fn place(store: &Store, home: &Path, project: &str) -> Result<Place, String> {
    store.with(|tx| {
        let s = integration::read(tx, project)?;
        let (name, _, dir) = ops::project_place(tx, project)?;
        Ok(Place { repo: integration::local_dir(&dir), s, name, home: home.to_path_buf() })
    })
}

/// The target as fetched now.
fn target_now(p: &Place) -> Result<String, String> {
    let (remote, branch) = integration::target_parts(&p.s.target).ok_or_else(|| integration::not_a_target(&p.s.target))?;
    let f = git(&p.repo, &["fetch", "--quiet", remote])?;
    if !f.status.success() {
        return Err(format!("{remote} could not be fetched: {}", said(&f)));
    }
    let o = git(&p.repo, &["rev-parse", &format!("refs/remotes/{remote}/{branch}")])?;
    if !o.status.success() {
        return Err(format!("{}: no such branch at {remote}.", p.s.target));
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// One step; the tasks it changed, for the pages.
pub fn step(store: &Store, runner: &dyn Runner, home: &Path, now: i64) -> Result<(), String> {
    match store.with(bs::busy)? {
        Some(b) => advance(store, runner, home, b, now),
        None => start_next(store, home, now),
    }
}

fn start_next(store: &Store, home: &Path, now: i64) -> Result<(), String> {
    let Some(project) = store.with(bs::next_project)? else { return Ok(()) };
    let p = place(store, home, &project)?;
    let base = match target_now(&p) {
        Ok(b) => b,
        Err(why) => return store.with(|tx| bs::waits(tx, &project, Some(&why))),
    };
    let queue = store.with(|tx| bs::waiting(tx, &project))?;
    let choice = choose(&p.repo, &base, &queue, p.s.size())?;
    store.with(|tx| {
        bs::waits(tx, &project, None)?;
        bs::open(tx, &project, &p.s, &base, &choice, now).map(drop)
    })
}

/// The candidate's tree: a worktree of the project's repository, detached, kept for its caches (§9).
fn tree_at(p: &Place, project: &str, candidate: &str) -> Result<PathBuf, String> {
    let tree = integration::tree_in(&p.home, project);
    if tree.join(".git").exists() {
        for args in [vec!["checkout", "--quiet", "--detach", "--force", candidate], vec!["clean", "-fdq"]] {
            let o = git(&tree, &args)?;
            if !o.status.success() {
                return Err(format!("The candidate's tree could not be made: {}", said(&o)));
            }
        }
    } else {
        let _ = git(&p.repo, &["worktree", "prune"]);
        std::fs::create_dir_all(tree.parent().unwrap_or(&tree)).map_err(|e| format!("The candidate's tree could not be made: {e}"))?;
        let o = git(&p.repo, &["worktree", "add", "--quiet", "--detach", &tree.to_string_lossy(), candidate])?;
        if !o.status.success() {
            return Err(format!("The candidate's tree could not be made: {}", said(&o)));
        }
    }
    Ok(tree)
}

fn log_of(p: &Place, project: &str, b: &Batch, step: &str) -> PathBuf {
    integration::tree_in(&p.home, project).with_file_name(format!("batch-{}.{}.{step}.log", b.number, b.round))
}

/// The variables every command gets (§9).
fn vars(store: &Store, p: &Place, b: &Batch, tree: &Path) -> Result<Vec<(&'static str, String)>, String> {
    let ids = store.with(|tx| b.tasks.iter().map(|k| bs::slug(tx, k)).collect::<Result<Vec<_>, _>>())?;
    Ok(vec![("TM_INTEGRATION_BATCH", b.number.to_string()), ("TM_INTEGRATION_ROUND", b.round.to_string()),
        ("TM_INTEGRATION_SHA", b.candidate.clone().unwrap_or_default()), ("TM_INTEGRATION_BASE", b.base.clone()),
        ("TM_INTEGRATION_TARGET", p.s.target.clone()), ("TM_INTEGRATION_TASKS", ids.join(" ")),
        ("TM_INTEGRATION_TREE", tree.display().to_string()), ("TM_INTEGRATION_PROJECT_DIR", p.repo.display().to_string())])
}

/// A line that ended: its output kept in the batch's log, its window closed; its status.
fn ended(runner: &dyn Runner, pane: &str, log: &Path, status: i32) -> i32 {
    let _ = std::fs::write(log, runner.output(pane));
    runner.close(pane);
    status
}

fn advance(store: &Store, runner: &dyn Runner, home: &Path, b: Batch, now: i64) -> Result<(), String> {
    let p = place(store, home, &b.project)?;
    match b.state.as_str() {
        "checking" | "publishing" | "after" => follow(store, runner, &p, b, now),
        "green" => publish(store, runner, &p, b, now),
        _ => Ok(()),
    }
}

/// A line of the batch's step: started, followed, ended.
fn follow(store: &Store, runner: &dyn Runner, p: &Place, b: Batch, now: i64) -> Result<(), String> {
    let (step, line, folder) = match b.state.as_str() {
        "checking" => ("check", p.s.check.clone(), None),
        "publishing" => ("publish", p.s.publish.clone(), None),
        _ => ("after", p.s.after.clone(), Some(p.repo.clone())),
    };
    let candidate = b.candidate.clone().unwrap_or_default();
    let Some(pane) = b.pane.clone() else {
        let folder = match folder {
            Some(f) => f,
            None => match tree_at(p, &b.project, &candidate) {
                Ok(t) => t,
                Err(why) => return red(store, &b, &why, now),
            },
        };
        let label = format!("batch-{}.{}.{step}", b.number, b.round);
        let v = vars(store, p, &b, &integration::tree_in(&p.home, &b.project))?;
        return match runner.start(&label, &folder, &line, &v) {
            Ok(pane) => store.with(|tx| bs::set_pane(tx, b.id, Some(&pane), now)),
            Err(why) => red(store, &b, &format!("The {step} did not start: {why}"), now),
        };
    };
    let log = log_of(p, &b.project, &b, step);
    let status = match runner.status(&pane) {
        // lost with its window: run again, not counted as red (AC-190)
        Err(_) => return store.with(|tx| { bs::set_pane(tx, b.id, None, now)?; ops::comment(tx, DISPATCHER, &task_id(tx, &b)?, &format!("Batch {}'s {step} ran again: its window was lost.", b.number), now).map(drop) }),
        Ok(None) if step == "check" && now - b.step_started.unwrap_or(now) > p.s.check_minutes * 60 => {
            runner.stop(&pane);
            ended(runner, &pane, &log, -1);
            return red(store, &b, &format!("The check ran over {} minutes.", p.s.check_minutes), now);
        }
        Ok(None) => return Ok(()),
        Ok(Some(code)) => ended(runner, &pane, &log, code),
    };
    store.with(|tx| bs::set_pane(tx, b.id, None, now))?;
    match (step, status) {
        ("check", 0) => store.with(|tx| { bs::round_ends(tx, &b, "green", None, now)?; bs::set_state(tx, b.id, "green", None, now) }),
        ("check", code) => red(store, &b, &format!("The check ended with status {code}. Its log: {}", log.display()), now),
        ("publish", 0) => landed(store, runner, p, b, now),
        ("publish", code) => not_published(store, &b, &format!("Publish ended with status {code}. Its log: {}", log.display()), now),
        (_, 0) => store.with(|tx| bs::set_state(tx, b.id, "ended", None, now)),
        (_, code) => store.with(|tx| {
            bs::set_state(tx, b.id, "after failed", Some(&format!("status {code}")), now)?;
            wait_for_human(tx, &b, &format!("After failed for batch {} with status {code}: the batch stays published and its tasks DONE. Its log: {}", b.number, log.display()), now)
        }),
    }
}

fn task_id(tx: &rusqlite::Transaction, b: &Batch) -> Result<String, String> {
    bs::slug(tx, b.task.as_deref().unwrap_or_default())
}

/// The integration task waits for a human, saying why (until its agent comes, AC-194).
fn wait_for_human(tx: &rusqlite::Transaction, b: &Batch, words: &str, now: i64) -> Result<(), String> {
    let id = task_id(tx, b)?;
    ops::questions(tx, DISPATCHER, &id, words, now)?;
    let t = ops::get(tx, &id)?;
    ops::place(tx, DISPATCHER, &t, "WAIT FOR HUMAN", now)
}

/// A red round: nothing published, the batch's size halved at its first, its integration task told why.
fn red(store: &Store, b: &Batch, why: &str, now: i64) -> Result<(), String> {
    store.with(|tx| {
        bs::round_ends(tx, b, "red", Some(why), now)?;
        if b.red_rounds == 0 {
            tx.execute("UPDATE project_integration SET size = max(1, size / 2) WHERE project = ?1", [&b.project]).map_err(sql)?;
        }
        tx.execute("UPDATE integration_batches SET red_rounds = red_rounds + 1 WHERE id = ?1", [b.id]).map_err(sql)?;
        bs::set_state(tx, b.id, bs::RED, Some(why), now)?;
        wait_for_human(tx, b, &format!("Batch {}'s round {} is red: {why}", b.number, b.round), now)
    })
}

fn not_published(store: &Store, b: &Batch, why: &str, now: i64) -> Result<(), String> {
    store.with(|tx| {
        bs::set_state(tx, b.id, "not published", Some(why), now)?;
        wait_for_human(tx, b, &format!("Batch {} is not published: {why}", b.number), now)
    })
}

/// A green candidate published only on the target it was checked on; a moved target builds it anew and checks again.
fn publish(store: &Store, runner: &dyn Runner, p: &Place, b: Batch, now: i64) -> Result<(), String> {
    let target = match target_now(p) {
        Ok(t) => t,
        Err(why) => return not_published(store, &b, &why, now),
    };
    let candidate = b.candidate.clone().unwrap_or_default();
    // published before terminumd stopped, or by the project's Publish: never built again on itself (AC-191)
    if git(&p.repo, &["merge-base", "--is-ancestor", &candidate, &target])?.status.success() {
        return landed(store, runner, p, b, now);
    }
    if target != b.base {
        return again(store, p, &b, &target, now);
    }
    if !p.s.publish.is_empty() {
        return store.with(|tx| bs::set_state(tx, b.id, "publishing", None, now));
    }
    let (remote, branch) = integration::target_parts(&p.s.target).ok_or_else(|| integration::not_a_target(&p.s.target))?;
    // never forced: no "+", the remote refuses what is not a fast-forward
    let pushed = git(&p.repo, &["push", "--quiet", remote, &format!("{candidate}:refs/heads/{branch}")])?;
    if pushed.status.success() {
        return landed(store, runner, p, b, now);
    }
    match target_now(p) {
        Ok(t) if t != b.base => again(store, p, &b, &t, now),
        _ => not_published(store, &b, &said(&pushed), now),
    }
}

/// The target moved under a green batch: its tasks tried anew on it, a new round.
fn again(store: &Store, p: &Place, b: &Batch, target: &str, now: i64) -> Result<(), String> {
    let queue: Vec<Queued> = store.with(|tx| b.tasks.iter().filter_map(|k| {
        let commit: Option<String> = tx.query_row("SELECT commit_sha FROM integration_entries WHERE task = ?1", [k], |r| r.get(0)).ok();
        commit.map(|c| bs::slug(tx, k).map(|id| Queued { task: k.clone(), id, commit: c }))
    }).collect::<Result<Vec<_>, _>>())?;
    let c = choose(&p.repo, target, &queue, queue.len().max(1))?;
    store.with(|tx| {
        for (q, why) in &c.returned {
            bs::give_back(tx, &q.task, &bs::conflict_words(q, why, &p.s, target), now)?;
        }
        bs::round_ends(tx, b, "moved", Some(&format!("{} moved to {}: checked again", p.s.target, short(target))), now)?;
        let keys: Vec<String> = c.chosen.iter().map(|q| q.task.clone()).collect();
        if keys.is_empty() {
            return bs::set_state(tx, b.id, "given up", Some("every task returned"), now).and_then(|()| close_task(tx, b, &format!("Batch {}: nothing published, all its tasks returned.", b.number), now));
        }
        // the held stay queued for the next batch
        for (q, _) in &c.held {
            tx.execute("UPDATE integration_entries SET batch = NULL WHERE task = ?1", [&q.task]).map_err(sql)?;
        }
        let state = if p.s.check.is_empty() { "green" } else { "checking" };
        tx.execute("UPDATE integration_batches SET tasks = ?2, base_sha = ?3, candidate_sha = ?4, round = round + 1, state = ?5, pane = NULL WHERE id = ?1",
                   rusqlite::params![b.id, serde_json::to_string(&keys).unwrap_or_default(), target, c.candidate, state]).map_err(sql)?;
        tx.execute("INSERT INTO integration_rounds (batch, round, tasks, base_sha, candidate_sha, result) VALUES (?1, ?2, ?3, ?4, ?5, 'built')",
                   rusqlite::params![b.id, b.round + 1, serde_json::to_string(&keys).unwrap_or_default(), target, c.candidate]).map_err(sql).map(drop)
    })
}

fn close_task(tx: &rusqlite::Transaction, b: &Batch, words: &str, now: i64) -> Result<(), String> {
    let id = task_id(tx, b)?;
    ops::comment(tx, DISPATCHER, &id, words, now)?;
    let t = ops::get(tx, &id)?;
    ops::place(tx, DISPATCHER, &t, "DONE", now)
}

/// Published: its tasks DONE with their results, the size doubled, the project's folder fast-forwarded, After next.
fn landed(store: &Store, _runner: &dyn Runner, p: &Place, b: Batch, now: i64) -> Result<(), String> {
    let target = target_now(p).unwrap_or_default();
    let candidate = b.candidate.clone().unwrap_or_default();
    if !git(&p.repo, &["merge-base", "--is-ancestor", &candidate, &target])?.status.success() {
        return not_published(store, &b, &format!("{} is not in {} after the publish.", short(&candidate), p.s.target), now);
    }
    let moved = folder_follows(p);
    store.with(|tx| {
        let ids: Vec<String> = b.tasks.iter().map(|k| bs::slug(tx, k)).collect::<Result<_, _>>()?;
        for (k, id) in b.tasks.iter().zip(&ids) {
            let (result, author): (String, String) = tx.query_row("SELECT result, author FROM integration_entries WHERE task = ?1", [k], |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
            ops::comment(tx, &author, id, &result, now)?;
            let mates: Vec<&str> = ids.iter().map(String::as_str).filter(|i| i != id).collect();
            let with = if mates.is_empty() { String::new() } else { format!(", with {}", and_list(&mates)) };
            ops::comment(tx, DISPATCHER, id, &format!("Integrated into {} at {} in batch {}{with}.", p.s.target, short(&candidate), b.number), now)?;
            let t = ops::get(tx, id)?;
            ops::place(tx, DISPATCHER, &t, "DONE", now)?;
        }
        tx.execute("UPDATE project_integration SET size = min(batch_most, size * 2) WHERE project = ?1", [&b.project]).map_err(sql)?;
        bs::round_ends(tx, &b, "published", None, now)?;
        let folder = match &moved {
            Ok(()) => String::new(),
            Err(w) => format!(" {}'s folder stays where it is: {w}", p.name),
        };
        close_task(tx, &b, &format!("Batch {} published at {}: {}.{folder}", b.number, short(&candidate), ids.join(", ")), now)?;
        let next = if p.s.after.is_empty() || moved.is_err() { "ended" } else { "after" };
        bs::set_state(tx, b.id, next, None, now)
    })
}

/// The project's folder fast-forwarded to the target when it stands clean on its branch (AC-191).
fn folder_follows(p: &Place) -> Result<(), String> {
    let (remote, branch) = integration::target_parts(&p.s.target).ok_or_else(|| integration::not_a_target(&p.s.target))?;
    let on = String::from_utf8_lossy(&git(&p.repo, &["branch", "--show-current"])?.stdout).trim().to_string();
    if on != branch {
        return Err(format!("it is on {}, not {branch}", if on.is_empty() { "no branch" } else { &on }));
    }
    let o = git(&p.repo, &["merge", "--ff-only", "--quiet", &format!("{remote}/{branch}")])?;
    if o.status.success() { Ok(()) } else { Err(said(&o)) }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
