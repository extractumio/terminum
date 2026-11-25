// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-284: a manager's charter draft changes the charter alone, and only while the Supervisor is off.

use serde_json::{Value, json};

use super::{ON, set_by_manager, setup};
use crate::supervisor::engine_tests::{Fixture, NOW, PROJECT};
use crate::supervisor::{DAY, settings};

/// A project whose owner saved a level, tiers, a budget and a muted detector (revision 1).
fn owned(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.agent("sonnet");
    f.agent("codex");
    let pack: settings::Settings = serde_json::from_value(json!({
        "level": "advisor",
        "tiers": {"small": "sonnet", "independent": "codex"},
        "budgets": {"judgment_percent": 5.0},
        "detectors": {"flow.failed_repeat": {"mute": {"reason": "noisy on this board", "until": NOW + 7 * DAY}}},
        "charter": {"mission": "old", "priority_review_at": NOW},
    })).unwrap();
    f.store.with(|tx| settings::write(tx, PROJECT, 0, pack, false).map(drop)).unwrap();
    f
}

fn pack(f: &Fixture) -> Value {
    f.store.with(|tx| Ok(serde_json::to_value(settings::read(tx, PROJECT)?.settings).unwrap())).unwrap()
}

fn charter() -> String {
    json!({
        "mission": "A tool that keeps invoices.",
        "users": ["Accountants"],
        "never": ["Never edit an existing store migration."],
        "never_checks": {"arch.migration_edited": "Never edit an existing store migration."},
        "priorities": ["Correct totals"],
        "allocation": {"Feature": 0.4, "Bug": 0.4, "Debt": 0.2},
    }).to_string()
}

#[test]
fn ac284_a_manager_s_charter_changes_the_charter_alone() {
    let f = owned("charter-alone");
    let before = pack(&f);
    let revision = f.store.with(|tx| set_by_manager(tx, PROJECT, 1, &charter())).unwrap();
    assert_eq!(revision, 2);
    let after = pack(&f);
    assert_eq!(after["charter"]["mission"], "A tool that keeps invoices.");
    assert_eq!(after["charter"]["never_checks"]["arch.migration_edited"], "Never edit an existing store migration.");
    assert_eq!(after["charter"]["priority_review_at"], NOW, "the owner's review date stays");
    for kept in ["level", "tiers", "budgets", "detectors", "probes", "guardrails", "language", "engine_board"] {
        assert_eq!(after[kept], before[kept], "{kept} is the owner's");
    }
    assert_eq!(after["detectors"]["flow.failed_repeat"]["mute"]["owner"], before["detectors"]["flow.failed_repeat"]["mute"]["owner"],
        "the mute keeps its owner");
}

#[test]
fn ac284_the_supervisor_on_refuses_the_draft() {
    let f = owned("charter-on");
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (PROJECT, NOW));
    assert_eq!(f.store.with(|tx| set_by_manager(tx, PROJECT, 1, &charter())).unwrap_err(), ON);
    assert_eq!(pack(&f)["charter"]["mission"], "old");
    // switched off again, the draft is the manager's to write
    f.write("UPDATE project_supervisor SET enabled = 0 WHERE project = ?1", [PROJECT]);
    assert_eq!(f.store.with(|tx| set_by_manager(tx, PROJECT, 1, &charter())).unwrap(), 2);
}

#[test]
fn ac284_a_stale_or_invalid_charter_keeps_nothing() {
    let f = owned("charter-invalid");
    let refused = |revision, text: &str| f.store.with(|tx| set_by_manager(tx, PROJECT, revision, text)).unwrap_err();
    assert_eq!(refused(0, &charter()), "Supervisor settings changed. Reload them and try again.");
    assert!(refused(1, "[1]").starts_with("charter: not a Supervisor charter"), "no object");
    assert!(refused(1, r#"{"level": "operator"}"#).starts_with("charter: not a Supervisor charter"), "the level is no charter field");
    assert!(refused(1, r#"{"never": ["x"], "never_checks": {"flow.starved": "x"}}"#).contains("cannot check a never rule"));
    assert!(refused(1, r#"{"allocation": {"Feature": 0.9}}"#).starts_with("Allocation shares"));
    assert_eq!(pack(&f)["charter"]["mission"], "old");
}

#[test]
fn ac284_setup_names_the_tiers_and_offers_the_templates() {
    let f = owned("charter-setup");
    let shown = f.store.with(|tx| setup(tx, PROJECT)).unwrap();
    assert_eq!(shown["enabled"], false);
    assert_eq!(shown["level"], "advisor");
    assert_eq!(shown["tiers"], json!({"small": "sonnet", "medium": null, "strong": null, "independent": "codex"}));
    assert_eq!(shown["revision"], 1);
    assert_eq!(shown["templates"].as_array().unwrap().len(), 4);
    assert!(shown["never_checks"].as_array().unwrap().iter().any(|c| c["detector"] == "arch.migration_edited"));
}
