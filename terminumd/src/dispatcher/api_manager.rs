// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-179: only an explicit manager reopen can start a DONE task again.

use rusqlite::Transaction;

use super::Press;
use crate::tasks::{closeout, ops};

impl Press {
    /// Read the current status in the transaction preparing or admitting the press, before any change.
    pub(in crate::dispatcher) fn manager_allows(&self, tx: &Transaction, id: &str) -> Result<(), String> {
        if self.manager.is_none() || self.manager_reopen { return Ok(()) }
        let task = ops::get(tx, id)?;
        if task.status != "DONE" { return Ok(()) }
        let evidence = completion(tx, &task.key)?.map(|(line, at)| format!(" ({line}; {at})")).unwrap_or_default();
        Err(format!("{} is DONE{evidence}. To run it again on purpose, call run_task with reopen \"yes\".", task.id))
    }
}

/// The newest landing or result line, skipping later unrelated comments; return only that line.
fn completion(tx: &Transaction, key: &str) -> Result<Option<(String, String)>, String> {
    let mut statement = tx.prepare_cached("SELECT coalesce(text, ''), strftime('%H:%M', at, 'unixepoch', 'localtime'),
        coalesce(json_extract(data, '$.left') = 'landed', 0) FROM events
        WHERE task = ?1 AND kind = 'comment' ORDER BY at DESC, rowid DESC").map_err(ops::sql)?;
    let mut rows = statement.query([key]).map_err(ops::sql)?;
    while let Some(row) = rows.next().map_err(ops::sql)? {
        let (text, at, landed): (String, String, bool) = (row.get(0).map_err(ops::sql)?, row.get(1).map_err(ops::sql)?, row.get(2).map_err(ops::sql)?);
        let line = if landed {
            text.lines().map(str::trim).find(|line| !line.is_empty()).map(str::to_string)
        } else {
            closeout::outcome(&text).filter(|line| !line.trim().is_empty()).map(|line| format!("Outcome: {}", line.trim()))
        };
        if let Some(line) = line {
            return Ok(Some((line, at)));
        }
    }
    Ok(None)
}
