// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-260: derive closed architecture measures from bounded local Git output.

use std::{collections::{BTreeMap, BTreeSet}, path::Path};

use serde::{Deserialize, Serialize};

use crate::integrate::git::is_commit;

use super::{DAY, probe_command::Failure, probe_git::{git, landed_edges}};

const HISTORY: i64 = 90 * DAY;
const MAX_FILES: usize = 20_000;
const LOCKS: &[&str] = &["Cargo.lock", ":(glob)**/Cargo.lock", "package-lock.json", ":(glob)**/package-lock.json"];

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct Architecture {
    pub since: i64,
    pub changes: Vec<Changed>,
    pub sizes: BTreeMap<String, u64>,
    pub near_cap: Option<Vec<(String, f64)>>,
    pub migrations: Vec<(i64, String)>,
    pub dependencies: Option<Vec<(i64, u64)>>,
    pub diffs: Vec<TaskDiff>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Changed {
    pub at: i64,
    pub path: String,
    pub churn: u64,
    pub fix: bool,
    /// Its commit, so files changed in one commit count once; empty in samples kept before it was recorded.
    #[serde(default)]
    pub commit: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct TaskDiff {
    pub at: i64,
    pub task: String,
    pub lines: u64,
}

#[derive(Clone)]
pub(super) struct Integrated {
    pub task: String,
    pub slug: String,
    pub created: i64,
    pub landed: i64,
    pub commit: Option<String>,
    pub kind: Option<String>,
    pub candidate: Option<String>,
    pub base: Option<String>,
}

fn safe_path(path: &str) -> bool {
    !path.is_empty() && path.len() <= 512 && !path.chars().any(char::is_control)
        && crate::sent::redact(path) == path
}

fn migration(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    path.split('/').any(|dir| matches!(dir, "migrations" | "migration" | "formats"))
        || name.strip_prefix("schema").and_then(|suffix| suffix.strip_suffix(".sql"))
            .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

fn commits(raw: &str) -> impl Iterator<Item = Result<(&str, i64, &str, &str), Failure>> {
    raw.split("\0\0").filter_map(|record| {
        let record = record.trim_start_matches('\0');
        if record.is_empty() { return None; }
        let (header, body) = record.split_once('\n').unwrap_or((record, ""));
        let value = (|| {
            let [sha, at, subject] = header.splitn(3, '\t').collect::<Vec<_>>().try_into().ok()?;
            if !is_commit(sha) { return None; }
            Some((sha, at.parse().ok()?, subject, body))
        })();
        Some(value.ok_or(Failure::Invalid))
    })
}

fn churn(raw: &str, by_merge: BTreeMap<String, Integrated>, out: &mut Architecture) -> Result<(), Failure> {
    for commit in commits(raw) {
        let (sha, at, subject, body) = commit?;
        let task = by_merge.get(sha);
        let conventional = subject.starts_with("fix:") || subject.strip_prefix("fix(")
            .is_some_and(|rest| rest.split_once("):").is_some_and(|(scope, _)| !scope.is_empty()));
        let fix = task.map_or(conventional, |task| task.kind.as_deref() == Some("Bug"));
        let mut total = 0u64;
        for entry in body.split_terminator('\0') {
            let [added, removed, path] = entry.splitn(3, '\t').collect::<Vec<_>>().try_into().map_err(|_| Failure::Invalid)?;
            if (added, removed) == ("-", "-") { continue; }
            let churn = added.parse::<u64>().ok().zip(removed.parse::<u64>().ok())
                .and_then(|(a, b)| a.checked_add(b)).ok_or(Failure::Invalid)?;
            if safe_path(path) {
                out.changes.push(Changed { at, path: path.into(), churn, fix, commit: sha.into() });
                if out.changes.len() > MAX_FILES { return Err(Failure::Output); }
            }
            total = total.checked_add(churn).ok_or(Failure::Invalid)?;
        }
        if let Some(task) = task {
            out.diffs.push(TaskDiff { at: task.landed, task: task.task.clone(), lines: total });
        }
    }
    Ok(())
}

fn sizes(raw: &str, paths: &BTreeSet<&str>) -> Result<BTreeMap<String, u64>, Failure> {
    let mut out = BTreeMap::new();
    for entry in raw.split_terminator('\0') {
        let (meta, path) = entry.split_once('\t').ok_or(Failure::Invalid)?;
        if !paths.contains(path) { continue; }
        let size = meta.split_ascii_whitespace().nth(3).ok_or(Failure::Invalid)?;
        if let Ok(bytes) = size.parse::<u64>() { out.insert(path.to_owned(), bytes); }
    }
    Ok(out)
}

fn lock_package(line: &str) -> Option<&str> {
    let line = line.trim();
    let package = if let Some(name) = line.strip_prefix("name = \"") {
        name.strip_suffix('"')?
    } else {
        line.strip_prefix('"')?.split_once("\": {")?.0.rsplit_once("node_modules/")?.1
    };
    (!package.is_empty() && package.len() <= 150
        && package.bytes().all(|c| c.is_ascii_alphanumeric() || b"@/._-".contains(&c))).then_some(package)
}

fn dependencies(raw: &str) -> Result<Vec<(i64, u64)>, Failure> {
    let mut results = Vec::new();
    let mut package_changes = BTreeMap::<&str, i64>::new();
    let mut at = 0;
    for line in raw.lines().chain(["\0\0"]) {
        if let Some(header) = line.strip_prefix('\0') {
            let added = package_changes.values().filter(|n| **n > 0).count() as u64;
            if added > 0 { results.push((at, added)); }
            package_changes.clear();
            if header == "\0" { break; }
            at = header.parse().map_err(|_| Failure::Invalid)?;
        } else if let Some((sign, text)) = line.as_bytes().split_first()
            && matches!(*sign, b'+' | b'-') && let Some(package) = lock_package(std::str::from_utf8(text).map_err(|_| Failure::Invalid)?)
        {
            *package_changes.entry(package).or_default() += if *sign == b'+' { 1 } else { -1 };
        }
    }
    Ok(results)
}

const SOURCE_FILES: &[&str] = &["*.rs", "*.ts", "*.tsx", "*.js", "*.jsx", "*.mjs", "*.css", "*.html",
    "*.py", "*.sh", "*.go", "*.java", "*.kt", "*.swift", "*.c", "*.h", "*.cpp", "*.hpp", "*.rb", "*.php", "*.lua"];

/// AC-212: a path the line cap, `arch.hotspot` and `arch.bug_cluster` cover: code, not a spec or a guide that every fix
/// updates by rule.
pub(super) fn source_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    SOURCE_FILES.iter().any(|glob| super::surfaces::component_matches(glob, name))
}

async fn file_lines(dir: &Path, head: &str, cap: Option<u32>) -> Result<Option<String>, Failure> {
    if cap.is_none() { return Ok(None); }
    let empty = git(dir, &["hash-object", "-t", "tree", "--stdin"]).await?;
    let sha = empty.trim();
    if !is_commit(sha) { return Err(Failure::Invalid); }
    let mut args = vec!["diff", "--numstat", "-z", "--no-renames", "--no-ext-diff", "--no-textconv", sha, head, "--"];
    args.extend_from_slice(SOURCE_FILES);
    git(dir, &args).await.map(Some)
}

fn near_limit(raw: &str, cap: u32) -> Result<Vec<(String, f64)>, Failure> {
    let mut found = Vec::new();
    for entry in raw.split_terminator('\0') {
        let [added, _, path] = entry.splitn(3, '\t').collect::<Vec<_>>().try_into().map_err(|_| Failure::Invalid)?;
        if added == "-" { continue; }
        let lines = added.parse::<u64>().map_err(|_| Failure::Invalid)?;
        let share = lines as f64 / f64::from(cap);
        if share >= 0.9 && safe_path(path) {
            found.push((path.to_owned(), share));
            if found.len() > MAX_FILES { return Err(Failure::Output); }
        }
    }
    Ok(found)
}

pub(super) async fn collect(dir: &Path, at: i64, integrated: Vec<Integrated>, cap: Option<u32>) -> Result<Architecture, Failure> {
    let head = git(dir, &["rev-parse", "--verify", "HEAD"]).await
        .map_err(|err| if err == Failure::Exit { Failure::Unavailable } else { err })?;
    let head = head.trim();
    if !is_commit(head) { return Err(Failure::Invalid); }
    let since = format!("--since=@{}", at - HISTORY);
    let until = format!("--until=@{at}");
    let base = ["log", "--first-parent", "--diff-merges=first-parent", &since, &until,
        "--no-renames", "--no-ext-diff", "--no-textconv", "--no-color"];
    let stats = base.iter().copied().chain(["--format=format:%x00%H%x09%ct%x09%s", "--numstat", "-z", head, "--"]).collect::<Vec<_>>();
    let modified = base.iter().copied().chain(["--format=format:%x00%H%x09%ct%x09%s", "--name-only", "--diff-filter=MT", "-z", head, "--"]).collect::<Vec<_>>();
    let locks = base.iter().copied().chain(["--format=tformat:%x00%ct", "-p", "--unified=0", head, "--"])
        .chain(LOCKS.iter().copied()).collect::<Vec<_>>();
    let tree = ["ls-tree", "-rlz", head];
    let (stats, modified, locks, files, mut edges, line_counts) = tokio::try_join!(
        git(dir, &stats), git(dir, &modified), git(dir, &locks), git(dir, &tree),
        landed_edges(dir, &integrated), file_lines(dir, head, cap))?;
    tokio::task::spawn_blocking(move || {
        let landed = integrated.into_iter().filter_map(|task| {
            edges.remove(&task.task).map(|edge| (edge.sha, task))
        }).collect();
        let mut result = Architecture { since: at - HISTORY, ..Architecture::default() };
        churn(&stats, landed, &mut result)?;
        let paths = result.changes.iter().map(|row| row.path.as_str()).collect();
        result.sizes = sizes(&files, &paths)?;
        for row in commits(&modified) {
            let (_, at, _, entries) = row?;
            result.migrations.extend(entries.split_terminator('\0')
                .filter(|path| safe_path(path) && migration(path)).map(|path| (at, path.to_owned())));
        }
        let has_supported_lock = files.split_terminator('\0').filter_map(|entry| entry.split_once('\t')).any(|(_, path)| {
            matches!(path.rsplit('/').next(), Some("Cargo.lock" | "package-lock.json"))
        }) || locks.contains("diff --git ");
        if has_supported_lock { result.dependencies = Some(dependencies(&locks)?); }
        result.near_cap = line_counts.zip(cap).map(|(text, limit)| near_limit(&text, limit)).transpose()?;
        Ok(result)
    }).await.map_err(|_| Failure::Invalid)?
}
