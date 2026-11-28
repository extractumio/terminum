// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-237: a reading belongs to the round that requested it, even after its pane closed.

use rusqlite::{OptionalExtension, Transaction};
use serde_json::Value;

use crate::tasks::ops::{rows, sql};
use crate::tmux::hookstate::Whose;
use super::{Dispatcher, usage};

impl Dispatcher {
    /// Older clients understand only turn-end reads. A Supervisor round asks only its main
    /// conversation because a background subagent reports a different session total.
    pub(crate) fn token_request(&self, key: &str, ev: &Value, whose: Whose, linked: impl FnOnce() -> bool) -> (bool, Option<String>) {
        let end = matches!(ev["event"].as_str(), Some("Stop" | "SessionEnd"));
        if !end && !matches!(ev["event"].as_str(), Some("PreToolUse" | "PostToolUse")) { return (false, None); }
        let panes = self.panes.lock();
        let pane = panes.get(key);
        if let Some(receipt) = pane.and_then(|p| p.round_receipt.as_ref()) {
            if whose != Whose::Main || !ev["session"].as_str().is_some_and(super::hooks::good_id) { return (false, None); }
            return if ev["round_usage"] == true { (true, Some(receipt.clone())) } else { (end, None) };
        }
        let held = pane.is_some();
        drop(panes);
        (end && (held || linked()), None)
    }

    pub(super) fn remember_round(&self, tx: &Transaction, key: &str, run: &str) -> Result<(), String> {
        let receipt = tx.query_row("SELECT usage_receipt FROM supervisor_rounds WHERE run = ?1", [run],
            |row| row.get::<_, Option<String>>(0)).optional().map_err(sql)?.flatten();
        if let Some(pane) = self.panes.lock().get_mut(key).filter(|p| p.run == run) {
            pane.round_receipt = receipt;
        }
        Ok(())
    }
}

pub(super) fn live_receipts(tx: &Transaction) -> Result<std::collections::HashMap<String, String>, String> {
    Ok(rows(tx, "SELECT run, usage_receipt FROM supervisor_rounds WHERE ended_at IS NULL AND usage_receipt IS NOT NULL",
        [], |r| Ok((r.get(0)?, r.get(1)?)))?.into_iter().collect())
}

/// Called in the same ordered transaction as the hook's charge. The receipt came only from this
/// daemon, to the main agent of this round. A reused pane or another agent session cannot spend it.
pub(super) fn record(tx: &Transaction, key: &str, ev: &Value, at: i64) -> Result<bool, String> {
    let (Some(receipt), Some(session), Some(kind)) =
        (ev["round_reading"].as_str(), ev["session"].as_str(), ev["agent"].as_str()) else { return Ok(false) };
    let Some((run, agent)) = tx.query_row("SELECT r.id, r.agent_id FROM supervisor_rounds q JOIN runs r ON r.id = q.run
        WHERE q.usage_receipt = ?1 AND r.pane_key = ?2 AND r.agent_session = ?3 AND r.kind = ?4",
        (receipt, key, session, kind), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .optional().map_err(sql)? else { return Ok(false) };
    let Some(models) = usage::by_model(&ev["tokens"]).filter(|m| !m.is_empty()) else { return Ok(false) };
    let Some(grew) = usage::heard_round(tx, &run, &agent, models, at, kind == "codex")? else { return Ok(false) };
    let final_read = matches!(ev["sample_event"].as_str(), Some("Stop" | "SessionEnd"));
    let changed = tx.execute("UPDATE supervisor_rounds SET delayed_usage = delayed_usage OR
        (first_usage_at IS NULL AND (found_at IS NOT NULL OR ended_at IS NOT NULL)),
        first_usage_at = coalesce(first_usage_at, ?2),
        final_usage_at = CASE WHEN ?3 AND (found_at IS NOT NULL OR ended_at IS NOT NULL OR failure IS NOT NULL)
            THEN CASE WHEN ?4 THEN ?2 ELSE coalesce(final_usage_at, ?2) END
            WHEN ?4 THEN NULL ELSE final_usage_at END WHERE run = ?1 AND (first_usage_at IS NULL OR ?4 OR
            (?3 AND final_usage_at IS NULL AND (found_at IS NOT NULL OR ended_at IS NOT NULL OR failure IS NOT NULL)))",
        (&run, at, final_read, grew)).map_err(sql)?;
    if changed != 0 { crate::supervisor::governor::judge_usage(tx, &run, at)?; }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::runs::{self, Run};
    use crate::supervisor::governor;
    use crate::tasks::store::Store;
    use serde_json::json;

    #[test]
    fn ac237_only_a_rounds_main_runtime_is_asked_before_turn_end() {
        let dispatcher = Dispatcher::default();
        dispatcher.track_pane("ordinary", "task-run", "project");
        dispatcher.track_pane("marked", "round", "project");
        dispatcher.panes.lock().get_mut("marked").unwrap().round_receipt = Some("receipt".into());
        let request = |pane, event, version, whose| dispatcher.token_request(pane,
            &json!({"session":"conversation","event":event,"round_usage":version}), whose, || false);
        for event in ["PreToolUse", "PostToolUse", "Stop", "SessionEnd"] {
            assert_eq!(request("marked", event, true, Whose::Main), (true, Some("receipt".into())));
            assert_eq!(request("marked", event, true, Whose::Sub), (false, None));
            assert_eq!(request("marked", event, true, Whose::Stale), (false, None));
        }
        assert_eq!(request("marked", "PreToolUse", false, Whose::Main), (false, None));
        assert_eq!(request("marked", "Stop", false, Whose::Main), (true, None));
        assert_eq!(request("ordinary", "PreToolUse", true, Whose::Main), (false, None));
        assert_eq!(request("ordinary", "Stop", true, Whose::Main), (true, None));
    }

    #[test]
    fn ac237_migration_keeps_an_early_stop_partial_and_repairs_historical_finding_status() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE supervisor_rounds(run TEXT PRIMARY KEY, found_at INTEGER, ended_at INTEGER,
                token_cap INTEGER, finding TEXT, failure TEXT);
            CREATE TABLE usage(run TEXT, at INTEGER, input INTEGER, output INTEGER, cache_read INTEGER, cache_write INTEGER);
            INSERT INTO supervisor_rounds VALUES ('early',20,25,100,'{}','The Supervisor round exceeded its token limit.'),
                ('late',20,25,100,'{}',NULL),('absent',20,25,100,'{}',NULL);
            INSERT INTO usage VALUES ('early',18,110,0,0,0),('late',25,30,0,0,0);").unwrap();
        conn.execute_batch(include_str!("../tasks/schema66.sql")).unwrap();
        let read = |run| {
            conn.query_row("SELECT first_usage_at, final_usage_at, delayed_usage, budget_overrun_at, failure IS NULL
                FROM supervisor_rounds WHERE run=?1", [run], |r| Ok((r.get::<_,Option<i64>>(0)?,r.get::<_,Option<i64>>(1)?,
                    r.get::<_,bool>(2)?,r.get::<_,Option<i64>>(3)?,r.get::<_,bool>(4)?))).unwrap()
        };
        assert_eq!(read("early"), (Some(18),None,false,Some(18),true));
        assert_eq!(read("late"), (Some(25),Some(25),true,None,true));
        assert_eq!(read("absent"), (None,None,false,None,true));
    }

    #[test]
    fn ac237_round_readings_survive_close_and_restart_and_charge_only_their_issuer() {
        let dir = crate::testutil::fresh_dir("round-readings");
        let path = dir.join("kanban.sqlite");
        let at = 2_000_000;
        let (run, receipt) = Store::open(path.clone()).with(|tx| {
            tx.execute("INSERT INTO project_supervisor(project,enabled,generation,enabled_at) VALUES('default',1,1,?1)", [at]).map_err(sql)?;
            let like = Run { kind:"codex".into(), project:"default".into(), agent_id:"fixture".into(), ..Run::default() };
            let run = runs::add(tx, &like, &[], runs::Started::of(runs::How::Supervisor), at)?;
            tx.execute("UPDATE runs SET pane_key='pane', agent_session='session' WHERE id=?1", [&run]).map_err(sql)?;
            tx.execute("INSERT INTO supervisor_rounds(id,run,project,generation,fingerprint,signal_revision,case_file,deadline,token_cap)
                VALUES ('case',?1,'default',1,'fixture',1,'{}',?2,100)", (&run, at + 100)).map_err(sql)?;
            let receipt: String = tx.query_row("SELECT usage_receipt FROM supervisor_rounds WHERE run=?1", [&run], |r| r.get(0)).map_err(sql)?;
            Ok((run, receipt))
        }).unwrap();
        let sample = |event, n, model| json!({"round_reading":receipt,"session":"session","agent":"codex","sample_event":event,
            "tokens":{"model":model,"input":n,"output":0,"cache_read":0}});
        let spent = |tx: &Transaction| governor::spending(tx, "default", at + 4).map(|s| s.judgment);
        Store::open(path.clone()).with(|tx| {
            let partial = sample("PreToolUse", 30, "model-a");
            record(tx, "another-pane", &partial, at)?;
            for field in ["round_reading", "session", "agent"] {
                let mut wrong = partial.clone();
                wrong[field] = json!("another");
                record(tx, "pane", &wrong, at)?;
            }
            assert_eq!(spent(tx)?.tokens, 0);
            tx.execute("UPDATE supervisor_rounds SET finding='{}', found_at=?1, ended_at=?1 WHERE run=?2", (at, &run)).map_err(sql)?;
            tx.execute("UPDATE runs SET state='closed' WHERE id=?1", [&run]).map_err(sql)?;
            let missing = spent(tx)?;
            assert_eq!((missing.missing_rounds, missing.held_tokens), (1, 100));
            record(tx, "pane", &partial, at)?;
            let partial = spent(tx)?;
            assert_eq!((partial.tokens, partial.held_tokens, partial.partial_rounds, partial.delayed_rounds), (30, 70, 1, 1));
            Ok(())
        }).unwrap();
        Store::open(path).with(|tx| {
            for n in [130, 130, 20] { record(tx, "pane", &sample("SessionEnd", n, "model-b"), at + 2)?; }
            let done = spent(tx)?;
            assert_eq!((done.tokens, done.held_tokens, done.missing_rounds, done.partial_rounds), (130, 0, 0, 0));
            let (success, overrun, charged, failures): (bool, i64, i64, i64) = tx.query_row(
                "SELECT finding IS NOT NULL AND failure IS NULL, budget_overrun_at, (SELECT count(*) FROM usage WHERE run=?1),
                    (SELECT count(*) FROM supervisor_signals WHERE detector='self.round_failed') FROM supervisor_rounds WHERE run=?1",
                [&run], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(sql)?;
            assert_eq!((success, overrun, charged, failures), (true, at + 2, 2, 0));
            let amounts: Vec<(String, i64)> = rows(tx, "SELECT model, sum(input) FROM usage WHERE run=?1 GROUP BY model ORDER BY model",
                [&run], |r| Ok((r.get(0)?, r.get(1)?)))?;
            assert_eq!(amounts, [("model-a".into(), 30), ("model-b".into(), 100)]);
            // A second, silent round reporting an explicit zero releases its own reservation.
            let like = Run { kind:"claude".into(), project:"default".into(), ..Run::default() };
            let silent = runs::add(tx, &like, &[], runs::Started::of(runs::How::Supervisor), at)?;
            tx.execute("UPDATE runs SET pane_key='closed-pane', agent_session='session' WHERE id=?1", [&silent]).map_err(sql)?;
            tx.execute("INSERT INTO supervisor_rounds(id,run,project,generation,fingerprint,signal_revision,case_file,deadline,token_cap,ended_at)
                VALUES ('silent',?1,'default',1,'silent',1,'{}',?2,60,?3)", (&silent, at+100, at)).map_err(sql)?;
            let receipt: String = tx.query_row("SELECT usage_receipt FROM supervisor_rounds WHERE run=?1", [&silent], |r| r.get(0)).map_err(sql)?;
            let mut zero = sample("SessionEnd", 0, "model");
            zero["agent"] = json!("claude");
            zero["round_reading"] = json!(receipt);
            let mut unknown = zero.clone();
            unknown["tokens"] = Value::Null;
            record(tx, "closed-pane", &unknown, at+3)?;
            assert_eq!(spent(tx)?.held_tokens, 60);
            record(tx, "closed-pane", &zero, at+4)?;
            let complete = spent(tx)?;
            assert_eq!((complete.sampled_runs, complete.missing_rounds, complete.held_tokens, complete.tokens), (2,0,0,130));
            Ok(())
        }).unwrap();
    }
}
