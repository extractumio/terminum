// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use std::collections::BTreeMap;

use crate::dashboard::{self, Span};
use crate::dispatcher::runs::GAVE_UP;
use crate::tasks::{ops::{rows, sql}, store::DEFAULT_PROJECT};

use super::flow::{FIRST_DONE, LAUNCH, events, one, project_events};
use super::impact::{Source, Unit};
use super::{Comparison, Evidence, Measurements, Query, Reading, Threshold, percentile};

const GUARD: &str = "A guard compares two indicators; the quantities behind each are sized by their own detectors (improve.involvement, improve.operations, owner.load, flow.failed_no_reason).";

fn local_time(tx: &rusqlite::Transaction, unix_ms: i64) -> Result<i64, String> {
    tx.query_row(&format!("SELECT {}", dashboard::local("?1")), [unix_ms], |r| r.get(0)).map_err(sql)
}

#[derive(Clone, Copy, Default)]
pub(super) struct Load {
    pub hours: f64,
    pub switch_count: f64,
    pub off_hours: f64,
    switches: f64,
    pub peak: f64,
    off: f64,
}

pub(super) fn owner_load(tx: &rusqlite::Transaction, project: &str, from: i64, until: i64) -> Result<Load, String> {
    let start = local_time(tx, from * 1000)?;
    let end = local_time(tx, until * 1000 + 999)?;
    let mut edges = vec![start];
    edges.extend((start.div_euclid(dashboard::DAY) + 1..=end.div_euclid(dashboard::DAY)).map(|d| d * dashboard::DAY));
    let span = Span { unit: "day", edges, until: end };
    let load = dashboard::you::project_load(tx, project, &span, from * 1000)?;
    Ok(Load {
        hours: load.active_ms as f64 / dashboard::HOUR as f64,
        switches: load.switches_per_hour,
        switch_count: load.switches as f64,
        off_hours: load.off_ms as f64 / dashboard::HOUR as f64,
        peak: load.peak as f64,
        off: load.off_hours_share.unwrap_or(0.0),
    })
}

fn load_windows(q: &Query<'_, '_>) -> Result<[Load; 2], String> {
    Ok([owner_load(q.tx, q.project, q.window.before, q.window.start - 1)?, owner_load(q.tx, q.project, q.window.start, q.window.end)?])
}

fn owner_evidence(q: &Query<'_, '_>) -> Result<Vec<Evidence>, String> {
    let project = if q.project == DEFAULT_PROJECT { "" } else { q.project };
    let events_sql = project_events();
    let mut evidence = rows(q.tx, "SELECT id, 'inputs' FROM inputs WHERE sender = 'human' AND coalesce(project, '') = ?1 AND at BETWEEN ?2 AND ?3
        UNION ALL SELECT id, 'messages' FROM messages WHERE sender = 'human' AND coalesce(project, '') = ?1 AND at BETWEEN ?2 AND ?3
        UNION ALL SELECT id, 'user_interaction' FROM user_interaction WHERE project = ?1 AND at BETWEEN ?2 AND ?3",
        (project, q.window.before * 1000, q.window.end * 1000 + 999), |r| Ok(match r.get::<_, String>(1)?.as_str() {
            "inputs" => Evidence::Inputs(r.get(0)?),
            "messages" => Evidence::Messages(r.get(0)?),
            _ => Evidence::UserInteraction(r.get(0)?),
        }))?;
    evidence.extend(rows(q.tx, &format!("{events_sql} SELECT id FROM project_events WHERE author = 'human' AND at >= ?3"),
        (q.project, q.window.end, q.window.before), |r| Ok(events(r.get(0)?)))?);
    Ok(evidence)
}

struct Done {
    at: i64,
    feature: bool,
    event: String,
    task: String,
}

fn done(q: &Query<'_, '_>) -> Result<Vec<Done>, String> {
    let events_sql = project_events();
    rows(q.tx, &format!("{events_sql}, {FIRST_DONE}
        SELECT e.at, coalesce(t.type = 'Feature', 0), e.id, e.task
        FROM first_done e JOIN tasks t ON t.id = e.task WHERE e.at >= ?3"),
        (q.project, q.window.end, q.window.before), |r| Ok(Done { at: r.get(0)?, feature: r.get(1)?, event: r.get(2)?, task: r.get(3)? }))
}

fn feature_rates(q: &Query<'_, '_>, loads: [Load; 2], done: &[Done]) -> Option<[f64; 2]> {
    if loads.iter().any(|s| s.hours == 0.0) {
        return None;
    }
    let mut features = [0; 2];
    for row in done.iter().filter(|row| row.feature) {
        features[usize::from(q.window.current(row.at))] += 1;
    }
    Some(std::array::from_fn(|i| f64::from(features[i]) / loads[i].hours))
}

pub(super) fn involvement(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let done = done(q)?;
    let Some([old, now]) = feature_rates(q, load_windows(q)?, &done) else {
        return Ok(Measurements::missing("Features per owner hour needs measured owner time in both windows."));
    };
    let mut evidence = owner_evidence(q)?;
    evidence.extend(done.into_iter().filter(|row| row.feature).flat_map(|r| [events(r.event), Evidence::Tasks(r.task)]));
    Ok(one(Reading::new("project", q.project, "features_per_owner_hour", now, evidence, Source::ExcessOwnerHours)
        .history(vec![old]).comparison(Comparison::Below)))
}

pub(super) fn load(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let [before, now] = load_windows(q)?;
    if before.hours == 0.0 || now.hours == 0.0 {
        return Ok(Measurements::missing("Owner load needs measured owner time in both windows."));
    }
    let evidence = owner_evidence(q)?;
    let readings = [("switches_per_hour", before.switches, now.switches, Source::ExcessSwitches),
        ("concurrent_sessions", before.peak, now.peak,
            Source::Unsized("A peak of concurrent sessions is not additive; its cost shows as switches and owner hours, which owner.load sizes.")),
        ("off_hours_share", before.off, now.off, Source::ExcessOffHours)].into_iter().map(|(metric, old, new, source)|
        Reading::new("project", q.project, metric, new, evidence.clone(), source).history(vec![old])).collect();
    Ok(Measurements { readings, ..Measurements::default() })
}

pub(super) const QUESTIONS: &str = "questions AS (
    SELECT e.id, e.task, e.at, e.seq, json_extract(e.data, '$.question.kind') AS kind,
        json_extract(e.data, '$.question.recommended') AS recommended,
        (SELECT min(n.seq) FROM project_events n WHERE n.task = e.task AND n.seq > e.seq
            AND n.kind = 'comment' AND json_extract(n.data, '$.questions') IS NOT NULL) AS next
    FROM project_events e WHERE e.kind = 'comment' AND json_extract(e.data, '$.questions') IS NOT NULL
), answers AS (
    SELECT q.id AS question, q.task, q.at AS asked_at, q.next, a.id AS answer, a.at AS answered_at,
        q.kind, json_extract(a.data, '$.choice.option') AS chosen, q.recommended, a.seq AS answer_seq
    FROM questions q LEFT JOIN project_events a ON a.seq = coalesce(
        (SELECT min(h.seq) FROM project_events h WHERE h.task = q.task AND h.seq > q.seq
            AND h.kind = 'comment' AND h.author = 'human' AND h.at >= q.at
            AND json_extract(h.data, '$.choice.question') = q.id),
        (SELECT min(h.seq) FROM project_events h WHERE h.task = q.task AND h.seq > q.seq
            AND (q.next IS NULL OR h.seq < q.next) AND h.kind = 'comment' AND h.author = 'human'
            AND h.text IS NOT NULL AND h.at >= q.at AND json_extract(h.data, '$.choice.question') IS NULL))
)";

pub(super) fn answer_latency(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let events_sql = project_events();
    let samples = rows(q.tx, &format!("{events_sql}, {QUESTIONS}
        SELECT a.question, a.asked_at, a.answer, a.answered_at, a.task,
            a.next IS NULL AND t.status = 'WAIT FOR HUMAN' AND t.archived_at IS NULL
        FROM answers a JOIN tasks t ON t.id = a.task WHERE a.answered_at >= ?3 OR a.answer IS NULL"),
        (q.project, q.window.end, q.window.before), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?,
            r.get::<_, Option<String>>(2)?, r.get::<_, Option<i64>>(3)?, r.get::<_, String>(4)?, r.get::<_, bool>(5)?)))?;
    let mut out = Measurements::default();
    let mut minutes: [Vec<f64>; 2] = Default::default();
    let mut answered = vec![];
    let mut pending = vec![];
    for (question, asked, answer, at, task, waiting) in samples {
        if let Some((answer, at)) = answer.zip(at) {
            minutes[usize::from(q.window.current(at))].push((at - asked) as f64 / 60.0);
            answered.extend([events(question), events(answer)]);
        } else if waiting {
            pending.extend([events(question), Evidence::Tasks(task)]);
        }
    }
    out.readings.push(Reading::new("project", q.project, "pending_answers", (pending.len() / 2) as f64, pending, Source::Present)
        .fixed(Threshold::Absolute(2.0)));
    if let [Some(old), Some(now)] = minutes.each_mut().map(|samples| percentile(samples, 50)) {
        out.readings.push(Reading::new("project", q.project, "median_answer_minutes", now, answered,
            Source::Unsized("A median has no additive count, and a question's wait is not WAIT FOR HUMAN task-days; pending answers rank as a present state.")).history(vec![old]));
    } else {
        out.complete_metrics.push("pending_answers");
        out.unavailable.push("Answer latency needs recorded answers in both windows.");
    }
    Ok(out)
}

// Legacy priority edits carry templated text, so require the entire generated sentence.
pub(super) const PRIORITY_EDIT: &str = "e.kind = 'edit' AND substr(e.text, -2) GLOB 'P[0-3]'
    AND e.text = (SELECT name FROM users WHERE id = e.author) || ' set severity ' || substr(e.text, -2)";

pub(super) fn operations(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let done = done(q)?;
    let mut completed = [0; 2];
    for row in &done {
        completed[usize::from(q.window.current(row.at))] += 1;
    }
    if completed.contains(&0) {
        return Ok(Measurements::missing("Operations per DONE task needs completions in both windows."));
    }
    let actions = owner_actions(q.tx, q.project, q.window.before, q.window.end)?;
    let mut counts = [0; 2];
    let mut strategic = [0; 2];
    let mut strategic_evidence = Vec::new();
    let mut evidence: Vec<_> = done.into_iter().map(|r| events(r.event)).collect();
    for (kind, id, at) in actions {
        let period = usize::from(q.window.current(at));
        if kind.is_operation() {
            counts[period] += 1;
            evidence.push(id);
        } else if kind.is_strategic() {
            strategic[period] += 1;
            strategic_evidence.push(id);
        }
    }
    let [old, now] = std::array::from_fn(|i| f64::from(counts[i]) / f64::from(completed[i]));
    let mut out = one(Reading::new("project", q.project, "operations_per_done", now, evidence, Source::ExcessOperations).history(vec![old]));
    let [earlier, recent] = strategic;
    out.readings.push(Reading::new("project", q.project, "recorded_strategic_acts", f64::from(recent),
        strategic_evidence, Source::Unsized(GUARD)).history(vec![f64::from(earlier)]).comparison(Comparison::Below));
    out.complete_metrics.extend(["operations_per_done", "recorded_strategic_acts"]);
    Ok(out)
}

fn owner_starts(tx: &rusqlite::Transaction, project: &str, from: i64, until: i64) -> Result<Vec<(Evidence, i64)>, String> {
    rows(tx, &format!("SELECT r.id, min(i.at) / 1000 FROM runs r JOIN inputs i ON i.run = r.id AND {LAUNCH}
        WHERE r.project = ?1 AND r.started_by = 'human'
        GROUP BY r.id HAVING min(i.at) BETWEEN ?2 * 1000 AND ?3 * 1000 + 999"),
        (project, from, until), |r| Ok((Evidence::Runs(r.get(0)?), r.get(1)?)))
}

#[derive(Clone, Copy)]
pub(super) enum OwnerAction {
    OperationalAnswer, StrategicAnswer, DesignSignoff, UnknownAnswer, Run, Move, Edit, Priority, PlanWork,
}

impl OwnerAction {
    pub fn is_operation(self) -> bool {
        matches!(self, Self::OperationalAnswer | Self::Run | Self::Move | Self::Edit)
    }

    fn is_strategic(self) -> bool {
        matches!(self, Self::StrategicAnswer | Self::DesignSignoff | Self::Priority | Self::PlanWork)
    }
}

pub(super) fn owner_actions(tx: &rusqlite::Transaction, project: &str, from: i64, until: i64) -> Result<Vec<(OwnerAction, Evidence, i64)>, String> {
    use OwnerAction as A;
    let events_sql = project_events();
    let mut out = rows(tx, &format!("{events_sql}, {QUESTIONS}
        SELECT a.answer, a.answered_at, CASE
            WHEN a.kind = 'operational' THEN 'operational'
            WHEN a.kind = 'strategic' AND a.chosen = 'approve' AND EXISTS (
                SELECT 1 FROM supervisor_design_gates g WHERE g.project = ?1
                  AND g.blocker = a.task AND g.owner_question = a.question) THEN 'signoff'
            WHEN a.kind = 'strategic' THEN 'strategic'
            ELSE 'unknown' END
        FROM answers a WHERE a.answered_at >= ?3
        UNION ALL
        SELECT e.id, e.at, CASE WHEN ({PRIORITY_EDIT}) THEN 'priority' ELSE e.kind END
        FROM project_events e JOIN tasks t ON t.id = e.task
        WHERE e.author = 'human' AND e.at >= ?3 AND (e.kind IN ('move', 'edit')
            OR (e.kind = 'made' AND t.type IN ('Feature', 'Story')))"),
        (project, until, from), |r| {
            let kind = match r.get::<_, String>(2)?.as_str() {
                "operational" => A::OperationalAnswer,
                "strategic" => A::StrategicAnswer,
                "signoff" => A::DesignSignoff,
                "unknown" => A::UnknownAnswer,
                "move" => A::Move,
                "edit" => A::Edit,
                "priority" => A::Priority,
                _ => A::PlanWork,
            };
            Ok((kind, events(r.get(0)?), r.get(1)?))
        })?;
    out.extend(owner_starts(tx, project, from, until)?.into_iter().map(|(id, at)| (A::Run, id, at)));
    Ok(out)
}

pub(super) fn churn(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let events_sql = project_events();
    let edits = rows(q.tx, &format!("{events_sql} SELECT task, id, at, substr(text, -2) FROM project_events
        AS e WHERE e.author = 'human' AND ({PRIORITY_EDIT}) AND e.at >= ?3 ORDER BY e.seq"),
        (q.project, q.window.end, q.window.before), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?)))?;
    let mut directions = BTreeMap::<String, (String, i8, String)>::new();
    let mut changes: [Vec<Evidence>; 2] = Default::default();
    for (task, id, at, severity) in edits {
        let direction = directions.get(&task).map_or(0, |(previous, old, event)| {
            let direction = match severity.cmp(previous) {
                std::cmp::Ordering::Greater => 1,
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
            };
            if direction != 0 && direction == -*old {
                changes[usize::from(q.window.current(at))].extend([events(event.clone()), events(id.clone())]);
            }
            direction
        });
        directions.insert(task, (severity, direction, id));
    }
    let [old, recent] = changes;
    let value = (recent.len() / 2) as f64;
    let baseline = (old.len() / 2) as f64;
    let mut out = one(Reading::new("project", q.project, "priority_reversals", value, recent.into_iter().chain(old).collect(),
        Source::Count(Unit::StrategicActs))
        .history(vec![baseline]));
    let starts = owner_starts(q.tx, q.project, q.window.before, q.window.end)?;
    let stops = rows(q.tx, &format!("{events_sql} SELECT id, at FROM project_events
        WHERE kind = 'why' AND author = 'dispatcher' AND text = ?4 AND at >= ?3"),
        (q.project, q.window.end, q.window.before, GAVE_UP), |r| Ok((events(r.get(0)?), r.get::<_, i64>(1)?)))?;
    // An owner's start is one of the owner's Run operations; a stop ends one Dispatcher launch.
    for (metric, rows, unit) in [("owner_starts", starts, Unit::Operations), ("owner_stops", stops, Unit::RunEvents)] {
        let prior = rows.iter().filter(|(_, at)| !q.window.current(*at)).count();
        let value = (rows.len() - prior) as f64;
        out.readings.push(Reading::new("project", q.project, metric, value, rows.into_iter().map(|(id, _)| id).collect(), Source::Count(unit)).history(vec![prior as f64]));
    }
    Ok(out)
}

pub(super) fn guard_broken(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let events_sql = project_events();
    let done = done(q)?;
    let loads = load_windows(q)?;
    let mut out = Measurements::default();
    let Some([old, now]) = feature_rates(q, loads, &done) else {
        out.unavailable.push("The store guard needs measured owner time in both windows.");
        return Ok(out);
    };
    let Threshold::Change { by } = q.detector.threshold else { unreachable!("validated change detector"); };
    let mut evidence = owner_evidence(q)?;
    evidence.extend(done.iter().map(|r| events(r.event.clone())));
    out.complete_metrics.extend(["lower_load_with_fewer_features", "lower_concurrency_with_fewer_features", "less_off_hours_with_fewer_features"]);
    if now <= old + by {
        out.complete_metrics.push("more_features_with_more_failures");
    }
    if now < old - by {
        for (metric, before, after) in [("lower_load_with_fewer_features", loads[0].switches, loads[1].switches),
            ("lower_concurrency_with_fewer_features", loads[0].peak, loads[1].peak),
            ("less_off_hours_with_fewer_features", loads[0].off, loads[1].off)] {
            out.readings.push(Reading::new("project", q.project, metric, after, evidence.clone(), Source::Unsized(GUARD)).history(vec![before]).comparison(Comparison::Below));
        }
    }
    if now > old + by {
        let failed = rows(q.tx, &format!("{events_sql} SELECT id, at FROM project_events
            WHERE kind = 'move' AND json_extract(data, '$.to') = 'FAILED' AND at >= ?3"),
            (q.project, q.window.end, q.window.before), |r| Ok((events(r.get(0)?), r.get::<_, i64>(1)?)))?;
        let mut totals = [0.0; 2];
        let mut failures = [0.0; 2];
        for r in done { totals[usize::from(q.window.current(r.at))] += 1.0; }
        for (id, at) in failed {
            let i = usize::from(q.window.current(at));
            totals[i] += 1.0;
            failures[i] += 1.0;
            evidence.push(id);
        }
        if totals.iter().all(|n| *n > 0.0) {
            out.complete_metrics.push("more_features_with_more_failures");
            out.readings.push(Reading::new("project", q.project, "more_features_with_more_failures",
                failures[1] / totals[1], evidence, Source::Unsized(GUARD)).history(vec![failures[0] / totals[0]]));
        }
    }
    Ok(out)
}

const STAMP_EVIDENCE: usize = 20;
const STAMP: &str = "Choosing the recommended option is a habit of answering, in no weekly project total; a standing rule's effect is measured by AC-240.";

/// `owner.rubber_stamp`: per question kind, how many of the owner's latest choices in a row took the recommended option.
pub(super) fn rubber_stamp(q: &Query<'_, '_>) -> Result<Measurements, String> {
    // A reply in words to a question with options takes no option, so it ends the streak too.
    let answers = rows(q.tx, &format!("{}, {QUESTIONS} SELECT kind, answer, answered_at, coalesce(chosen = recommended, 0)
        FROM answers WHERE kind IS NOT NULL AND answer IS NOT NULL ORDER BY answer_seq", project_events()),
        (q.project, q.window.end), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, bool>(3)?)))?;
    if answers.is_empty() { return Ok(Measurements::missing("No answered question with options is retained.")); }
    // Per kind: the streak's length, its latest answers as evidence, and when the kind was last answered.
    let mut streaks = BTreeMap::<String, (usize, Vec<Evidence>, i64)>::new();
    for (kind, event, at, recommended) in answers {
        let (count, evidence, last) = streaks.entry(kind).or_default();
        *last = at;
        if recommended {
            *count += 1;
            evidence.push(events(event));
            if evidence.len() > STAMP_EVIDENCE { evidence.remove(0); }
        } else {
            (*count, *evidence) = (0, Vec::new());
        }
    }
    // A kind not answered in the window has no current habit to report.
    Ok(Measurements {
        readings: streaks.into_iter().filter(|(_, (_, _, last))| q.window.current(*last)).map(|(kind, (count, evidence, _))|
            Reading::new("question_kind", kind, "recommended_streak", count as f64, evidence, Source::Unsized(STAMP))
                .comparison(Comparison::AtLeast)).collect(),
        complete_metrics: vec!["recommended_streak"],
        ..Measurements::default()
    })
}
