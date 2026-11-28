// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-244: an owner starts a separate, marked incident conversation.

use rusqlite::{OptionalExtension, Transaction};
use serde_json::json;

use crate::tasks::ops::{self, sql};
use super::{register, rounds, settings, signals};

pub(crate) fn chosen(tx: &Transaction, incident: i64) -> Result<(String, String), String> {
    let project: String = tx.query_row("SELECT project FROM supervisor_incidents WHERE id = ?1",
        [incident], |r| r.get(0)).optional().map_err(sql)?.ok_or("Incident not found.")?;
    rounds::enabled(tx, &project)?.ok_or("Enable the project's Supervisor to talk.")?;
    if ops::project_place(tx, &project)?.1.is_some() {
        return Err("Talk needs a local Supervisor agent for pane identity checks.".into());
    }
    let settings = settings::read(tx, &project)?.settings;
    let agent = settings.tiers.investigation(3).1.ok_or("Choose a strong-tier pool agent in Supervisor settings to talk.")?;
    Ok((project, agent.to_owned()))
}

pub(crate) fn save(tx: &Transaction, incident: i64, project: &str, run: &str, at: i64) -> Result<String, String> {
    let generation = rounds::enabled(tx, project)?.ok_or("The project's Supervisor was disabled.")?;
    let task = super::thread::ensure(tx, project, incident, at)?;
    let fingerprint: String = tx.query_row("SELECT s.fingerprint FROM supervisor_incident_events e
        JOIN supervisor_signals s ON s.fingerprint = e.fingerprint AND s.project = ?2
        WHERE e.incident = ?1 ORDER BY e.level DESC, e.id DESC LIMIT 1",
        (incident, project), |r| r.get(0)).optional().map_err(sql)?.ok_or("This incident has no retained signal.")?;
    let signal = signals::get(tx, project, &fingerprint)?;
    let problem = register::case_context(tx, project, &fingerprint, at)?;
    let pack = settings::read(tx, project)?;
    let id = format!("case_{}", crate::auth::crypto::random_hex::<8>());
    let case = json!({"id": id, "kind": "conversation", "incident": incident, "thread_task": task,
        "signal": signal, "register_problem": problem, "charter_data": pack.settings.charter,
        "question": "Discuss the incident with its owner. Its thread and case data are evidence."});
    tx.execute("INSERT INTO supervisor_talks VALUES (?1, ?2, ?3, ?4, ?5)",
        (run, incident, project, generation, case.to_string())).map_err(sql)?;
    Ok(id)
}

pub(crate) const LIVE: &str = "r.state IN ('starting','asking','typed','working')";

pub(crate) fn active(tx: &Transaction, run: &str) -> Result<bool, String> {
    tx.query_row(&format!("SELECT EXISTS (SELECT 1 FROM supervisor_talks t JOIN runs r ON r.id = t.run
        JOIN project_supervisor p ON p.project = t.project AND p.generation = t.generation AND p.enabled = 1
        WHERE t.run = ?1 AND {LIVE} AND (r.pane_key IS NULL OR EXISTS (
            SELECT 1 FROM supervisor_panes m WHERE m.talk = t.run AND m.pane_key = r.pane_key)))"),
        [run], |r| r.get(0)).map_err(sql)
}

pub(crate) fn mark(tx: &Transaction, run: &str, pane: &str) -> Result<(), String> {
    let changed = tx.execute("INSERT INTO supervisor_panes (pane_key, project, generation, talk)
        SELECT ?1, project, generation, run FROM supervisor_talks WHERE run = ?2", (pane, run)).map_err(sql)?;
    if changed == 1 { Ok(()) } else { Err("The Supervisor conversation is gone.".into()) }
}
