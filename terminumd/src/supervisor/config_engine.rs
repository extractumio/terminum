// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-253: a typed engine request carries no source project history.

use rusqlite::Transaction;

use crate::tasks::ops::{self, NewTask, sql};

use super::{Rule, config_changes::EngineRequirement};

pub(super) fn create(tx: &Transaction, board: &str, detector: Rule, requirement: EngineRequirement, change: i64, at: i64) -> Result<String, String> {
    let kind = match requirement {
        EngineRequirement::Query => "named query",
        EngineRequirement::Tool => "tool",
        EngineRequirement::ProbeKind => "probe kind",
        EngineRequirement::RecordedField => "recorded field",
    };
    let author = crate::roles::supervisor::author(tx, board)?;
    let text = format!("Review whether detector {} needs a new {kind}. Specify its contract and add tests before enabling it.", detector.id());
    let task = ops::create(tx, &author, NewTask { title: format!("Feat: Review a Supervisor {kind}"),
        text, project: Some(board.into()), ..Default::default() }, at)?;
    tx.execute("INSERT INTO supervisor_task_marks (task, project, estimate, outside) VALUES (?1, ?2, 0, 1)",
        (&task.key, board)).map_err(sql)?;
    tx.execute("UPDATE supervisor_config_changes SET task = ?2, decision = 'queued' WHERE id = ?1",
        (change, &task.key)).map_err(sql)?;
    Ok(task.id)
}
