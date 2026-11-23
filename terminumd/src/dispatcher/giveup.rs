//! AC-138: the human's Give up stops its linked turn cooperatively and leaves a durable task-tool notice.

use rusqlite::{OptionalExtension, Transaction};
use serde_json::json;

use super::{gate, runs::Run, store};
use crate::app::Shared;
use crate::tasks::{ops, store::DISPATCHER};

pub(crate) fn words(ids: &[String]) -> String {
    format!("[terminum Dispatcher] {}: The human gave up on this task. Stop your background work and subagents, start nothing new, and end your turn.", ids.join(", "))
}

pub(super) fn record(tx: &Transaction, run: &Run) -> Result<(), String> {
    for id in &run.ids {
        ops::log(tx, &ops::key_of(tx, id)?, DISPATCHER, "gave_up", "The human gave up on its run.", Some(&json!({"run": run.id})), ops::now())?;
    }
    Ok(())
}

pub(super) fn was_given_up(tx: &Transaction, run: &str) -> Result<bool, String> {
    tx.query_row("SELECT EXISTS (SELECT 1 FROM run_tasks m JOIN events e ON e.task = m.task
        WHERE m.run = ?2 AND e.kind = 'gave_up' AND e.author = ?1 AND json_extract(e.data, '$.run') = ?2)",
        (DISPATCHER, run), |row| row.get(0)).map_err(ops::sql)
}

pub(crate) fn check_tool(tx: &Transaction, pane: &str, id: Option<&str>) -> Result<(), String> {
    let Some(id) = id else { return Ok(()) };
    let query = format!("SELECT r.rowid, t.slug FROM events e JOIN runs r ON r.id = json_extract(e.data, '$.run')
        JOIN tasks t ON t.id = e.task
        WHERE e.kind = 'gave_up' AND e.author = ?1 AND r.pane_key = ?2 AND e.task = {}
        ORDER BY r.rowid DESC LIMIT 1", crate::tasks::slugs::key_named("?3"));
    let cancelled: Option<(i64, String)> = tx.query_row(&query, (DISPATCHER, pane, id), |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(ops::sql)?;
    let Some((cancelled, task)) = cancelled else { return Ok(()) };
    let newer: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM runs WHERE rowid > ?1 AND pane_key = ?2)",
        (cancelled, pane), |row| row.get(0)).map_err(ops::sql)?;
    if newer { Ok(()) } else { Err(words(&[task])) }
}

pub(super) async fn interrupt(app: &Shared, run: &Run, decided: std::time::Instant) {
    let sent = match run.place(app) {
        Some((server, pane)) => gate::interrupt_and_deliver(app, run, &server, &pane, &words(&run.ids), decided).await.map_err(|why| why.to_string()),
        None => Err("its pane or host is not reachable".into()),
    };
    let text = match sent {
        Ok(()) => "Give up stop instruction delivered.".to_string(),
        Err(why) => format!("Give up stop instruction not delivered: {why}."),
    };
    let to_log = run.clone();
    if let Err(why) = store(app, move |tx| {
        for id in &to_log.ids {
            ops::log(tx, &ops::key_of(tx, id)?, DISPATCHER, "told", &text, None, ops::now())?;
        }
        Ok(())
    }).await {
        tracing::warn!(event = "dispatcher.give_up", error = %why);
    }
    super::told_all(app, run);
}

#[cfg(test)]
#[path = "giveup_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "giveup/order_tests.rs"]
mod order_tests;
