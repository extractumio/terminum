// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Auto-Dispatcher's setting and a task's agent as the store keeps them (AC-170).

use super::*;
use crate::tasks::ops::NewTask;
use crate::tasks::store::Store;

fn store(name: &str) -> Store {
    let s = Store::open(crate::testutil::fresh_dir(name).join("kanban.sqlite"));
    s.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('api', 'api', 0, '/srv', 0)", []).map_err(sql)?;
        tx.execute("INSERT INTO agents (id, name, kind, command, color, at_once) VALUES ('p', 'Picker', 'claude', 'claude', 0, 1), ('a', 'Smith', 'claude', 'claude', 1, 1)", []).map_err(sql)?;
        super::super::slugs::fill(tx)
    }).unwrap();
    s
}

#[test]
fn ac170_a_projects_setting_is_off_until_a_human_sets_it() {
    store("auto-setting").with(|tx| {
        let made = setting(tx, "api")?;
        assert_eq!(made, Setting { on: false, agent: None, default: None, off_why: None, on_at: 0, severities: vec!["P0".into(), "P1".into()], wait: 10, day_most: 10 }, "as a project is made");
        assert!(due_for(tx)?.is_empty());
        let put = |body: Value, at: i64| setting(tx, "api")?.with(&body).and_then(|new| set(tx, "api", &new, at));
        assert_eq!(put(json!({"on": true}), 5).unwrap_err(), "Choose the agent that picks first: its settings.");
        assert_eq!(put(json!({"agent": "gone"}), 5).unwrap_err(), "That agent is gone.");
        assert_eq!(put(json!({"agent": "p", "default": "gone"}), 5).unwrap_err(), "That agent is gone.");
        assert_eq!(put(json!({"severities": ["P9"]}), 5).unwrap_err(), "Severity: one of P0, P1, P2, P3");
        assert_eq!(put(json!({"severities": [1, "P0"]}), 5).unwrap_err(), "severities: a list of names");
        assert_eq!(put(json!({"wait_minutes": 1441}), 5).unwrap_err(), "Wait: 0 to 1440 minutes");
        assert_eq!(put(json!({"day_most": 0}), 5).unwrap_err(), "Tasks a day: 1 to 1000");
        assert_eq!(put(json!({"day_most": 1001}), 5).unwrap_err(), "Tasks a day: 1 to 1000");
        assert_eq!(put(json!({"day_most": null}), 5).unwrap_err(), "Tasks a day: 1 to 1000");
        assert_eq!(put(json!({"on": true, "agent": "p", "severities": []}), 5).unwrap_err(), "Tick a severity it takes first: with none it starts nothing.");
        // its agents chosen while off; the switch then sends "on" alone and keeps the rest
        put(json!({"agent": "p", "default": "a", "severities": ["P2", "P0"], "wait_minutes": 0, "day_most": 3}), 5)?;
        let chosen = Setting { agent: Some("p".into()), default: Some("a".into()), severities: vec!["P0".into(), "P2".into()], wait: 0, day_most: 3, ..made };
        assert_eq!(setting(tx, "api")?, chosen);
        put(json!({"on": true}), 7)?;
        assert_eq!(setting(tx, "api")?, Setting { on: true, on_at: 7, ..chosen.clone() });
        put(json!({"default": null}), 9)?;
        assert_eq!(setting(tx, "api")?, Setting { on: true, on_at: 7, default: None, ..chosen.clone() }, "changed while on: on since then");
        // it turns itself off: its agent stays chosen, and on again counts anew
        turn_off(tx, "api", "its round API-3 failed.")?;
        assert_eq!(setting(tx, "api")?, Setting { off_why: Some("its round API-3 failed.".into()), on_at: 7, default: None, ..chosen.clone() });
        assert_eq!(super::super::projectapi::settings(tx)?["api"]["auto"], json!({"on": false, "agent": "p", "default": null, "severities": ["P0", "P2"], "wait_minutes": 0, "day_most": 3, "started_day": 0, "off_why": "its round API-3 failed."}));
        put(json!({"wait_minutes": 0}), 11)?;
        assert_eq!(setting(tx, "api")?.off_why.as_deref(), Some("its round API-3 failed."), "saved while off, why stays");
        put(json!({"on": true}), 12)?;
        assert_eq!(due_for(tx)?.len(), 1);
        assert_eq!(setting(tx, "api")?, Setting { on: true, on_at: 12, default: None, ..chosen.clone() }, "on again: no why");
        put(json!({"on": false}), 13)?;
        assert_eq!(setting(tx, "api")?, Setting { on_at: 12, default: None, ..chosen }, "off by a human: no why");
        Ok(())
    }).unwrap();
}

#[test]
fn ac170_a_tasks_agent_is_a_humans_choice_kept_in_its_history() {
    store("auto-agent").with(|tx| {
        let t = ops::create(tx, HUMAN, NewTask { title: "t".into(), project: Some("api".into()), ..Default::default() }, 1)?;
        assert_eq!(agent_of(tx, &t.key)?, None);
        assert_eq!(set_agent(tx, &t.id, Some("gone"), 2).unwrap_err(), "That agent is gone.");
        set_agent(tx, &t.id, Some("a"), 2)?;
        assert_eq!(agent_of(tx, &t.key)?.as_deref(), Some("a"));
        assert!(ops::history(tx, &t.id)?.to_string().contains("Its agent: Smith"));
        set_agent(tx, &t.id, Some(""), 3)?;
        assert_eq!(agent_of(tx, &t.key)?, None);
        // an agent deleted since is none
        set_agent(tx, &t.id, Some("a"), 4)?;
        tx.execute("DELETE FROM agents WHERE id = 'a'", []).map_err(sql)?;
        assert_eq!(agent_of(tx, &t.key)?, None);
        Ok(())
    }).unwrap();
}
