// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! One step of the integration queue (`terminum-integration.md` AC-188…AC-191): with no batch in work, the next
//! project's batch is built; else the batch in work moves on: its check started, followed or ended, a green one
//! published only while its target has not moved (else built anew on it and checked again), its tasks DONE, the
//! project's folder fast-forwarded, After run once. Nothing but a candidate whose own check was green is published;
//! a project whose integration was turned off gives its batch up.

use std::path::{Path, PathBuf};

use super::batch::choose;
use super::git::{self, short};
use super::runner::Runner;
use super::store::{self as bs, Batch};
use crate::tasks::integration::{self, Settings};
use crate::tasks::ops;
use crate::tasks::store::Store;

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
    git::run(&p.repo, &["fetch", "--quiet", remote]).map_err(|e| format!("{remote} could not be fetched: {e}"))?;
    git::run(&p.repo, &["rev-parse", &format!("refs/remotes/{remote}/{branch}")]).map_err(|_| format!("{}: no such branch at {remote}.", p.s.target))
}

/// One step.
pub fn step(store: &Store, runner: &dyn Runner, home: &Path, now: i64) -> Result<(), String> {
    match store.with(bs::busy)? {
        Some(b) => advance(store, runner, home, b, now),
        None => start_next(store, home, now),
    }
}

fn start_next(store: &Store, home: &Path, now: i64) -> Result<(), String> {
    let Some(project) = store.with(bs::next_project)? else { return Ok(()) };
    let p = place(store, home, &project)?;
    // why the queue waits is said, never only logged (CLAUDE.md §1)
    let built = target_now(&p).and_then(|base| {
        let queue = store.with(|tx| bs::waiting(tx, &project))?;
        choose(&p.repo, &base, &queue, p.s.size()).map(|c| (base, c))
    });
    store.with(|tx| match &built {
        Ok((base, choice)) => {
            bs::waits(tx, &project, None)?;
            bs::open(tx, &project, &p.s, base, choice, now).map(drop)
        }
        Err(why) => bs::waits(tx, &project, Some(why)),
    })
}

/// The candidate's tree: a worktree of the project's repository, detached, kept for its caches (§9).
fn tree_at(p: &Place, project: &str, candidate: &str) -> Result<PathBuf, String> {
    let tree = integration::tree_in(&p.home, project);
    let made = if tree.join(".git").exists() {
        git::run(&tree, &["checkout", "--quiet", "--detach", "--force", candidate]).and_then(|_| git::run(&tree, &["clean", "-fdq"]))
    } else {
        let _ = git::run(&p.repo, &["worktree", "prune"]);
        std::fs::create_dir_all(tree.parent().unwrap_or(&tree)).map_err(|e| e.to_string())
            .and_then(|()| git::run(&p.repo, &["worktree", "add", "--quiet", "--detach", &tree.to_string_lossy(), candidate]))
    };
    made.map(|_| tree).map_err(|e| format!("The candidate's tree could not be made: {e}"))
}

fn log_of(p: &Place, project: &str, b: &Batch, step: &str) -> PathBuf {
    integration::tree_in(&p.home, project).with_file_name(format!("batch-{}.{}.{step}.log", b.number, b.round))
}

/// The variables every command gets (§9).
fn vars(store: &Store, p: &Place, b: &Batch, tree: &Path) -> Vec<(&'static str, String)> {
    let ids: Vec<String> = store.with(|tx| Ok(b.tasks.iter().filter_map(|k| bs::slug(tx, k).ok()).collect())).unwrap_or_default();
    vec![("TM_INTEGRATION_BATCH", b.number.to_string()), ("TM_INTEGRATION_ROUND", b.round.to_string()),
        ("TM_INTEGRATION_SHA", b.candidate.clone().unwrap_or_default()), ("TM_INTEGRATION_BASE", b.base.clone()),
        ("TM_INTEGRATION_TARGET", p.s.target.clone()), ("TM_INTEGRATION_TASKS", ids.join(" ")),
        ("TM_INTEGRATION_TREE", tree.display().to_string()), ("TM_INTEGRATION_PROJECT_DIR", p.repo.display().to_string())]
}

/// A line that ended: its output kept in the batch's log, its window closed; its status.
fn ended(runner: &dyn Runner, pane: &str, log: &Path, status: i32) -> i32 {
    if let Err(e) = crate::logs::snapshot(log, &runner.output(pane)) {
        tracing::warn!(event = "integration.log", path = %log.display(), error = %e);
    }
    runner.close(pane);
    status
}

fn advance(store: &Store, runner: &dyn Runner, home: &Path, b: Batch, now: i64) -> Result<(), String> {
    let p = place(store, home, &b.project)?;
    // turned off: nothing more is merged or pushed (AC-186); a published batch's After runs on
    if !p.s.on && b.state != "after" {
        if let Some(pane) = &b.pane {
            runner.stop(pane);
            runner.close(pane);
        }
        return store.with(|tx| bs::give_up(tx, &b, &format!("integration was turned off for {}", p.name), now));
    }
    match b.state.as_str() {
        "checking" | "publishing" | "after" => follow(store, runner, &p, b, now),
        "green" => publish(store, &p, b, now),
        // a task left the batch: the rest built anew on the target as it is, a running line ended first
        "rebuild" => {
            if let Some(pane) = &b.pane {
                runner.stop(pane);
                runner.close(pane);
            }
            match target_now(&p) {
                Ok(t) => again(store, &p, &b, &t, now),
                Err(why) => not_published(store, &b, &why, now),
            }
        }
        _ => Ok(()),
    }
}

/// A line of the batch's step: started, followed, ended; the pane cleared in the transaction that moves the state on.
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
        let v = vars(store, p, &b, &integration::tree_in(&p.home, &b.project));
        return match runner.start(&label, &folder, &line, &v) {
            Ok(pane) => store.with(|tx| bs::set_pane(tx, b.id, Some(&pane), now)),
            Err(why) => red(store, &b, &format!("The {step} did not start: {why}"), now),
        };
    };
    let log = log_of(p, &b.project, &b, step);
    let status = match runner.status(&pane) {
        // lost with its window: run again, not counted as red (AC-190)
        Err(_) => return store.with(|tx| { bs::set_pane(tx, b.id, None, now)?; bs::tell_task(tx, &b, &format!("Batch {}'s {step} runs again: its window was lost.", b.number), now) }),
        Ok(None) if step == "check" && now - b.step_started.unwrap_or(now) > p.s.check_minutes * 60 => {
            runner.stop(&pane);
            ended(runner, &pane, &log, -1);
            return red(store, &b, &format!("The check ran over {} minutes.", p.s.check_minutes), now);
        }
        Ok(None) => return Ok(()),
        Ok(Some(code)) => ended(runner, &pane, &log, code),
    };
    match (step, status) {
        ("check", 0) => store.with(|tx| { bs::set_pane(tx, b.id, None, now)?; bs::round_ends(tx, &b, "green", None, now)?; bs::set_state(tx, b.id, "green", None, now) }),
        ("check", code) => red(store, &b, &format!("The check ended with status {code}. Its log: {}", log.display()), now),
        ("publish", 0) => match target_now(p) {
            Ok(t) => landed(store, p, b, &t, now),
            Err(why) => not_published(store, &b, &why, now),
        },
        ("publish", code) => not_published(store, &b, &format!("Publish ended with status {code}. Its log: {}", log.display()), now),
        (_, 0) => store.with(|tx| { bs::set_pane(tx, b.id, None, now)?; bs::set_state(tx, b.id, "ended", None, now) }),
        (_, code) => store.with(|tx| {
            bs::set_pane(tx, b.id, None, now)?;
            bs::set_state(tx, b.id, "after failed", Some(&format!("status {code}")), now)?;
            bs::wait_for_human(tx, &b, &format!("After failed for batch {} with status {code}: the batch stays published and its tasks DONE. Its log: {}", b.number, log.display()), now)
        }),
    }
}

/// A red round: nothing published, the batch's size halved at its first, its integration task told why.
fn red(store: &Store, b: &Batch, why: &str, now: i64) -> Result<(), String> {
    store.with(|tx| {
        bs::set_pane(tx, b.id, None, now)?;
        bs::round_ends(tx, b, "red", Some(why), now)?;
        bs::red_round(tx, b)?;
        bs::set_state(tx, b.id, bs::RED, Some(why), now)?;
        bs::wait_for_human(tx, b, &format!("Batch {}'s round {} is red: {why}", b.number, b.round), now)
    })
}

fn not_published(store: &Store, b: &Batch, why: &str, now: i64) -> Result<(), String> {
    store.with(|tx| {
        bs::set_pane(tx, b.id, None, now)?;
        bs::set_state(tx, b.id, "not published", Some(why), now)?;
        bs::wait_for_human(tx, b, &format!("Batch {} is not published: {why}", b.number), now)
    })
}

/// A green candidate published only on the target it was checked on; a moved target builds it anew and checks again.
fn publish(store: &Store, p: &Place, b: Batch, now: i64) -> Result<(), String> {
    let target = match target_now(p) {
        Ok(t) => t,
        Err(why) => return not_published(store, &b, &why, now),
    };
    let candidate = b.candidate.clone().unwrap_or_default();
    // published before terminumd stopped, or by the project's Publish: never built again on itself (AC-191)
    if git::ok(&p.repo, &["merge-base", "--is-ancestor", &candidate, &target])? {
        return landed(store, p, b, &target, now);
    }
    if target != b.base {
        return again(store, p, &b, &target, now);
    }
    if !p.s.publish.is_empty() {
        return store.with(|tx| bs::set_state(tx, b.id, "publishing", None, now));
    }
    let (remote, branch) = integration::target_parts(&p.s.target).ok_or_else(|| integration::not_a_target(&p.s.target))?;
    // never forced: no "+", the remote refuses what is not a fast-forward
    match git::run(&p.repo, &["push", "--quiet", remote, &format!("{candidate}:refs/heads/{branch}")]) {
        Ok(_) => match target_now(p) {
            Ok(t) => landed(store, p, b, &t, now),
            Err(why) => not_published(store, &b, &why, now),
        },
        Err(refused) => match target_now(p) {
            Ok(t) if t != b.base => again(store, p, &b, &t, now),
            _ => not_published(store, &b, &refused, now),
        },
    }
}

/// The target moved under a green batch, or a task left it: its tasks tried anew on `target`, a new round.
fn again(store: &Store, p: &Place, b: &Batch, target: &str, now: i64) -> Result<(), String> {
    let queue = store.with(|tx| bs::queued_of(tx, b))?;
    let c = choose(&p.repo, target, &queue, queue.len().max(1))?;
    store.with(|tx| {
        for (q, why) in &c.returned {
            bs::give_back(tx, &q.task, &bs::conflict_words(q, why, &p.s, target), now)?;
        }
        bs::round_ends(tx, b, "moved", Some(&format!("{} moved to {} or a task left: built anew", p.s.target, short(target))), now)?;
        if c.chosen.is_empty() {
            return bs::give_up(tx, b, "every task returned", now);
        }
        // the held stay queued for the next batch
        for (q, _) in &c.held {
            bs::unbatch(tx, &q.task)?;
        }
        let state = if p.s.check.is_empty() { "green" } else { "checking" };
        bs::new_round(tx, b, &c.chosen, target, c.candidate.as_deref(), state)
    })
}

/// Published: its tasks DONE with their results, the size doubled, the project's folder fast-forwarded, After next.
/// Nothing after the push may fail for a task gone meanwhile: its DONE is skipped, the rest go on.
fn landed(store: &Store, p: &Place, b: Batch, target: &str, now: i64) -> Result<(), String> {
    let candidate = b.candidate.clone().unwrap_or_default();
    if !git::ok(&p.repo, &["merge-base", "--is-ancestor", &candidate, target])? {
        return not_published(store, &b, &format!("{} is not in {} after the publish.", short(&candidate), p.s.target), now);
    }
    let moved = folder_follows(p);
    store.with(|tx| {
        bs::set_pane(tx, b.id, None, now)?;
        let ids = bs::done_all(tx, &b, &p.s.target, &candidate, now)?;
        bs::resize(tx, &b.project, true)?;
        bs::round_ends(tx, &b, "published", None, now)?;
        let folder = match &moved {
            Ok(()) => String::new(),
            Err(w) => format!(" {}'s folder stays where it is: {w}", p.name),
        };
        bs::close_task(tx, &b, &format!("Batch {} published at {}: {}.{folder}", b.number, short(&candidate), ids.join(", ")), now)?;
        let next = if p.s.after.is_empty() || moved.is_err() { "ended" } else { "after" };
        bs::set_state(tx, b.id, next, None, now)
    })
}

/// The project's folder fast-forwarded to the target when it stands clean on its branch (AC-191).
fn folder_follows(p: &Place) -> Result<(), String> {
    let (remote, branch) = integration::target_parts(&p.s.target).ok_or_else(|| integration::not_a_target(&p.s.target))?;
    let on = git::run(&p.repo, &["branch", "--show-current"])?;
    if on != branch {
        return Err(format!("it is on {}, not {branch}", if on.is_empty() { "no branch" } else { &on }));
    }
    git::run(&p.repo, &["merge", "--ff-only", "--quiet", &format!("{remote}/{branch}")]).map(drop)
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;

