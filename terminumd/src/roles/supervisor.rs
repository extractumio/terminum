// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-231 and AC-232: the store owns pane identity and the owner owns the level.

use rusqlite::{OptionalExtension, Transaction};
use serde_json::Value;

use crate::app::Shared;
use crate::supervisor::{self, Level};
use crate::tasks::{api::store, ops, tools::Who};

const OBSERVER_TOOLS: &[&str] = &["list_tasks", "get_task"];
const ADVISOR_TOOLS: &[&str] = &["create_task", "add_comment", "ask_human"];
const OPERATOR_TOOLS: &[&str] = &["run_task", "promote_queued_task", "stop_task", "list_runs", "get_dashboard"];

pub(crate) fn task_tool(tool: &str) -> bool {
    OBSERVER_TOOLS.contains(&tool) || ADVISOR_TOOLS.contains(&tool)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Role {
    #[default]
    Worker,
    Manager,
    Supervisor(Option<Level>),
}

impl From<bool> for Role {
    fn from(manager: bool) -> Self {
        if manager { Self::Manager } else { Self::Worker }
    }
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::Manager => "manager",
            Self::Supervisor(_) => "supervisor",
        }
    }

    pub fn level(self) -> Option<Level> {
        match self {
            Self::Supervisor(level) => level,
            _ => None,
        }
    }

    pub fn allows(self, tool: &str) -> bool {
        match self {
            Self::Manager => true,
            Self::Worker => !super::MANAGER_TOOLS.contains(&tool),
            Self::Supervisor(None) => false,
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
            Self::Supervisor(None) => "This Supervisor session is inactive.".into(),
            _ => super::not_yours(tool),
        }
    }
}

pub(crate) struct Mark {
    pub project: String,
    pub role: Role,
}

/// Even an old mark wins over a human's project membership or a + Session run.
pub(crate) fn marked(tx: &Transaction, key: &str) -> Result<Option<Mark>, String> {
    let row: Option<(String, bool)> = tx.prepare_cached("SELECT m.project, EXISTS(
        SELECT 1 FROM project_supervisor s WHERE s.project = m.project AND s.enabled = 1 AND s.generation = m.generation
        ) FROM supervisor_panes m WHERE m.pane_key = ?1").map_err(ops::sql)?
        .query_row([key], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(ops::sql)?;
    row.map(|(project, active)| {
        let level = active.then(|| supervisor::level(tx, &project)).transpose()?;
        Ok(Mark { project, role: Role::Supervisor(level) })
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
        "stop_task" => super::tasks::stop(app, &actor, &args, true).await,
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
    if let Some(words) = done["full"]["words"].as_str() {
        return Err(format!("Not started: {words} Nothing waits."));
    }
    Ok(format!("{id} runs on {}. Dispatcher sent the fixed task prompt.", agent.name))
}
