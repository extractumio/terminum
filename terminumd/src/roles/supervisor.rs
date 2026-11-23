// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-231 and AC-232: the store owns pane identity and the owner owns the level.

use rusqlite::{OptionalExtension, Transaction};
use serde_json::Value;

use crate::app::Shared;
use crate::supervisor::{self, Level};
use crate::tasks::{api::store, ops, tools::Who};

const OBSERVER_TOOLS: &[&str] = &["list_tasks", "get_task"];
const ADVISOR_TOOLS: &[&str] = &["add_comment", "ask_human"];
const OPERATOR_TOOLS: &[&str] = &["run_task", "promote_queued_task", "stop_task", "list_runs", "get_dashboard"];

pub(crate) fn task_tool(tool: &str) -> bool {
    OBSERVER_TOOLS.contains(&tool) || ADVISOR_TOOLS.contains(&tool)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Role {
    #[default]
    Worker,
    Manager,
    Token { manager: bool },
    Supervisor(Option<Level>),
    Reflection(bool),
}

impl From<bool> for Role {
    fn from(manager: bool) -> Self {
        if manager { Self::Manager } else { Self::Worker }
    }
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Self::Worker | Self::Token { manager: false } => "worker",
            Self::Manager | Self::Token { manager: true } => "manager",
            Self::Supervisor(_) => "supervisor",
            Self::Reflection(_) => "reflection",
        }
    }

    pub fn level(self) -> Option<Level> {
        match self {
            Self::Supervisor(level) => level,
            Self::Reflection(true) => Some(Level::Observer),
            _ => None,
        }
    }

    pub fn is_supervisor(self) -> bool {
        matches!(self, Self::Supervisor(_) | Self::Reflection(_))
    }

    pub fn allows(self, tool: &str) -> bool {
        if let Self::Reflection(active) = self {
            return active && matches!(tool, "get_case_file" | supervisor::reflection::TOOL);
        }
        if tool == supervisor::reflection::TOOL { return false; }
        if supervisor::holds::TOOLS.contains(&tool) {
            return self == Self::Supervisor(Some(Level::Operator));
        }
        if supervisor::round_tools::ADVISOR.contains(&tool) {
            return matches!(self, Self::Supervisor(Some(Level::Advisor | Level::Operator)));
        }
        if supervisor::round_tools::NAMES.contains(&tool) {
            return matches!(self, Self::Supervisor(Some(_)));
        }
        match self {
            Self::Token { .. } if tool == supervisor::review::TOOL => false,
            Self::Manager | Self::Token { manager: true } => true,
            Self::Worker | Self::Token { manager: false } => !super::MANAGER_TOOLS.contains(&tool),
            Self::Supervisor(None) | Self::Reflection(_) => false,
            Self::Supervisor(Some(level)) => {
                OBSERVER_TOOLS.contains(&tool)
                    || (level != Level::Observer && ADVISOR_TOOLS.contains(&tool))
                    || (level == Level::Operator && OPERATOR_TOOLS.contains(&tool))
            }
        }
    }

    pub fn refusal(self, tool: &str) -> String {
        match self {
            Self::Supervisor(Some(level)) => format!("The {level:?} Supervisor cannot call {tool:?}."),
            Self::Supervisor(None) | Self::Reflection(false) => "This Supervisor session is inactive.".into(),
            Self::Reflection(true) => format!("A reflection cannot call {tool:?}."),
            _ => super::not_yours(tool),
        }
    }
}

pub(crate) struct Mark {
    pub project: String,
    pub role: Role,
    pub run: Option<String>,
}

/// Even an old mark wins over a human's project membership or a + Session run.
pub(crate) fn marked(tx: &Transaction, key: &str) -> Result<Option<Mark>, String> {
    let active_round = supervisor::rounds::ACTIVE;
    let row: Option<(String, bool, Option<String>)> = tx.prepare_cached(&format!("SELECT m.project, EXISTS(
        SELECT 1 FROM project_supervisor s WHERE s.project = m.project AND s.enabled = 1 AND s.generation = m.generation
        ) AND NOT EXISTS (
            SELECT 1 FROM supervisor_rounds q WHERE q.id = m.round AND NOT ({active_round})
        ) AND (m.talk IS NULL OR EXISTS (SELECT 1 FROM runs r
            WHERE r.id = m.talk AND r.pane_key = m.pane_key AND {})),
        coalesce(m.talk, (SELECT run FROM supervisor_rounds WHERE id = m.round))
        FROM supervisor_panes m WHERE m.pane_key = ?1", crate::supervisor::talk::LIVE)).map_err(ops::sql)?
        .query_row((key, ops::now()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(ops::sql)?;
    row.map(|(project, active, run)| {
        let role = match &run {
            Some(run) if supervisor::rounds::is_reflection(tx, run)? => Role::Reflection(active),
            _ => Role::Supervisor(active.then(|| supervisor::level(tx, &project)).transpose()?),
        };
        Ok(Mark { project, role, run })
    }).transpose()
}

pub(crate) fn marked_panes(tx: &Transaction) -> Result<Vec<String>, String> {
    ops::rows(tx, "SELECT pane_key FROM supervisor_panes", [], |r| r.get(0))
}

pub(crate) const PEER_UNKNOWN: &str = "terminumd cannot tell which process calls, so while a Supervisor session is open it runs no pane's tool for it.";

/// A call on this Mac's socket whose process the system does not name, while a Supervisor's pane is marked.
pub(crate) fn unjudged(peer: Option<super::peer::Peer>, marks: &[String]) -> bool {
    peer.is_some_and(|p| p.0.is_none()) && !marks.is_empty()
}

pub(crate) fn require(tx: &Transaction, key: &str, project: &str, tool: &str) -> Result<(), String> {
    let role = marked(tx, key)?.filter(|m| m.project == project).map_or(Role::Supervisor(None), |m| m.role);
    if role.allows(tool) { Ok(()) } else { Err(role.refusal(tool)) }
}

pub(crate) const START_ARGS: [&str; 2] = ["id", "agent"];

pub(crate) fn run_args(args: &Value) -> Result<(), String> {
    if args.as_object().is_some_and(|obj| obj.keys().all(|k| START_ARGS.contains(&k.as_str()))) {
        Ok(())
    } else {
        Err("The Supervisor's run_task takes only a task ID and an agent. Dispatcher supplies the prompt.".into())
    }
}

/// This entry receives only a server-resolved mark and a call from its own pane.
pub(crate) async fn run(app: &Shared, key: &str, other: bool, mark: Mark, tool: String, args: Value) -> Result<String, String> {
    if !mark.role.allows(&tool) {
        return Err(mark.role.refusal(&tool));
    }
    if other && !super::reads(&tool) {
        return Err("Only the Supervisor's main agent changes the project.".into());
    }
    if supervisor::holds::TOOLS.contains(&tool.as_str()) || tool == "stop_task" {
        return supervisor::holds_api::tool(app, key.into(), mark.project, tool, args).await;
    }
    if supervisor::round_tools::known(&tool) {
        return supervisor::round_tools::run(app, key.into(), mark.project, tool, args).await;
    }
    if tool == "ask_human" && mark.run.is_some() {
        return Err("For a Supervisor decision, call propose_decision on this investigation's register problem.".into());
    }
    if matches!(tool.as_str(), "add_comment" | "ask_human" | "promote_queued_task") {
        let (pane, scope, action, id) = (key.to_owned(), mark.project.clone(), tool.clone(), super::arg(&args, "id")?.to_owned());
        let blocked = store(app, move |tx| {
            require(tx, &pane, &scope, &action)?;
            let task = ops::get(tx, &id)?;
            if task.project != scope { return Err("The task is outside this project.".into()); }
            supervisor::task_policy::record_override(tx, &task, ops::now())
        }).await?;
        if let Some(why) = blocked { return Err(why); }
    }
    if task_tool(&tool) {
        let who = Who { key: key.into(), project: Some(mark.project), title: "Supervisor".into(),
            agent: "supervisor".into(), dir: None, supervisor: true };
        return crate::agentapi::task_tool_of(app, who, None, other, tool, args).await;
    }
    let actor = super::Boss { key: key.into(), user: user(&mark.project),
        project: mark.project, name: "Supervisor".into() };
    match tool.as_str() {
        "run_task" => start(app, actor, &args).await,
        "promote_queued_task" => super::tasks::promote_queued(app, &actor, &args, true).await,
        "list_runs" => super::tasks::list_runs(app, &actor).await,
        "get_dashboard" => super::tasks::dashboard(app, &actor, &args).await,
        _ => Err(mark.role.refusal(&tool)),
    }
}

pub(crate) fn author(tx: &Transaction, project: &str) -> Result<String, String> {
    ops::agent_named(tx, user(project), "Supervisor")
}

/// The project's Supervisor as the store's user who writes.
fn user(project: &str) -> String {
    format!("supervisor:{project}")
}

async fn start(app: &Shared, actor: super::Boss, args: &Value) -> Result<String, String> {
    run_args(args)?;
    let id = super::arg(args, "id")?.to_string();
    let name = super::arg(args, "agent")?.to_string();
    let named = actor.clone();
    let agent = store(app, move |tx| super::tasks::agent_named(tx, &named.project, &name)).await?;
    let done = crate::dispatcher::api::press_by_supervisor(app, &id, &agent.id, actor).await?;
    if let Some(cards) = done["decision_cards"].as_array() {
        let ids = cards.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
        return Ok(format!("Owner decision cards: {ids}."));
    }
    if let Some(words) = done["full"]["words"].as_str() {
        return Err(format!("Not started: {words} Nothing waits."));
    }
    Ok(format!("{id} runs on {}. Dispatcher sent the fixed task prompt.", agent.name))
}
