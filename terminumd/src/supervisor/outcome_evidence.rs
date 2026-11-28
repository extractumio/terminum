// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Code-owned evidence for the seven kinds of Supervisor decisions (AC-246).

use rusqlite::{OptionalExtension, Transaction};
use crate::tasks::ops::sql;
use super::flow::{LAUNCH, RETURNED, archived_unrun};
use super::outcomes::{Action, Judgment, Kind, Verdict::{Right, Undecided, Wrong}};

const MISSING: Judgment = (Undecided, "The store has no conclusive evidence yet.");

pub(super) fn check(tx: &Transaction, project: &str, action: &Action, at: i64) -> Result<Judgment, String> {
    match action.kind {
        Kind::Hold => hold(tx, project, action),
        Kind::Fix => fix(tx, project, action),
        Kind::Run => run(tx, project, action),
        Kind::Alert => alert(tx, project, action),
        Kind::Cause => cause(tx, project, action),
        Kind::Card => card(tx, project, action),
        Kind::Nudge => nudge(tx, project, action, at),
    }
}

pub(super) fn hold_target(tx: &Transaction, project: &str, kind: &str, target: &str)
    -> Result<Option<String>, String>
{
    let query = match kind {
        "stop_run" => "SELECT json_array(title, body, agent, project) FROM tasks WHERE project = ?1 AND id = ?2",
        "take_out" => "SELECT json_array(t.title, t.body, i.target, e.commit_sha) FROM tasks t
            JOIN integration_entries e ON e.task = t.id JOIN project_integration i ON i.project = t.project
            WHERE t.project = ?1 AND t.id = ?2",
        "pause_integration" => "SELECT json_array(i.target, i.red_bug, r.ran_sha, r.green_at)
            FROM projects p LEFT JOIN project_integration i ON i.project = p.id
            LEFT JOIN integration_red r ON r.project = p.id WHERE p.id = ?1 AND ?2 = ''",
        "pause_dispatch" => "SELECT json_array(auto_agent, auto_default, auto_supervisor, auto_severities, auto_mode)
            FROM projects WHERE id = ?1 AND ?2 = ''",
        _ => unreachable!("store checked the hold kind"),
    };
    tx.query_row(query, (project, target), |r| r.get::<_, String>(0)).optional().map_err(sql)
        .map(|words| words.map(|text| crate::auth::crypto::sha256_hex(text.as_bytes())))
}

/// A new agent, command, or retained conversation is evidence of a different approach.
pub(super) fn changed_approach(tx: &Transaction, id: i64) -> Result<bool, String> {
    tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_holds h
        JOIN runs old ON old.id = json_extract(h.undo_data, '$.run')
        JOIN runs new ON new.id = coalesce(h.replacement_run, h.pending_run) AND new.project = h.project
        WHERE h.id = ?1 AND h.kind = 'stop_run' AND (
          (new.command, new.agent_id) IS NOT (old.command, old.agent_id)
          OR (old.agent_session IS NOT NULL AND new.agent_session IS NOT old.agent_session)))", [id],
        |r| r.get(0)).map_err(sql)
}

fn hold(tx: &Transaction, project: &str, a: &Action) -> Result<Judgment, String> {
    let row = tx.query_row("SELECT h.id, wrong.verdict IS NOT NULL,
        EXISTS (SELECT 1 FROM events e JOIN tasks t ON t.id = e.task AND t.project = h.project
          WHERE h.kind IN ('take_out','pause_integration') AND e.author = 'dispatcher'
            AND json_extract(e.data, '$.target_red') = 1 AND e.at >= h.applied_at
            AND (h.released_at IS NULL OR e.at <= h.released_at)
            AND (h.target = '' OR h.target = e.task OR json_extract(e.data, '$.target_recheck') = 1))
        FROM supervisor_holds h
        LEFT JOIN supervisor_outcomes wrong ON wrong.action = ?3 AND wrong.days = 0 AND wrong.verdict = 'wrong'
        WHERE h.project = ?1 AND h.id = ?2", (project, &a.source, a.id), |r|
        Ok((r.get::<_, i64>(0)?, r.get::<_, bool>(1)?, r.get::<_, bool>(2)?))).optional().map_err(sql)?;
    let Some((id, overridden, red)) = row else { return Ok(MISSING) };
    if overridden {
        let success: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_holds h
          LEFT JOIN tasks t ON t.id = h.target AND t.project = h.project
          LEFT JOIN integration_red r ON r.project = h.project
          WHERE h.id = ?1 AND (
            (t.status = 'DONE' AND t.updated_at >= h.released_at AND (h.kind <> 'stop_run'
              OR EXISTS (SELECT 1 FROM run_tasks m WHERE m.run = h.replacement_run AND m.task = t.id AND m.result)))
            OR (h.kind = 'pause_integration' AND r.green_at >= h.released_at)))",
            [id], |r| r.get(0)).map_err(sql)?;
        return Ok((Wrong, if success { "The unchanged subject succeeded after the owner released the hold." }
            else { "The owner released the unchanged subject." }));
    }
    Ok(if red { (Right, "The target was still red while the hold stood.") }
        else if changed_approach(tx, id)? { (Right, "The next run used a different approach.") } else { MISSING })
}

fn problem(tx: &Transaction, project: &str, fingerprint: &str, since: i64) -> Result<Option<String>, String> {
    tx.query_row("SELECT CASE
        WHEN p.state = 'resolved' AND EXISTS (SELECT 1 FROM supervisor_problem_events e
          WHERE e.problem = p.id AND e.state = 'resolved' AND e.at >= ?3) THEN 'resolved'
        WHEN p.state = 'recurred' AND EXISTS (SELECT 1 FROM supervisor_problem_signals member
          JOIN supervisor_signals s USING (fingerprint) WHERE member.problem = p.id
            AND s.last_seen >= ?3 AND s.state IN ('new','raised','held','recurred')) THEN 'recurred'
        END FROM supervisor_problems p JOIN supervisor_problem_signals y ON y.problem = p.id
        WHERE p.project = ?1 AND y.fingerprint = ?2", (project, fingerprint, since), |r| r.get(0))
        .optional().map_err(sql).map(Option::flatten)
}

fn fix(tx: &Transaction, project: &str, a: &Action) -> Result<Judgment, String> {
    let row = tx.query_row(&format!("SELECT status, {},
        (SELECT max(e.at) FROM events e WHERE e.task = t.id AND e.kind = 'move' AND json_extract(e.data, '$.to') = 'DONE')
        FROM tasks t WHERE project = ?1 AND id = ?2", archived_unrun()), (project, &a.target), |r|
        Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?, r.get::<_, Option<i64>>(2)?)))
        .optional().map_err(sql)?;
    let Some((status, unrun, done)) = row else { return Ok(MISSING) };
    if unrun { return Ok((Wrong, "The fix was archived without running.")); }
    let Some(done) = done.filter(|_| status == "DONE") else { return Ok(MISSING) };
    Ok(match problem(tx, project, &a.fingerprint, done.max(a.at))?.as_deref() {
        Some("resolved") => (Right, "The fix finished and its problem stayed resolved."),
        Some("recurred") => (Wrong, "The problem recurred after the fix finished."),
        _ => MISSING,
    })
}

fn run(tx: &Transaction, project: &str, a: &Action) -> Result<Judgment, String> {
    let row = tx.query_row(&format!("WITH launches AS (
        SELECT m.run, min(i.at / 1000) AS at FROM run_tasks m JOIN inputs i ON i.run = m.run
        WHERE m.task = ?2 AND i.at >= ?3 * 1000 AND {LAUNCH} GROUP BY m.run),
        start AS (SELECT run, at FROM launches WHERE run = ?4 OR ?4 = '' ORDER BY at, run LIMIT 1),
        episode AS (SELECT coalesce((SELECT at FROM start), ?3) AS began,
            coalesce((SELECT min(at) FROM launches WHERE at > (SELECT at FROM start)
              OR ((SELECT at FROM start) IS NULL AND run <> ?4)), 9223372036854775807) AS ended)
        SELECT (SELECT json_extract(e.data, '$.left') FROM events e WHERE e.task = t.id AND e.at >= began AND e.at < ended
              AND {RETURNED} ORDER BY e.at, e.rowid LIMIT 1),
            EXISTS (SELECT 1 FROM events e WHERE e.task = t.id AND e.at >= began AND e.at < ended
              AND e.kind = 'move' AND json_extract(e.data, '$.to') = 'FAILED'),
            EXISTS (SELECT 1 FROM events e WHERE e.task = t.id AND e.at >= began AND e.at < ended
              AND e.kind = 'why' AND e.text = ?5),
            EXISTS (SELECT 1 FROM start) AND (EXISTS (SELECT 1 FROM events e WHERE e.task = t.id AND e.kind = 'move'
                AND json_extract(e.data, '$.to') = 'DONE' AND e.at >= began AND e.at < ended)
              OR (t.status = 'DONE' AND t.updated_at >= began AND t.updated_at < ended))
        FROM tasks t, episode WHERE t.project = ?1 AND t.id = ?2"),
        (project, &a.target, a.at, &a.baseline, crate::dispatcher::runs::GAVE_UP), |r|
        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, bool>(1)?, r.get::<_, bool>(2)?, r.get::<_, bool>(3)?))).optional().map_err(sql)?;
    let Some((returned, failed, stopped, done)) = row else { return Ok(MISSING) };
    // the episode's first typed integration exit (`terminum-integration.md` AC-187) outweighs a later completion
    Ok(match (returned.as_deref(), stopped, failed, done) {
        (Some("red"), ..) => (Wrong, "Integration returned the task for a red check."),
        (Some("conflict"), ..) => (Wrong, "Integration returned the task for a conflict."),
        (Some("out"), ..) => (Wrong, "Integration gave the task back without landing it."),
        (Some(_), ..) => (Wrong, "Integration returned the task."),
        (None, true, ..) => (Wrong, "The owner stopped the run."),
        (None, false, true, _) => (Wrong, "The task failed."),
        (None, false, false, true) => (Right, "The task ran and finished without return."),
        (None, false, false, false) => MISSING,
    })
}

fn alert(tx: &Transaction, project: &str, a: &Action) -> Result<Judgment, String> {
    let (wrong, acted) = tx.query_row("SELECT
        EXISTS (SELECT 1 FROM supervisor_outcomes o WHERE o.action = ?1 AND o.days = 0 AND o.verdict = 'wrong'),
        EXISTS (SELECT 1 FROM supervisor_incident_events e JOIN supervisor_incidents i ON i.id = e.incident
          LEFT JOIN supervisor_holds h ON h.id = e.related_id
          WHERE i.project = ?2 AND i.id = ?3 AND e.at >= ?4 AND e.actor = 'human' AND e.id > CAST(?6 AS INTEGER)
          AND (e.kind = 'acknowledged' OR (e.kind IN ('hold_kept','hold_released') AND h.fingerprint = ?5)))",
        (a.id, project, &a.target, a.at, &a.fingerprint, &a.source), |r|
        Ok((r.get::<_, bool>(0)?, r.get::<_, bool>(1)?))).map_err(sql)?;
    if wrong { return Ok((Wrong, "The owner marked the alert wrong.")); }
    if acted { return Ok((Right, "The owner acted on the incident.")); }
    let held: Vec<String> = crate::tasks::ops::rows(tx, "SELECT CAST(id AS TEXT) FROM supervisor_holds
        WHERE project = ?1 AND incident = ?2 AND fingerprint = ?3 AND shadow = 0",
        (project, &a.target, &a.fingerprint), |r| r.get(0))?;
    for id in held {
        if let Some(h) = super::outcomes::find(tx, project, Kind::Hold, &id)? && hold(tx, project, &h)?.0 == Right {
            return Ok((Right, "The reported hold proved right."));
        }
    }
    Ok(MISSING)
}

fn cause(tx: &Transaction, project: &str, a: &Action) -> Result<Judgment, String> {
    let done: Option<i64> = tx.query_row("SELECT max(e.at) FROM supervisor_problem_signals y
        JOIN supervisor_problems p ON p.id = y.problem JOIN supervisor_problem_tasks l ON l.problem = p.id
        JOIN tasks t ON t.id = l.task AND t.project = p.project AND t.status = 'DONE'
        JOIN supervisor_actions f ON f.project = p.project AND f.kind = 'fix' AND f.source = t.id AND f.baseline = ?3
        JOIN events e ON e.task = t.id AND e.kind = 'move' AND json_extract(e.data, '$.to') = 'DONE'
        WHERE p.project = ?1 AND y.fingerprint = ?2 AND e.at >= ?4",
        (project, &a.fingerprint, &a.source, a.at), |r| r.get(0)).map_err(sql)?;
    let Some(done) = done else { return Ok(MISSING) };
    Ok(match problem(tx, project, &a.fingerprint, done)?.as_deref() {
        Some("resolved") => (Right, "The fix based on this cause held."),
        Some("recurred") => (Wrong, "The problem recurred with its fix in place."),
        _ => MISSING,
    })
}

fn card(tx: &Transaction, project: &str, a: &Action) -> Result<Judgment, String> {
    let row = tx.query_row("SELECT t.status, t.archived_at IS NOT NULL,
        json_extract(q.data, '$.question.recommended'),
        (SELECT json_extract(e.data, '$.choice.option') FROM events e WHERE e.task = t.id
          AND e.kind = 'comment' AND e.author = 'human' AND json_extract(e.data, '$.choice.question') = q.id),
        EXISTS (SELECT 1 FROM events e WHERE e.task = t.id AND e.rowid > q.rowid AND e.kind = 'comment'
          AND e.author = 'human' AND json_extract(e.data, '$.choice.question') IS NULL
          AND e.rowid < coalesce((SELECT min(next.rowid) FROM events next WHERE next.task = t.id
            AND next.rowid > q.rowid AND json_extract(next.data, '$.questions') = 1), 9223372036854775807)),
        (SELECT e.at FROM events e WHERE e.task = t.id AND e.kind = 'comment' AND e.author = 'human'
          AND json_extract(e.data, '$.choice.question') = q.id)
        FROM tasks t JOIN events q ON q.task = t.id AND q.id = ?3
        WHERE t.project = ?1 AND t.id = ?2", (project, &a.target, &a.source), |r|
        Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?, r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?, r.get::<_, bool>(4)?, r.get::<_, Option<i64>>(5)?))).optional().map_err(sql)?;
    let Some((status, archived, recommended, selected, free_text, answered_at)) = row else { return Ok(MISSING) };
    if selected.is_none() {
        return Ok(if !free_text && (archived || matches!(status.as_str(), "DONE" | "FAILED")) {
            (Wrong, "The decision card closed without an answer.")
        } else { MISSING });
    }
    let Some(answered) = answered_at.filter(|_| a.baseline == "problem") else { return Ok(MISSING) };
    Ok(match problem(tx, project, &a.fingerprint, answered)?.as_deref() {
        Some("resolved") => (Right, "The selected fix met the protected yardstick."),
        Some("recurred") if selected == recommended => (Wrong, "The recommended fix failed the protected yardstick."),
        _ => MISSING,
    })
}

fn nudge(tx: &Transaction, project: &str, a: &Action, at: i64) -> Result<Judgment, String> {
    let row = tx.query_row("SELECT state, last_seen,
        (SELECT evaluated_at FROM supervisor_evaluations WHERE project = s.project AND detector = s.detector)
        FROM supervisor_signals s WHERE s.project = ?1 AND fingerprint = ?2",
        (project, &a.fingerprint), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?)))
        .optional().map_err(sql)?;
    let Some((state, seen, checked)) = row else { return Ok(MISSING) };
    Ok(if state == "resolved" && checked.is_some_and(|stamp| stamp > a.at && stamp <= at) {
        (Right, "The detector cleared the signal after the comment.")
    } else if seen > a.at && matches!(state.as_str(), "new" | "raised" | "held" | "recurred") {
        (Wrong, "The detector still reported the signal after the comment.")
    } else { MISSING })
}
