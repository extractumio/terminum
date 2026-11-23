// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-240, AC-242: measured weekly inputs and sections owned by code.

use rusqlite::Transaction;
use serde_json::{Value, json};

use crate::tasks::ops::{rows, sql};
use super::{Rule, WEEK, Window, decisions as cards, flow, governor, measure_detector, people, register, scoreboard, settings, signals};

pub(super) fn decisions(tx: &Transaction, project: &str, at: i64) -> Result<String, String> {
    decisions_with_spending(tx, project, at, &governor::spending(tx, project, at)?)
}

fn decisions_with_spending(tx: &Transaction, project: &str, at: i64, spending: &governor::Spending) -> Result<String, String> {
    let pending = cards::pending(tx, project)?;
    let budget = rows(tx, "WITH cards AS (
        SELECT project, task FROM supervisor_budget_cards UNION ALL SELECT project, task FROM supervisor_charter_questions
        UNION ALL SELECT project, task FROM supervisor_drift_questions WHERE at > ?2
        ) SELECT t.slug, t.shown FROM cards b JOIN tasks t ON t.id = b.task AND t.project = b.project
        WHERE b.project = ?1 AND t.status = 'WAIT FOR HUMAN' AND t.archived_at IS NULL
          AND NOT EXISTS (SELECT 1 FROM events e WHERE e.task = t.id AND e.author = 'human' AND e.kind = 'comment'
              AND e.rowid > (SELECT max(q.rowid) FROM events q WHERE q.task = t.id AND json_extract(q.data, '$.questions') = 1))",

        (project, at - WEEK), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut lines = Vec::new();
    if spending.judgment.tokens >= spending.judgment.limit_tokens
        || spending.judgment.day_tokens >= spending.judgment.day_limit_tokens || spending.judgment.round_limit_tokens == 0
    {
        lines.push(format!("- {}", governor::JUDGMENT_NOTICE));
    }
    for (task, title) in budget {
        lines.push(format!("- {task} · {title}"));
    }
    for card in &pending {
        let title = card.task.as_ref().map_or_else(|| format!("Register problem {}", card.problem), Clone::clone);
        let question = card.question.lines().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");
        lines.push(format!("- {title}\n{question}"));
    }
    let reduced: i64 = tx.query_row("SELECT count(*) FROM supervisor_decisions d JOIN events e ON e.task = d.task
        WHERE d.project = ?1 AND d.kind = 'standing_rule' AND e.at > ?2 AND e.author = 'human'
        AND json_extract(e.data, '$.choice.option') = json_extract((
            SELECT q.data FROM events q WHERE q.id = json_extract(e.data, '$.choice.question')), '$.question.recommended')",
        (project, at - WEEK), |r| r.get(0)).map_err(sql)?;
    let added: i64 = tx.query_row("SELECT count(*) FROM supervisor_decisions d JOIN tasks t ON t.id = d.task
        WHERE d.project = ?1 AND t.created_at > ?2", (project, at - WEEK), |r| r.get(0)).map_err(sql)?;
    lines.extend(rows(tx, "SELECT question FROM supervisor_drift_questions
        WHERE project = ?1 AND task IS NULL AND at > ?2 ORDER BY at", (project, at - WEEK),
        |r| r.get::<_, String>(0))?);
    if lines.is_empty() { lines.push("No decisions are waiting.".into()); }
    let (old, recent) = question_repeats(tx, project, at)?;
    lines.push(format!("\n{added} decision cards opened this week. {reduced} standing rules accepted as recommended. Repeated questions: {recent}, compared with {old} the week before."));
    Ok(lines.join("\n"))
}

pub(super) fn window(at: i64) -> Window { Window { before: at - 2 * WEEK + 1, start: at - WEEK + 1, end: at } }

pub(super) fn bottlenecks(tx: &Transaction, project: &str, at: i64) -> Result<String, String> {
    let saved = settings::read(tx, project)?.settings;
    let mut lines = Vec::new();
    for (rule, metric, label, proposal) in [
        (Rule::AnswerLatency, "median_answer_minutes", "Median answer latency (minutes)", "Set aside one time to answer the waiting cards."),
        (Rule::PriorityChurn, "priority_reversals", "Priority reversals", "Agree on the priority criteria before the next reprioritization."),
    ] {
        let detector = saved.detector(rule);
        let measurement = measure_detector(tx, project, &detector, window(at), true)?;
        for reading in measurement.readings.into_iter().filter(|r| r.metric == metric && !r.history.is_empty()) {
            let Some((baseline, limit)) = reading.limits(detector.threshold) else { continue };
            if reading.value > baseline && reading.comparison.fires(reading.value, limit) {
                lines.push(format!("- {label}: {:.1} vs {:.1} usual. Excess: {:.1}. {proposal}",
                    reading.value, baseline, reading.value - baseline));
            }
        }
    }
    let (old, recent) = question_repeats(tx, project, at)?;
    if old > 0 && recent > old {
        lines.push(format!("- Repeated identical questions: {recent} vs {old} usual. {} extra answers requested. Consider a standing rule for the repeat.",
            recent - old));
    }
    if lines.is_empty() { lines.push("No measured owner bottleneck exceeded its usual range.".into()); }
    Ok(lines.join("\n"))
}

fn question_repeats(tx: &Transaction, project: &str, at: i64) -> Result<(i64, i64), String> {
    tx.query_row(&format!("{}, counts AS (
         SELECT (at >= ?4) AS recent, text, count(*) AS n FROM project_events
         WHERE at >= ?3 AND kind = 'comment' AND json_extract(data, '$.questions') IS NOT NULL
         AND text IS NOT NULL GROUP BY recent, text)
         SELECT coalesce(sum(CASE WHEN NOT recent THEN n - 1 END), 0),
                coalesce(sum(CASE WHEN recent THEN n - 1 END), 0)
         FROM counts", flow::project_events()), (project, at, at - 2 * WEEK + 1, at - WEEK + 1),
         |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)
}

pub(super) fn context(tx: &Transaction, project: &str, task: &str, at: i64) -> Result<Value, String> {
    let saved = settings::read(tx, project)?.settings;
    let (first, _) = flow::history_bounds(tx, project, at)?;
    let weeks = scoreboard::read(tx, project, at, first)?;
    let board = scoreboard::indicators(&weeks);
    let indicators = board.iter().map(|i| json!({"name":i.name,"verdict":i.verdict.label(),"guard":i.guard,
        "measures":i.lines.iter().map(|line| json!({"name":line.name,"current":line.current,"usual":line.before,
            "noise":line.noise,"missing":line.missing})).collect::<Vec<_>>()})).collect::<Vec<_>>();
    let allocation = measure_detector(tx, project, &saved.detector(Rule::Allocation), window(at), true)?;
    let allocation_rows = allocation.readings.into_iter().map(|r| json!({"task_type":r.subject_id,
        "metric":r.metric,"value":r.value,"comparison":r.history})).collect::<Vec<_>>();
    let rates = super::spend::review_rates(tx, project, window(at))?;
    let problems = register::ranked(tx, project, at, true)?;
    let top_evidence = problems.iter().filter(|p| p.active()).take(3).flat_map(|p| &p.yardstick)
        .map(|y| signals::get(tx, project, &y.fingerprint)).collect::<Result<Vec<_>, _>>()?;
    let last_ok: Option<i64> = tx.query_row("SELECT last_ok FROM project_supervisor WHERE project = ?1", [project], |r| r.get(0)).map_err(sql)?;
    let misses = rows(tx, "SELECT detector, count(*) FROM supervisor_signals
        WHERE project = ?1 AND detector LIKE 'self.%' AND last_seen > ?2
        GROUP BY detector ORDER BY detector", (project, at-WEEK), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let spending = governor::spending(tx, project, at)?;
    let scorecard = super::measures::read(tx, project, at, &spending)?;
    let owner_hours = people::owner_load(tx, project, at - WEEK + 1, at)?.hours;
    Ok(json!({"task":task,"project":project,"decisions":decisions_with_spending(tx, project, at, &spending)?,"problems":problems,
        "top_problem_signals":top_evidence,"charter_data":saved.charter,"allocation":{"readings":allocation_rows,"missing":allocation.unavailable},"agent_rates":rates,"indicators":indicators,
        "any_improved_without_regression":scoreboard::period_improved(&board),"weeks_measured":weeks.len(),
        "owner_hours":owner_hours,"owner_bottlenecks":bottlenecks(tx, project, at)?,"spending":spending,"scorecard":scorecard,
        "supervisor_diagnostics":misses,"collector_last_ok":last_ok}))
}
