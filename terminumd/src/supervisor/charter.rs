// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-284: a project's manager drafts its Supervisor's charter while the Supervisor is off, and reads what the
//! onboarding skill needs (terminum-agents.md AC-283). Everything but the charter stays the owner's.

use rusqlite::Transaction;
use serde_json::{json, Value};

use super::settings::{self, Charter};

/// The tiers in `Tiers::choices`' order, as the Supervisor tab names them.
pub(crate) const TIERS: [&str; 4] = ["small", "medium", "strong", "independent"];

/// What a manager's write answers while the collector runs: the owner alone changes a live yardstick.
pub(crate) const ON: &str = "The Supervisor is on: only the owner changes its charter, in the Supervisor tab.";

fn enabled(tx: &Transaction, project: &str) -> Result<bool, String> {
    Ok(super::rounds::enabled(tx, project)?.is_some())
}

/// Whether a manager may write over `charter`: it is empty, or the manager's draft the owner has not saved.
fn draftable(charter: &Charter) -> bool {
    charter.drafted_by_manager || serde_json::to_value(charter).ok() == serde_json::to_value(Charter::default()).ok()
}

/// The Supervisor's part of a project's setup: its switch, level, tiers by agent name, revision, charter, the
/// project-type templates and checkable never lines a draft starts from, and its gaps as (what, who closes it).
pub(crate) fn setup(tx: &Transaction, project: &str, human: &str) -> Result<(Value, Vec<(String, String)>), String> {
    let saved = settings::read(tx, project)?;
    let s = &saved.settings;
    let on = enabled(tx, project)?;
    let (mut tiers, mut unset) = (serde_json::Map::new(), Vec::new());
    for (tier, id) in TIERS.into_iter().zip(s.tiers.choices()) {
        if id.is_none() { unset.push(tier); }
        tiers.insert(tier.into(), json!(id.map(|id| crate::tasks::agents::name_of(tx, id)).transpose()?));
    }
    let mut gaps = Vec::new();
    if s.charter.drafted_by_manager {
        gaps.push(("the Supervisor's charter is a manager's draft".into(), format!("{human} Supervisor tab: review it, then Save")));
    } else if s.charter.mission.trim().is_empty() {
        gaps.push(("the Supervisor's charter is empty".into(), if on { format!("{human} Supervisor tab (it is on)") }
            else if draftable(&s.charter) { "you: set_supervisor_charter".into() } else { format!("{human} Supervisor tab (the owner's)") }));
    }
    if !unset.is_empty() {
        gaps.push((format!("Supervisor tiers unset: {}", unset.join(", ")), format!("{human} Supervisor tab, from the pool's agents")));
    }
    if !on {
        gaps.push(("the Supervisor is off".into(), format!("{human} Supervisor tab: Level, then Enabled")));
    }
    let shown = json!({
        "enabled": on, "level": s.level, "tiers": tiers, "revision": saved.revision, "charter": s.charter,
        "templates": super::templates::choices(), "never_checks": super::breach::checks(),
    });
    Ok((shown, gaps))
}

/// What a manager's write answers when the owner has a charter: a draft never replaces the owner's yardstick.
pub(crate) const OWNERS: &str = "The charter is the owner's: only the owner changes it, in the Supervisor tab.";

/// Writes `text` (a charter's JSON) as the charter of revision `revision`, checked as the tab's save checks it,
/// while the Supervisor is off and its charter is empty or a manager's draft the owner has not saved; every
/// setting outside the charter stays. The tab marks the draft until the owner's Save. Answers the new revision.
pub(crate) fn set_by_manager(tx: &Transaction, project: &str, revision: i64, text: &str) -> Result<i64, String> {
    if enabled(tx, project)? {
        return Err(ON.into());
    }
    let charter: Charter = serde_json::from_str(text)
        .map_err(|e| format!("charter: not a Supervisor charter ({e})."))?;
    if charter.priority_review_at.is_some() || charter.allocation_review_at.is_some() || !charter.outside_indicators.is_empty() {
        return Err("charter: its review dates and outside indicators are the owner's; leave them out.".into());
    }
    let mut saved = settings::read(tx, project)?;
    if !draftable(&saved.settings.charter) {
        return Err(OWNERS.into());
    }
    saved.settings.charter = charter;
    // the owner's save path: the detectors, their mutes and owners, the tiers and the level are the ones read here
    Ok(settings::write_draft(tx, project, revision, saved.settings)?.revision)
}

#[cfg(test)]
#[path = "charter_tests.rs"]
mod tests;
