// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use serde_json::json;

use super::waiting;
use crate::supervisor::{engine_tests::{Fixture, NOW, PROJECT}, register, register_tests, signals};

fn eligible(f: &Fixture, fingerprint: &str) -> bool {
    let candidates = f.store.with(|tx| waiting(tx, NOW + 300, Some(fingerprint))).unwrap();
    !candidates.is_empty()
}

fn finished_round(f: &Fixture, run: &str, at: i64) {
    f.run("fix", run, "agent", at);
    let case = f.store.with(|tx| Ok(json!({"signal": signals::get(tx, PROJECT, "signal")?}))).unwrap();
    f.write("INSERT INTO supervisor_rounds (id, run, project, generation, fingerprint, signal_revision,
        case_file, deadline, finding, found_at, ended_at)
        SELECT ?1, ?1, project, 1, fingerprint, revision, ?2, ?3 + 900, '{}', ?3, ?3
        FROM supervisor_signals WHERE fingerprint = 'signal'", (run, case.to_string(), at));
}

#[test]
fn ac233_a_checked_cause_with_an_open_fix_waits_for_escalation_or_recurrence() {
    let f = Fixture::new("known-cause");
    register_tests::enabled(&f, json!({}));
    register_tests::signal(&f, "signal", PROJECT, 2, 0.25, false);
    f.write("UPDATE supervisor_signals SET detector = 'owner.priority_churn', subject_kind = 'project', metric = 'owner_starts'
        WHERE fingerprint = 'signal'", []);
    register_tests::signal(&f, "unrelated", "another", 2, 0.25, false);
    f.task("fix", Some("Bug"), "P1", "TODO", NOW);
    f.agent("agent");
    f.store.with(|tx| {
        let id = register::open(tx, PROJECT, register_tests::draft("Known cause", &["signal"]), NOW)?;
        tx.execute("INSERT INTO supervisor_problem_tasks (problem, task) VALUES (?1, 'fix')", [id])
            .map_err(crate::tasks::ops::sql)?;
        Ok(())
    }).unwrap();
    assert!(eligible(&f, "signal"), "a linked fix alone does not prove a previous investigation");
    finished_round(&f, "initial", NOW);

    for n in 1..=23 {
        f.write("UPDATE supervisor_signals SET evidence = ?1, count = count + 1, last_seen = ?2
            WHERE fingerprint = 'signal'", (json!([{"table":"events","id":format!("sample-{n}")}]).to_string(), NOW + n));
        assert!(!eligible(&f, "signal"), "ordinary revision {n} of the same known cause");
    }
    let status = || f.store.with(|tx| super::status(tx, "signal")).unwrap().unwrap();
    let covered = status();
    assert!(covered.contains("initial") && covered.contains("Fix "), "{covered}");
    assert!(eligible(&f, "unrelated"), "a different fingerprint still qualifies");
    f.write("INSERT INTO supervisor_round_requests (fingerprint,owner_requested) VALUES ('signal',1)", []);
    assert!(eligible(&f, "signal"), "an explicit investigation still qualifies");
    assert_eq!(status(), "Investigation requested.");
    f.write("DELETE FROM supervisor_round_requests", []);

    f.write("UPDATE supervisor_signals SET scores = json_set(scores, '$.impact', 3), last_seen = ?1
        WHERE fingerprint = 'signal'", [NOW + 24]);
    assert!(eligible(&f, "signal"), "an increase in level qualifies despite the open fix");
    finished_round(&f, "escalation", NOW + 25);
    f.write("UPDATE supervisor_signals SET state = 'resolved' WHERE fingerprint = 'signal'", []);
    f.write("UPDATE supervisor_signals SET state = 'recurred', last_seen = ?1 WHERE fingerprint = 'signal'", [NOW + 30]);
    assert!(eligible(&f, "signal"), "a distinct episode at the same level qualifies");
    finished_round(&f, "recurrence", NOW + 31);
    f.write("UPDATE supervisor_signals SET evidence = '[]' WHERE fingerprint = 'signal'", []);
    assert!(!eligible(&f, "signal"), "the new episode also gets just one round while the fix is open");
    f.write("UPDATE tasks SET archived_at = ?1 WHERE id = 'fix'", [NOW + 32]);
    assert!(eligible(&f, "signal"), "an archived fix cannot cover a live signal");
    f.write("UPDATE tasks SET archived_at = NULL, status = 'DONE' WHERE id = 'fix'", []);
    assert!(eligible(&f, "signal"), "a completed fix cannot cover an ongoing failure");
    f.write("UPDATE tasks SET status = 'IN PROGRESS' WHERE id = 'fix'", []);
    f.write("UPDATE supervisor_rounds SET case_file = '{}'", []);
    assert!(eligible(&f, "signal"), "without a retained level and episode the cause needs checking");
}

#[test]
fn ac233_lower_level_recurrence_obeys_its_trigger_and_runs_once_per_episode() {
    for stuck in [false, true] {
        let f = Fixture::new(if stuck { "stuck-recurred" } else { "weekly-recurred" });
        register_tests::enabled(&f, json!({}));
        register_tests::signal(&f, "signal", "subject", 1, 0.05, false);
        f.task("fix", Some("Bug"), "P2", "IN PROGRESS", NOW);
        f.agent("agent");
        if stuck {
            f.write("UPDATE supervisor_signals SET detector = 'flow.stuck'", []);
        } else {
            for weeks_ago in [0, 1, 2] {
                f.write("INSERT INTO supervisor_signal_weeks VALUES ('signal', ?1)",
                    [NOW / crate::supervisor::WEEK - weeks_ago]);
            }
        }
        assert!(eligible(&f, "signal"));
        finished_round(&f, "initial", NOW);
        f.write("UPDATE supervisor_rounds SET failure = 'fixture launch failure', finding = NULL", []);
        f.write("UPDATE supervisor_signals SET evidence = ?1", [json!([{"table":"events","id":"extra"}]).to_string()]);
        assert!(eligible(&f, "signal"), "a failed round has no checked finding for later evidence");
        f.write("UPDATE supervisor_rounds SET finding = '{}', failure = NULL", []);
        f.store.with(|tx| {
            let id = register::open(tx, PROJECT, register_tests::draft("Old cause", &["signal"]), NOW)?;
            tx.execute("UPDATE supervisor_problems SET state = 'fix_in_work' WHERE id = ?1", [id])
                .map_err(crate::tasks::ops::sql)?;
            tx.execute("INSERT INTO supervisor_problem_tasks VALUES (?1, 'fix')", [id])
                .map_err(crate::tasks::ops::sql)?;
            Ok(())
        }).unwrap();
        f.write("UPDATE supervisor_signals SET evidence = '[]', last_seen = ?1", [NOW + 10]);
        assert!(!eligible(&f, "signal"), "one round in this episode");
        f.write("UPDATE supervisor_signals SET state = 'resolved'", []);
        f.write("UPDATE supervisor_signals SET state = 'recurred', last_seen = ?1", [NOW + 20]);
        assert!(eligible(&f, "signal"), "a recurrence qualifies even with the original fix still open");
        if !stuck {
            f.write("DELETE FROM supervisor_signal_weeks WHERE week = ?1", [NOW / crate::supervisor::WEEK - 2]);
            assert!(!eligible(&f, "signal"), "a recurrence still needs three of eight weeks");
        }
    }
}
