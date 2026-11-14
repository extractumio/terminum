// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A project's integration settings (`terminum-integration.md` AC-186): on or off, the target, the commands
//! Check, Publish and After, the batch's size, the check's time, and the agent that analyses a red batch with its
//! budget, red rounds and prompt. Only a signed-in page writes them; nothing here runs a command.

use std::path::{Path as FsPath, PathBuf};
use std::process::Command;

use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use rusqlite::{OptionalExtension, Transaction};
use serde_json::{json, Value};

use super::api::{answer, store};
use super::ops::{self, sql};
use crate::app::Shared;
use crate::http::off;

/// The integration agent's prompt when the project has none of its own (§9 "The agent's prompt").
pub const DEFAULT_PROMPT: &str = include_str!("integration_prompt.txt");
const MOST_LINE: usize = 2000;
const MOST_PROMPT: usize = 8 * 1024;
/// The variables each command gets (§9 "Commands").
const VARIABLES: [&str; 8] = ["TM_INTEGRATION_BATCH", "TM_INTEGRATION_ROUND", "TM_INTEGRATION_SHA", "TM_INTEGRATION_BASE",
    "TM_INTEGRATION_TARGET", "TM_INTEGRATION_TASKS", "TM_INTEGRATION_TREE", "TM_INTEGRATION_PROJECT_DIR"];
const ON_THIS_MAC: &str = "Integration runs for projects on this Mac.";

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub on: bool,
    pub target: String,
    pub check: String,
    pub publish: String,
    pub after: String,
    pub batch_most: i64,
    pub check_minutes: i64,
    pub agent: Option<String>,
    pub agent_budget_minutes: i64,
    pub red_rounds_most: i64,
    /// Empty: DEFAULT_PROMPT.
    pub prompt: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { on: false, target: "origin/main".into(), check: String::new(), publish: String::new(), after: String::new(),
            batch_most: 8, check_minutes: 90, agent: None, agent_budget_minutes: 60, red_rounds_most: 3, prompt: String::new() }
    }
}

impl Settings {
    fn shown(&self) -> Value {
        json!({"on": self.on, "target": self.target, "check": self.check, "publish": self.publish, "after": self.after,
            "batch_most": self.batch_most, "check_minutes": self.check_minutes, "agent": self.agent,
            "agent_budget_minutes": self.agent_budget_minutes, "red_rounds_most": self.red_rounds_most, "prompt": self.prompt})
    }

    /// What a page sent; a field left out keeps its default.
    fn from_body(body: &Value) -> Result<Settings, String> {
        let d = Settings::default();
        let text = |k: &str, or: &str| -> Result<String, String> {
            match &body[k] { Value::Null => Ok(or.to_string()), Value::String(s) => Ok(s.clone()), _ => Err(format!("{k}: text.")) }
        };
        let number = |k: &str, or: i64, words: &str| -> Result<i64, String> {
            match &body[k] { Value::Null => Ok(or), v => v.as_i64().ok_or_else(|| words.to_string()) }
        };
        Ok(Settings {
            on: match &body["on"] { Value::Null => false, v => v.as_bool().ok_or("on: true or false")? },
            target: text("target", &d.target)?.trim().to_string(),
            check: text("check", "")?.trim().to_string(),
            publish: text("publish", "")?.trim().to_string(),
            after: text("after", "")?.trim().to_string(),
            batch_most: number("batch_most", d.batch_most, BATCH)?,
            check_minutes: number("check_minutes", d.check_minutes, CHECK_MINUTES)?,
            agent: match &body["agent"] { Value::Null => None, Value::String(a) => Some(a.clone()).filter(|a| !a.is_empty()), _ => return Err("agent: an agent's id, or none.".into()) },
            agent_budget_minutes: number("agent_budget_minutes", d.agent_budget_minutes, BUDGET)?,
            red_rounds_most: number("red_rounds_most", d.red_rounds_most, RED_ROUNDS)?,
            prompt: text("prompt", "")?,
        })
    }
}

const BATCH: &str = "Tasks in a batch: 1 to 8.";
const CHECK_MINUTES: &str = "Check ends after: 1 to 1440 minutes.";
const BUDGET: &str = "Time budget: 5 to 1440 minutes.";
const RED_ROUNDS: &str = "Red rounds: 1 to 10.";

/// The fields' own rules, whatever the project: AC-186's refusals in its words.
pub fn check_fields(s: &Settings) -> Result<(), String> {
    for (name, line) in [("Check", &s.check), ("Publish", &s.publish), ("After", &s.after)] {
        if line.chars().count() > MOST_LINE || line.chars().any(char::is_control) {
            return Err(format!("{name}: one line, at most 2,000 characters."));
        }
    }
    if target_parts(&s.target).is_none() {
        return Err(not_a_target(&s.target));
    }
    let within = |n: i64, low: i64, high: i64, words: &str| if (low..=high).contains(&n) { Ok(()) } else { Err(words.to_string()) };
    within(s.batch_most, 1, 8, BATCH)?;
    within(s.check_minutes, 1, 1440, CHECK_MINUTES)?;
    within(s.agent_budget_minutes, 5, 1440, BUDGET)?;
    within(s.red_rounds_most, 1, 10, RED_ROUNDS)?;
    if s.prompt.len() > MOST_PROMPT {
        return Err("Prompt: at most 8 KiB.".into());
    }
    Ok(())
}

fn not_a_target(target: &str) -> String {
    format!("{}: name a remote's branch, as origin/main.", if target.is_empty() { "Target" } else { target })
}

/// `origin/main` as (origin, main).
fn target_parts(target: &str) -> Option<(&str, &str)> {
    let (remote, branch) = target.split_once('/')?;
    let fine = |s: &str| !s.is_empty() && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c));
    (fine(remote) && fine(branch)).then_some((remote, branch))
}

/// Where integration can run, for one that is on: a git checkout on this Mac, a git that has merge-tree's
/// --write-tree (2.38), and the target as a remote's branch it has fetched. Runs git: off the async workers.
pub fn check_place(dir: &FsPath, target: &str, git: &str) -> Result<(), String> {
    let run = |args: &[&str]| Command::new(git).arg("-C").arg(dir).args(args).output().map_err(|e| format!("git: {e}"));
    let version = String::from_utf8_lossy(&Command::new(git).arg("--version").output().map_err(|e| format!("git: {e}"))?.stdout).to_string();
    let number = version.split_whitespace().nth(2).unwrap_or("").to_string();
    let mut parts = number.split('.').map(|n| n.parse::<u32>().unwrap_or(0));
    if (parts.next().unwrap_or(0), parts.next().unwrap_or(0)) < (2, 38) {
        return Err(format!("git 2.38 or later is needed: this is {}.", number.trim_end_matches(char::is_alphabetic)));
    }
    if !dir.is_dir() || !run(&["rev-parse", "--git-dir"])?.status.success() {
        return Err(format!("{} is not a git checkout.", dir.display()));
    }
    let (remote, branch) = target_parts(target).ok_or_else(|| not_a_target(target))?;
    if !run(&["rev-parse", "--verify", "--quiet", &format!("refs/remotes/{remote}/{branch}^{{commit}}")])?.status.success() {
        return Err(format!("{target}: no such branch at {remote}."));
    }
    Ok(())
}

/// A project's settings; the defaults, off, when it has none.
pub fn read(tx: &Transaction, project: &str) -> Result<Settings, String> {
    tx.query_row("SELECT is_on, target, check_line, publish_line, after_line, batch_most, check_minutes, agent, agent_budget_minutes, red_rounds_most, prompt
                  FROM project_integration WHERE project = ?1", [project], |r| Ok(Settings {
        on: r.get::<_, i64>(0)? != 0, target: r.get(1)?, check: r.get(2)?, publish: r.get(3)?, after: r.get(4)?, batch_most: r.get(5)?,
        check_minutes: r.get(6)?, agent: r.get(7)?, agent_budget_minutes: r.get(8)?, red_rounds_most: r.get(9)?, prompt: r.get(10)?,
    })).optional().map_err(sql).map(Option::unwrap_or_default)
}

/// Kept as given, its agent one of the project's own pool.
pub fn keep(tx: &Transaction, project: &str, s: &Settings) -> Result<(), String> {
    ops::check_project(tx, project)?;
    check_fields(s)?;
    if let Some(agent) = &s.agent {
        super::agents::chosen(tx, agent, project)?;
    }
    let prompt = if s.prompt.trim() == DEFAULT_PROMPT.trim() { "" } else { s.prompt.as_str() };   // the default stays the default
    tx.execute("INSERT INTO project_integration (project, is_on, target, check_line, publish_line, after_line, batch_most, check_minutes, agent, agent_budget_minutes, red_rounds_most, prompt)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                ON CONFLICT (project) DO UPDATE SET is_on = ?2, target = ?3, check_line = ?4, publish_line = ?5, after_line = ?6, batch_most = ?7,
                  check_minutes = ?8, agent = ?9, agent_budget_minutes = ?10, red_rounds_most = ?11, prompt = ?12",
               rusqlite::params![project, s.on as i64, s.target, s.check, s.publish, s.after, s.batch_most, s.check_minutes, s.agent, s.agent_budget_minutes, s.red_rounds_most, prompt])
        .map(drop).map_err(sql)
}

/// The project's folder on this Mac, `~` read as the home folder.
fn local_dir(dir: &str) -> PathBuf {
    PathBuf::from(super::folder::folder_in(&crate::config::home().to_string_lossy(), dir))
}

/// Where the candidate is built, for "What runs".
pub fn tree_of(project: &str) -> PathBuf {
    crate::config::tm_home().join("integration").join(project).join("tree")
}

/// "What runs", in terminumd's words: each line with the folder it runs in, and what it gets.
fn runs(project: &str, dir: &str, s: &Settings) -> Vec<String> {
    let home = crate::config::home().to_string_lossy().to_string();
    let tree = crate::tmux::titles::tilde(&tree_of(project).to_string_lossy(), &home);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    vec![
        format!("The candidate is built in {tree}: {} and a merge of each task's commit.", s.target),
        if s.check.is_empty() { "No check: a candidate that merges is published.".into() } else { format!("Check runs `{}` in {tree}.", s.check) },
        if s.publish.is_empty() { format!("terminum pushes the candidate to {}, never forced.", s.target) } else { format!("Publish runs `{}` in {tree}.", s.publish) },
        if s.after.is_empty() { "Nothing runs after.".into() } else { format!("After runs `{}` in {dir}, once a batch.", s.after) },
        format!("Each runs by your login shell, `{shell} -lc`, with your PATH and logins, and {}; nothing of terminum's token.", VARIABLES.join(", ")),
    ]
}

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/task-projects/{id}/integration", get(shown).put(set))
}

async fn shown(State(app): State<Shared>, Path(project): Path<String>) -> Response {
    answer(store(&app, move |tx| {
        ops::check_project(tx, &project)?;
        let (_, host, dir) = ops::project_place(tx, &project)?;
        let s = read(tx, &project)?;
        let mut v = s.shown();
        v["default_prompt"] = json!(DEFAULT_PROMPT);
        v["here"] = json!(host.is_none());
        v["runs"] = json!(runs(&project, &dir, &s));
        Ok(v)
    }).await)
}

/// Where it runs is judged first, its git off the async workers and outside the store's lock; the fields when kept.
async fn set(State(app): State<Shared>, Path(project): Path<String>, Json(body): Json<Value>) -> Response {
    let s = match Settings::from_body(&body) {
        Ok(s) => s,
        Err(e) => return answer::<Value>(Err(e)),
    };
    let wanted = project.clone();
    let place = store(&app, move |tx| { ops::check_project(tx, &wanted)?; ops::project_place(tx, &wanted) }).await;
    let ready = match place {
        Ok((_, Some(_), _)) => Err(ON_THIS_MAC.to_string()),
        Ok((name, None, dir)) if s.on => {
            let target = s.target.clone();
            off(move || { ops::no_folder(&name, &dir)?; check_place(&local_dir(&dir), &target, "git") }).await
        }
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    };
    answer(match ready {
        Ok(()) => store(&app, move |tx| keep(tx, &project, &s)).await.map(|()| json!({"ok": true})),
        Err(e) => Err(e),
    })
}

#[cfg(test)]
#[path = "integration_tests.rs"]
mod tests;
