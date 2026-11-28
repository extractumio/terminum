// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-261: local release tags and the first one containing each retained task commit.

use std::{collections::{BTreeMap, HashMap}, path::{Path, PathBuf}, time::Duration};

use serde::{Deserialize, Serialize};

use crate::integrate::git::{FIRST_PARENT_FORMAT, commit_edge, is_commit};

use super::{probe_arch::Integrated, probe_command::{Failure, command, output}};

const GIT_OUTPUT: usize = 8 * 1024 * 1024;
const GIT_LIMIT: Duration = Duration::from_secs(8);
const MAX_TAGS: usize = 1024;
const MAX_GRAPH: usize = 30_000;

pub(super) async fn git(dir: &Path, args: &[&str]) -> Result<String, Failure> {
    let mut cmd = command("/usr/bin/git", dir);
    cmd.args(["--no-pager", "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null",
        "-c", "protocol.allow=never"])
        .envs([("GIT_CONFIG_NOSYSTEM", "1"), ("GIT_CONFIG_GLOBAL", "/dev/null"), ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_OPTIONAL_LOCKS", "0"), ("GIT_NO_LAZY_FETCH", "1"), ("GIT_NO_REPLACE_OBJECTS", "1")])
        .args(args);
    String::from_utf8(output(cmd, GIT_LIMIT, GIT_OUTPUT).await?).map_err(|_| Failure::Invalid)
}

pub(super) async fn common_dir(dir: &Path) -> Result<PathBuf, Failure> {
    let out = git(dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await
        .map_err(|e| if e == Failure::Exit { Failure::Unavailable } else { e })?;
    let path = PathBuf::from(out.strip_suffix('\n').ok_or(Failure::Invalid)?);
    if !path.is_absolute() { return Err(Failure::Invalid); }
    Ok(path)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Tag {
    pub name: String,
    pub sha: String,
    pub at: i64,
    pub patch: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct Releases {
    pub tags: Vec<Tag>,
    pub tasks: BTreeMap<String, TaskRelease>,
    pub shallow: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct TaskRelease {
    pub created: i64,
    pub landed: i64,
    pub first_tag: Option<usize>,
}

#[derive(Clone)]
struct Commit {
    pub task: String,
    pub created: i64,
    pub landed: i64,
    pub sha: String,
}

fn version(tag: &str) -> Option<bool> {
    // Stable release refs only. Ignore aliases and prereleases.
    let mut parts = tag.strip_prefix('v')?.split('.');
    let [major, minor, patch] = [parts.next()?, parts.next()?, parts.next()?];
    let number = |n: &str| {
        if n.is_empty() || (n.len() > 1 && n.starts_with('0')) || !n.bytes().all(|b| b.is_ascii_digit()) { return None; }
        n.parse::<u64>().ok()
    };
    number(major)?;
    number(minor)?;
    let patch = number(patch)?;
    parts.next().is_none().then_some(patch > 0)
}

async fn tags(dir: &Path, at: i64) -> Result<Vec<Tag>, Failure> {
    let fields = "%(refname:strip=2)%09%(objecttype)%09%(objectname)%09%(committerdate:unix)%09%(*objecttype)%09%(*objectname)%09%(taggerdate:unix)";
    let raw = git(dir, &["for-each-ref", &format!("--format={fields}"), "refs/tags/v*"]).await?;
    let mut tags = Vec::new();
    for line in raw.lines() {
        let [name, kind, object, committed, peeled_kind, peeled, tagged] =
            line.split('\t').collect::<Vec<_>>().try_into().map_err(|_| Failure::Invalid)?;
        let Some(patch) = version(name) else { continue; };
        let (sha, timestamp) = match (kind, peeled_kind) {
            ("commit", _) => (object, committed),
            ("tag", "commit") => (peeled, tagged),
            _ => continue,
        };
        let timestamp = timestamp.parse::<i64>().ok().filter(|t| *t > 0).ok_or(Failure::Invalid)?;
        if !is_commit(sha) { return Err(Failure::Invalid); }
        if timestamp <= at {
            tags.push(Tag { name: name.into(), sha: sha.into(), at: timestamp, patch });
        }
    }
    if tags.len() > MAX_TAGS { return Err(Failure::Output); }
    tags.sort_by(|a, b| (a.at, &a.name).cmp(&(b.at, &b.name)));
    Ok(tags)
}

pub(super) struct LandedEdge {
    pub sha: String,
    pub commit: String,
}

pub(super) async fn landed_edges(dir: &Path, landings: &[Integrated]) -> Result<HashMap<String, LandedEdge>, Failure> {
    let candidates = landings.iter().filter_map(|task| task.candidate.as_deref())
        .filter(|sha| is_commit(sha)).collect::<std::collections::BTreeSet<_>>();
    if candidates.is_empty() { return Ok(HashMap::new()); }
    let mut args = vec!["log", "--first-parent", "--max-count=8192", FIRST_PARENT_FORMAT];
    args.extend(candidates);
    args.push("--");
    let history = git(dir, &args).await?;
    let edges = history.lines().filter_map(commit_edge).map(|edge| (edge.sha, edge)).collect::<HashMap<_, _>>();
    Ok(landings.iter().filter_map(|task| {
        let mut next = task.candidate.as_deref()?;
        let base = task.base.as_deref()?;
        let mut found = None;
        for _ in 0..128 {
            if next == base { return found.map(|edge| (task.task.clone(), edge)); }
            let edge = edges.get(next)?;
            if edge.subject.strip_prefix(crate::integrate::batch::MERGE_SUBJECT) == Some(task.slug.as_str())
                && let Some(commit) = edge.second
                && task.commit.as_deref().is_none_or(|expected| expected == commit)
                && found.replace(LandedEdge { sha: edge.sha.into(), commit: commit.into() }).is_some()
            {
                return None;
            }
            next = edge.first;
        }
        None
    }).collect())
}

async fn retained_commits(dir: &Path, landings: Vec<Integrated>) -> Result<Vec<Commit>, Failure> {
    let mut edges = if landings.iter().any(|task| task.commit.is_none()) { landed_edges(dir, &landings).await? }
        else { HashMap::new() };
    Ok(landings.into_iter().filter_map(|task| {
        let sha = task.commit.filter(|sha| is_commit(sha)).or_else(|| edges.remove(&task.task).map(|edge| edge.commit))?;
        Some(Commit { task: task.task, created: task.created, landed: task.landed, sha })
    }).collect())
}

pub(super) async fn collect(dir: &Path, at: i64, landings: Vec<Integrated>) -> Result<Releases, Failure> {
    common_dir(dir).await?;
    let (tags, commits, shallow) = tokio::try_join!(tags(dir, at), retained_commits(dir, landings),
        git(dir, &["rev-parse", "--is-shallow-repository"]))?;
    let shallow = match shallow.trim() {
        "true" => true, "false" => false, _ => return Err(Failure::Invalid),
    };
    let mut contains = HashMap::<&str, Vec<u64>>::new();
    let graph;
    if !commits.is_empty() && !tags.is_empty() {
        // Visit descendants before their parents. Each commit carries the bitset of
        // release tags that reach it, so a task filters that set by its creation time.
        let width = tags.len().div_ceil(64);
        let limit = format!("--max-count={}", MAX_GRAPH + 1);
        let mut args = vec!["rev-list", "--topo-order", "--parents", &limit];
        args.extend(tags.iter().map(|tag| tag.sha.as_str()));
        args.push("--");
        graph = git(dir, &args).await?;
        for (index, tag) in tags.iter().enumerate() {
            let bits = contains.entry(&tag.sha).or_insert_with(|| vec![0; width]);
            bits[index / 64] |= 1 << (index % 64);
        }
        for (index, line) in graph.lines().enumerate() {
            if index == MAX_GRAPH { return Err(Failure::Output); }
            let mut commits = line.split_ascii_whitespace();
            let sha = commits.next().ok_or(Failure::Invalid)?;
            let marked = contains.get(sha).ok_or(Failure::Invalid)?.clone();
            for parent in commits {
                let bits = contains.entry(parent).or_insert_with(|| vec![0; width]);
                for (word, inherited) in bits.iter_mut().zip(&marked) { *word |= inherited; }
            }
        }
    }
    let tasks = commits.into_iter().map(|commit| {
        let first_tag = contains.get(commit.sha.as_str()).and_then(|bits| {
            let start = tags.partition_point(|tag| tag.at < commit.created);
            (start..tags.len()).find(|i| bits[*i / 64] & (1 << (*i % 64)) != 0)
        });
        (commit.task, TaskRelease { created: commit.created, landed: commit.landed, first_tag })
    }).collect();
    Ok(Releases { tags, tasks, shallow })
}
