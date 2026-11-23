// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-248: closed fields from the project ledger, in observation order.

use rusqlite::Transaction;
use serde_json::{Value, json};

use crate::tasks::ops::rows;
use super::signals;

pub(super) fn build(tx: &Transaction, project: &str, kind: &str, subject: &str, signal: Option<&str>, at: i64)
    -> Result<Vec<Value>, String>
{
    let mut events = rows(tx, "WITH scope AS (
        SELECT id, fingerprint, target, detector, response, kind, source
        FROM supervisor_actions WHERE project = ?1 AND ?2 = 'action' AND id = CAST(?3 AS INTEGER)
    ), incident_scope AS (
        SELECT CAST(?3 AS INTEGER) AS id WHERE ?2 = 'incident'
        UNION SELECT CAST(target AS INTEGER) FROM scope WHERE kind = 'alert'
        UNION SELECT h.incident FROM supervisor_holds h JOIN scope a ON a.kind = 'hold' AND a.source = CAST(h.id AS TEXT)
          WHERE h.project = ?1
    ), related_signals AS (
        SELECT ?4 AS fingerprint
        UNION SELECT e.fingerprint FROM supervisor_incident_events e JOIN incident_scope i ON i.id = e.incident
        UNION SELECT y.fingerprint FROM supervisor_problem_signals y JOIN supervisor_problems p ON p.id = y.problem
          WHERE ?2 = 'problem' AND p.project = ?1 AND p.id = CAST(?3 AS INTEGER)
        UNION SELECT s.fingerprint FROM supervisor_rounds r JOIN supervisor_signals s ON s.fingerprint = r.fingerprint
          WHERE ?2 = 'run' AND r.project = ?1 AND r.run = ?3
    ), actions AS (
        SELECT a.id, a.at, a.detector, a.response, a.kind, a.target FROM supervisor_actions a
        WHERE a.project = ?1 AND (
            a.id IN (SELECT id FROM scope) OR a.fingerprint IN (SELECT fingerprint FROM related_signals)
            OR (a.detector, a.response) IN (SELECT detector, response FROM scope)
            OR (?2 = 'task' AND a.target = substr(?3, 1, instr(?3, '@') - 1))
            OR (?2 = 'rule' AND a.detector || '/' || a.response = substr(?3, 1, instr(?3, '@') - 1)))
        ORDER BY (a.id IN (SELECT id FROM scope)) DESC, a.at DESC, a.id DESC LIMIT 40
    ), timeline AS (
        SELECT first_seen AS observed, 'signal' AS kind, fingerprint AS ref,
            json_object('detector', detector, 'metric', metric, 'state', state, 'last_seen', last_seen,
              'measure', json(measure), 'evidence', json(evidence)) AS data
        FROM supervisor_signals WHERE project = ?1 AND fingerprint IN (SELECT fingerprint FROM related_signals)
        UNION ALL SELECT a.at, 'action', CAST(a.id AS TEXT),
            json_object('detector', a.detector, 'response', a.response, 'action', a.kind, 'target', a.target)
        FROM actions a
        UNION ALL SELECT o.at, 'outcome', CAST(o.action AS TEXT),
            json_object('days', o.days, 'verdict', o.verdict, 'reason', o.evidence)
        FROM supervisor_outcomes o JOIN actions a ON a.id = o.action
        UNION ALL SELECT b.opened_at, 'breaker', CAST(b.id AS TEXT),
            json_object('detector', b.detector, 'response', b.response, 'action', b.opened_by)
        FROM supervisor_breakers b JOIN actions a ON b.opened_by = a.id
        UNION ALL SELECT h.released_at, 'hold_released', CAST(h.id AS TEXT), json_object('actor', h.released_by)
        FROM supervisor_holds h JOIN supervisor_actions a ON a.kind = 'hold' AND a.source = CAST(h.id AS TEXT)
        JOIN actions selected ON selected.id = a.id WHERE h.project = ?1 AND h.released_at IS NOT NULL
        UNION ALL SELECT e.at, 'owner_activity', e.id, json_object('actor', e.author)
        FROM supervisor_human_actions e JOIN tasks t ON t.id = e.task
        WHERE t.project = ?1 AND ((?2 = 'task' AND t.id = substr(?3, 1, instr(?3, '@') - 1)) OR t.id IN (SELECT target FROM actions))
        UNION ALL SELECT e.at, 'incident_' || e.kind, CAST(e.id AS TEXT),
            json_object('incident', e.incident, 'fingerprint', e.fingerprint, 'detector', e.detector,
                'actor', e.actor, 'level', e.level, 'value', e.value, 'threshold', e.threshold, 'related_id', e.related_id)
        FROM supervisor_incident_events e JOIN incident_scope i ON i.id = e.incident
        JOIN supervisor_incidents j ON j.id = e.incident AND j.project = ?1
        UNION ALL SELECT coalesce((SELECT min(i.at) / 1000 FROM inputs i
                WHERE i.run = q.run AND i.component = 'dispatcher.start' AND i.kind = 'launch'), ?5), 'source_round', q.id,
            json_object('case', q.id, 'ended_at', q.ended_at, 'deadline', q.deadline, 'token_cap', q.token_cap, 'failed', q.failure IS NOT NULL)
        FROM supervisor_rounds q WHERE ?2 = 'run' AND q.project = ?1 AND q.run = ?3
        UNION ALL SELECT coalesce((SELECT min(i.at) / 1000 FROM inputs i
                WHERE i.run = q.run AND i.component = 'dispatcher.start' AND i.kind = 'launch'), ?5), 'source_weekly_review', q.run,
            json_object('token_cap', q.cap, 'overrun', q.overrun)
        FROM supervisor_review_runs q WHERE ?2 = 'run' AND q.project = ?1 AND q.run = ?3
        UNION ALL SELECT a.at, 'problem_state', CAST(a.id AS TEXT), json_object('state', a.state)
        FROM supervisor_problem_events a JOIN supervisor_problems p ON p.id = a.problem
        WHERE ?2 = 'problem' AND p.project = ?1 AND p.id = CAST(?3 AS INTEGER)
    )
    SELECT observed, kind, ref, data FROM timeline WHERE observed <= ?5
    ORDER BY (kind IN ('source_round','source_weekly_review') OR
        (kind IN ('action','outcome') AND ref = (SELECT CAST(id AS TEXT) FROM scope))) DESC,
        observed DESC, kind DESC, ref DESC LIMIT 127", (project, kind, subject, signal, at),
        |r| {
            let event_type: String = r.get(1)?;
            let mut data: Value = signals::decode(r, 3)?;
            if let Some(evidence) = data.get_mut("evidence").and_then(Value::as_array_mut) { evidence.truncate(16); }
            Ok(json!({"at":r.get::<_, i64>(0)?,"kind":event_type,"ref":r.get::<_, String>(2)?,"data":data}))
        })?;
    events.reverse();
    Ok(events)
}
