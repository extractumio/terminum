// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-284: a project's manager drafts its Supervisor's charter while the Supervisor is off, and reads what the
//! onboarding skill needs (terminum-agents.md AC-283). Everything but the charter stays the owner's.

use rusqlite::{OptionalExtension, Transaction};
use serde_json::{json, Value};

use super::settings::{self, Charter};
use crate::tasks::ops::sql;

/// What a manager's write answers while the collector runs: the owner alone changes a live yardstick.
pub(crate) const ON: &str = "The Supervisor is on: only the owner changes its charter, in the Supervisor tab.";

fn enabled(tx: &Transaction, project: &str) -> Result<bool, String> {
    Ok(tx.query_row("SELECT enabled FROM project_supervisor WHERE project = ?1", [project], |r| r.get::<_, bool>(0))
        .optional().map_err(sql)?.unwrap_or(false))
}

/// The Supervisor's part of a project's setup: its switch, level, tiers by agent name, revision, charter, and the
/// project-type templates and checkable never lines a draft starts from.
pub(crate) fn setup(tx: &Transaction, project: &str) -> Result<Value, String> {
    let saved = settings::read(tx, project)?;
    let s = &saved.settings;
    let name = |id: Option<&str>| id.map(|id| crate::tasks::agents::name_of(tx, id)).transpose();
    let [small, medium, strong, independent] = s.tiers.choices();
    Ok(json!({
        "enabled": enabled(tx, project)?,
        "level": s.level,
        "tiers": {"small": name(small)?, "medium": name(medium)?, "strong": name(strong)?, "independent": name(independent)?},
        "revision": saved.revision,
        "charter": s.charter,
        "templates": super::templates::choices(),
        "never_checks": super::breach::checks(),
    }))
}

/// Replaces the charter of revision `revision` with `text` (a charter's JSON), checked as the tab's save checks it;
/// the review dates and outside indicators stay, as do every setting outside the charter. Answers the new revision.
pub(crate) fn set_by_manager(tx: &Transaction, project: &str, revision: i64, text: &str) -> Result<i64, String> {
    if enabled(tx, project)? {
        return Err(ON.into());
    }
    let mut charter: Charter = serde_json::from_str(text)
        .map_err(|e| format!("charter: not a Supervisor charter ({e})."))?;
    let mut saved = settings::read(tx, project)?;
    let was = std::mem::replace(&mut saved.settings.charter, Charter::default());
    charter.priority_review_at = was.priority_review_at;
    charter.allocation_review_at = was.allocation_review_at;
    charter.outside_indicators = was.outside_indicators;
    saved.settings.charter = charter;
    // the owner's save path: the detectors, their mutes and owners, the tiers and the level are the ones read here
    Ok(settings::write(tx, project, revision, saved.settings, false)?.revision)
}

#[cfg(test)]
#[path = "charter_tests.rs"]
mod tests;
