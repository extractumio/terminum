// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use serde_json::json;

use crate::tasks::{ops, schedules};
use super::*;
use crate::supervisor::engine_tests::{Fixture, NOW, PROJECT};

#[test]
fn ac259_quarterly_audit_unedited_result() {
    let f = Fixture::new("audit");
    f.agent("strong");
    f.agent("auditor");
    f.write("UPDATE agents SET kind = 'claude' WHERE id = 'auditor'", []);
    f.write("INSERT INTO project_supervisor (project,enabled,generation,enabled_at) VALUES (?1,1,1,?2)", (PROJECT, NOW));
    f.store.with(|tx| {
        let tiers = |independent| serde_json::from_value(json!({
            "tiers":{"strong":"strong","independent":independent},"budgets":{"judgment_floor_tokens":1000}
        })).unwrap();
        settings::write(tx, PROJECT, 0, tiers(None::<&str>), false)?;
        assert!(!sync(tx, PROJECT, NOW)?);
        assert!(shown(tx, PROJECT)?["status"].as_str().unwrap().contains("independent"));
        settings::write(tx, PROJECT, 1, tiers(Some("auditor")), false)?;
        assert!(sync(tx, PROJECT, NOW)?);
        assert!(!sync(tx, PROJECT, NOW + 1)?);
        let schedule = schedules::all(tx)?.into_iter().find(|s| s.supervisor_audit).unwrap();
        assert!(schedule.supervisor_review);
        assert_eq!(schedule.agent, "auditor");
        assert_eq!(schedule.cron.as_deref(), Some(CRON));
        assert!(can_run(tx, PROJECT, "strong", NOW).is_err());
        let cap = can_run(tx, PROJECT, "auditor", NOW)?;
        let task = ops::create(tx, crate::tasks::store::DISPATCHER, ops::NewTask {
            title: "Supervisor quarterly audit".into(), text: DIRECTIONS.into(), scheduled: true,
            project: Some(PROJECT.into()), ..Default::default()
        }, NOW)?;
        schedules::fired(tx, &schedule, (NOW, 0), Some(&task.key), "started", None, NOW)?;
        tx.execute("INSERT INTO runs (id,task,project,agent_id,kind,command,folder,state,started_by,started_how,pane_key)
            VALUES ('audit-run',?1,?2,'auditor','claude','','','working','dispatcher','schedule','tmux:local:test:%1')",
            (&task.key, PROJECT)).map_err(sql)?;
        tx.execute("INSERT INTO run_tasks (run,task) VALUES ('audit-run',?1)", [&task.key]).map_err(sql)?;
        crate::supervisor::review::bind_run(tx, &task.key, "audit-run", NOW)?;
        assert!(governor::spending(tx, PROJECT, NOW)?.judgment.held_tokens >= cap);
        assert!(can_run(tx, PROJECT, "auditor", NOW).is_err());

        tx.execute("INSERT INTO supervisor_reflections
            (fingerprint,project,generation,subject_kind,subject_id,at,triggers,detail)
            VALUES ('reflection-test',?1,1,'task','test',?2,'[]','reflection-canary')", (PROJECT, NOW)).map_err(sql)?;
        tx.execute("INSERT INTO supervisor_actions
            (project,detector,response,kind,source,fingerprint,target,at)
            VALUES (?1,'flow.failed_repeat','R2','card','question','signal','fixture-task',?2)", (PROJECT, NOW)).map_err(sql)?;
        tx.execute("INSERT INTO supervisor_problems
            (project,title,lens,hypothesis,disproof,options,confidence,effort,state,opened_at)
            VALUES (?1,'Fixture problem','flow','fixture-hypothesis','disproof','[]',1,1,'confirmed',?2)", (PROJECT, NOW)).map_err(sql)?;
        let evidence = context(tx, PROJECT, "audit-run", json!({}))?;
        assert_eq!(evidence["items"][0]["source"], "question");
        for section in ["ledger", "signals", "misses", "register"] {
            let page = context(tx, PROJECT, "audit-run", json!({"section":section}))?;
            assert!(!page.to_string().contains("reflection-canary"));
        }
        assert_eq!(context(tx, PROJECT, "audit-run", json!({"section":"register"}))?["items"][0]["hypothesis"], "fixture-hypothesis");
        assert!(context(tx, PROJECT, "audit-run", json!({"section":"reflections"})).is_err());
        assert!(context(tx, "other", "audit-run", json!({})).is_err());

        let pane = "tmux:local:test:%1";
        let mark = crate::roles::supervisor::marked(tx, pane)?.unwrap();
        assert_eq!(mark.role, crate::roles::Role::Audit(true));
        assert!(mark.role.allows(TOOL));
        assert!(mark.role.allows("finish_task"));
        assert!(!mark.role.allows(crate::supervisor::config_changes::PROPOSE));
        crate::supervisor::review::require(tx, pane, PROJECT, &task.id)?;
        let report = "  Missed evidence\n\n  Noise and unsupported claims.\n";
        complete(tx, PROJECT, &task, "dispatcher", report, NOW + 20)?;
        assert_eq!(shown(tx, PROJECT)?["latest"]["report"], report);
        assert!(complete(tx, PROJECT, &task, "dispatcher", "duplicate", NOW + 21).is_err());
        schedules::skipped(tx, &schedule, (NOW + 30, 0), "No eligible slot.", NOW + 30)?;
        let status = shown(tx, PROJECT)?;
        assert_eq!(status["status"], "No eligible slot.");
        assert_eq!(status["latest"]["report"], report);
        Ok(())
    }).unwrap();
}
