// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! One step of the integration queue (`terminum-integration.md` AC-188…AC-191): with no batch in work, the next
//! project's batch is built; else the batch in work moves on: its check started, followed or ended, a green one
//! published only while its target has not moved (else built anew on it and checked again), its tasks DONE. A
//! published batch leaves the work to its After and Cleanup, which run beside the next batch (`tail`). Nothing but a
//! candidate whose own check was green is published; a project whose integration was turned off gives its batch up.

use std::path::{Path, PathBuf};

use super::batch::{choose, Queued};
use super::git::{self, short};
use super::runner::Runner;
use super::retry_notes;
use super::store::{self as bs, Batch};
use crate::tasks::integration::{self, Settings};
use crate::tasks::ops;
use crate::tasks::store::Store;

/// A project's facts for a step: its settings, name and repository.
pub(super) struct Place {
    pub(super) s: Settings,
    pub(super) name: String,
    pub(super) repo: PathBuf,
    /// terminum's own folder (`TM_HOME`): the candidate's tree and logs are under it
    pub(super) home: PathBuf,
}

pub(super) fn place(store: &Store, home: &Path, project: &str) -> Result<Place, String> {
    store.with(|tx| {
        let s = integration::read(tx, project)?;
        let (name, _, dir) = ops::project_place(tx, project)?;
        Ok(Place { repo: integration::local_dir(&dir), s, name, home: home.to_path_buf() })
    })
}

/// The target as fetched now.
fn target_now(p: &Place) -> Result<String, String> {
    let (remote, branch) = integration::target_of(&p.s.target)?;
    let fetch = || git::run(&p.repo, &["fetch", "--quiet", remote]);
    let mut fetched = fetch();
    // a fetch beside it (an After's, a Prepare's) holding the same ref: tried once more after a second
    if matches!(&fetched, Err(e) if e.contains("cannot lock ref")) {
        std::thread::sleep(std::time::Duration::from_secs(1));
        fetched = fetch();
    }
    fetched.map_err(|e| format!("{remote} could not be fetched: {e}"))?;
    git::run(&p.repo, &["rev-parse", &format!("refs/remotes/{remote}/{branch}")]).map_err(|_| format!("{}: no such branch at {remote}.", p.s.target))
}

/// One step.
pub fn step(store: &Store, runner: &dyn Runner, home: &Path, now: i64) -> Result<(), String> {
    tidy(store, home)?;
    // a queued task's Prepare and Verify going wrong holds no batch back: said, and tried again next beat
    if let Err(e) = super::stages::verify_step(store, runner, home, now) {
        tracing::warn!(event = "integration.verify", error = %e);
    }
    let work = match store.with(bs::busy)? {
        Some(b) => advance(store, runner, home, b, now),
        None => start_next(store, home, now),
    };
    // after the work, whatever it met: a batch published in this step takes its After's turn in it
    work.and(super::tail::step(store, runner, home, now))
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
    worktree(p, integration::tree_in(&p.home, project), candidate).map_err(|e| format!("The candidate's tree could not be made: {e}"))
}

/// A detached worktree of the project's repository at `commit`, made or moved there, clean.
pub(super) fn worktree(p: &Place, tree: PathBuf, commit: &str) -> Result<PathBuf, String> {
    let made = if tree.join(".git").exists() {
        git::run(&tree, &["checkout", "--quiet", "--detach", "--force", commit]).and_then(|_| git::run(&tree, &["clean", "-fdq"]))
    } else {
        let _ = git::run(&p.repo, &["worktree", "prune"]);
        std::fs::create_dir_all(tree.parent().unwrap_or(&tree)).map_err(|e| e.to_string())
            .and_then(|()| git::run(&p.repo, &["worktree", "add", "--quiet", "--detach", &tree.to_string_lossy(), commit]))
    };
    made.map(|_| tree)
}

fn log_of(p: &Place, project: &str, b: &Batch, step: &str) -> PathBuf {
    integration::log_in(&p.home, project, b.number, b.round, step)
}

/// The variables every command gets (§9).
fn vars(store: &Store, p: &Place, b: &Batch, tree: &Path) -> Vec<(&'static str, String)> {
    let ids: Vec<String> = store.with(|tx| Ok(b.tasks.iter().filter_map(|k| bs::slug(tx, k).ok()).collect())).unwrap_or_default();
    vec![("TM_INTEGRATION_BATCH", b.number.to_string()), ("TM_INTEGRATION_ROUND", b.round.to_string()),
        ("TM_INTEGRATION_SHA", b.candidate.clone().unwrap_or_default()), ("TM_INTEGRATION_BASE", b.base.clone()),
        ("TM_INTEGRATION_TARGET", p.s.target.clone()), ("TM_INTEGRATION_TASKS", ids.join(" ")),
        ("TM_INTEGRATION_TREE", tree.display().to_string()), ("TM_INTEGRATION_PROJECT_DIR", p.repo.display().to_string())]
}

/// What a step's window says now (AC-190): still running, lost with its window (run again, never red), or ended
/// with its status, its output kept in `log` and the window closed; `over` its time, it is ended with OVER.
pub(super) enum Window {
    Running,
    Lost,
    Ended(i32),
}

/// The status of a window ended for running over its time: no program's own.
pub(super) const OVER: i32 = -1;

pub(super) fn window(runner: &dyn Runner, pane: &str, log: &Path, over: bool) -> Window {
    match runner.status(pane) {
        Err(_) => Window::Lost,
        Ok(None) if over => {
            runner.stop(pane);
            Window::Ended(ended(runner, pane, log, OVER))
        }
        Ok(None) => Window::Running,
        Ok(Some(code)) => Window::Ended(ended(runner, pane, log, code)),
    }
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
    // turned off: nothing more is merged or pushed (AC-186); a published batch's After runs on (`tail`)
    if !p.s.on {
        if let Some(pane) = &b.pane {
            runner.stop(pane);
            runner.close(pane);
        }
        return store.with(|tx| bs::give_up(tx, &b, &format!("integration was turned off for {}", p.name), now));
    }
    match b.state.as_str() {
        "checking" | "publishing" => follow(store, runner, &p, b, now),
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
pub(super) fn follow(store: &Store, runner: &dyn Runner, p: &Place, b: Batch, now: i64) -> Result<(), String> {
    let (step, line, folder) = match b.state.as_str() {
        "checking" => ("check", p.s.check.clone(), None),
        "publishing" => ("publish", p.s.publish.clone(), None),
        "cleanup" => ("cleanup", p.s.cleanup.clone(), Some(p.repo.clone())),
        _ => ("after", p.s.after.clone(), Some(p.repo.clone())),
    };
    let candidate = b.candidate.clone().unwrap_or_default();
    let Some(pane) = b.pane.clone() else {
        let folder = match folder {
            Some(f) => f,
            None => match tree_at(p, &b.project, &candidate) {
                Ok(t) => t,
                Err(why) => return red(store, p, &b, &why, now),
            },
        };
        if step == "check" && let Err(why) = retry_notes::clear(&folder) {
            return red(store, p, &b, &why, now);
        }
        let label = format!("batch-{}.{}.{step}", b.number, b.round);
        let v = vars(store, p, &b, &integration::tree_in(&p.home, &b.project));
        return match runner.start(&label, &folder, &line, &v) {
            Ok(pane) => store.with(|tx| bs::set_pane(tx, b.id, Some(&pane), now)),
            Err(why) => red(store, p, &b, &format!("The {step} did not start: {why}"), now),
        };
    };
    let log = log_of(p, &b.project, &b, step);
    // After holds the machine's tail for every project: over the check's time it fails as a check does (AC-191)
    let over = matches!(step, "check" | "after") && now - b.step_started.unwrap_or(now) > p.s.check_minutes * 60;
    let status = match window(runner, &pane, &log, over) {
        // After lost with its window is not run again (AC-191): what it did is not known
        Window::Lost if step == "after" => return store.with(|tx| {
            bs::set_pane(tx, b.id, None, now)?;
            let words = format!("After's outcome is not known for batch {}: its window was lost. After is not run again.", b.number);
            bs::tell_task(tx, &b, &words, now)?;
            bs::set_state(tx, b.id, after_after(&p.s), Some(&words), now)
        }),
        // lost with its window: run again, not counted as red (AC-190)
        Window::Lost => return store.with(|tx| { bs::set_pane(tx, b.id, None, now)?; bs::tell_task(tx, &b, &format!("Batch {}'s {step} runs again: its window was lost.", b.number), now) }),
        Window::Running => return Ok(()),
        Window::Ended(code) => code,
    };
    if step == "check" && let Some(notes) = retry_notes::read(&integration::tree_in(&p.home, &b.project)) {
        store.with(|tx| bs::tell_task(tx, &b, &format!("Batch {} round {} Check has retry diagnostics (AC-197).\nCheck output (data):\n{notes}", b.number, b.round), now))?;
    }
    match (step, status) {
        ("check", OVER) => red(store, p, &b, &format!("The check ran over {} minutes.", p.s.check_minutes), now),
        ("check", 0) => store.with(|tx| { bs::set_pane(tx, b.id, None, now)?; bs::round_ends(tx, &b, "green", None, now)?; bs::set_state(tx, b.id, "green", None, now) }),
        ("check", code) => red(store, p, &b, &format!("The check ended with status {code}. Its log: {}", log.display()), now),
        ("publish", 0) => match target_now(p) {
            Ok(t) => landed(store, p, b, &t, now),
            Err(why) => not_published(store, &b, &why, now),
        },
        ("publish", code) => not_published(store, &b, &format!("Publish ended with status {code}. Its log: {}", log.display()), now),
        // Cleanup ends the batch whatever its status: it undoes nothing (AC-198)
        ("cleanup", code) => store.with(|tx| {
            bs::set_pane(tx, b.id, None, now)?;
            if code != 0 {
                bs::tell_task(tx, &b, &format!("Batch {}'s Cleanup ended with status {code}; nothing is undone. Its log: {}", b.number, log.display()), now)?;
            }
            // what its After's turn said stays on the batch's row
            bs::set_state(tx, b.id, "ended", b.why.as_deref(), now)
        }),
        (_, 0) => store.with(|tx| { bs::set_pane(tx, b.id, None, now)?; bs::set_state(tx, b.id, after_after(&p.s), None, now) }),
        (_, code) => store.with(|tx| {
            let how = if code == OVER { format!("it ran over {} minutes", p.s.check_minutes) } else { format!("status {code}") };
            bs::set_pane(tx, b.id, None, now)?;
            bs::set_state(tx, b.id, bs::AFTER_FAILED, Some(&how), now)?;
            bs::wait_for_human(tx, &b, &format!("After failed for batch {} with {how}: the batch stays published and its tasks DONE. Its log: {}", b.number, log.display()), now)
        }),
    }
}

/// A red round: nothing published, the batch's size halved at its first; its integration agent analyses it while
/// red rounds are left (AC-194), in a worktree of its own at the candidate; else, or with none, a human decides.
fn red(store: &Store, p: &Place, b: &Batch, why: &str, now: i64) -> Result<(), String> {
    let said = format!("Batch {}'s round {} is red: {why}", b.number, b.round);
    // who decides, and the words for a human when it is one
    let agent: Result<PathBuf, String> = match &p.s.agent {
        None => Err(format!("{said} The project has no integration agent: decide by the batch's keys.")),
        Some(_) if b.red_rounds + 1 >= p.s.red_rounds_most => Err(format!("{said} {} red rounds: decide by the batch's keys, or change Check.", p.s.red_rounds_most)),
        Some(_) => worktree(p, integration::agent_tree_in(&p.home, &b.project, b.number), b.candidate.as_deref().unwrap_or_default())
            .map_err(|e| format!("{said} Its agent could not start: {e}")),
    };
    store.with(|tx| {
        bs::set_pane(tx, b.id, None, now)?;
        bs::round_ends(tx, b, "red", Some(why), now)?;
        bs::red_round(tx, b)?;
        bs::set_state(tx, b.id, bs::RED, Some(why), now)?;
        match &agent {
            Ok(tree) => {
                tx.execute("UPDATE integration_batches SET agent_wanted = 1, agent_tree = ?2 WHERE id = ?1", rusqlite::params![b.id, tree.display().to_string()]).map_err(ops::sql)?;
                bs::tell_task(tx, b, &format!("{said} Its integration agent analyses it."), now)
            }
            Err(words) => bs::wait_for_human(tx, b, words, now),
        }
    })
}

/// An ended batch's agent worktree removed with git; kept in the store while git could not remove it.
fn tidy(store: &Store, home: &Path) -> Result<(), String> {
    let done: Vec<(i64, String, String)> = store.with(|tx| ops::rows(tx, "SELECT id, project, agent_tree FROM integration_batches WHERE agent_tree IS NOT NULL AND state IN ('ended', 'given up')", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))))?;
    for (id, project, tree) in done {
        let removed = place(store, home, &project).and_then(|p| git::run(&p.repo, &["worktree", "remove", "--force", &tree]));
        if removed.is_ok() || !Path::new(&tree).exists() {
            store.with(|tx| tx.execute("UPDATE integration_batches SET agent_tree = NULL WHERE id = ?1", [id]).map(drop).map_err(ops::sql))?;
        }
    }
    Ok(())
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
    let (remote, branch) = integration::target_of(&p.s.target)?;
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

/// The queue's name of a batch's fixes, picked together onto the target: no task's key.
const FIX: &str = "fix";

/// The target moved under a green batch, or a task left it: its tasks tried anew on `target`, a new round.
fn again(store: &Store, p: &Place, b: &Batch, target: &str, now: i64) -> Result<(), String> {
    let (i_id, tasks) = store.with(|tx| Ok((bs::task_of(tx, b)?.unwrap_or_default(), bs::queued_of(tx, b)?)))?;
    // its agent's fixes first: each one's own change picked onto the target as it is now, together one merge named
    // by the integration task (AC-194), then its tasks
    let (fix, fixed, dropped) = if b.fixes.is_empty() {
        (None, Vec::new(), Vec::new())
    } else {
        let scratch = super::agent::scratch_of(&integration::agent_tree_in(&p.home, &b.project, b.number), "pick");
        // picks that could not run leave every fix out, said on the integration task: the queue never stalls on them
        let (head, left_out) = super::agent::picked_onto(&p.repo, &scratch, &b.fixes, target)
            .unwrap_or_else(|e| (target.to_string(), b.fixes.iter().map(|f| (f.clone(), e.clone())).collect()));
        let fixed: Vec<String> = b.fixes.iter().filter(|f| !left_out.iter().any(|(d, _)| d == *f)).cloned().collect();
        ((head != target).then(|| Queued { task: FIX.into(), id: i_id.clone(), commit: head }), fixed, left_out)
    };
    let queue: Vec<Queued> = fix.into_iter().chain(tasks).collect();
    // the fix stands on the target and is first: it always merges, never held or returned
    let mut c = choose(&p.repo, target, &queue, queue.len().max(1))?;
    c.chosen.retain(|q| q.task != FIX);
    store.with(|tx| {
        for (f, why) in &dropped {
            bs::tell_task(tx, b, &format!("Batch {}: its fix {} is left out on {} at {}: {why}", b.number, short(f), p.s.target, short(target)), now)?;
        }
        for (q, why) in &c.returned {
            bs::give_back(tx, &q.task, &bs::conflict_words(q, why, &p.s, target), bs::Left::Conflict, now)?;
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
        bs::new_round(tx, b, &c.chosen, &fixed, target, c.candidate.as_deref(), state)
    })
}

/// Published: its tasks DONE with their results, the size doubled; the project's folder and After wait for their
/// turn (`tail`), so the next batch is built and checked meanwhile. Nothing after the push may fail for a task gone
/// meanwhile: its DONE is skipped, the rest go on.
fn landed(store: &Store, p: &Place, b: Batch, target: &str, now: i64) -> Result<(), String> {
    let candidate = b.candidate.clone().unwrap_or_default();
    if !git::ok(&p.repo, &["merge-base", "--is-ancestor", &candidate, target])? {
        return not_published(store, &b, &format!("{} is not in {} after the publish.", short(&candidate), p.s.target), now);
    }
    store.with(|tx| {
        bs::set_pane(tx, b.id, None, now)?;
        bs::set_state(tx, b.id, bs::PUBLISHED, None, now)?;
        let ids = bs::done_all(tx, &b, &p.s.target, &candidate, now)?;
        bs::resize(tx, &b.project, true)?;
        bs::round_ends(tx, &b, "published", None, now)?;
        bs::close_task(tx, &b, &format!("Batch {} published at {}: {}.", b.number, short(&candidate), ids.join(", ")), now)?;
        // a human deciding a failed After hears what was published on top of it (AC-191)
        if let Some(failed) = bs::after_failed(tx, &b.project)? {
            bs::tell_task(tx, &failed, &format!("Batch {} was published on top of batch {}; its After waits for this task's end.", b.number, failed.number), now)?;
        }
        Ok(())
    })
}

/// What follows a published batch's After: its Cleanup, when the project has one (AC-198).
pub(super) fn after_after(s: &Settings) -> &'static str {
    if s.cleanup.is_empty() { "ended" } else { "cleanup" }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
pub(crate) mod tests;

