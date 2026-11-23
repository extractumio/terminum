// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-250: one classifier for the live gate and its history preview.

use std::path::Path;

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use crate::integrate::git;
use crate::tasks::ops::{self, rows};

#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Surfaces {
    pub t1: Vec<String>,
    pub t2: Vec<String>,
    pub t3: Vec<String>,
    pub t3_terms: Vec<String>,
    pub existing_review: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Tier { T0, T1, T2, T3 }

impl Tier {
    pub fn number(self) -> u8 { self as u8 }
}

pub(super) struct Match {
    pub tier: Tier,
    pub existing_review: bool,
    pub evidence: String,
    pub paths: Vec<String>,
}

impl Surfaces {
    pub fn enabled(&self) -> bool { !self.t2.is_empty() || !self.t3.is_empty() }

    pub fn validate(&self) -> Result<(), String> {
        for patterns in [&self.t1, &self.t2, &self.t3, &self.existing_review, &self.t3_terms] {
            if patterns.len() > 64 || patterns.iter().any(|p| p.trim().is_empty() || p.len() > 240
                || p.chars().any(char::is_control)) {
                return Err("Use at most 64 nonblank surface rules per field, each up to 240 bytes.".into());
            }
        }
        Ok(())
    }

    pub fn text_match(&self, title: &str, text: &str) -> Option<Match> {
        if self.t3.is_empty() || self.t3_terms.is_empty() { return None; }
        let words = format!("{title}\n{text}").to_lowercase();
        self.t3_terms.iter().find(|term| {
            words.match_indices(&term.to_lowercase()).any(|(start, found)| {
                !words[..start].ends_with(char::is_alphanumeric)
                    && !words[start + found.len()..].starts_with(char::is_alphanumeric)
            })
        }).map(|term| Match { tier: Tier::T3, existing_review: false,
            evidence: format!("Task text matched the rule {term:?}."), paths: Vec::new() })
    }

    pub fn classify(&self, changes: &[Change]) -> Option<Match> {
        let mut tier = Tier::T0;
        let mut names = Vec::new();
        let mut paths = Vec::new();
        let mut existing_review = true;
        for change in changes {
            let matches = |patterns: &[String]| patterns.iter().any(|p| path_matches(p, &change.path));
            let found = if matches(&self.t3) {
                if change.added { Tier::T2 } else { Tier::T3 }
            } else if matches(&self.t2) {
                Tier::T2
            } else if matches(&self.t1) {
                Tier::T1
            } else {
                continue;
            };
            tier = tier.max(found);
            if found >= Tier::T2 {
                existing_review &= matches(&self.existing_review);
                paths.push(change.path.clone());
            }
            if names.len() < 5 {
                names.push(format!("{} {}", if change.added { "added" } else { "changed" }, change.path.escape_debug()));
            }
        }
        paths.sort_unstable();
        paths.dedup();
        (tier != Tier::T0).then(|| Match { tier, existing_review, paths, evidence: format!("Diff: {}.", names.join(", ")) })
    }
}

fn path_matches(pattern: &str, path: &str) -> bool {
    let parts: Vec<_> = path.split('/').collect();
    let mut matched = vec![false; parts.len() + 1];
    matched[0] = true;
    for rule in pattern.split('/') {
        if rule == "**" {
            for i in 1..matched.len() {
                matched[i] |= matched[i - 1];
            }
        } else {
            for i in (1..matched.len()).rev() {
                matched[i] = matched[i - 1] && component_matches(rule, parts[i - 1]);
            }
            matched[0] = false;
        }
    }
    matched[parts.len()]
}

fn component_matches(pattern: &str, part: &str) -> bool {
    let mut pattern = pattern.chars().peekable();
    let mut part = part.chars().peekable();
    let mut retry = None;
    loop {
        match (pattern.peek().copied(), part.peek().copied()) {
            (None, None) => return true,
            (Some('*'), _) => {
                pattern.next();
                retry = Some((pattern.clone(), part.clone()));
            }
            (Some(want), Some(got)) if want == '?' || want == got => {
                pattern.next();
                part.next();
            }
            _ => {
                let Some((after_star, remaining)) = retry.as_mut() else { return false };
                if remaining.next().is_none() { return false; }
                pattern = after_star.clone();
                part = remaining.clone();
            }
        }
    }
}

pub(super) struct Change {
    pub path: String,
    pub added: bool,
}

pub(super) fn changes(repo: &Path, base: &str, head: &str) -> Result<Vec<Change>, String> {
    changes_of(repo, base, Some(head))
}

fn changes_worktree(repo: &Path, base: &str) -> Result<Vec<Change>, String> {
    changes_of(repo, base, None)
}

fn changes_of(repo: &Path, base: &str, head: Option<&str>) -> Result<Vec<Change>, String> {
    let mut args = vec!["diff", "--name-status", "-z", "--no-renames", "--no-ext-diff", "--no-textconv",
        "--ignore-submodules=none", base];
    args.extend(head);
    args.push("--");
    let out = git::local_output(repo, &args)?;
    let mut fields = out.split_terminator('\0');
    let mut changes = Vec::new();
    while let Some(code) = fields.next() {
        let path = fields.next().ok_or("Git returned an incomplete diff entry.")?;
        changes.push(Change { path: path.into(), added: code == "A" });
    }
    Ok(changes)
}

pub(super) fn fork(repo: &Path, target: &str, head: &str) -> Result<String, String> {
    if !git::is_commit(target) || !git::is_commit(head) { return Err("A full commit ID is required.".into()); }
    git::local_output(repo, &["merge-base", target, head]).map(|base| base.trim().to_owned())
}

pub(super) struct PastTask {
    id: String,
    text_tier: Option<Tier>,
    candidate: Option<String>,
    base: Option<String>,
}

const MAX_TASKS: usize = 400;
const WINDOW: i64 = 30 * super::DAY;

pub(super) fn history(tx: &Transaction, project: &str, at: i64, map: &Surfaces) -> Result<Vec<PastTask>, String> {
    ops::check_project(tx, project)?;
    rows(tx, "SELECT t.slug, t.title, CASE WHEN ?5 THEN t.body ELSE '' END, b.candidate_sha, b.base_sha
        FROM tasks t LEFT JOIN integration_batches b ON b.project = t.project
         AND b.id = (SELECT i.id FROM integration_batches i JOIN json_each(i.tasks) j
            WHERE i.project = t.project AND j.value = t.id AND i.candidate_sha IS NOT NULL
                AND i.state IN ('published','after','after failed','after waits','cleanup','ended')
            ORDER BY i.id DESC LIMIT 1)
        WHERE t.project = ?1 AND t.status = 'DONE' AND t.archived_at IS NULL
            AND substr(t.slug, 1, 2) NOT IN ('S_', 'I_')
            AND NOT EXISTS (SELECT 1 FROM supervisor_task_marks m WHERE m.task = t.id)
            AND (SELECT max(e.at) FROM events e WHERE e.task = t.id AND e.kind = 'move') BETWEEN ?2 AND ?3
        ORDER BY t.updated_at DESC, t.number DESC LIMIT ?4",
        (project, at - WINDOW, at, MAX_TASKS as i64 + 1, !map.t3_terms.is_empty() && !map.t3.is_empty()), |r| Ok(PastTask {
            id: r.get(0)?,
            text_tier: map.text_match(r.get_ref(1)?.as_str()?, r.get_ref(2)?.as_str()?).map(|hit| hit.tier),
            candidate: r.get(3)?, base: r.get(4)?,
        }))
}

#[derive(Default, Debug, Serialize)]
pub(super) struct Preview {
    pub sampled: usize,
    pub without_diff: usize,
    pub t2: usize,
    pub t3: usize,
    pub truncated: bool,
    pub empty: bool,
}

pub(super) fn preview(map: &Surfaces, repo: &Path, tasks: Vec<PastTask>) -> Preview {
    let mut result = Preview { truncated: tasks.len() > MAX_TASKS, empty: !map.enabled(), ..Preview::default() };
    if result.empty { return result; }
    let mut batches = std::collections::HashMap::new();
    for task in tasks.into_iter().take(MAX_TASKS) {
        result.sampled += 1;
        let diff = task.candidate.zip(task.base).and_then(|(candidate, base)| {
            let edges = batches.entry((candidate, base)).or_insert_with_key(|(candidate, base)| merge_edges(repo, base, candidate));
            let (parent, merge) = edges.get(&task.id)?;
            changes(repo, parent, merge).ok()
        });
        result.without_diff += usize::from(diff.is_none());
        let by_diff = diff.and_then(|diff| map.classify(&diff));
        match task.text_tier.into_iter().chain(by_diff.map(|hit| hit.tier)).max() {
            Some(Tier::T2) => result.t2 += 1,
            Some(Tier::T3) => result.t3 += 1,
            _ => {}
        }
    }
    result
}

fn merge_edges(repo: &Path, base: &str, candidate: &str) -> std::collections::HashMap<String, (String, String)> {
    if !git::is_commit(base) || !git::is_commit(candidate) { return Default::default(); }
    let range = format!("{base}..{candidate}");
    let Ok(log) = git::local_output(repo, &["log", "--first-parent", "--merges", "-n", "128",
        git::FIRST_PARENT_FORMAT, &range, "--"]) else { return Default::default() };
    log.lines().filter_map(|line| {
        let edge = git::commit_edge(line)?;
        let id = edge.subject.strip_prefix(crate::integrate::batch::MERGE_SUBJECT)?;
        edge.second.map(|_| (id.into(), (edge.first.into(), edge.sha.into())))
    }).collect()
}

/// Observe only an assigned, unambiguous local worktree belonging to the project's repository.
pub(crate) fn observe_trees(store: &crate::tasks::store::Store, trees: &[crate::integrate::notices::Tree], at: i64) -> Result<(), String> {
    use crate::tasks::integration;
    use std::collections::BTreeSet;
    use super::{design_gates, settings, verify_review};

    let scopes: BTreeSet<_> = trees.iter().filter(|tree| tree.working).map(|tree| tree.project.as_str()).collect();
    for project in scopes {
        let selected = store.snapshot(super::SCHEMA, |tx| {
            if !super::register::advisor_enabled(tx, project)? { return Ok(None); }
            let saved = settings::read(tx, project)?;
            let map = saved.settings.charter.surfaces;
            if !map.enabled() { return Ok(None); }
            let Ok(repo) = verify_review::project_dir(tx, project) else { return Ok(None); };
            let target = verify_review::reference(&integration::read(tx, project)?.target)?;
            Ok(Some((saved.revision, map, repo, target)))
        })?;
        let Some((revision, map, repo, target)) = selected else { continue };
        let Ok(own) = git::checkout(&repo) else { continue };
        let Ok(target) = git::local_output(&repo, &["rev-parse", "--verify", &target]) else { continue };
        let candidates: Vec<_> = trees.iter().filter(|t| t.working && t.project == project)
            .take(30).filter_map(|t| Some((t, git::checkout(&t.dir).ok()?))).collect();
        for (tree, checkout) in &candidates {
            let root = &checkout.root;
            if own.root == *root || checkout.common != own.common
                || candidates.iter().filter(|(_, other)| other.root == *root).count() > 1
            {
                continue;
            }
            let Ok(base) = fork(root, target.trim(), &checkout.head) else { continue };
            let Ok(diff) = changes_worktree(root, &base) else { continue };
            let Some(hit) = map.classify(&diff).filter(|hit| hit.tier >= Tier::T2) else { continue };
            store.with(|tx| {
                let task = ops::get(tx, &tree.id)?;
                if task.key == tree.task && task.project == project && task.status == "IN PROGRESS"
                    && task.session.as_deref() == Some(&tree.session) && task.archived_at.is_none()
                    && super::register::advisor_enabled(tx, project)?
                    && settings::revision(tx, project)? == revision
                {
                    design_gates::observe(tx, &task, hit, true, at)?;
                }
                Ok(())
            })?;
        }
    }
    Ok(())
}
