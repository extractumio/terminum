// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213: a checked round's typed critical escalation, its refusals, and how long it stands.

use serde_json::json;

use crate::tasks::ops::{self, sql};
use super::{Rule, engine_tests::{Fixture, PROJECT}, escalations::{self, Request, Score}, signals};

const KEY: &str = "escalated-signal";

fn signal(f: &Fixture, level: u8, at: i64) {
    let scores = json!({"impact": level, "blast": 0, "reversibility": 0, "confidence": 1.0});
    let measure = json!({"value": 5, "baseline": 2, "threshold": 4, "window_start": at - 60, "window_end": at});
    f.write("INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind, subject_id, metric, evidence,
        measure, scores, first_seen, last_seen, count, state) VALUES (?1, ?2, ?3, 'project', ?2, 'failed_runs',
            '[{\"table\":\"runs\",\"id\":\"judged\"}]', ?4, ?5, ?6, ?6, 1, 'raised')",
        (KEY, PROJECT, Rule::FailedRepeat.id(), measure.to_string(), scores.to_string(), at));
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (PROJECT, at));
}

fn request(level: u8) -> Request {
    Request { score: Score::Blast, level, reason: "Every project agent shares the failing setup.".into() }
}

#[test]
fn ac213_a_round_can_ask_only_to_raise_a_signal_to_critical_with_strong_confidence() {
    let f = Fixture::new("escalation-check");
    let at = ops::now();
    signal(&f, 2, at);
    f.store.with(|tx| {
        let s = signals::get(tx, PROJECT, KEY)?;
        assert!(request(1).check(&s, 0.9).unwrap_err().contains("only raise"), "a lower level is refused");
        assert!(request(2).check(&s, 0.9).unwrap_err().contains("level 3"), "the same level is no escalation");
        assert!(request(3).check(&s, 0.6).unwrap_err().contains("0.7"), "a weak finding cannot escalate");
        request(3).check(&s, 0.7)?;
        Ok(())
    }).unwrap();
}

#[test]
fn ac213_an_accepted_escalation_raises_the_level_and_the_incident_while_its_evidence_stands() {
    let f = Fixture::new("escalation-record");
    let at = ops::now();
    signal(&f, 2, at);
    f.store.with(|tx| {
        let s = signals::get(tx, PROJECT, KEY)?;
        escalations::record(tx, &s, "case_judged", &request(3), at)?;
        let raised = signals::get(tx, PROJECT, KEY)?;
        assert_eq!((raised.level, raised.response, raised.scores.blast), (3, "R5", 0), "the measured scores stay as measured");
        assert!(raised.escalated.is_some());
        let (level, kind): (u8, String) = tx.query_row("SELECT m.level, e.kind FROM supervisor_incident_signals m
            JOIN supervisor_incident_events e ON e.id = m.event WHERE m.fingerprint = ?1", [KEY],
            |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
        assert_eq!((level, kind.as_str()), (3, "escalated"));
        assert!(request(3).check(&raised, 0.9).is_err(), "a level 3 signal needs no second escalation");

        tx.execute("UPDATE supervisor_signals SET evidence = '[{\"table\":\"runs\",\"id\":\"later\"}]' WHERE fingerprint = ?1", [KEY])
            .map_err(sql)?;
        let lapsed = signals::get(tx, PROJECT, KEY)?;
        assert_eq!((lapsed.level, lapsed.escalated), (2, None), "new evidence is judged afresh");
        Ok(())
    }).unwrap();
}
