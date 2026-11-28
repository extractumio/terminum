// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213, AC-238: a charter "never" is broken only when a live reading checks it. The owner ties a
//! line of the never list to a detector whose firing is a counted fact (`Rule::checked_fact`); level 3
//! from blast radius or reversibility alone is never a breach.

use std::collections::BTreeMap;

use serde::Serialize;

use super::Rule;

/// A detector the owner can tie to a never line, with its fact in the owner's words, for the tab.
#[derive(Serialize)]
pub(super) struct Check {
    detector: &'static str,
    fact: &'static str,
}

pub(super) fn checks() -> Vec<Check> {
    Rule::ALL.iter().filter_map(|rule| rule.checked_fact().map(|fact| Check { detector: rule.id(), fact })).collect()
}

/// Each tie names a checkable detector and a line of the never list as saved.
pub(super) fn validate(never: &[String], ties: &BTreeMap<String, String>) -> Result<(), String> {
    for (name, line) in ties {
        if !Rule::named(name).is_ok_and(|rule| rule.id() == name && rule.checked_fact().is_some()) {
            let names = checks().iter().map(|c| c.detector).collect::<Vec<_>>().join(", ");
            return Err(format!("{name} cannot check a never rule. Use one of: {names}."));
        }
        if !never.iter().any(|l| l == line) {
            return Err(format!("{name} is tied to a line that is not in the never list."));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "breach_tests.rs"]
pub(super) mod tests;
