// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Choosing a batch (`terminum-integration.md` AC-188, AC-189): in the queue's order, each commit tried on the
//! candidate so far by `git merge-tree`, no checkout; one that merges joins as its own merge commit, "Integrate <ID>"
//! (`git commit-tree`: no hook runs); one that conflicts with the target returns, one that conflicts only with the
//! batch is held back, naming what it waits for; a commit carrying another queued task's waits for that one.

use std::path::Path;

use super::git::output as git;

/// A queued commit, as the batch sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Queued {
    pub task: String,
    pub id: String,
    pub commit: String,
}

/// What a pass over the queue made of it.
#[derive(Debug, Default, PartialEq)]
pub struct Choice {
    /// in the batch, in the queue's order
    pub chosen: Vec<Queued>,
    /// the candidate's commit: the target and a merge commit a chosen task; none with none chosen
    pub candidate: Option<String>,
    /// back to their agents: (task, why), a conflict with the target in its files
    pub returned: Vec<(Queued, Conflict)>,
    /// queued still: (task, the IDs it waits for)
    pub held: Vec<(Queued, Vec<String>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Conflict {
    /// with the target, in these files
    Target(Vec<String>),
    /// it carries a queued task's commits, and that task returned
    Carries(String),
    /// git could not merge it: its words
    Git(String),
}

fn out(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// `git merge-tree` of two commits: Ok(tree) when clean, Err(Some(files)) on a conflict, Err(None) when git
/// could not merge at all.
pub(super) fn merge(repo: &Path, ours: &str, theirs: &str) -> Result<Result<String, Vec<String>>, String> {
    let o = git(repo, &["merge-tree", "--write-tree", "--name-only", "--no-messages", ours, theirs])?;
    let text = out(&o);
    let mut lines = text.lines();
    let tree = lines.next().unwrap_or_default().to_string();
    match o.status.code() {
        Some(0) => Ok(Ok(tree)),
        Some(1) => Ok(Err(lines.filter(|l| !l.is_empty()).map(str::to_string).collect())),
        _ => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
    }
}

/// Whether `a` is an ancestor of `b`.
fn under(repo: &Path, a: &str, b: &str) -> Result<bool, String> {
    Ok(git(repo, &["merge-base", "--is-ancestor", a, b])?.status.success())
}

/// A merge commit of `tree` on `parent` and `commit`, named by the task's ID alone (§9: no title reaches history).
fn commit_tree(repo: &Path, tree: &str, parent: &str, commit: &str, id: &str) -> Result<String, String> {
    let o = git(repo, &["commit-tree", tree, "-p", parent, "-p", commit, "-m", &format!("Integrate {id}")])?;
    if !o.status.success() {
        return Err(format!("git commit-tree: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(out(&o))
}

/// One pass over `queue` (its order) onto `base`, at most `size` chosen.
pub fn choose(repo: &Path, base: &str, queue: &[Queued], size: usize) -> Result<Choice, String> {
    let mut c = Choice::default();
    let mut cur = base.to_string();
    for q in queue {
        if c.chosen.len() >= size {
            break;
        }
        // a commit that carries another queued task's: never in a batch without it (AC-189)
        let mut carried = None;
        for other in queue.iter().filter(|o| o.task != q.task) {
            if under(repo, &other.commit, &q.commit)? {
                carried = Some(other);
                break;
            }
        }
        if let Some(other) = carried {
            if c.returned.iter().any(|(r, _)| r.task == other.task) {
                c.returned.push((q.clone(), Conflict::Carries(other.id.clone())));
                continue;
            }
            if !c.chosen.iter().any(|ch| ch.task == other.task) {
                c.held.push((q.clone(), vec![other.id.clone()]));
                continue;
            }
        }
        match merge(repo, &cur, &q.commit) {
            Err(words) => c.returned.push((q.clone(), Conflict::Git(words))),
            Ok(Ok(tree)) => {
                cur = commit_tree(repo, &tree, &cur, &q.commit, &q.id)?;
                c.chosen.push(q.clone());
            }
            Ok(Err(_)) => match merge(repo, base, &q.commit) {
                Ok(Err(alone)) => c.returned.push((q.clone(), Conflict::Target(alone))),
                Err(words) => c.returned.push((q.clone(), Conflict::Git(words))),
                // it merges with the target, not with the batch: it waits for those it does not merge with
                Ok(Ok(_)) => {
                    let mut waits = Vec::new();
                    for ch in &c.chosen {
                        if merge(repo, &ch.commit, &q.commit)?.is_err() {
                            waits.push(ch.id.clone());
                        }
                    }
                    if waits.is_empty() {
                        waits = c.chosen.iter().map(|ch| ch.id.clone()).collect();
                    }
                    c.held.push((q.clone(), waits));
                }
            },
        }
    }
    if !c.chosen.is_empty() {
        c.candidate = Some(cur);
    }
    Ok(c)
}

#[cfg(test)]
#[path = "batch_tests.rs"]
mod tests;
