// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The board's INTEGRATION column (terminum-integration.md AC-193), as terminumd gives it, on real repositories.
use super::*;
use crate::integrate::engine::tests::{batch, go, queued, sh, status, world, Sh};
use crate::tasks::integration::Settings;

#[test]
fn ac193_the_batch_in_work_heads_the_queue_in_its_order() {
    let w = world("ic-column", Settings { check: "true".into(), batch_most: 1, ..Settings::default() });
    w.s.with(|tx| tx.execute("UPDATE project_integration SET size = 1 WHERE project = 'api'", []).map(drop).map_err(sql)).unwrap();
    let (a, b, c) = (queued(&w, 1, "b.txt", "b\n"), queued(&w, 2, "c.txt", "c\n"), queued(&w, 3, "d.txt", "d\n"));
    go(&w, &Sh { hang: true, ..Default::default() }, 2);
    let v = w.s.with(|tx| of_project(tx, "api")).unwrap();
    assert_eq!(v["batch"]["words"], "checking");
    assert_eq!(v["batch"]["tasks"], json!([a]));
    assert_eq!(v["queue"], json!([{"id": b, "place": "1st", "verifying": false, "waits_for": null}, {"id": c, "place": "2nd", "verifying": false, "waits_for": null}]),
               "the queue's order, the batch's tasks not in it");
    assert_eq!((v["paused"].as_bool(), v["paused_by"].is_null()), (Some(false), true));
}

#[test]
fn ac193_each_state_in_words() {
    let w = world("ic-words", Settings { check: "exit 3".into(), ..Settings::default() });
    queued(&w, 1, "b.txt", "b\n");
    go(&w, &Sh::default(), 4);
    let b = batch(&w);
    let words = |state: &str, round: i64, why: Option<&str>| w.s.with(|tx| state_words(tx, &Batch { state: state.into(), round, ..b.clone() }, why)).unwrap();
    assert_eq!(words("red", 1, None), "red: waits for you", "no agent: the human decides");
    assert_eq!(words("checking", 2, None), "checking again, round 2");
    assert_eq!(words("rebuild", 1, None), "building");
    assert_eq!(words("publishing", 1, None), "publishing");
    assert_eq!(words("after", 1, None), "After runs");
    assert_eq!(words("not published", 1, Some("push refused")), "not published: push refused");
}

#[test]
fn ac193_a_task_returned_says_so_until_it_is_queued_again() {
    let w = world("ic-returned", Settings { check: "true".into(), ..Settings::default() });
    let a = queued(&w, 1, "b.txt", "b\n");
    w.s.with(|tx| { let k = ops::key_of(tx, &a)?; bs::give_back(tx, &k, "returned: conflict", bs::Left::Conflict, 5) }).unwrap();
    assert_eq!(status(&w, &a), "IN PROGRESS");
    assert_eq!(w.s.with(|tx| of_project(tx, "api")).unwrap()["returned"][&a], "returned: conflict");
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
