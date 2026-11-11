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
        assert_eq!(setting(tx, "api")?, Setting::default());
        assert!(on(tx)?.is_empty());
        assert_eq!(set(tx, "api", Some("gone"), None, 5).unwrap_err(), "That agent is gone.");
        assert_eq!(set(tx, "api", Some("p"), Some("gone"), 5).unwrap_err(), "That agent is gone.");
        set(tx, "api", Some("p"), Some("a"), 5)?;
        assert_eq!(setting(tx, "api")?, Setting { agent: Some("p".into()), default: Some("a".into()), on_at: 5, ..Default::default() });
        saw(tx, "api", "x:1")?;
        set(tx, "api", Some("p"), Some("a"), 7)?;
        assert_eq!(setting(tx, "api")?.seen.as_deref(), Some("x:1"), "a Save that changes nothing makes no round");
        // its default changed while on: the round is judged anew, it stays on since then
        set(tx, "api", Some("p"), None, 9)?;
        assert_eq!(setting(tx, "api")?, Setting { agent: Some("p".into()), on_at: 5, ..Default::default() });
        turn_off(tx, "api", "its round API-3 failed.")?;
        assert_eq!(setting(tx, "api")?, Setting { off_why: Some("its round API-3 failed.".into()), on_at: 5, ..Default::default() });
        assert_eq!(super::super::projectapi::settings(tx)?["api"]["auto"], json!({"agent": null, "default": null, "off_why": "its round API-3 failed."}));
        set(tx, "api", None, Some("a"), 12)?;
        assert_eq!(setting(tx, "api")?, Setting { default: Some("a".into()), on_at: 5, ..Default::default() }, "off by a human: no why");
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
