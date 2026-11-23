// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213, AC-238: a charter "never" is broken only when a live reading checks it. The owner ties a
//! line of the never list to a detector whose firing is a discrete fact, not a statistic; level 3 from
//! blast radius or reversibility alone is never a breach.

use std::collections::BTreeMap;

use serde::Serialize;

use super::{Detector, Finding, Rule};

/// A detector whose reading is a counted fact in the store or a probe's sample, the reading's metric
/// that carries it, and the fact in the owner's words.
#[derive(Serialize)]
pub(super) struct Check {
    #[serde(skip)]
    rule: Rule,
    #[serde(skip)]
    metric: &'static str,
    detector: &'static str,
    fact: &'static str,
}

/// Only these can establish a charter breach; a statistic, or a fact that fires most weeks in an
/// active project (a lockfile gaining a package), cannot.
pub(super) const CHECKS: &[Check] = &[
    Check { rule: Rule::MigrationEdited, metric: "existing_migration_edits", detector: "arch.migration_edited",
        fact: "An existing migration file is edited" },
];

fn checkable(name: &str) -> bool {
    CHECKS.iter().any(|c| c.detector == name)
}

/// Each tie names a checkable detector and a line of the never list as saved.
pub(super) fn validate(never: &[String], checks: &BTreeMap<String, String>) -> Result<(), String> {
    for (name, line) in checks {
        if !checkable(name) {
            let names = CHECKS.iter().map(|c| c.detector).collect::<Vec<_>>().join(", ");
            return Err(format!("{name} cannot check a never rule. Use one of: {names}."));
        }
        if !never.iter().any(|l| l == line) {
            return Err(format!("{name} is tied to a line that is not in the never list."));
        }
    }
    Ok(())
}

/// The owner's never line a finding breaks: its detector is tied to one and the finding is that
/// detector's fact. A finding always carries its evidence rows, which the signal keeps.
pub(super) fn line(detector: &Detector, found: &Finding) -> Option<String> {
    let line = detector.never.as_ref()?;
    CHECKS.iter().any(|c| c.rule == detector.rule && c.metric == found.metric).then(|| line.clone())
}

#[cfg(test)]
#[test]
fn each_check_names_its_own_detector() {
    for c in CHECKS { assert_eq!(c.rule.id(), c.detector); }
}

#[cfg(test)]
#[path = "breach_tests.rs"]
mod tests;
