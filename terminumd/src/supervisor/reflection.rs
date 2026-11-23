// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-248: durable reflection requests and proposal-only results.

use rusqlite::{OptionalExtension, Transaction, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tasks::{comments, ops::{self, NewTask, rows, sql}};
use super::{governor, outcomes, reflection_timeline, rounds, settings, signals};

pub(crate) const TOOL: &str = "record_reflection";
pub(crate) const TIME_LIMIT: i64 = 30 * 60;

pub(super) fn action(tx: &Transaction, project: &str, action: &outcomes::Action, trigger: &str, at: i64) -> Result<(), String> {
    enqueue(tx, project, ("action", &action.id.to_string(), Some(&action.fingerprint)), trigger, &Value::Null, at)
}

pub(super) fn enqueue(tx: &Transaction, project: &str, (kind, subject, signal): (&str, &str, Option<&str>),
    trigger: &str, detail: &Value, at: i64) -> Result<(), String>
{
    let Some(generation) = rounds::enabled(tx, project)? else { return Ok(()) };
    let key = format!("reflection_{}", crate::auth::crypto::sha256_hex(
        &serde_json::to_vec(&(project, generation, kind, subject)).map_err(|e| e.to_string())?));
    tx.execute("INSERT INTO supervisor_reflections
        (fingerprint, project, generation, subject_kind, subject_id, signal, at, triggers, detail)
        VALUES (?1,?2,?3,?4,?5,nullif(?6,''),?7,json_array(?8),?9)
        ON CONFLICT (fingerprint) DO UPDATE SET triggers = json_insert(triggers, '$[#]', ?8)
        WHERE NOT EXISTS (SELECT 1 FROM json_each(triggers) WHERE value = ?8)",
        params![key, project, generation, kind, subject, signal, at, trigger, detail.to_string()]).map(drop).map_err(sql)
}

pub(super) fn budget(tx: &Transaction, project: &str, spend: &governor::Spending, at: i64) -> Result<(), String> {
    let crossed = budget_crossings(tx, project, spend)?;
    if crossed.is_empty() { return Ok(()); }
    enqueue(tx, project, ("budget", &at.to_string(), None), "budget_exceeded", &json!(crossed), at)
}

pub(super) fn usage_overrun(tx: &Transaction, project: &str, run: &str, limit: i64, tokens: f64, at: i64) -> Result<(), String> {
    let crossed = budget_crossings(tx, project, &governor::spending(tx, project, at)?)?;
    // Keep the budget state current even when the overrunning round itself was reflection.
    if is_round(tx, run)? { return Ok(()); }
    enqueue(tx, project, ("run", run, None), "budget_exceeded",
        &json!({"tokens":tokens,"limit":limit,"project_limits":crossed}), at)
}

fn budget_crossings(tx: &Transaction, project: &str, spend: &governor::Spending) -> Result<Vec<Value>, String> {
    let Some(generation) = rounds::enabled(tx, project)? else { return Ok(Vec::new()) };
    let judgment = &spend.judgment;
    let fixes = &spend.fixes;
    let mut crossed = Vec::new();
    for (metric, used, limit) in [
        ("judgment_week_tokens", judgment.tokens as f64, judgment.limit_tokens as f64),
        ("judgment_day_tokens", judgment.day_tokens as f64, judgment.day_limit_tokens as f64),
        ("fix_dollars", (fixes.dollars - fixes.exception_dollars).max(0.), fixes.limit_dollars),
    ] {
        if used <= limit {
            tx.execute("DELETE FROM supervisor_budget_overruns WHERE project = ?1 AND metric = ?2",
                (project, metric)).map_err(sql)?;
        } else {
            let changed = tx.execute("INSERT INTO supervisor_budget_overruns VALUES (?1,?2,?3)
                ON CONFLICT (project, metric) DO UPDATE SET generation = excluded.generation
                WHERE generation <> excluded.generation", (project, generation, metric)).map_err(sql)?;
            if changed > 0 { crossed.push(json!({"metric":metric,"used":used,"limit":limit})); }
        }
    }
    Ok(crossed)
}

pub(super) fn waiting(tx: &Transaction, at: i64, key: Option<&str>) -> Result<Vec<(String, String)>, String> {
    rows(tx, "SELECT r.project, r.fingerprint FROM supervisor_reflections r
        JOIN project_supervisor p ON p.project = r.project AND p.generation = r.generation AND p.enabled
        WHERE r.round IS NULL AND (?1 IS NULL OR r.fingerprint = ?1) AND (r.retry_at IS NULL OR r.retry_at <= ?2)
        ORDER BY r.at, r.fingerprint LIMIT 16", (key, at), |r| Ok((r.get(0)?, r.get(1)?)))
}

pub(super) fn reason(tx: &Transaction, key: &str, reason: &str) -> Result<(), String> {
    tx.execute("UPDATE supervisor_reflections SET reason = ?2, retry_at = ?3 WHERE fingerprint = ?1 AND round IS NULL",
        (key, reason, ops::now() + 5)).map(drop).map_err(sql)
}

pub(super) fn case(tx: &Transaction, project: &str, key: &str, at: i64) -> Result<Value, String> {
    let (kind, subject, signal, triggers, detail, observed): (String, String, Option<String>, Value, Value, i64) =
        tx.query_row("SELECT subject_kind, subject_id, signal, triggers, detail, at FROM supervisor_reflections
            WHERE project = ?1 AND fingerprint = ?2 AND round IS NULL", (project, key),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, signals::decode(r, 3)?, signals::decode(r, 4)?, r.get(5)?))).map_err(sql)?;
    let mut timeline = reflection_timeline::build(tx, project, &kind, &subject, signal.as_deref(), at)?;
    timeline.push(json!({"at":observed,"kind":"trigger","ref":subject,"data":detail,"reasons":triggers}));
    timeline.sort_by_key(|event| event["at"].as_i64());
    for (index, row) in timeline.iter_mut().enumerate() { row["number"] = json!(index + 1); }
    let rates = super::spend::review_rates(tx, project, super::review_data::window(at))?;
    Ok(json!({
        "kind": "reflection", "subject_kind": kind, "subject_id": subject, "triggers": triggers,
        "timeline": timeline, "agent_rates": rates, "settings_revision": settings::revision(tx, project)?,
        "tools": ["get_case_file", TOOL],
        "question": "What evidence was missed, what was noise, what was wrong, and what should change?",
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    change: String,
    replay: String,
    evidence: Vec<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Routing {
    agent: String,
    task_type: String,
    change: String,
    why: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultDraft {
    missed: String,
    noise: String,
    wrong: String,
    proposals: Vec<Proposal>,
    routing: Option<Routing>,
}

fn prose(value: &str) -> Result<String, String> {
    let clean = crate::sent::redact(&crate::plans::rules::clean_with(value, false));
    if clean.is_empty() || clean.chars().count() > 1_000 {
        Err("Give each reflection explanation in 1–1000 characters.".into())
    } else { Ok(clean) }
}

fn routing_card(tx: &Transaction, project: &str, case: &Value, draft: Routing, at: i64) -> Result<String, String> {
    let agent = crate::tasks::agents::chosen(tx, &draft.agent, project)?;
    if !crate::tasks::attrs::TYPES.iter().any(|(kind, _)| *kind == draft.task_type) {
        return Err("Choose a known task type for the routing proposal.".into());
    }
    let rates = case["agent_rates"].as_array().and_then(|rates| rates.iter().find(|r|
        r["agent"] == draft.agent && r["task_type"] == draft.task_type));
    let measured = ["failure", "return", "questions", "cost"].map(|metric| {
        rates.and_then(|r| r.get(metric)).filter(|r| !r["value"].is_null()).map_or_else(
            || format!("{metric} unavailable"), |r| format!("{metric} {:.4} (n = {})",
                r["value"].as_f64().unwrap_or_default(), r["sample"]))
    });
    let case_id = case["id"].as_str().ok_or("This reflection case has no ID.")?;
    let words = format!("Why you are needed:\nOnly you can change the project's routing.\n\nRecommended:\n1. Review the proposed change. Keep current routing while you decide.\n\nDetails:\nAgent {} · {}. {} {} Case {case_id}. Measured rates: {}.",
        prose(&agent.name)?, draft.task_type, prose(&draft.change)?, prose(&draft.why)?, measured.join(", "));
    let by = crate::roles::supervisor::author(tx, project)?;
    let card = ops::create(tx, &by, NewTask { title: "Supervisor routing decision".into(), text: words.clone(),
        project: Some(project.into()), severity: Some("P1".into()), ..NewTask::default() }, at)?;
    super::task_policy::mark_card(tx, &card)?;
    let choice = json!({"kind":"strategic","recommended":"keep","options":[
        {"id":"keep","label":"Keep current routing"},{"id":"review","label":"Review proposed routing"}]});
    let question = comments::wait_for_owner(tx, &by, &card, &words, &choice, at)?;
    outcomes::unattributed(tx, project, outcomes::Kind::Card, &question, &card.key, at)?;
    tx.execute("UPDATE supervisor_reflections SET routing_card = ?2 WHERE round = ?1",
        (case_id, &card.key)).map_err(sql)?;
    Ok(card.id)
}

pub(super) fn complete(tx: &Transaction, project: &str, run: &str, case: &Value, value: Value, at: i64) -> Result<Option<String>, String> {
    if case["kind"] != "reflection" { return Err("Only a live reflection can submit this result.".into()); }
    let draft: ResultDraft = serde_json::from_value(value).map_err(|_| "Invalid reflection result.")?;
    let len = case["timeline"].as_array().map_or(0, Vec::len);
    if draft.proposals.len() > 8 { return Err("Give at most eight reflection proposals.".into()); }
    let proposals = draft.proposals.into_iter().map(|p| {
        if p.evidence.is_empty() || p.evidence.len() > len || p.evidence.iter().any(|&i| i == 0 || i > len) {
            return Err("Cite the numbered timeline events from this case.".to_string());
        }
        Ok(json!({"change":prose(&p.change)?,"replay":prose(&p.replay)?,"evidence":p.evidence}))
    }).collect::<Result<Vec<_>, _>>()?;
    let result = json!({"missed":prose(&draft.missed)?,"noise":prose(&draft.noise)?,"wrong":prose(&draft.wrong)?,
        "settings_revision":case["settings_revision"],"proposals":proposals});
    let changed = tx.execute("UPDATE supervisor_rounds SET finding = ?2, found_at = ?3
        WHERE run = ?1 AND deadline > ?3 AND finding IS NULL AND failure IS NULL",
        (run, result.to_string(), at)).map_err(sql)?;
    if changed == 0 { return Err("The reflection is no longer active.".into()); }
    draft.routing.map(|p| routing_card(tx, project, case, p, at)).transpose()
}

pub(super) fn shown(tx: &Transaction, project: &str) -> Result<Vec<Value>, String> {
    rows(tx, "SELECT json_object('case', q.id, 'at', r.at, 'triggers', json(r.triggers), 'waiting', r.reason,
        'result', json(q.finding), 'failure', q.failure, 'ended_at', q.ended_at, 'routing_card', t.slug)
        FROM supervisor_reflections r LEFT JOIN supervisor_rounds q ON q.id = r.round
        LEFT JOIN tasks t ON t.id = r.routing_card WHERE r.project = ?1 AND
        (q.run IS NOT NULL OR r.generation = (SELECT generation FROM project_supervisor WHERE project = ?1 AND enabled))
        ORDER BY r.at DESC, r.fingerprint LIMIT 20", [project], |r| signals::decode(r, 0))
}

pub(super) fn is_round(tx: &Transaction, run: &str) -> Result<bool, String> {
    tx.query_row("SELECT 1 FROM supervisor_reflections r JOIN supervisor_rounds q ON q.id = r.round WHERE q.run = ?1",
        [run], |_| Ok(true)).optional().map(|r| r.unwrap_or(false)).map_err(sql)
}

pub(super) fn source_agent(tx: &Transaction, project: &str, key: &str) -> Result<Option<String>, String> {
    tx.query_row("SELECT u.agent_id FROM supervisor_reflections r
        JOIN runs u ON u.project = r.project AND (
          (r.subject_kind = 'run' AND u.id = r.subject_id)
          OR (r.subject_kind = 'action' AND u.id = (
              SELECT CASE WHEN a.kind IN ('run','fix') THEN a.baseline WHEN a.kind = 'cause' THEN a.source END
              FROM supervisor_actions a WHERE a.project = r.project AND a.id = CAST(r.subject_id AS INTEGER)))
          OR (r.subject_kind = 'problem' AND u.id = (
              SELECT p.source_round FROM supervisor_problems p
              WHERE p.project = r.project AND p.id = CAST(r.subject_id AS INTEGER))))
        WHERE r.project = ?1 AND r.fingerprint = ?2", (project, key), |r| r.get(0)).optional().map_err(sql)
}
