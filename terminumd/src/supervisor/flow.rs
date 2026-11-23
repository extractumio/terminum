// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use rusqlite::Transaction;
use std::collections::{BTreeMap, BTreeSet};

use crate::dispatcher::{pool::Slots, waits};
use crate::tasks::{agents, ops::{CURRENT_PROJECT_HISTORY, rows, sql}, store::DEFAULT_PROJECT};

use super::impact::{Source, Unit};
use super::{Comparison, Evidence, Measurements, Query, Reading, Threshold, WEEK, percentile};

// Event IDs survive maintenance; rowid orders transitions within a second. Prior-project history
// cannot be recovered reliably because the old move did not record its project as structured data.
pub(super) fn project_events() -> String {
    format!("WITH project_events AS NOT MATERIALIZED (
        SELECT e.id, e.task, e.at, e.kind, e.author, e.text, e.data, e.rowid AS seq FROM events e JOIN tasks t ON t.id = e.task
        WHERE t.project = ?1 AND e.at <= ?2 {CURRENT_PROJECT_HISTORY}
    )")
}

pub(super) fn history_bounds(tx: &Transaction, project: &str, limit: i64) -> Result<(i64, i64), String> {
    let key = if project == DEFAULT_PROJECT { "" } else { project };
    tx.query_row(&format!("{}, kept(at) AS (
        SELECT at FROM project_events
        UNION ALL SELECT created_at FROM tasks WHERE project = ?1 AND NOT EXISTS
            (SELECT 1 FROM events WHERE task = tasks.id AND kind = 'project')
        UNION ALL SELECT at / 1000 FROM user_interaction WHERE project = ?3
        UNION ALL SELECT at / 1000 FROM messages WHERE coalesce(project, '') = ?3
        UNION ALL SELECT i.at / 1000 FROM inputs i LEFT JOIN runs r ON r.id = i.run
            WHERE coalesce(i.project, r.project, '') = ?3
        UNION ALL SELECT u.at FROM usage u JOIN runs r ON r.id = u.run WHERE r.project = ?1
        UNION ALL SELECT f.at FROM schedule_fires f JOIN schedules s ON s.id = f.schedule WHERE s.project = ?1
        UNION ALL SELECT r.ended FROM integration_rounds r JOIN integration_batches b ON b.id = r.batch WHERE b.project = ?1
        UNION ALL SELECT min(at) FROM supervisor_points WHERE project = ?1 AND at <= ?2
        UNION ALL SELECT max(at) FROM supervisor_points WHERE project = ?1 AND at <= ?2
        UNION ALL SELECT min(at) FROM supervisor_probe_samples WHERE project = ?1 AND at <= ?2
        UNION ALL SELECT max(at) FROM supervisor_probe_samples WHERE project = ?1 AND at <= ?2
        UNION ALL SELECT min(at) FROM integration_verify_results WHERE project = ?1 AND at <= ?2
        UNION ALL SELECT max(at) FROM integration_verify_results WHERE project = ?1 AND at <= ?2
        UNION ALL SELECT min(at) FROM supervisor_hook_counts WHERE project = ?1 AND at <= ?2
        UNION ALL SELECT max(at) FROM supervisor_hook_counts WHERE project = ?1 AND at <= ?2
        ) SELECT coalesce(min(at), ?2), coalesce(max(at), 0) FROM kept WHERE at BETWEEN 1 AND ?2", project_events()),
        (project, limit, key), |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)
}

pub(super) const FIRST_DONE: &str = "first_done AS NOT MATERIALIZED (
    SELECT e.id, e.task, e.at FROM project_events e WHERE e.kind = 'move' AND json_extract(e.data, '$.to') = 'DONE'
    AND e.seq = (SELECT min(x.seq) FROM project_events x WHERE x.task = e.task AND x.kind = 'move'
        AND json_extract(x.data, '$.to') = 'DONE')
)";

pub(super) const LAUNCH: &str = "i.component = 'dispatcher.start' AND i.kind = 'launch' AND i.error IS NULL";

pub(super) fn events(id: String) -> Evidence { Evidence::Events(id) }
fn task(id: &str) -> Evidence { Evidence::Tasks(id.into()) }

pub(super) fn wait_human(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let events_sql = project_events();
    let history = rows(q.tx, &format!("{events_sql}
        SELECT e.id, o.id, (o.at - e.at) / 60.0 FROM project_events e JOIN project_events o
        ON o.seq = (SELECT min(x.seq) FROM project_events x WHERE x.task = e.task AND x.seq > e.seq
            AND x.kind = 'move' AND json_extract(x.data, '$.from') = 'WAIT FOR HUMAN')
        WHERE e.kind = 'move' AND json_extract(e.data, '$.to') = 'WAIT FOR HUMAN' AND o.at >= ?3 AND o.at >= e.at"),
        (q.project, q.window.end, q.window.before), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?)))?;
    let mut out = Measurements::default();
    let candidates = if q.historical {
        out.unavailable.push("Historical queued presses and blocking links are unavailable.");
        rows(q.tx, &format!("{events_sql}
            SELECT t.id, w.id, w.at FROM tasks t JOIN project_events w ON w.task = t.id
            WHERE t.project = ?1 AND w.kind = 'move'
              AND json_extract(w.data, '$.to') = 'WAIT FOR HUMAN'
              AND (t.archived_at IS NULL OR t.archived_at > ?2)
              AND NOT EXISTS (SELECT 1 FROM project_events x WHERE x.task = t.id AND x.seq > w.seq
                AND x.kind IN ('move', 'archive'))"), (q.project, q.window.end),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
    } else {
        rows(q.tx, &format!("{events_sql}
            SELECT t.id, w.id, w.at FROM tasks t LEFT JOIN project_events w
            ON w.seq = (SELECT max(x.seq) FROM project_events x WHERE x.task = t.id AND x.kind = 'move'
                AND json_extract(x.data, '$.to') = 'WAIT FOR HUMAN')
            WHERE t.project = ?1 AND t.status = 'WAIT FOR HUMAN' AND t.archived_at IS NULL AND NOT ({})",
            waits::queued_task("t")), (q.project, q.window.end), |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<i64>>(2)?)))?
    };
    let baseline = match q.detector.threshold {
        Threshold::Baseline { percentile: p, floor } => {
            let baseline = percentile(&mut history.iter().map(|(_, _, n)| *n).collect::<Vec<_>>(), p).unwrap_or(floor);
            out.threshold_hint = Some(("wait_minutes", baseline.max(floor)));
            baseline
        }
        Threshold::Absolute(limit) => {
            out.threshold_hint = Some(("wait_minutes", limit));
            0.0
        }
        Threshold::Change { .. } => 0.0,
    };
    let history_evidence: Vec<_> = history.into_iter().flat_map(|(a, b, _)| [events(a), events(b)]).collect();
    for (id, wait, at) in candidates {
        let Some((wait, at)) = wait.zip(at) else {
            if out.unavailable.is_empty() {
                out.unavailable.push("Some waiting tasks lack a retained wait transition.");
            }
            continue;
        };
        let blocked = if q.historical { Vec::new() } else { rows(q.tx, "SELECT b.id FROM task_links l JOIN tasks b ON b.id = l.task
            WHERE l.other = ?1 AND l.kind = 'blocked_by' AND b.project = ?2 AND b.archived_at IS NULL
                AND b.status NOT IN ('DONE', 'FAILED') ORDER BY b.id", (&id, q.project), |r| r.get::<_, String>(0))? };
        let evidence = vec![task(&id), events(wait)];
        if !blocked.is_empty() {
            let mut evidence = evidence.clone();
            evidence.extend(blocked.iter().map(|b| Evidence::TaskLinks { task: b.clone(), other: id.clone() }));
            let count = blocked.len();
            out.readings.push(Reading::new("task", &id, "blocked_tasks", count as f64, evidence, Source::BlockingLinks).fixed(Threshold::Absolute(0.0)));
        }
        let mut age = Reading::new("task", &id, "wait_minutes", (q.window.end - at) as f64 / 60.0, evidence, Source::WaitDays).history(vec![baseline]);
        if age.exceeds(q.detector.threshold).is_some() {
            age.evidence.extend(history_evidence.iter().cloned());
            out.readings.push(age);
        }
    }
    Ok(out)
}

pub(super) fn reasonless(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let events_sql = project_events();
    let length = q.window.end - q.window.start;
    let history_start = q.window.start - 4 * length;
    let failures = rows(q.tx, &format!("{events_sql}
        SELECT e.id, e.at FROM project_events e WHERE e.kind = 'move' AND json_extract(e.data, '$.to') = 'FAILED' AND e.at >= ?3
        AND NOT EXISTS (SELECT 1 FROM project_events w WHERE w.task = e.task AND w.at = e.at
            AND w.seq > e.seq AND w.kind = 'why' AND w.author = 'dispatcher' AND length(trim(w.text)) > 0
            AND NOT EXISTS (SELECT 1 FROM project_events m WHERE m.task = e.task AND m.kind = 'move' AND m.seq BETWEEN e.seq + 1 AND w.seq))
        AND NOT EXISTS (SELECT 1 FROM project_events w WHERE w.task = e.task AND w.kind = 'comment'
            AND w.author = e.author AND w.seq < e.seq AND w.at BETWEEN e.at - 60 AND e.at
            AND json_extract(w.data, '$.questions') IS NULL AND length(trim(w.text)) > 0)"),
        (q.project, q.window.end, history_start), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let mut value = 0.0;
    let mut history = vec![0.0; 4];
    let mut evidence = Vec::new();
    for (id, at) in failures {
        if q.window.current(at) {
            value += 1.0;
        } else {
            history[((at - history_start) / length) as usize] += 1.0;
        }
        evidence.push(events(id));
    }
    Ok(one(Reading::new("project", q.project, "reasonless_failures", value, evidence, Source::Count(Unit::FailureEvents)).history(history)))
}

// The run lifecycle state describes the pane, not its task's disposition. Link a FAILED transition
// to the latest launch of a run carrying that task, with its retained run row as evidence.
pub(super) fn failed_runs() -> String {
    run_events("failed_runs", "e.kind = 'move' AND json_extract(e.data, '$.to') = 'FAILED'")
}

pub(super) const RETURNED: &str = "e.kind = 'comment' AND e.author = 'dispatcher' AND json_extract(e.data, '$.left') IN ('red', 'conflict', 'out')";

pub(super) fn returned_runs() -> String {
    run_events("returned_runs", RETURNED)
}

fn run_events(name: &str, condition: &str) -> String {
    format!("{name} AS (
    SELECT e.id, e.task, e.at, (SELECT r.id FROM run_tasks m JOIN runs r ON r.id = m.run
        JOIN inputs i ON i.run = r.id AND {LAUNCH}
        WHERE m.task = e.task AND r.project = ?1 AND i.at <= e.at * 1000 + 999
        ORDER BY i.at DESC, i.id DESC LIMIT 1) AS run
    FROM project_events e WHERE {condition}
)")
}

pub(super) fn failed_repeat(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let events_sql = project_events();
    let failed = failed_runs();
    let rows = rows(q.tx, &format!("{events_sql}, {failed} SELECT task, run, id,
            (SELECT count(*) FROM run_tasks WHERE run = f.run) = 1 FROM failed_runs f
        WHERE at >= ?3 ORDER BY task, at, id"), (q.project, q.window.end, q.window.start),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?, r.get::<_, bool>(3)?)))?;
    let mut groups = BTreeMap::<String, BTreeMap<String, (String, bool)>>::new();
    let mut out = Measurements::default();
    for (task, run, event, sole_task) in rows {
        if let Some(run) = run {
            groups.entry(task).or_default().insert(run, (event, sole_task));
        } else {
            out.unavailable = vec!["Some failures lack a retained run launch."];
        }
    }
    for (task, runs) in groups {
        let count = runs.len() as f64;
        let attributed = runs.iter().filter(|(_, (_, sole_task))| *sole_task).map(|(run, _)| run.clone()).collect();
        let evidence = runs.into_iter().flat_map(|(run, (event, _))| [Evidence::Runs(run), events(event)]).collect();
        out.readings.push(Reading::new("task", task, "failed_runs", count, evidence, Source::Count(Unit::FailureEvents))
            .attributed_runs(attributed).comparison(Comparison::AtLeast));
    }
    Ok(out)
}

pub(super) fn rerun(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let events_sql = project_events();
    let launches = rows(q.tx, &format!("{events_sql}
        SELECT m.task, r.id, i.id, e.id, (SELECT count(*) FROM run_tasks WHERE run = r.id) = 1
        FROM inputs i JOIN runs r ON r.id = i.run
        JOIN run_tasks m ON m.run = r.id JOIN tasks t ON t.id = m.task
        JOIN project_events e ON e.seq = (SELECT max(d.seq) FROM project_events d
            WHERE d.task = m.task AND d.kind = 'move' AND json_extract(d.data, '$.to') = 'DONE' AND d.at < i.at / 1000)
        WHERE {LAUNCH} AND i.at BETWEEN ?3 * 1000 AND ?2 * 1000 + 999
            AND t.project = ?1 AND r.project = ?1
        ORDER BY m.task, i.at, i.id"), (q.project, q.window.end, q.window.start),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?, r.get::<_, bool>(4)?)))?;
    let mut groups = BTreeMap::<String, (BTreeSet<String>, BTreeSet<String>, Vec<Evidence>)>::new();
    for (task, run, input, done, sole_task) in launches {
        let (runs, attributed, evidence) = groups.entry(task).or_default();
        if runs.insert(run.clone()) {
            if sole_task {
                attributed.insert(run.clone());
            }
            evidence.extend([Evidence::Runs(run), Evidence::Inputs(input), events(done)]);
        }
    }
    Ok(Measurements { readings: groups.into_iter().map(|(task, (runs, attributed, evidence))|
        Reading::new("task", task, "runs_after_done", runs.len() as f64, evidence, Source::Count(Unit::RunEvents))
            .attributed_runs(attributed)).collect(), ..Measurements::default() })
}

pub(super) fn starved(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let events_sql = project_events();
    let agents = agents::of_project(q.tx, q.project)?;
    if agents.is_empty() {
        return Ok(Measurements::missing("No project agent can take a slot."));
    }
    let slots = Slots::read(q.tx)?;
    let tasks = rows(q.tx, &format!("{events_sql}
        SELECT t.id, coalesce(m.at, t.created_at), m.id, t.agent FROM tasks t LEFT JOIN project_events m
        ON m.seq = (SELECT max(e.seq) FROM project_events e WHERE e.task = t.id AND e.kind = 'move'
            AND json_extract(e.data, '$.to') = 'TODO')
        WHERE t.project = ?1 AND t.status = 'TODO' AND t.severity IN ('P0','P1') AND t.archived_at IS NULL
        AND NOT EXISTS (SELECT 1 FROM task_links l JOIN tasks b ON b.id = l.other
            WHERE l.task = t.id AND l.kind = 'blocked_by' AND (b.status <> 'DONE' OR b.archived_at IS NOT NULL))"),
        (q.project, q.window.end), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?)))?;
    let mut out = Measurements::default();
    for (id, entered, event, assigned) in tasks {
        if entered <= 0 || entered > q.window.end || (event.is_none() && moved_project(q, &id)?) {
            out.unavailable = vec!["Some TODO age history is unavailable."];
            continue;
        }
        let mut free = false;
        for agent in agents.iter().filter(|a| assigned.as_ref().is_none_or(|id| a.id == *id)) {
            if slots.check_for(q.tx, agent, q.project, Some(&id), None)?.is_none() {
                free = true;
                break;
            }
        }
        if free {
            let evidence = std::iter::once(task(&id)).chain(event.map(events)).collect();
            out.readings.push(Reading::new("task", id, "todo_minutes", (q.window.end - entered) as f64 / 60.0, evidence, Source::OverdueDays));
        }
    }
    Ok(out)
}

fn moved_project(q: &Query<'_, '_>, id: &str) -> Result<bool, String> {
    q.tx.query_row("SELECT EXISTS (SELECT 1 FROM events WHERE task = ?1 AND kind = 'project')", [id], |r| r.get(0))
        .map_err(crate::tasks::ops::sql)
}

pub(super) fn bugs(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let bugs = rows(q.tx, "SELECT id, created_at FROM tasks WHERE project = ?1 AND archived_at IS NULL
        AND type = 'Bug' AND severity IN ('P0','P1') AND status NOT IN ('DONE','FAILED') AND created_at BETWEEN 1 AND ?2",
        (q.project, q.window.end), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    Ok(Measurements { readings: bugs.into_iter().map(|(id, at)| Reading::new(
        "task", &id, "open_bug_minutes", (q.window.end - at) as f64 / 60.0, vec![task(&id)], Source::OverdueDays)).collect(), ..Measurements::default() })
}

pub(super) fn lead_time(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let events_sql = project_events();
    let samples = rows(q.tx, &format!("{events_sql}, {FIRST_DONE}
        SELECT e.id, e.task, e.at, (e.at - t.created_at) / 60.0 FROM first_done e JOIN tasks t ON t.id = e.task
        WHERE e.at >= ?3 AND t.created_at BETWEEN 1 AND e.at
        AND NOT EXISTS (SELECT 1 FROM events p WHERE p.task = e.task AND p.kind = 'project')"),
        (q.project, q.window.end, q.window.before), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, f64>(3)?)))?;
    let (mut recent, mut earlier, mut evidence) = (vec![], vec![], vec![]);
    for (id, task_id, at, duration) in samples {
        (if q.window.current(at) { &mut recent } else { &mut earlier }).push(duration);
        evidence.extend([events(id), task(&task_id)]);
    }
    if recent.is_empty() || earlier.is_empty() {
        return Ok(Measurements::missing("Lead time needs DONE tasks in both windows."));
    }
    let mut readings = Vec::new();
    for (name, p) in [("median_lead_minutes", 50), ("p90_lead_minutes", 90)] {
        let value = percentile(&mut recent, p).unwrap();
        let baseline = percentile(&mut earlier, p).unwrap();
        readings.push(Reading::new("project", q.project, name, value, evidence.clone(), Source::LeadDays).history(vec![baseline]));
    }
    Ok(Measurements { readings, ..Measurements::default() })
}

pub(super) fn backlog(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let events_sql = project_events();
    let events_in_window = rows(q.tx, &format!("{events_sql}, {FIRST_DONE}
        SELECT id, at, 1 FROM project_events WHERE at >= ?3 AND kind = 'made'
        UNION ALL SELECT id, at, 0 FROM first_done WHERE at >= ?3"),
        (q.project, q.window.end, q.window.start), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, bool>(2)?)))?;
    let mut weeks = BTreeMap::<i64, (i64, Vec<Evidence>)>::new();
    for (id, at, made) in events_in_window {
        let (net, evidence) = weeks.entry((q.window.end - at) / WEEK).or_default();
        *net += if made { 1 } else { -1 };
        evidence.push(events(id));
    }
    let mut count = 0;
    let mut evidence = Vec::new();
    for week in 0..(q.window.end - q.window.start) / WEEK {
        let Some((net, rows)) = weeks.remove(&week) else { break };
        if net <= 0 { break; }
        count += 1;
        evidence.extend(rows);
    }
    Ok(one(Reading::new("project", q.project, "growing_weeks", f64::from(count), evidence, Source::BacklogGrowth).comparison(Comparison::AtLeast)))
}

pub(super) fn schedule(q: &Query<'_, '_>) -> Result<Measurements, String> {
    if !q.historical && !q.tx.query_row("SELECT EXISTS (SELECT 1 FROM schedules WHERE project = ?1 AND deleted_at IS NULL)",
        [q.project], |r| r.get::<_, bool>(0)).map_err(sql)?
    {
        return Ok(Measurements::missing("No project routines are configured."));
    }
    let events_sql = project_events();
    let fires = rows(q.tx, &format!("{events_sql}
        SELECT s.id, f.id, f.state IN ('failed','skipped'), f.missed,
            (SELECT e.id FROM project_events e WHERE e.task = f.task AND e.kind = 'move' AND e.at >= ?3
                AND json_extract(e.data, '$.to') = 'FAILED' ORDER BY e.seq DESC LIMIT 1)
        FROM schedule_fires f JOIN schedules s ON s.id = f.schedule WHERE s.project = ?1 AND f.at BETWEEN ?3 AND ?2"),
        (q.project, q.window.end, q.window.start), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, bool>(2)?,
            r.get::<_, i64>(3)?, r.get::<_, Option<String>>(4)?)))?;
    let mut trouble = BTreeMap::<String, (i64, Vec<Evidence>)>::new();
    let mut failures = BTreeMap::<String, Vec<Evidence>>::new();
    for (schedule, id, broken, missed, failed) in fires {
        if broken {
            let (count, evidence) = trouble.entry(schedule.clone()).or_default();
            *count += 1 + missed;
            evidence.push(Evidence::ScheduleFires(id));
        }
        if let Some(event) = failed {
            failures.entry(schedule).or_default().extend([Evidence::ScheduleFires(id), events(event)]);
        }
    }
    let mut readings: Vec<_> = trouble.into_iter().map(|(id, (count, evidence))|
        Reading::new("schedule", id, "skipped_or_failed_fires", count as f64, evidence, Source::Count(Unit::ScheduleEvents))).collect();
    readings.extend(failures.into_iter().map(|(id, evidence)| {
        let value = (evidence.len() / 2) as f64;
        Reading::new("schedule", id, "failed_routine_tasks", value, evidence, Source::Count(Unit::FailureEvents)).fixed(Threshold::Absolute(1.0))
    }));
    Ok(Measurements { readings, ..Measurements::default() })
}

pub(super) fn one(reading: Reading) -> Measurements {
    Measurements { readings: vec![reading], ..Measurements::default() }
}
