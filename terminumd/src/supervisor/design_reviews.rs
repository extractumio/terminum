// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-252: one design lifecycle with bounded critic slots and owner escalation.

use rusqlite::{OptionalExtension, Transaction};
use serde_json::{Value, json};

use crate::tasks::ops::{self, rows, sql};
use super::{design_gates as gates, governor, settings};

pub(super) const PREFIX: &str = "design_";

pub(super) fn waiting(tx: &Transaction, at: i64, key: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let key = key.and_then(|k| k.strip_prefix(PREFIX));
    rows(tx, "SELECT g.project, 'design_' || g.task FROM supervisor_design_gates g
        JOIN project_supervisor p USING (project) JOIN tasks t ON t.id = g.task AND t.project = g.project
        WHERE p.enabled AND g.status = 'waiting' AND NOT g.existing_review AND g.deadline > ?1
            AND t.archived_at IS NULL AND t.status NOT IN ('DONE','FAILED') AND (?2 IS NULL OR g.task = ?2)
            AND (g.tier = 2 OR g.brief IS NOT NULL)
            AND (g.tier = 3 OR NOT EXISTS (SELECT 1 FROM supervisor_design_gates higher
                WHERE higher.project = g.project AND higher.tier = 3 AND higher.status = 'waiting'
                    AND NOT higher.existing_review AND higher.brief IS NOT NULL AND higher.deadline > ?1))
            AND NOT EXISTS (SELECT 1 FROM supervisor_rounds q
                WHERE q.fingerprint = 'design_' || g.task AND q.ended_at IS NULL)
            AND ((g.architecture_round IS NULL AND NOT EXISTS (
                SELECT 1 FROM supervisor_design_records v WHERE v.task = g.task AND v.role = 'architecture' AND v.tier >= g.tier))
              OR (g.tier = 3 AND g.independent_round IS NULL AND EXISTS (
                SELECT 1 FROM supervisor_design_records v WHERE v.task = g.task AND v.role = 'architecture' AND v.tier >= g.tier AND v.verdict = 'approved')
              AND NOT EXISTS (SELECT 1 FROM supervisor_design_records v WHERE v.task = g.task AND v.role = 'independent')))
        ORDER BY g.tier DESC, g.opened, g.task LIMIT 32",
        (at, key), |r| Ok((r.get(0)?, r.get(1)?)))
}

fn slug(tx: &Transaction, gate: &gates::Gate) -> Result<String, String> {
    tx.query_row("SELECT slug FROM tasks WHERE id = ?1 AND project = ?2",
        (&gate.key, &gate.project), |r| r.get(0)).map_err(sql)
}

pub(super) fn candidate(tx: &Transaction, project: &str, key: &str, at: i64) -> Result<(String, &'static str), String> {
    let task = key.strip_prefix(PREFIX).ok_or("Not a design review.")?;
    let gate = gates::get(tx, task)?.filter(|g| g.project == project).ok_or("No such task design.")?;
    if !super::register::advisor_enabled(tx, project)? || waiting(tx, at, Some(key))?.is_empty() {
        return Err("The design is no longer ready for review.".into());
    }
    let saved = settings::read(tx, project)?.settings;
    if !saved.charter.surfaces.enabled() { return Err("The project has no active design gates.".into()); }
    let (agent, role) = if gates::approved(tx, task, "architecture", gate.tier)? {
        (saved.tiers.reflection_agent(tx, project)?, "independent")
    } else {
        let strong = saved.tiers.investigation(3).1.ok_or("Choose a strong tier agent to review designs.")?;
        (strong.to_owned(), "architecture")
    };
    let assignee = ops::get(tx, &slug(tx, &gate)?)?;
    let author = ops::pool_user_id(&agent);
    if assignee.owner == author {
        return Err("Choose a design reviewer other than the task's implementer.".into());
    }
    Ok((agent, role))
}

pub(super) fn case(tx: &Transaction, key: &str, role: &str) -> Result<(Value, i64), String> {
    let task = key.strip_prefix(PREFIX).ok_or("Not a design review.")?;
    let gate = gates::get(tx, task)?.ok_or("No such task design.")?;
    let id = slug(tx, &gate)?;
    let title = ops::get(tx, &id)?.title;
    let data = |s: &str| crate::sent::redact(s).chars().take(2000).collect::<String>();
    let architecture: Option<String> = tx.query_row("SELECT record FROM supervisor_design_records
        WHERE task = ?1 AND role = 'architecture'", [&gate.key], |r| r.get(0)).optional().map_err(sql)?;
    Ok((json!({"kind":"design_review","task":id,"tier":gate.tier,"role":role,
        "title_data":data(&title),"task_data":data(&ops::text_of(tx, &id)?),
        "evidence_data":data(&gate.evidence),
        "brief_data":gate.brief.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
        "architecture_record_data":architecture.and_then(|s| serde_json::from_str::<Value>(&s).ok())}), gate.deadline))
}

pub(super) fn started(tx: &Transaction, key: &str, role: &str, case: &str, at: i64) -> Result<(), String> {
    let task = key.strip_prefix(PREFIX).ok_or("Not a design review.")?;
    let gate = gates::get(tx, task)?.ok_or("No such task design.")?;
    let changed = match role {
        "architecture" => tx.execute("UPDATE supervisor_design_gates SET architecture_round = ?2
            WHERE task = ?1 AND architecture_round IS NULL", (task, case)),
        _ => tx.execute("UPDATE supervisor_design_gates SET independent_round = ?2
            WHERE task = ?1 AND independent_round IS NULL", (task, case)),
    }.map_err(sql)?;
    if changed != 1 { return Err("This critic slot was already started.".into()); }
    gates::block(tx, &gate, &ops::get(tx, &slug(tx, &gate)?)?, at)
}

pub(super) fn submitted(tx: &Transaction, project: &str, run: &str, input: Value, at: i64) -> Result<String, String> {
    if !super::register::advisor_enabled(tx, project)? { return Err("The Supervisor can no longer review designs.".into()); }
    let (task_key, role, author, name): (String, String, String, String) = tx.query_row("SELECT g.task,
        CASE WHEN g.architecture_round = q.id THEN 'architecture' ELSE 'independent' END, r.agent_id, r.agent_name
        FROM supervisor_rounds q JOIN runs r ON r.id = q.run JOIN supervisor_design_gates g
        ON q.id IN (g.architecture_round, g.independent_round) AND g.project = q.project
        WHERE q.run = ?1 AND q.project = ?2 AND q.deadline > ?3 AND q.finding IS NULL
            AND q.failure IS NULL AND q.ended_at IS NULL AND g.status = 'waiting'",
        (run, project, at), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional().map_err(sql)?
        .ok_or("This design review has ended.")?;
    let id = crate::tasks::tools::arg(&input, "id")?;
    let task = ops::get(tx, id)?;
    if task.key != task_key || task.project != project { return Err("Review the task named in this case.".into()); }
    let draft: gates::Record = serde_json::from_value(input).map_err(|_| "Supply the role, verdict, compatibility and rejected advice fields.")?;
    if draft.role != role { return Err("Use the critic role assigned in this case.".into()); }
    gates::record(tx, &task, &ops::pool_user(tx, &author, &name)?, draft, at)?;
    tx.execute("UPDATE supervisor_rounds SET finding = '{\"design_review\":true}', found_at = ?2 WHERE run = ?1",
        (run, at)).map_err(sql)?;
    Ok(task.id)
}

const DUE: &str = "WITH gates AS (
    SELECT g.task, t.slug, CASE
        WHEN t.status IN ('DONE','FAILED') OR t.archived_at IS NOT NULL OR t.project <> g.project THEN 'retire'
        WHEN NOT coalesce(p.enabled, 0) OR json_extract(s.pack, '$.level') = 'observer'
            OR (coalesce(json_array_length(s.pack, '$.charter.surfaces.t2'), 0) = 0
            AND coalesce(json_array_length(s.pack, '$.charter.surfaces.t3'), 0) = 0) THEN
            CASE WHEN EXISTS (SELECT 1 FROM task_links l WHERE l.task = g.task
                AND l.other = g.blocker AND l.kind = 'blocked_by') THEN 'unblock' END
        WHEN g.tier = 2 AND a.verdict = 'approved' THEN 'complete'
        WHEN g.tier = 3 AND g.brief IS NOT NULL AND a.verdict = 'approved'
            AND b.verdict = 'approved' AND a.author <> b.author THEN
            CASE WHEN g.owner_question IS NULL THEN 'owner'
                WHEN EXISTS (SELECT 1 FROM events e WHERE e.task = g.blocker AND e.author = 'human'
                    AND e.kind = 'comment' AND json_extract(e.data, '$.choice.question') = g.owner_question
                    AND json_extract(e.data, '$.choice.option') = 'approve') THEN 'complete' END
        WHEN g.status = 'waiting' AND (a.verdict = 'changes_requested' OR b.verdict = 'changes_requested')
            THEN 'changes'
        WHEN g.status = 'waiting' AND ((g.deadline > 0 AND g.deadline <= ?1)
            OR q.failure IS NOT NULL OR w.failure IS NOT NULL) THEN 'late'
        END AS action, g.tier, g.opened
    FROM supervisor_design_gates g JOIN tasks t ON t.id = g.task
    LEFT JOIN project_supervisor p ON p.project = g.project
    LEFT JOIN supervisor_settings s ON s.project = g.project
    LEFT JOIN supervisor_design_records a ON a.task = g.task AND a.role = 'architecture' AND a.tier >= g.tier
    LEFT JOIN supervisor_design_records b ON b.task = g.task AND b.role = 'independent' AND b.tier >= g.tier
    LEFT JOIN supervisor_rounds q ON q.id = g.architecture_round
    LEFT JOIN supervisor_rounds w ON w.id = g.independent_round
    WHERE g.status <> 'closed')
    SELECT task, slug, action FROM gates WHERE action IS NOT NULL ORDER BY tier DESC, opened LIMIT 128";

pub(crate) fn due(tx: &Transaction, at: i64) -> Result<bool, String> {
    tx.prepare_cached(DUE).map_err(sql)?.exists([at]).map_err(sql)
}

pub(crate) fn advance(tx: &Transaction, at: i64) -> Result<Vec<String>, String> {
    let keys = rows(tx, DUE, [at], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let mut changed = Vec::new();
    for (key, id, action) in keys {
        let gate = gates::get(tx, &key)?.ok_or("Missing design gate.")?;
        let task = ops::get(tx, &id)?;
        match action.as_str() {
            "retire" | "complete" => gates::close(tx, &gate, &task, at)?,
            "unblock" => gates::unblock(tx, &task, &gate.project, gate.blocker.as_deref(), at)?,
            "owner" => gates::escalate(tx, &gate, &task, "Both critics approved the recorded design.", at)?,
            "changes" => gates::escalate(tx, &gate, &task, "A critic requested changes. Read the verdict on the task.", at)?,
            "late" => gates::escalate(tx, &gate, &task, "The design review could not finish within its 20-minute window.", at)?,
            _ => return Err("Unknown design review transition.".into()),
        }
        changed.push(id);
    }
    Ok(changed)
}

pub(super) fn admission_failed(tx: &Transaction, key: &str, why: &str, at: i64) -> Result<(), String> {
    if why != governor::JUDGMENT_NOTICE { return Ok(()); }
    let Some(task_key) = key.strip_prefix(PREFIX) else { return Ok(()) };
    let Some(gate) = gates::get(tx, task_key)? else { return Ok(()) };
    gates::escalate(tx, &gate, &ops::get(tx, &slug(tx, &gate)?)?, why, at)
}

pub(crate) fn is_round(tx: &Transaction, run: &str) -> Result<bool, String> {
    tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_rounds WHERE run = ?1 AND substr(fingerprint, 1, 7) = 'design_')",
        [run], |r| r.get(0)).map_err(sql)
}
