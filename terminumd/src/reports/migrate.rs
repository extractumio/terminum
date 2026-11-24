// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Waiting reports move to the Inbox (terminum-reports.md AC-274): a task that waits in WAIT FOR HUMAN with its report
//! (its newest move marked `report`) becomes a report, unread, with that result as its body, and the task is DONE with
//! a comment linking it. Run at every start in the store's opening transaction; one moved never waits again, so it is
//! done once.

use rusqlite::Transaction;

use super::record::{self, role, New};
use super::sources::{summary_of, titled};
use crate::tasks::ops::{self, rows, sql};
use crate::tasks::store::DISPATCHER;

/// A task waiting with its report, by its newest move's mark: its key, the move's author and time.
const WAITING: &str = "SELECT t.id, m.author, m.at FROM tasks t JOIN events m ON m.rowid = (SELECT e.rowid FROM events e WHERE e.task = t.id AND e.kind = 'move'
        ORDER BY e.at DESC, e.rowid DESC LIMIT 1)
    WHERE t.status = 'WAIT FOR HUMAN' AND coalesce(json_extract(m.data, '$.report'), 0) = 1 ORDER BY t.number";

/// Every waiting report moved: their IDs.
pub fn waiting_reports(tx: &Transaction) -> Result<Vec<String>, String> {
    let waiting: Vec<(String, String, i64)> = rows(tx, WAITING, [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let mut moved = Vec::new();
    for (key, author, at) in waiting {
        let t = ops::get(tx, &key)?;
        // the result is the mover's newest comment up to the move: finish_task keeps it so (terminum-dispatcher.md AC-123)
        let result: String = tx.query_row("SELECT coalesce((SELECT text FROM events WHERE task = ?1 AND kind = 'comment' AND author = ?2 AND at <= ?3
                AND text IS NOT NULL ORDER BY at DESC, rowid DESC LIMIT 1), '')", (&key, &author, at), |r| r.get(0)).map_err(sql)?;
        let schedule = crate::tasks::schedules::of_task(tx, &key)?.map(|s| s.id);
        let new = New { project: t.project.clone(), title: titled(&t.title), summary: summary_of(&result), body: result, author,
            role: role::TERMINUM, session: t.session.clone(), task: Some(key.clone()), schedule, ..New::default() };
        let (_, id, _) = record::create(tx, new, at)?;     // made at the result's time, its task linking it
        let words = format!("its report went to the Inbox as {id}");
        ops::move_marked(tx, DISPATCHER, &t.id, "DONE", Some(("inbox", &words)), ops::now())?;
        moved.push(id);
    }
    Ok(moved)
}
