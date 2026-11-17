// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The board's INTEGRATION column (terminum-integration.md AC-193), as terminumd gives it, on real repositories.
use super::*;
use crate::integrate::engine::tests::{batch, go, queued, sh, status, world, Sh, World};
use crate::tasks::integration::Settings;

/// The column's cards as the board reads them, by status: their order and what each says.
fn column(w: &World, status: &str) -> Vec<(String, String)> {
    w.s.with(|tx| {
        let tasks = crate::tasks::query::list(tx, &crate::tasks::query::Query { statuses: vec![status.into()], projects: vec!["api".into()], ..Default::default() })?;
        let marks = marks(tx, &tasks, ops::now())?;
        Ok(tasks.iter().map(|t| (t.id.clone(), marks.get(&t.key).and_then(|m| m["mark"].as_str()).unwrap_or("").to_string())).collect())
    }).unwrap()
}

#[test]
fn ac193_the_batch_in_work_heads_the_queue_in_its_order() {
    let w = world("ic-column", Settings { check: "true".into(), batch_most: 1, ..Settings::default() });
    w.s.with(|tx| tx.execute("UPDATE project_integration SET size = 1 WHERE project = 'api'", []).map(drop).map_err(sql)).unwrap();
    let (a, b, c) = (queued(&w, 1, "b.txt", "b\n"), queued(&w, 2, "c.txt", "c\n"), queued(&w, 3, "d.txt", "d\n"));
    go(&w, &Sh { hang: true, ..Default::default() }, 2);
    let i = { let bt = batch(&w); w.s.with(|tx| bs::task_of(tx, &bt)).unwrap().unwrap() };
    let shown = column(&w, INTEGRATION);
    assert_eq!(shown.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), [i.as_str(), &a, &b, &c], "its integration task, its task, then the queue");
    assert!(shown[0].1.starts_with("batch 1 · checking · "), "{:?}", shown[0]);
    assert_eq!(&shown[1..], [(a, "in batch 1".to_string()), (b, "1st".to_string()), (c, "2nd".to_string())]);
}

#[test]
fn ac193_a_paused_queue_or_one_waiting_says_so_in_its_head() {
    let w = world("ic-notes", Settings::default());
    assert_eq!(w.s.with(notes).unwrap()["api"], json!({"paused": false, "note": null}));
    w.s.with(|tx| tx.execute("UPDATE project_integration SET paused = 1 WHERE project = 'api'", []).map(drop).map_err(sql)).unwrap();
    assert!(w.s.with(notes).unwrap()["api"]["note"].as_str().unwrap().starts_with("Paused by "));
}

#[test]
fn ac193_each_state_in_words() {
    let w = world("ic-words", Settings { check: "exit 3".into(), ..Settings::default() });
    queued(&w, 1, "b.txt", "b\n");
    go(&w, &Sh::default(), 4);
    let b = batch(&w);
    let words = |state: &str, round: i64, why: Option<&str>| w.s.with(|tx| state_words(tx, &Batch { state: state.into(), round, why: why.map(str::to_string), ..b.clone() })).unwrap();
    assert_eq!(words("red", 1, None), "red: waits for you", "no agent: the human decides");
    assert_eq!(words("checking", 2, None), "checking again, round 2");
    assert_eq!(words("rebuild", 1, None), "building");
    assert_eq!(words("publishing", 1, None), "publishing");
    assert_eq!(words("published", 1, None), "published: After waits for its turn");
    assert_eq!(words("after", 1, None), "After runs");
    assert_eq!(words("not published", 1, Some("push refused")), "not published: push refused");
}

#[test]
fn ac193_a_task_returned_says_so_until_it_is_queued_again() {
    let w = world("ic-returned", Settings { check: "true".into(), ..Settings::default() });
    let a = queued(&w, 1, "b.txt", "b\n");
    w.s.with(|tx| { let k = ops::key_of(tx, &a)?; bs::give_back(tx, &k, "returned: conflict", bs::Left::Conflict, 5) }).unwrap();
    assert_eq!(column(&w, "IN PROGRESS"), [(a, "returned: conflict".to_string())]);
}

#[test]
fn ac193_a_humans_integrate_queues_its_sessions_head_or_says_integrates_words() {
    let w = world("ic-human", Settings::default());
    let id = w.s.with(|tx| {
        let t = ops::create(tx, HUMAN, crate::tasks::ops::NewTask { title: "h".into(), text: "b".into(), project: Some("api".into()), ..Default::default() }, 1)?;
        ops::move_to(tx, crate::tasks::store::DISPATCHER, &t.id, "IN PROGRESS", 2)?;
        Ok(t.id)
    }).unwrap();
    let tree = w.root.join("tree-h");
    sh(&w.api, &["worktree", "add", "-q", "--detach", tree.to_str().unwrap(), "origin/main"]);
    assert!(crate::tasks::integration_queue::integrate_by_human(&w.s, &id, Some(tree.to_str().unwrap())).unwrap_err().contains("already in origin/main"));
    std::fs::write(tree.join("h.txt"), "h\n").unwrap();
    assert!(crate::tasks::integration_queue::integrate_by_human(&w.s, &id, Some(tree.to_str().unwrap())).unwrap_err().starts_with("Commit your work first"));
    sh(&tree, &["add", "h.txt"]);
    sh(&tree, &["commit", "-qm", "h"]);
    let said = crate::tasks::integration_queue::integrate_by_human(&w.s, &id, Some(tree.to_str().unwrap())).unwrap();
    assert!(said.contains("is queued 1st"), "{said}");
    assert_eq!(status(&w, &id), "INTEGRATION");
    let kept: String = w.s.with(|tx| tx.query_row("SELECT result FROM integration_entries", [], |r| r.get(0)).map_err(sql)).unwrap();
    assert!(kept.starts_with("Queued by ") && kept.ends_with(": no result from its agent."), "{kept}");
}

#[test]
fn ac193_batches_newest_first_with_the_greens_of_the_last_20() {
    let w = world("ic-batches", Settings::default());
    queued(&w, 1, "b.txt", "b\n");
    go(&w, &Sh::default(), 2);
    queued(&w, 2, "c.txt", "c\n");
    go(&w, &Sh::default(), 2);
    let v = w.s.with(|tx| batches_of(tx, "api", None)).unwrap();
    assert_eq!(v["batches"].as_array().unwrap().iter().map(|b| b["number"].as_i64().unwrap()).collect::<Vec<_>>(), [2, 1]);
    assert_eq!((v["green"].as_i64(), v["of"].as_i64(), v["batches"][0]["result"].as_str()), (Some(2), Some(2), Some("green")));
    assert_eq!(w.s.with(|tx| batches_of(tx, "api", Some(2))).unwrap()["batches"].as_array().unwrap().len(), 1, "older than 2: one");
}

#[test]
fn ac193_a_humans_take_out_puts_it_back_in_work_and_never_a_batchs_own_task() {
    let w = world("ic-out", Settings { check: "true".into(), ..Settings::default() });
    let a = queued(&w, 1, "b.txt", "b\n");
    w.s.with(|tx| out(tx, &a, 5)).unwrap();
    assert_eq!(status(&w, &a), "IN PROGRESS");
    assert_eq!(w.s.with(|tx| out(tx, &a, 6)).unwrap_err(), format!("{a} is not queued."));
    queued(&w, 2, "c.txt", "c\n");
    go(&w, &Sh { hang: true, ..Default::default() }, 2);
    let bt = batch(&w);   // read outside: the store's lock is not taken twice
    let i = w.s.with(|tx| bs::task_of(tx, &bt)).unwrap().unwrap();
    assert!(w.s.with(|tx| out(tx, &i, 7)).unwrap_err().contains("ends with its batch"));
}

#[test]
fn ac193_a_red_batchs_cards_hold_its_keys_and_a_checking_ones_none() {
    let w = world("ic-keys", Settings { check: "exit 3".into(), ..Settings::default() });
    let a = queued(&w, 1, "b.txt", "b\n");
    go(&w, &Sh::default(), 4);
    let b = batch(&w);
    assert_eq!(b.state, "red");
    let i = w.s.with(|tx| bs::task_of(tx, &b)).unwrap().unwrap();
    let mark = |id: &str| w.s.with(|tx| {
        let t = ops::get(tx, id)?;
        Ok(marks(tx, std::slice::from_ref(&t), ops::now())?.remove(&t.key).unwrap_or_default())
    }).unwrap();
    let card = mark(&a);
    assert_eq!((card["batch_id"].as_i64(), card["keys"].clone()), (Some(b.id), json!(["take-out", "check-again", "return", "return-all"])), "{card}");
    let head = mark(&i);
    assert_eq!((head["head"].as_bool(), head["keys"].clone()), (Some(true), json!(["check-again", "return", "return-all"])), "{head}");
    w.s.with(|tx| tx.execute("UPDATE integration_batches SET state = 'checking' WHERE id = ?1", [b.id]).map(drop).map_err(sql)).unwrap();
    assert_eq!(mark(&a)["keys"], json!(["take-out"]), "nothing to decide while it is checked: only Take out");
}

#[test]
fn ac193_integrate_where_the_project_integrates_and_never_on_a_batchs_task() {
    let w = world("ic-integrate", Settings::default());
    let id = w.s.with(|tx| {
        let t = ops::create(tx, HUMAN, crate::tasks::ops::NewTask { title: "w".into(), text: "b".into(), project: Some("api".into()), ..Default::default() }, 1)?;
        ops::move_to(tx, crate::tasks::store::DISPATCHER, &t.id, "IN PROGRESS", 2)?;
        Ok(t.id)
    }).unwrap();
    let keys = || w.s.with(|tx| { let t = ops::get(tx, &id)?; Ok(marks(tx, std::slice::from_ref(&t), ops::now())?.remove(&t.key).unwrap_or_default()) }).unwrap()["keys"].clone();
    assert_eq!(keys(), json!(["integrate"]));
    w.s.with(|tx| tx.execute("UPDATE project_integration SET is_on = 0 WHERE project = 'api'", []).map(drop).map_err(sql)).unwrap();
    assert_eq!(keys(), Value::Null, "integration off: no key");
}
