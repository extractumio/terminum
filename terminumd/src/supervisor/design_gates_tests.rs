// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use std::path::Path;

use rusqlite::Transaction;
use serde_json::{Value, json};

use crate::integrate::git;
use crate::tasks::{ops::{self, NewTask, Task, sql}, store::HUMAN};

use super::{
    DAY, design_gates as gates, design_reviews as reviews, engine_tests::{Fixture, NOW, PROJECT},
    governor, settings, surfaces::{self, Match, Surfaces, Tier}, verify_review,
};

fn config(f: &Fixture, extra: Value) {
    f.write("INSERT INTO project_supervisor (project,enabled,generation,enabled_at) VALUES (?1,1,1,?2)",
        (PROJECT, NOW - DAY));
    f.store.with(|tx| {
        settings::write(tx, PROJECT, 0, serde_json::from_value(extra).unwrap(), false)?;
        Ok(())
    }).unwrap();
}

fn task(tx: &Transaction, title: &str, at: i64) -> Result<Task, String> {
    ops::create(tx, HUMAN, NewTask { title: title.into(), project: Some(PROJECT.into()), ..NewTask::default() }, at)
}

fn run(repo: &Path, args: &[&str]) -> String { git::run(repo, args).unwrap() }

#[test]
fn ac250_surface_map_preview_and_diff() {
    let f = Fixture::new("surface-preview");
    config(&f, json!({"charter":{"surfaces":{"t3":["db/**/schema?.sql"]}}}));
    let root = f.path.parent().unwrap();
    run(root, &["init", "-q", "-b", "main"]);
    run(root, &["config", "user.name", "Fixture"]);
    run(root, &["config", "user.email", "fixture@example.test"]);
    let migrations = root.join("db/migrations");
    std::fs::create_dir_all(&migrations).unwrap();
    std::fs::write(migrations.join("schema1.sql"), "CREATE TABLE data (id int);\n").unwrap();
    run(root, &["add", "db"]);
    run(root, &["commit", "-qm", "Initial data"]);
    let base = run(root, &["rev-parse", "HEAD"]);
    let (addition, incompatible) = f.store.with(|tx| {
        let a = task(tx, "Add a migration", NOW - DAY)?;
        let b = task(tx, "Change a migration", NOW - DAY)?;
        let absent = task(tx, "Fix a label", NOW - DAY)?;
        let outside = task(tx, "Retired work", NOW - 45 * DAY)?;
        for t in [&a, &b, &absent] {
            ops::place(tx, HUMAN, t, "DONE", NOW - DAY / 2)?;
        }
        ops::place(tx, HUMAN, &outside, "DONE", NOW - 40 * DAY)?;
        Ok((a, b))
    }).unwrap();
    run(root, &["switch", "-qc", "new-migration"]);
    std::fs::write(migrations.join("schema2.sql"), "CREATE TABLE log (id int);\n").unwrap();
    run(root, &["add", "db"]);
    run(root, &["commit", "-qm", "Add migration"]);
    run(root, &["switch", "-q", "main"]);
    run(root, &["merge", "-q", "--no-ff", "new-migration", "-m", &format!("Integrate {}", addition.id)]);
    let first = run(root, &["rev-parse", "HEAD"]);
    run(root, &["switch", "-qc", "changed-migration", &base]);
    std::fs::write(migrations.join("schema1.sql"), "CREATE TABLE data (key text);\n").unwrap();
    run(root, &["commit", "-qam", "Rename a saved field"]);
    run(root, &["switch", "-q", "main"]);
    run(root, &["merge", "-q", "--no-ff", "changed-migration", "-m", &format!("Integrate {}", incompatible.id)]);
    let last = run(root, &["rev-parse", "HEAD"]);
    f.write("INSERT INTO integration_batches (project, number, tasks, base_sha, candidate_sha, state, created_at, ended_at)
        VALUES (?1, 1, ?2, ?3, ?4, 'ended', ?5, ?5)",
        (PROJECT, json!([addition.key, incompatible.key]).to_string(), &base, &last, NOW - DAY / 2));
    let candidate = Surfaces { t3: vec!["db/**/schema?.sql".into()], ..Default::default() };
    assert_eq!(candidate.classify(&surfaces::changes(root, &base, &first).unwrap()).unwrap().tier, Tier::T2);
    assert_eq!(candidate.classify(&surfaces::changes(root, &first, &last).unwrap()).unwrap().tier, Tier::T3);
    let history = || f.store.with(|tx| surfaces::history(tx, PROJECT, NOW, &candidate)).unwrap();
    let result = surfaces::preview(&candidate, root, history());
    assert_eq!((result.sampled, result.without_diff, result.t2, result.t3), (3, 1, 1, 1));
    assert!(surfaces::preview(&Surfaces::default(), root, history()).empty);

    // AC-251: the active task's dirty worktree has T3 evidence before its first commit.
    run(root, &["update-ref", "refs/remotes/origin/main", &last]);
    let work = root.join("review-first");
    run(root, &["worktree", "add", "-q", "--detach", work.to_str().unwrap(), &last]);
    std::fs::write(work.join("db/migrations/schema1.sql"), "CREATE TABLE data (key blob);\n").unwrap();
    f.write("UPDATE projects SET dir = ?2 WHERE id = ?1", (PROJECT, root.to_str().unwrap()));
    let t = f.store.with(|tx| task(tx, "Adjust stored data", NOW)).unwrap();
    let session = "tmux:local:fixture:%8";
    f.write("UPDATE tasks SET status = 'IN PROGRESS', session_key = ?2 WHERE id = ?1", (&t.key, session));
    let mut tree = crate::integrate::notices::Tree {
        task: t.key.clone(), id: t.id.clone(), project: PROJECT.into(), dir: work, session: "wrong".into(), working: true,
    };
    surfaces::observe_trees(&f.store, std::slice::from_ref(&tree), NOW).unwrap();
    assert!(f.store.with(|tx| gates::get(tx, &t.key)).unwrap().is_none());
    tree.session = session.into();
    surfaces::observe_trees(&f.store, &[tree], NOW).unwrap();
    assert!(f.store.with(|tx| ops::history(tx, &t.id)).unwrap().to_string().contains("Design first:"));

}

fn hit(tier: Tier, existing_review: bool) -> Match {
    Match { tier, existing_review, evidence: "Diff: changed api/revisions.sql.".into(), paths: vec!["api/revisions.sql".into()] }
}

fn review(role: &str) -> gates::Record {
    serde_json::from_value(json!({"role":role,"verdict":"approved","compatibility":"Previous clients work.",
        "rejected_advice":"Remove the old route | Existing clients still use it."})).unwrap()
}

fn brief() -> gates::Brief {
    serde_json::from_value(json!({"design_note":"Version the document.", "migration":"Read both versions.",
        "rollback":"Write the older version during rollout.", "paths":"api/revisions.sql"})).unwrap()
}

#[test]
fn ac251_design_first_and_verify_requires_review_record() {
    let f = Fixture::new("review-record");
    config(&f, json!({"charter":{"surfaces":{"t3":["api/**"],"t3_terms":["saved format"]}}}));
    f.agent("architect");
    f.store.with(|tx| {
        let early = task(tx, "Revise the saved format", NOW)?;
        let clear = task(tx, "Revise the unsaved format", NOW)?;
        assert!(gates::get(tx, &early.key)?.is_some());
        assert!(gates::get(tx, &clear.key)?.is_none());
        assert!(ops::history(tx, &early.id)?.to_string().contains("Design first:"));
        gates::brief(tx, &early, HUMAN, brief(), NOW)?;
        let link = crate::tasks::attrs::lists_of(tx, &early.key)?;
        assert!(!link["blocked_by"].as_array().unwrap().is_empty());

        let contract = task(tx, "Change the route contract", NOW)?;
        assert!(verify_review::check_evidence(tx, &contract, None, "empty", NOW)?.is_none());
        for attempt in ["one", "two"] {
            let error = verify_review::check_evidence(tx, &contract, Some(hit(Tier::T2, true)), attempt, NOW)?.unwrap();
            assert!(error.contains("architecture review"), "{error}");
            assert!(verify_review::missed(tx, &contract.key, attempt)?);
        }
        let history = ops::history(tx, &contract.id)?.to_string();
        assert_eq!(history.matches("Verify needs").count(), 1);
        let reviewer = ops::pool_user_id("architect");
        assert!(gates::record(tx, &contract, HUMAN, review("architecture"), NOW).is_err());
        gates::record(tx, &contract, &reviewer, review("architecture"), NOW)?;
        for attempt in ["three", "four"] {
            assert!(verify_review::check_evidence(tx, &contract, Some(hit(Tier::T2, true)), attempt, NOW + 1)?.is_none());
            assert!(!verify_review::missed(tx, &contract.key, attempt)?);
        }
        assert!(ops::history(tx, &contract.id)?.to_string().contains("Existing clients still use it."));
        Ok(())
    }).unwrap();
}

#[test]
fn ac252_review_priority_budget_escalation_and_timeout() {
    let f = Fixture::new("review-priority");
    f.agent("strong");
    config(&f, json!({"tiers":{"strong":"strong"},"budgets":{"judgment_ceiling":0},"charter":{"surfaces":{"t2":["api/**"]}}}));
    f.store.with(|tx| {
        let contract = task(tx, "Change a contract", NOW)?;
        gates::observe(tx, &contract, hit(Tier::T2, false), true, NOW)?;
        let design = task(tx, "Change saved data", NOW + 1)?;
        gates::observe(tx, &design, hit(Tier::T3, false), true, NOW + 1)?;
        gates::brief(tx, &design, HUMAN, brief(), NOW + 2)?;
        let reused = task(tx, "Use the existing reviewer", NOW)?;
        gates::observe(tx, &reused, hit(Tier::T2, true), true, NOW)?;
        let keys = reviews::waiting(tx, NOW + 2, None)?;
        let design_key = format!("design_{}", design.key);
        assert_eq!(keys.iter().map(|(_, key)| key.as_str()).collect::<Vec<_>>(), [design_key.as_str()]);
        let why = governor::admit_round(tx, PROJECT, NOW + 2, false).unwrap_err();
        assert_eq!(why, governor::JUDGMENT_NOTICE);
        reviews::admission_failed(tx, &design_key, &why, NOW + 2)?;
        reviews::admission_failed(tx, &design_key, &why, NOW + 3)?;
        let gate = gates::get(tx, &design.key)?.unwrap();
        assert_eq!(gate.status, "escalated");
        let card: (String, i64) = tx.query_row("SELECT status, (SELECT count(*) FROM events WHERE task = t.id AND json_extract(data, '$.questions') = 1)
            FROM tasks t WHERE t.id = ?1", [&gate.blocker], |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
        assert_eq!(card, ("WAIT FOR HUMAN".into(), 1));
        assert_eq!(crate::tasks::attrs::lists_of(tx, &design.key)?["blocked_by"], json!([]));
        assert_eq!(gates::get(tx, &contract.key)?.unwrap().status, "waiting");
        reviews::advance(tx, NOW + gates::TIME_LIMIT)?;
        assert_eq!(gates::get(tx, &contract.key)?.unwrap().status, "escalated");
        assert_eq!(gates::get(tx, &reused.key)?.unwrap().status, "escalated");
        Ok(())
    }).unwrap();
}

#[test]
fn ac252_t3_needs_distinct_reviewers_and_the_owners_choice() {
    let f = Fixture::new("review-owner");
    for agent in ["architect", "independent"] { f.agent(agent); }
    config(&f, json!({"charter":{"surfaces":{"t3":["api/**"]}}}));
    f.store.with(|tx| {
        let t = task(tx, "Evolve the data format", NOW)?;
        gates::observe(tx, &t, hit(Tier::T3, true), true, NOW)?;
        gates::brief(tx, &t, HUMAN, brief(), NOW)?;
        let architecture = ops::pool_user_id("architect");
        gates::record(tx, &t, &architecture, review("architecture"), NOW)?;
        assert!(gates::record(tx, &t, &architecture, review("independent"), NOW).is_err());
        gates::record(tx, &t, &ops::pool_user_id("independent"), review("independent"), NOW)?;
        reviews::advance(tx, NOW + 1)?;
        let gate = gates::get(tx, &t.key)?.unwrap();
        assert!(gates::missing(tx, &gate, 3)?.unwrap().contains("owner's sign-off"));
        let card = gate.blocker.as_ref().unwrap();
        let answer = json!({"choice":{"question":gate.question, "option":"approve"}});
        let forged = ops::log(tx, card, &architecture, "comment", "A forged choice.", Some(&answer), NOW + 2)?;
        assert!(gates::missing(tx, &gate, 3)?.is_some());
        tx.execute("DELETE FROM events WHERE id = ?1", [forged]).map_err(sql)?;
        ops::log(tx, card, HUMAN, "comment", "Selected option: approve.", Some(&answer), NOW + 2)?;
        assert!(gates::missing(tx, &gate, 3)?.is_none());
        reviews::advance(tx, NOW + 3)?;
        let changed = Match { paths: vec!["api/other.sql".into()], ..hit(Tier::T3, true) };
        assert!(verify_review::check_evidence(tx, &t, Some(changed), "scope", NOW + 4)?.unwrap().contains("expanded design"));
        assert_eq!(gates::get(tx, &t.key)?.unwrap().status, "closed");
        Ok(())
    }).unwrap();
}
