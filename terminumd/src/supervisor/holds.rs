// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-245: scoped hold decisions, using the same state transitions as the board.

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::dispatcher::runs::{self, Phase, Run};
use crate::integrate::column;
use crate::roles::supervisor as authority;
use crate::tasks::{auto, integration_queue as queue, ops::{self, Task, sql}, store::HUMAN};

use super::{Evidence, Level, Mode, Rule, Signal, attach::{self, HoldState}, incidents, rounds, settings,
    signals::{self, State}};

pub(crate) const TOOLS: &[&str] = &["hold", "release_hold"];
const PAUSED_DISPATCH: &str = "Paused by the Supervisor.";
/// What follows an interim verification hold without an answer (AC-225), on every surface.
pub(super) const INTERIM_END: &str = "Without an answer the hold ends with its verification, unless verification confirms the signal.";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind { StopRun, PauseDispatch, PauseIntegration, TakeOut }

impl Kind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::StopRun => "stop_run", Self::PauseDispatch => "pause_dispatch",
            Self::PauseIntegration => "pause_integration", Self::TakeOut => "take_out",
        }
    }

    pub(super) fn read(name: &str) -> Self {
        match name {
            "stop_run" => Self::StopRun, "pause_dispatch" => Self::PauseDispatch,
            "pause_integration" => Self::PauseIntegration, "take_out" => Self::TakeOut,
            _ => unreachable!("hold kind checked by the store"),
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::StopRun => "Run stopped", Self::PauseDispatch => "Auto-Dispatcher paused",
            Self::PauseIntegration => "Integration queue paused", Self::TakeOut => "Task taken out of integration",
        }
    }

    fn undo(self) -> &'static str {
        match self {
            Self::StopRun => "RUN", Self::PauseDispatch => "Turn on Auto-Dispatcher",
            Self::PauseIntegration => "Resume integration", Self::TakeOut => "Queue for integration",
        }
    }

    fn project_wide(self) -> bool {
        matches!(self, Self::PauseDispatch | Self::PauseIntegration)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub signal: String,
    pub kind: Kind,
    pub task: Option<String>,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct Hold {
    pub id: i64,
    pub kind: Kind,
    pub label: &'static str,
    pub undo: &'static str,
    pub task: Option<String>,
    pub signal: String,
    detector: String,
    response: String,
    level: u8,
    reason: String,
    actor: String,
    applied_at: i64,
    shadow: bool,
    pub(super) incident: Option<i64>,
    released_by: Option<String>,
    released_at: Option<i64>,
    /// An interim verification hold's deadline: it ends with its round unless code confirms its signal (AC-233).
    pub until: Option<i64>,
    /// Why an interim hold could not end with its round; it then stays until a key releases it.
    pub end_failed: Option<String>,
    #[serde(skip)]
    undo_data: Option<String>,
}

const SELECT: &str = "SELECT h.id, h.kind, t.slug, h.fingerprint, h.detector, h.response, h.level,
    h.reason, h.actor, h.applied_at, h.shadow, h.incident, h.released_by, h.released_at, h.undo_data, h.until, h.end_failed
    FROM supervisor_holds h LEFT JOIN tasks t ON t.id = h.target AND t.project = h.project";

fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<Hold> {
    let kind = Kind::read(&r.get::<_, String>(1)?);
    Ok(Hold { id: r.get(0)?, kind, label: kind.label(), undo: kind.undo(), task: r.get(2)?,
        signal: r.get(3)?, detector: r.get(4)?, response: r.get(5)?, level: r.get(6)?, reason: r.get(7)?,
        actor: r.get(8)?, applied_at: r.get(9)?, shadow: r.get(10)?, incident: r.get(11)?,
        released_by: r.get(12)?, released_at: r.get(13)?, undo_data: r.get(14)?, until: r.get(15)?, end_failed: r.get(16)? })
}

pub(super) fn list(tx: &Transaction, project: &str) -> Result<Vec<Hold>, String> {
    ops::rows(tx, &format!("{SELECT} WHERE h.project = ?1 AND h.shadow = 0 AND h.released_at IS NULL ORDER BY h.id DESC"),
        [project], read)
}

pub(super) fn get(tx: &Transaction, project: &str, id: i64) -> Result<Hold, String> {
    tx.query_row(&format!("{SELECT} WHERE h.project = ?1 AND h.id = ?2 AND h.shadow = 0 AND h.released_at IS NULL"),
        (project, id), read).optional().map_err(sql)?.ok_or_else(|| "That hold is no longer in effect in this project.".into())
}

fn scoped_task(tx: &Transaction, project: &str, id: &str) -> Result<Task, String> {
    let task = ops::get(tx, id)?;
    if task.project != project || task.archived_at.is_some() {
        return Err("Choose an active task in the Supervisor's project.".into());
    }
    Ok(task)
}

fn state(tx: &Transaction, signal: &Signal, at: i64) -> Result<HoldState, String> {
    let project = &signal.project;
    let settings::Saved { revision, settings: pack } = settings::read(tx, project)?;
    if rounds::enabled(tx, project)?.is_none() || pack.level != Level::Operator {
        return Err("Holds require an enabled Operator Supervisor.".into());
    }
    if signal.level < 2 || !matches!(signal.state, State::New | State::Shadow | State::Raised | State::Held | State::Recurred) {
        return Err("A hold requires a current signal at level 2 or above.".into());
    }
    let rule = Rule::named(&signal.detector)?;
    let mode = pack.mode(rule);
    if pack.muted(rule, at) || matches!(mode, Mode::Off | Mode::Digest)
        || super::config_lifecycle::quieted(tx, project, rule)?
    {
        return Err("This detector does not allow holds.".into());
    }
    if mode == Mode::Shadow || signal.state == State::Shadow {
        return Ok(HoldState::Shadow);
    }
    let scanned: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM project_supervisor
        WHERE project = ?1 AND last_ok IS NOT NULL AND settings_revision = ?2)",
        (project, revision), |r| r.get(0)).map_err(sql)?;
    if !scanned { return Err("The Supervisor must scan the current settings before applying a hold.".into()); }
    let state = attach::read(tx, project)?.map_or(HoldState::Inactive, |report| report.hold_state(rule, at, signal.charter_breach));
    if state == HoldState::Inactive { return Err("This detector is not ready for holds.".into()); }
    Ok(state)
}

pub(super) enum Disposition {
    Applied(Applied),
    Card(super::breakers::Card),
}

pub(super) struct Applied {
    pub id: i64,
    pub shadow: bool,
    pub run: Option<Box<Run>>,
    pub task: Option<String>,
    pub nudges: super::nudges::Nudges,
}

pub(super) fn apply(tx: &Transaction, project: &str, request: Request, actor: &str, at: i64) -> Result<Disposition, String> {
    apply_as(tx, project, request, actor, at, false)
}

/// AC-233: code's interim hold while a low-confidence signal is verified. It needs no confidence, and an open
/// breaker refuses it rather than making a decision card for a hold that ends with its round.
pub(super) fn apply_interim(tx: &Transaction, project: &str, request: Request, at: i64) -> Result<Applied, String> {
    match apply_as(tx, project, request, "system", at, true)? {
        Disposition::Applied(applied) => Ok(applied),
        Disposition::Card(_) => unreachable!("an interim hold refuses an open breaker before diverting"),
    }
}

fn apply_as(tx: &Transaction, project: &str, request: Request, actor: &str, at: i64, interim: bool) -> Result<Disposition, String> {
    let signal = signals::get(tx, project, &request.signal)?;
    let shadow = state(tx, &signal, at)? == HoldState::Shadow;
    let reason = request.reason.trim();
    if reason.is_empty() || reason.chars().count() > 1000 || reason.chars().any(char::is_control) {
        return Err("Give a hold reason of 1 to 1000 plain-text characters.".into());
    }
    let kind = request.kind;
    if request.task.is_some() == kind.project_wide() {
        return Err("Name a task for a run or queue-entry hold only.".into());
    }
    if signal.scores.confidence < super::STRONG && !shadow && !interim {
        return Err("This signal needs verification before a hold can apply.".into());
    }
    let task = request.task.as_deref().map(|id| scoped_task(tx, project, id)).transpose()?;
    let target = task.as_ref().map_or("", |t| t.key.as_str());
    if let Some(id) = tx.query_row("SELECT id FROM supervisor_holds WHERE project = ?1 AND kind = ?2 AND target = ?3
        AND released_at IS NULL AND (shadow = 0 OR (?4 AND fingerprint = ?5)) ORDER BY shadow, id LIMIT 1",
        (project, kind.name(), target, shadow, &signal.fingerprint), |r| r.get::<_, i64>(0)).optional().map_err(sql)?
    {
        return Err(format!("Hold {id} already covers this request."));
    }
    if let Some(t) = &task {
        let relates = |id: &str| id == t.id || id == t.key;
        if !(signal.subject_kind == "task" && relates(&signal.subject_id)
            || signal.evidence.iter().any(|e| matches!(e, Evidence::Tasks(id) if relates(id))))
        {
            return Err("The signal does not name the held task.".into());
        }
        super::task_policy::check(tx, t, at)?;
    }
    if shadow {
        let id = insert(tx, &signal, kind, target, (reason, actor, at), true, None)?;
        super::outcomes::record(tx, &signal, super::outcomes::Kind::Hold, &id.to_string(), target, "", at)?;
        return Ok(Disposition::Applied(Applied { id, shadow, run: None, task: None, nudges: Vec::new() }));
    }
    let blocked: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_holds
        WHERE fingerprint = ?1 AND kind = ?2 AND target = ?3 AND released_by = ?4 AND released_at > ?5)",
        (&signal.fingerprint, kind.name(), target, HUMAN, at - super::DAY), |r| r.get(0)).map_err(sql)?;
    if blocked { return Err("The owner released this hold. The Supervisor waits 24 hours.".into()); }
    if interim && super::breakers::open_id(tx, project, &signal.detector, signal.response)?.is_some() {
        return Err("The rule's breaker is open.".into());
    }
    if let Some(card) = super::breakers::divert(tx, &signal, kind.name(), target, reason, at)? {
        return Ok(Disposition::Card(card));
    }
    let baseline = super::outcome_evidence::hold_target(tx, project, kind.name(), target)?;
    let mut run = None;
    let undo = match (kind, task.as_ref()) {
        (Kind::PauseDispatch, None) => {
            if !auto::setting(tx, project)?.on { return Err("Auto-Dispatcher is already off.".into()); }
            auto::turn_off(tx, project, PAUSED_DISPATCH)?;
            None
        }
        (Kind::PauseIntegration, None) => {
            let paused: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM project_integration
                WHERE project = ?1 AND paused)", [project], |r| r.get(0)).map_err(sql)?;
            if paused { return Err("The integration queue is already paused.".into()); }
            column::set_paused(tx, project, true)?;
            None
        }
        (Kind::TakeOut, Some(t)) => {
            let saved = queue::held_entry(tx, &t.key)?;
            let author = authority::author(tx, project)?;
            let event = column::out_as(tx, &t.id, &author, at)?;
            Some(serde_json::to_string(&(event, saved)).map_err(|e| e.to_string())?)
        }
        (Kind::StopRun, Some(t)) => {
            let current = stoppable(tx, t, at)?;
            let stopped = runs::let_go(tx, &current.id)?;
            let author = authority::author(tx, project)?;
            for id in &current.open {
                ops::log(tx, &ops::key_of(tx, id)?, &author, "why", reason, None, at)?;
            }
            let saved = RunUndo { run: stopped.id.clone(), tasks: current.open };
            run = Some(Box::new(stopped));
            Some(serde_json::to_string(&saved).map_err(|e| e.to_string())?)
        }
        _ => unreachable!("task requirement checked before mutation"),
    };
    let id = insert(tx, &signal, kind, target, (reason, actor, at), false, undo.as_deref())?;
    super::outcomes::record(tx, &signal, super::outcomes::Kind::Hold, &id.to_string(), target, baseline.as_deref().unwrap_or(""), at)?;
    if kind.project_wide() || signal.level == 3 {
        incidents::attach_hold(tx, &signal, id, kind.project_wide(), actor, at)?;
    }
    let nudges = super::nudges::held(tx, &signal, at)?;
    Ok(Disposition::Applied(Applied { id, shadow, run, task: task.map(|t| t.id), nudges }))
}

/// The task's marked run while it works: what a run hold stops and what a runaway raise names (AC-245, AC-213).
fn marked_run(tx: &Transaction, task: &Task) -> Result<Option<Run>, String> {
    Ok(runs::holding(tx, &task.key)?.filter(|r| r.project == task.project
        && r.pane_key.is_some() && r.holds_slot() && r.state != Phase::Ending && !r.is_starting() && r.open.contains(&task.id)))
}

fn stoppable(tx: &Transaction, task: &Task, at: i64) -> Result<Run, String> {
    let run = marked_run(tx, task)?.ok_or("The task has no marked run to stop.")?;
    for id in &run.open {
        let affected = scoped_task(tx, &task.project, id)?;
        super::task_policy::check(tx, &affected, at)?;
        if affected.status != "IN PROGRESS" {
            return Err("An open task of this run has changed status.".into());
        }
    }
    Ok(run)
}

fn insert(tx: &Transaction, s: &Signal, kind: Kind, target: &str, audit: (&str, &str, i64),
    shadow: bool, undo: Option<&str>) -> Result<i64, String>
{
    tx.query_row("INSERT INTO supervisor_holds (project, fingerprint, detector, response, level, kind, target,
        reason, actor, applied_at, shadow, undo_data) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        RETURNING id", params![s.project, s.fingerprint, s.detector, s.response, s.level, kind.name(), target,
        audit.0, audit.1, audit.2, shadow, undo], |r| r.get(0)).map_err(sql)
}

/// Task `key` when it is an active task of `project`.
fn task_by_key(tx: &Transaction, project: &str, key: &str) -> Result<Option<Task>, String> {
    let slug = tx.query_row("SELECT coalesce(slug, CAST(number AS TEXT)) FROM tasks WHERE id = ?1 AND project = ?2",
        (key, project), |r| r.get::<_, String>(0)).optional().map_err(sql)?;
    Ok(slug.and_then(|slug| scoped_task(tx, project, &slug).ok()))
}

/// The marked run of task `key` while it works, if it is an active task of `project`.
pub(super) fn marked_run_of(tx: &Transaction, project: &str, key: &str) -> Result<Option<Run>, String> {
    task_by_key(tx, project, key)?.map_or(Ok(None), |task| marked_run(tx, &task))
}

/// Code stops only the marked run a cost outlier's runaway raise names (AC-213), with its retained usage evidence.
pub(super) fn automatic(tx: &Transaction, project: &str, fingerprint: &str, at: i64) -> Result<Option<Disposition>, String> {
    let signal = signals::get(tx, project, fingerprint)?;
    let Some(raised) = &signal.raised else { return Ok(None) };
    if signal.detector != Rule::CostOutlier.id() || signal.level != 3 || signal.scores.confidence < super::STRONG
        || signal.subject_kind != "task" || signal.metric != "task_dollars"
    {
        return Ok(None);
    }
    let Ok(eligibility) = state(tx, &signal, at) else { return Ok(None) };
    let Some(task) = task_by_key(tx, project, &signal.subject_id)? else { return Ok(None) };
    let Ok(run) = stoppable(tx, &task, at) else { return Ok(None) };
    if run.id != raised.run || !signal.evidence.iter().any(|e| matches!(e, Evidence::Runs(id) if id == &run.id)) {
        return Ok(None);
    }
    let seen: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_holds
        WHERE project = ?1 AND kind = 'stop_run' AND target = ?2 AND
        ((shadow = 0 AND (released_at IS NULL OR fingerprint = ?3)) OR (shadow = 1 AND fingerprint = ?3 AND ?4)))",
        (project, &task.key, fingerprint, eligibility == HoldState::Shadow), |r| r.get(0)).map_err(sql)?;
    if seen { return Ok(None); }
    apply(tx, project, Request { signal: fingerprint.into(), kind: Kind::StopRun, task: Some(task.id),
        reason: "The current run is a critical cost outlier.".into() }, "system", at).map(Some)
}

pub(super) fn release(tx: &Transaction, project: &str, hold: &Hold, actor: &str, at: i64) -> Result<(), String> {
    match hold.kind {
        Kind::PauseDispatch => {
            let mut setting = auto::setting(tx, project)?;
            if setting.on || setting.off_why.as_deref() != Some(PAUSED_DISPATCH) {
                return Err("Auto-Dispatcher's pause changed. Use its project switch.".into());
            }
            setting.on = true;
            auto::set(tx, project, &setting, at)?;
        }
        Kind::PauseIntegration => column::set_paused(tx, project, false)?,
        Kind::TakeOut => {
            let (event, entry) = serde_json::from_str(hold.undo_data.as_deref().ok_or("The previous queue entry is missing.")?)
                .map_err(|e| format!("The previous queue entry is not readable: {e}"))?;
            let task = scoped_task(tx, project, hold.task.as_deref().ok_or("The held task is gone.")?)?;
            // An interim hold's end undoes code's own action: a human's recent touch does not keep it.
            if actor != HUMAN && hold.until.is_none() { super::task_policy::check(tx, &task, at)?; }
            queue::restore_entry(tx, &task, event, entry, at)?;
        }
        Kind::StopRun => return Err("Use RUN to release a stopped run.".into()),
    }
    record_release(tx, hold.id, actor, at)
}

fn record_release(tx: &Transaction, id: i64, actor: &str, at: i64) -> Result<(), String> {
    let before = tx.query_row("SELECT project, kind, target FROM supervisor_holds WHERE id = ?1 AND released_at IS NULL",
        [id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))).optional().map_err(sql)?;
    if let Some((project, kind, target)) = before
        && actor == HUMAN
        && let Some(action) = super::outcomes::find(tx, &project, super::outcomes::Kind::Hold, &id.to_string())?
    {
        if !action.baseline.is_empty()
            && super::outcome_evidence::hold_target(tx, &project, &kind, &target)?.as_deref() == Some(&action.baseline)
            && !super::outcome_evidence::changed_approach(tx, id)?
        {
            super::outcomes::judge(tx, &project, &action, 0, at,
                (super::outcomes::Verdict::Wrong, "The owner released the unchanged subject."))?;
        }
        super::reflection::action(tx, &project, &action, "human_override", at)?;
    }
    let released = tx.query_row("UPDATE supervisor_holds SET released_by = ?2, released_at = ?3, replacement_run = pending_run,
        pending_run = NULL, pending_by = NULL WHERE id = ?1 AND released_at IS NULL RETURNING incident, target",
        (id, actor, at), |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, String>(1)?))).optional().map_err(sql)?;
    let Some((incident, target)) = released else { return Ok(()) };
    if actor == HUMAN && !target.is_empty() {
        ops::log(tx, &target, actor, "why", "Released its Supervisor hold.", None, at)?;
    }
    if let Some(incident) = incident {
        incidents::hold_event(tx, incident, id, "hold_released", actor, at)?;
    }
    Ok(())
}

pub(crate) fn integration_held(tx: &Transaction, task: &Task) -> Result<bool, String> {
    tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_holds WHERE project = ?1 AND kind = 'take_out'
        AND target = ?2 AND shadow = 0 AND released_at IS NULL)", (&task.project, &task.key), |r| r.get(0)).map_err(sql)
}

/// The board's own inverse keys clear the same record as the Supervisor screen.
pub(crate) fn board_release(tx: &Transaction, project: &str, kind: Kind, target: &str, actor: &str, at: i64) -> Result<(), String> {
    let hold = tx.query_row("SELECT id FROM supervisor_holds WHERE project = ?1 AND kind = ?2 AND target = ?3
        AND shadow = 0 AND released_at IS NULL", (project, kind.name(), target), |r| r.get(0)).optional().map_err(sql)?;
    if let Some(id) = hold { record_release(tx, id, actor, at)?; }
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub(crate) struct RunUndo {
    pub run: String,
    pub tasks: Vec<String>,
}

pub(crate) fn run_undo(tx: &Transaction, project: &str, id: i64) -> Result<(RunUndo, Option<String>), String> {
    let hold = get(tx, project, id)?;
    if hold.kind != Kind::StopRun { return Err("That hold does not cover a stopped run.".into()); }
    let undo = serde_json::from_str(hold.undo_data.as_deref().ok_or("The held run is missing.")?)
        .map_err(|e| format!("The held run is not readable: {e}"))?;
    let pending = tx.query_row("SELECT pending_run FROM supervisor_holds WHERE id = ?1", [id], |r| r.get(0)).map_err(sql)?;
    Ok((undo, pending))
}

pub(crate) fn check_run_release(tx: &Transaction, project: &str, id: i64, run: &str, tasks: &[Task],
    supervisor: bool) -> Result<(), String>
{
    let (undo, pending) = run_undo(tx, project, id)?;
    if pending.as_deref().unwrap_or(&undo.run) != run
        || tasks.len() != undo.tasks.len() || !tasks.iter().all(|t| undo.tasks.contains(&t.id))
    {
        return Err("The held run or its tasks have changed.".into());
    }
    for task in tasks {
        let current = scoped_task(tx, project, &task.id)?;
        let latest = runs::last_of(tx, &current.key)?.filter(|r| r.id == run)
            .ok_or("A task has another run. Use RUN on the task as it stands.")?;
        if latest.holds_slot() {
            return Err("A release of this hold is already starting.".into());
        }
        if current.status != "FAILED" {
            return Err("A task of the held run changed status. Use RUN on the task as it stands.".into());
        }
        if supervisor {
            super::task_policy::check(tx, &current, ops::now())?;
            let agent = crate::tasks::agents::in_project(tx, &latest.agent_id, project)?;
            if agent.is_none_or(|a| a.kind != latest.kind || a.line != latest.command) {
                return Err("The held run's agent settings changed. The owner can release it with RUN.".into());
            }
        }
    }
    Ok(())
}

/// Reserve the undo on the exact replacement run. A launch failure leaves the hold active.
pub(crate) fn reserve_run_release(tx: &Transaction, id: i64, actor: &str, run: &str) -> Result<(), String> {
    if actor != HUMAN {
        let (project, fingerprint, response, target) = tx.query_row("SELECT project, fingerprint, response, target
            FROM supervisor_holds WHERE id = ?1 AND released_at IS NULL", [id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))).map_err(sql)?;
        let signal = signals::get(tx, &project, &fingerprint)?;
        super::outcomes::record_rule(tx, (&signal, &response), super::outcomes::Kind::Run,
            &format!("{run}/{target}"), &target, run, ops::now())?;
    }
    tx.execute("UPDATE supervisor_holds SET pending_run = ?2, pending_by = ?3 WHERE id = ?1 AND released_at IS NULL",
        (id, run, actor)).map(drop).map_err(sql)
}

pub(crate) fn reserve_board_run(tx: &Transaction, project: &str, task: &str, run: &str) -> Result<(), String> {
    tx.execute("UPDATE supervisor_holds SET pending_run = ?3, pending_by = ?4
        WHERE project = ?1 AND target = ?2 AND kind = 'stop_run' AND released_at IS NULL AND shadow = 0",
        (project, task, run, HUMAN)).map(drop).map_err(sql)
}

pub(crate) fn run_launched(tx: &Transaction, run: &str, at: i64) -> Result<bool, String> {
    let pending = ops::rows(tx, "SELECT id, pending_by FROM supervisor_holds WHERE pending_run = ?1",
        [run], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for (id, actor) in &pending { record_release(tx, *id, actor, at)?; }
    Ok(!pending.is_empty())
}
