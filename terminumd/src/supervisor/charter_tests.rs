// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-284: a manager's charter draft changes the charter alone, and only while the Supervisor is off.

use serde_json::{Value, json};

use super::{ON, OWNERS, set_by_manager, setup};
use crate::supervisor::engine_tests::{Fixture, NOW, PROJECT};
use crate::supervisor::{DAY, settings};

/// A project whose owner saved a level, tiers, a budget and a muted detector but no charter (revision 1).
fn owned(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.agent("sonnet");
    f.agent("codex");
    let pack: settings::Settings = serde_json::from_value(json!({
        "level": "advisor",
        "tiers": {"small": "sonnet", "independent": "codex"},
        "budgets": {"judgment_percent": 5.0},
        "detectors": {"flow.failed_repeat": {"mute": {"reason": "noisy on this board", "until": NOW + 7 * DAY}}},
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
    assert_eq!(after["charter"]["drafted_by_manager"], true, "the tab marks the draft");
    for kept in ["level", "tiers", "budgets", "detectors", "probes", "guardrails", "language", "engine_board"] {
        assert_eq!(after[kept], before[kept], "{kept} is the owner's");
    }
    assert_eq!(after["detectors"]["flow.failed_repeat"]["mute"]["owner"], before["detectors"]["flow.failed_repeat"]["mute"]["owner"],
        "the mute keeps its owner");
    // the manager revises its own draft; the owner's Save makes it the owner's, and the next draft is refused
    assert_eq!(f.store.with(|tx| set_by_manager(tx, PROJECT, 2, &charter())).unwrap(), 3);
    let reviewed: settings::Settings = serde_json::from_value(pack(&f)).unwrap();
    f.store.with(|tx| settings::write(tx, PROJECT, 3, reviewed, false).map(drop)).unwrap();
    assert_eq!(pack(&f)["charter"].get("drafted_by_manager"), None, "the owner's Save clears the mark");
    assert_eq!(f.store.with(|tx| set_by_manager(tx, PROJECT, 4, &charter())).unwrap_err(), OWNERS);
}

#[test]
fn ac284_the_owner_s_charter_or_the_supervisor_on_refuses_the_draft() {
    let f = owned("charter-on");
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (PROJECT, NOW));
    assert_eq!(f.store.with(|tx| set_by_manager(tx, PROJECT, 1, &charter())).unwrap_err(), ON);
    assert_eq!(pack(&f)["charter"]["mission"], "");
    // paused, its empty charter is the manager's to draft; a paused Supervisor's reviewed charter is not
    f.write("UPDATE project_supervisor SET enabled = 0 WHERE project = ?1", [PROJECT]);
    assert_eq!(f.store.with(|tx| set_by_manager(tx, PROJECT, 1, &charter())).unwrap(), 2);
    let g = owned("charter-owners");
    let mut pack_g: settings::Settings = serde_json::from_value(pack(&g)).unwrap();
    pack_g.charter = serde_json::from_value(json!({"allocation": {"Feature": 0.5, "Bug": 0.5}})).unwrap();
    g.store.with(|tx| settings::write(tx, PROJECT, 1, pack_g, false).map(drop)).unwrap();
    assert_eq!(g.store.with(|tx| set_by_manager(tx, PROJECT, 2, &charter())).unwrap_err(), OWNERS, "any owner's field makes it the owner's");
    let (_, gaps) = g.store.with(|tx| setup(tx, PROJECT, "the human, in the")).unwrap();
    assert_eq!(gaps[0].1, "the human, in the Supervisor tab (the owner's)", "the gap and the write agree");
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
    assert!(refused(1, r#"{"priority_review_at": 1}"#).contains("are the owner's"), "an owner's field is refused, not dropped");
    assert_eq!(pack(&f)["charter"]["mission"], "");
}

#[test]
fn ac284_setup_names_the_tiers_and_offers_the_templates() {
    let f = owned("charter-setup");
    let (_, gaps) = f.store.with(|tx| setup(tx, PROJECT, "the human, in the")).unwrap();
    assert_eq!(gaps[0], ("the Supervisor's charter is empty".to_string(), "you: set_supervisor_charter".to_string()));
    f.store.with(|tx| set_by_manager(tx, PROJECT, 1, &charter())).unwrap();
    let (shown, gaps) = f.store.with(|tx| setup(tx, PROJECT, "the human, in the")).unwrap();
    assert_eq!(gaps, [("the Supervisor's charter is a manager's draft".to_string(), "the human, in the Supervisor tab: review it, then Save".to_string()),
        ("Supervisor tiers unset: medium, strong".to_string(), "the human, in the Supervisor tab, from the pool's agents".to_string()),
        ("the Supervisor is off".to_string(), "the human, in the Supervisor tab: Level, then Enabled".to_string())], "its mission is drafted");
    assert_eq!(shown["enabled"], false);
    assert_eq!(shown["level"], "advisor");
    assert_eq!(shown["tiers"], json!({"small": "sonnet", "medium": null, "strong": null, "independent": "codex"}));
    assert_eq!(shown["revision"], 2);
    assert_eq!(shown["templates"].as_array().unwrap().len(), 4);
    assert!(shown["never_checks"].as_array().unwrap().iter().any(|c| c["detector"] == "arch.migration_edited"));
}
