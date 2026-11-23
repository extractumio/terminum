// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213, AC-221, AC-238: a live reading establishes a charter breach only through the owner's tie.

use serde_json::json;

use crate::supervisor::engine_tests::{Fixture, NOW, PROJECT};
use crate::tasks::ops::{self, rows, sql};

use super::super::{DAY, Mode, Rule, config_lifecycle, evaluate, fix_tasks, probe_arch::Architecture, probes, register, settings};

const NEVER: &str = "Lose user data.";

fn attached(f: &Fixture, checks: serde_json::Value) {
    f.write("UPDATE projects SET dir = ?1 WHERE id = ?2", (f.path.parent().unwrap().to_str().unwrap(), PROJECT));
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (PROJECT, NOW - 2 * DAY));
    let pack = json!({"budgets":{"fixes_ceiling":0.0}, "charter":{"never":[NEVER, "Break a release."], "never_checks":checks}});
    f.store.with(|tx| settings::write(tx, PROJECT, 0, serde_json::from_value(pack).unwrap(), false).map(drop)).unwrap();
}

fn edited_migration(f: &Fixture) {
    let arch = Architecture { since: NOW - 90 * DAY, migrations: vec![(NOW - DAY, "schema1.sql".into())], ..Architecture::default() };
    f.store.with(|tx| probes::publish_sample(tx, probes::ARCH, NOW - 10, serde_json::to_string(&arch).unwrap())).unwrap();
}

fn live(f: &Fixture) -> super::super::Evaluation {
    let mut detector = f.store.with(|tx| Ok(settings::read(tx, PROJECT)?.settings.detector(Rule::MigrationEdited))).unwrap();
    detector.mode = Mode::Live;
    evaluate(&f.store, PROJECT, NOW, &[detector]).unwrap()
}

/// A level 3 signal from its reversibility alone, with a problem that needs a fix.
fn reversible(f: &Fixture) {
    let scores = json!({"impact":2,"blast":0,"reversibility":3,"confidence":1.0,"charter_breach":false,
        "impact_basis":{"ranked":true,"shares":[{"week":1,"complete":true,"unit":"failure_events","amount":30.0,"total":100.0,"noise":0.0}]}});
    let measure = json!({"value":4,"threshold":2,"baseline":2,"window_start":NOW-DAY,"window_end":NOW});
    f.write("INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind, subject_id, metric,
        evidence, measure, scores, first_seen, last_seen, count, state)
        VALUES ('reversible', ?1, 'flow.failed_repeat', 'task', 'A', 'failed_runs', ?2, ?3, ?4, ?5, ?5, 1, 'new')",
        (PROJECT, r#"[{"table":"events","id":"event-a"}]"#, measure.to_string(), scores.to_string(), NOW));
    let draft = serde_json::from_value(json!({"title":"Irreversible repeat", "lens":"flow", "hypothesis":"A repeat loses work.",
        "disproof":"One attempt also loses it.", "confidence":0.8, "effort":1, "signals":["reversible"],
        "fix_options":[{"layer":"check", "action":"Stop the repeat."}]})).unwrap();
    f.store.with(|tx| register::open(tx, PROJECT, draft, NOW).map(drop)).unwrap();
}

#[test]
fn ac213_an_edited_migration_breaks_a_never_only_when_the_owner_tied_it() {
    let f = Fixture::new("breach-untied");
    attached(&f, json!({}));
    edited_migration(&f);
    let found = &live(&f).detectors[0].findings[0];
    assert_eq!((found.subject_id.as_str(), found.charter_breach, found.charter_never.as_deref()), ("schema1.sql", false, None));
    assert!(found.scores.impact < 3, "an untied fact is an ordinary signal");
    f.store.with(|tx| {
        assert!(!config_lifecycle::protected(tx, PROJECT, Rule::MigrationEdited, &settings::read(tx, PROJECT)?.settings)?);
        Ok(())
    }).unwrap();

    let f = Fixture::new("breach-tied");
    attached(&f, json!({"arch.migration_edited": NEVER}));
    edited_migration(&f);
    let evaluation = live(&f);
    let found = &evaluation.detectors[0].findings[0];
    assert_eq!((found.charter_breach, found.charter_never.as_deref(), found.scores.impact), (true, Some(NEVER), 3));
    evaluation.record(&f.store).unwrap();
    let signal = super::super::signals(&f.store, PROJECT).unwrap().into_iter().find(|s| s.detector == "arch.migration_edited").unwrap();
    assert!(signal.charter_breach, "holds and quiet hours read this flag (AC-221, AC-228)");
    assert_eq!(signal.charter_never.as_deref(), Some(NEVER));
    assert_eq!(signal.evidence.len(), 1, "the probe sample that established it");
    f.store.with(|tx| {
        // The tie keeps the owner's coverage before the detector ever fires.
        assert!(config_lifecycle::protected(tx, PROJECT, Rule::MigrationEdited, &settings::read(tx, PROJECT)?.settings)?);
        Ok(())
    }).unwrap();
}

#[test]
fn ac238_at_the_fixes_ceiling_only_a_checked_breach_gets_its_fix() {
    let f = Fixture::new("breach-ceiling");
    attached(&f, json!({"arch.migration_edited": NEVER}));
    reversible(&f);
    f.store.with(|tx| {
        register::advance(tx, PROJECT, NOW)?;
        let changed = fix_tasks::advance(tx, PROJECT, NOW)?;
        assert_eq!(changed.len(), 1);
        assert_eq!(ops::get(tx, &changed[0])?.status, "WAIT FOR HUMAN", "level 3 from reversibility waits on the budget card");
        let ranked = register::ranked(tx, PROJECT, NOW, false)?;
        assert!(!ranked[0].exception && ranked[0].breaches.is_empty());
        Ok(())
    }).unwrap();

    edited_migration(&f);
    live(&f).record(&f.store).unwrap();
    f.store.with(|tx| {
        register::advance(tx, PROJECT, NOW + 1)?;
        let changed = fix_tasks::advance(tx, PROJECT, NOW + 1)?;
        assert_eq!(changed.len(), 1);
        let task = ops::get(tx, &changed[0])?;
        assert_eq!(task.severity, "P0");
        let text = ops::text_of(tx, &task.id)?;
        assert!(text.contains("exempt from the spend limit") && text.contains(NEVER) && text.contains("detector `arch.migration_edited`"), "{text}");
        let marks: Vec<bool> = rows(tx, "SELECT exception FROM supervisor_task_marks WHERE task = ?1", [&task.key], |r| r.get(0))?;
        assert_eq!(marks, [true]);
        let breach = register::ranked(tx, PROJECT, NOW + 1, false)?.into_iter().find(|p| p.exception).unwrap();
        assert_eq!((breach.breaches.len(), breach.breaches[0].never.as_str(), breach.breaches[0].detector.as_str()),
            (1, NEVER, "arch.migration_edited"));
        let reversible: i64 = tx.query_row("SELECT count(*) FROM supervisor_problem_tasks t JOIN supervisor_problem_signals y
            USING (problem) WHERE y.fingerprint = 'reversible'", [], |r| r.get(0)).map_err(sql)?;
        assert_eq!(reversible, 0, "the ordinary level 3 problem still waits");
        Ok(())
    }).unwrap();
}

#[test]
fn ac213_a_tie_names_a_checkable_detector_and_a_saved_never_line() {
    let f = Fixture::new("breach-validation");
    for (checks, error) in [
        (json!({"arch.dependency": NEVER}), "arch.dependency cannot check a never rule. Use one of: arch.migration_edited."),
        (json!({"git_migrations": NEVER}), "git_migrations cannot check a never rule. Use one of: arch.migration_edited."),
        (json!({"arch.migration_edited": "Lose user data"}), "arch.migration_edited is tied to a line that is not in the never list."),
    ] {
        let pack = json!({"charter":{"never":[NEVER], "never_checks":checks}});
        let saved = f.store.with(|tx| settings::write(tx, PROJECT, 0, serde_json::from_value(pack).unwrap(), false).map(drop));
        assert_eq!(saved.unwrap_err(), error);
    }
}
