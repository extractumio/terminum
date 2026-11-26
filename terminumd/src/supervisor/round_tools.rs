// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-234 and AC-235: case access and typed findings on a live, project-scoped round.

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::app::Shared;
use crate::roles::supervisor;
use crate::tasks::{api::store, ops::{self, now, sql}};

use super::{config_changes, Evidence, Measure, Rule, Window, register, round_evidence, rounds, settings, signals, task_policy};

pub(crate) const NAMES: &[&str] = &["get_case_file", "list_signals", "get_signal", "get_metric", "record_finding"];
pub(crate) const ADVISOR: &[&str] = &["open_problem", "update_problem", "propose_rank", "propose_decision", "post_incident_answer", HAND_TO_HUMAN, config_changes::PREVIEW, config_changes::PROPOSE];
/// A flow.stuck round's own task, whose remaining step is a human's, to WAIT FOR HUMAN with its question (AC-213).
pub(crate) const HAND_TO_HUMAN: &str = "hand_to_human";

pub(crate) fn known(tool: &str) -> bool {
    NAMES.contains(&tool) || ADVISOR.contains(&tool) || matches!(tool, super::reflection::TOOL | super::design_gates::RECORD)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RankArgs { problem: i64, task: String }

async fn register_tool(app: &Shared, key: String, project: String, tool: String, value: Value) -> Result<String, String> {
    let done = store(app, move |tx| {
        supervisor::require(tx, &key, &project, &tool)?;
        let at = now();
        let marked = supervisor::marked(tx, &key)?.ok_or("There is no Supervisor role in this pane.")?;
        let reply = match tool.as_str() {
            "open_problem" => {
                let id = register::open(tx, &project, args(value)?, at)?;
                register::require_finding(tx, id, marked.run.as_deref())?;
                json!({"problem": id})
            }
            "update_problem" => {
                let change: register::Update = args(value)?;
                let id = change.id;
                register::update(tx, &project, change, at)?;
                register::require_finding(tx, id, marked.run.as_deref())?;
                json!({"problem": id})
            }
            "propose_decision" => super::decisions::propose(tx, &project, marked.run.as_deref(), args(value)?, at)?,
            "propose_rank" => {
                let input: RankArgs = args(value)?;
                let task = ops::get(tx, &input.task)?;
                if task.project != project { return Err("The task is outside this project.".into()); }
                if let Some(why) = task_policy::record_override(tx, &task, at)? { return Ok(Err(why)); }
                task_policy::propose(tx, &project, input.problem, &task, marked.run.as_deref(), at)?;
                json!({"task": task.id, "problem": input.problem, "proposed": true})
            }
            _ => return Err("Unknown Supervisor register tool.".into()),
        };
        Ok(Ok(reply.to_string()))
    }).await?;
    if done.is_ok() { app.supervisor.wake(); }
    done
}


const MOST_EVIDENCE: usize = 128;
const MOST_CLAIMS: usize = 16;
const MOST_PROSE: usize = 2_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseArgs { id: String }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs { after: Option<String> }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignalArgs { id: String }

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MetricArgs { query: String, signal: String }

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    query: String,
    signal: String,
    value: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Finding {
    cause: String,
    pub(super) confidence: f64,
    disproof: String,
    fix_proposal: String,
    evidence: Vec<Evidence>,
    measurements: Vec<Claim>,
    /// A typed request for a critical escalation, checked against the retained evidence (AC-213).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    escalate: Option<super::escalations::Request>,
}

/// AC-213: the task of this round's own current flow.stuck signal goes to WAIT FOR HUMAN with the round's questions, the
/// Supervisor's, while nobody works on it and no human or manager acted on it in 24 hours; an Answer reaches the
/// conversation its session holds now, as AC-285 says. The task is the signal's, never the round's choice.
async fn hand_to_human(app: &Shared, key: String, project: String, value: Value) -> Result<String, String> {
    use super::signals::State;
    let live = app.clone();
    let task = store(app, move |tx| {
        let run = run_of(tx, &key, &project, HAND_TO_HUMAN)?;
        let at = now();
        if let Some(extra) = value.as_object().and_then(|o| o.keys().find(|k| !matches!(k.as_str(), "questions" | "kind" | "options" | "recommended"))) {
            return Err(format!("{HAND_TO_HUMAN} takes questions, kind, options and recommended; not {extra}: its task is the signal's."));
        }
        let round: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_rounds WHERE run = ?1 AND project = ?2)",
            (&run, &project), |r| r.get(0)).map_err(sql)?;
        let signal = super::action_tasks::run_signal(tx, &project, &run)?
            .filter(|s| round && s.detector == Rule::Stuck.id() && s.subject_kind == "task")
            .ok_or("Only a flow.stuck investigation hands its own task to a human.")?;
        if !matches!(signal.state, State::New | State::Raised | State::Recurred | State::Held) {
            return Err("Its flow.stuck signal is no longer current.".into());
        }
        let slug: Option<String> = tx.query_row("SELECT coalesce(slug, CAST(number AS TEXT)) FROM tasks WHERE id = ?1",
            [&signal.subject_id], |r| r.get(0)).optional().map_err(sql)?;
        let task = slug.filter(|_| super::flow::still_stuck(tx, &project, &signal.subject_id, &signal.evidence).unwrap_or(false))
            .map(|slug| ops::get(tx, &slug)).transpose()?
            .ok_or("Its task moved, was pressed, run or written on since its signal: it stays as it is.")?;
        let session = task.session.clone().ok_or("Its task has no session.")?;
        let (state, agent) = super::samples::live(tx, &live, &task.key, &session, &project)?;
        if state == super::samples::WORKED {
            return Err(format!("Someone works on {} now: it stays as it is.", task.id));
        }
        // a human's or manager's action refuses it, its diagnostic kept (AC-239)
        if let Some(why) = task_policy::record_override(tx, &task, at)? { return Ok(Err(why)); }
        let questions = crate::plans::rules::clean(value["questions"].as_str().ok_or("questions is required.")?);
        if let Some(why) = crate::tasks::closeout::unshaped(&task.id, &questions) { return Err(why); }
        let asked_in = live.hooks.session(&session).map(|(_, s)| s).filter(|_| agent);
        let author = supervisor::author(tx, &project)?;
        let event = crate::tasks::comments::wait_in(tx, &author, &task, &questions, &value, asked_in.as_deref(), at)?;
        super::action_tasks::comment(tx, &task, &key, &event, true, at)?;
        Ok(Ok(task.id))
    }).await??;
    crate::tasks::api::told(app, &task);
    app.supervisor.wake();
    Ok(format!("{task} waits for the human, with your questions."))
}

fn run_of(tx: &Transaction, key: &str, project: &str, tool: &str) -> Result<String, String> {
    supervisor::require(tx, key, project, tool)?;
    tx.query_row("SELECT coalesce(m.talk, (SELECT run FROM supervisor_rounds WHERE id = m.round))
        FROM supervisor_panes m WHERE m.pane_key = ?1 AND m.project = ?2 AND (m.talk IS NOT NULL OR m.round IS NOT NULL)",
        (key, project), |r| r.get(0)).optional().map_err(sql)?
        .ok_or("There is no live investigation in this pane.".into())
}

fn case_file(tx: &Transaction, run: &str) -> Result<Value, String> {
    let case: String = tx.query_row("SELECT case_file FROM supervisor_rounds WHERE run = ?1 UNION ALL SELECT case_file FROM supervisor_talks WHERE run = ?1",
        [run], |r| r.get(0)).map_err(sql)?;
    serde_json::from_str(&case).map_err(|e| e.to_string())
}

fn args<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "The tool arguments do not match the required fields and types.".into())
}

async fn configuration(app: &Shared, key: String, project: String, tool: String, value: Value) -> Result<String, String> {
    let snapshot = app.clone();
    if tool == config_changes::PREVIEW {
        return crate::http::off(move || snapshot.tasks.snapshot(super::SCHEMA, |tx| {
            let run = run_of(tx, &key, &project, &tool)?;
            config_changes::preview(tx, &project, &run, args(value)?, now()).map(|result| result.to_string())
        })).await;
    }
    let (scope, pane, name) = (project.clone(), key.clone(), tool.clone());
    let (run, verified) = crate::http::off(move || snapshot.tasks.snapshot(super::SCHEMA, |tx| {
        let run = run_of(tx, &pane, &scope, &name)?;
        let verified = config_changes::verify(tx, &scope, &run, args(value)?, now())?;
        Ok((run, verified))
    })).await?;
    let result = store(app, move |tx| {
        if run_of(tx, &key, &project, &tool)? != run {
            return Err("The live Supervisor round changed. Replay the proposal again.".into());
        }
        config_changes::submit(tx, &project, &run, verified, now())
    }).await?;
    if let Some(task) = &result.task { crate::tasks::api::told(app, task); }
    app.supervisor.wake();
    Ok(json!(result).to_string())
}

pub(crate) async fn run(app: &Shared, key: String, project: String, tool: String, value: Value) -> Result<String, String> {
    if matches!(tool.as_str(), config_changes::PREVIEW | config_changes::PROPOSE) {
        return configuration(app, key, project, tool, value).await;
    }
    if tool == super::design_gates::RECORD {
        let task = store(app, move |tx| {
            let run = run_of(tx, &key, &project, &tool)?;
            super::design_reviews::submitted(tx, &project, &run, value, now())
        }).await?;
        crate::tasks::api::told(app, &task);
        return Ok("Design review recorded on the task.".into());
    }
    if tool == super::reflection::TOOL {
        let card = store(app, move |tx| {
            let run = run_of(tx, &key, &project, &tool)?;
            let case = case_file(tx, &run)?;
            super::reflection::complete(tx, &project, &run, &case, value, now())
        }).await?;
        if let Some(card) = card { crate::tasks::api::told(app, &card); }
        app.supervisor.wake();
        return Ok("Reflection recorded with its configuration proposals.".into());
    }
    if tool == "post_incident_answer" {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Answer { text: String }
        let input: Answer = args(value)?;
        let task = store(app, move |tx| {
            let run = run_of(tx, &key, &project, &tool)?;
            super::thread::answer(tx, &project, &run, &input.text, now())
        }).await?;
        crate::tasks::api::told(app, &task);
        return Ok(format!("Answered on {task}."));
    }
    if tool == HAND_TO_HUMAN {
        return hand_to_human(app, key, project, value).await;
    }
    if ADVISOR.contains(&tool.as_str()) {
        return register_tool(app, key, project, tool, value).await;
    }
    if tool == "record_finding" {
        let done = store(app, move |tx| {
            let run = run_of(tx, &key, &project, &tool)?;
            let case = case_file(tx, &run)?;
            let why = match accept(tx, &project, &case, value) {
                Ok(finding) => {
                    let at = now();
                    let text = serde_json::to_string(&finding).map_err(|e| e.to_string())?;
                    let changed = tx.execute("UPDATE supervisor_rounds SET finding = ?2, found_at = ?3
                        WHERE run = ?1 AND deadline > ?3 AND finding IS NULL", (&run, text, at)).map_err(sql)?;
                    if changed == 1 {
                        if let Some(s) = super::action_tasks::run_signal(tx, &project, &run)? {
                            super::outcomes::record(tx, &s, super::outcomes::Kind::Cause, &run, &s.fingerprint, "", at)?;
                            if let Some(request) = &finding.escalate {
                                let round = case["id"].as_str().unwrap_or_default();
                                super::escalations::record(tx, &s, round, request, at)?;
                            }
                        }
                        return Ok(Ok("Finding checked and recorded.".into()));
                    }
                    let recorded: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_rounds WHERE run = ?1 AND finding IS NOT NULL)",
                        [&run], |r| r.get(0)).map_err(sql)?;
                    if recorded { return Ok(Ok("A finding is already recorded.".into())); }
                    "Finding arrived after the deadline.".into()
                }
                Err(why) => why,
            };
            rounds::failure(tx, &run, &why, true)?;
            Ok(Err(why))
        }).await?;
        if done.is_ok() { app.supervisor.wake(); }
        return done;
    }
    let app = app.clone();
    crate::http::off(move || app.tasks.snapshot(super::SCHEMA, |tx| {
        let run = run_of(tx, &key, &project, &tool)?;
        let response = match tool.as_str() {
            "get_case_file" => {
                let input: CaseArgs = args(value)?;
                let case = case_file(tx, &run)?;
                if case["id"] != input.id { return Err("That case does not belong to this round.".into()); }
                case
            }
            "list_signals" => {
                let input: ListArgs = args(value)?;
                let mut page = signals::page(tx, &project, input.after.as_deref(), 21)?;
                let more = page.len() > 20;
                page.truncate(20);
                json!({"next": if more { page.last().map(|s| &s.fingerprint) } else { None }, "signals": page})
            }
            "get_signal" => {
                let input: SignalArgs = args(value)?;
                json!(signals::get(tx, &project, &input.id)?)
            }
            "get_metric" => {
                let input: MetricArgs = args(value)?;
                json!(metric(tx, &project, &case_file(tx, &run)?, &input.query, &input.signal)?)
            }
            _ => return Err("Unknown Supervisor round tool.".into()),
        };
        Ok(response.to_string())
    })).await
}

#[derive(Deserialize)]
struct Scope {
    detector: String,
    subject_kind: String,
    subject_id: String,
    metric: String,
    measure: Measure,
}

#[derive(Serialize)]
pub(super) struct Reading {
    query: &'static str,
    measure: Measure,
    evidence: Vec<Evidence>,
    /// Whether code's recomputation still crosses the detector's threshold.
    pub fires: bool,
}

pub(super) fn metric(tx: &Transaction, project: &str, case: &Value, query: &str, fingerprint: &str) -> Result<Reading, String> {
    let rule = Rule::named(query)?;
    let case_signal = &case["signal"];
    let scope: Scope = if case_signal["fingerprint"] == fingerprint {
        args(case_signal.clone())?
    } else {
        args(json!(signals::get(tx, project, fingerprint)?))?
    };
    if scope.detector != rule.id() {
        return Err("The named query did not measure this signal.".into());
    }
    let Measure { window_start: start, window_end: end, .. } = scope.measure;
    let window = Window { before: start - (end - start), start, end };
    let detector = settings::read(tx, project)?.settings.detector(rule);
    let measure = super::measure_detector(tx, project, &detector, window, true)?;
    let entry = measure.readings.into_iter().find(|r| r.key() == (&*scope.subject_kind, &*scope.subject_id, &*scope.metric))
        .ok_or("The named query cannot currently recompute the signal's measure.")?;
    let (baseline, threshold) = entry.limits(detector.threshold).ok_or("The query has no comparable baseline.")?;
    let fires = entry.exceeds(detector.threshold).is_some();
    Ok(Reading { query: rule.id(), measure: Measure { value: entry.value, baseline, threshold,
        window_start: start, window_end: end }, evidence: entry.evidence, fires })
}

fn accept(tx: &Transaction, project: &str, case: &Value, value: Value) -> Result<Finding, String> {
    let mut finding: Finding = args(value)?;
    let reason = finding.escalate.as_mut().map(|e| &mut e.reason);
    for prose in [&mut finding.cause, &mut finding.disproof, &mut finding.fix_proposal].into_iter().chain(reason) {
        if prose.trim().is_empty() || prose.chars().count() > MOST_PROSE {
            return Err("Supply a cause, what would disprove it, and a fix proposal within 2,000 characters each.".into());
        }
        if prose.chars().any(|c| c.is_numeric()) {
            return Err("Put numbers in measurements, where the named queries can check them.".into());
        }
        *prose = crate::sent::redact(prose);
    }
    if !(0.0..=1.0).contains(&finding.confidence) || finding.evidence.is_empty()
        || finding.evidence.len() > MOST_EVIDENCE || finding.measurements.len() > MOST_CLAIMS
    {
        return Err("Supply confidence between zero and one, one to 128 evidence IDs and at most 16 measurements.".into());
    }
    let own = |claim: &Claim| case["signal"]["fingerprint"] == claim.signal;
    let cites_own = finding.measurements.iter().any(own);
    if case["kind"] == "verification" && !cites_own {
        return Err("A verification must cite the recomputed measurement for its signal.".into());
    }
    if let Some(request) = &finding.escalate {
        let signal = signals::get(tx, project, case["signal"]["fingerprint"].as_str().unwrap_or_default())?;
        request.check(&signal, finding.confidence)?;
        if !cites_own { return Err("A critical escalation must cite the recomputed measurement for its signal.".into()); }
    }
    for (i, id) in finding.evidence.iter().enumerate() {
        if round_evidence::resolve(tx, project, id)?.is_none() {
            return Err(format!("Evidence item {} does not resolve to a row in this project.", i + 1));
        }
    }
    let mut own_fires = false;
    for (i, claim) in finding.measurements.iter().enumerate() {
        let reading = metric(tx, project, case, &claim.query, &claim.signal)?;
        if claim.value != reading.measure.value {
            return Err(format!("Measurement {} claims {} but the named query returned {}.",
                i + 1, claim.value, reading.measure.value));
        }
        own_fires |= own(claim) && reading.fires;
    }
    if finding.escalate.is_some() && !own_fires {
        return Err("A critical escalation needs its signal's recomputed measurement to still cross its threshold.".into());
    }
    Ok(finding)
}

#[cfg(test)]
#[path = "handoff_tests.rs"]
mod handoff_tests;
