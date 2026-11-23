// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-220: the project pack contains preferences. Command authority lives separately.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::tasks::ops::sql;
use super::{Detector, Mode, Rule, Threshold};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Level {
    Observer,
    Advisor,
    #[default]
    Operator,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Tiers {
    small: Option<String>,
    medium: Option<String>,
    strong: Option<String>,
    independent: Option<String>,
}

impl Tiers {
    pub(super) fn investigation(&self, level: u8) -> (&'static str, Option<&str>) {
        match level {
            3 => ("strong", self.strong.as_deref()),
            2 => ("medium", self.medium.as_deref()),
            _ => ("small", self.small.as_deref()),
        }
    }

    fn choices(&self) -> [Option<&str>; 4] {
        [self.small.as_deref(), self.medium.as_deref(), self.strong.as_deref(), self.independent.as_deref()]
    }

    pub fn is_empty(&self) -> bool {
        self.choices().into_iter().all(|id| id.is_none())
    }

    // A deleted agent never grants round authority to a replacement with the same name.
    fn retain_project(&mut self, tx: &Transaction, project: &str, reject: bool) -> Result<(), String> {
        let mut has = tx.prepare_cached("SELECT EXISTS(SELECT 1 FROM agents WHERE id = ?1 AND project = ?2)").map_err(sql)?;
        for (tier, slot) in [("Small", &mut self.small), ("Medium", &mut self.medium),
            ("Strong", &mut self.strong), ("Independent", &mut self.independent)]
        {
            if let Some(id) = slot.as_deref()
                && !has.query_row((id, project), |r| r.get::<_, bool>(0)).map_err(sql)?
            {
                if reject {
                    return Err(format!("{tier}: choose a tier agent from this project's pool."));
                }
                *slot = None;
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Budgets {
    pub judgment_percent: f64,
    pub fixes_percent: f64,
    pub judgment_floor_tokens: u64,
    pub investigation_reserve_percent: f64,
    pub judgment_ceiling: Option<f64>,
    pub fixes_ceiling: Option<f64>,
    pub daily_token_ceiling: Option<u64>,
    pub round_token_ceiling: Option<u64>,
}

impl Default for Budgets {
    fn default() -> Self {
        Self { judgment_percent: 3., fixes_percent: 10., judgment_floor_tokens: 1_000_000,
            investigation_reserve_percent: 30., judgment_ceiling: None, fixes_ceiling: None,
            daily_token_ceiling: None, round_token_ceiling: None }
    }
}

impl Budgets {
    fn investigation_room(&self) -> bool {
        self.daily_token_ceiling != Some(0) && self.round_token_ceiling != Some(0)
            && self.judgment_ceiling != Some(0.)
    }

    fn valid(&self) -> bool {
        [self.judgment_percent, self.fixes_percent, self.investigation_reserve_percent]
            .into_iter().all(|v| (0.0..=100.0).contains(&v))
            && [self.judgment_ceiling, self.fixes_ceiling]
                .into_iter().flatten().all(|v| v.is_finite() && v >= 0.)
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Charter {
    mission: String,
    users: Vec<String>,
    risk_appetite: String,
    never: Vec<String>,
    priorities: Vec<String>,
    priority_review_at: Option<i64>,
    pub allocation: BTreeMap<String, f64>,
    allocation_review_at: Option<i64>,
    indicators: Indicators,
    outside_indicators: BTreeMap<String, String>,
    surfaces: Surfaces,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Surfaces {
    t2: Vec<String>,
    t3: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Indicators {
    involvement: bool,
    stability: bool,
    lead_time: bool,
    releases: bool,
    backlog: bool,
    cognitive_load: bool,
    operations: bool,
}

impl Default for Indicators {
    fn default() -> Self {
        Self { involvement: true, stability: true, lead_time: true, releases: true,
            backlog: true, cognitive_load: true, operations: true }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Probes {
    git: bool,
    gate_record: bool,
    code_host: bool,
}

impl Default for Probes {
    fn default() -> Self {
        Self { git: true, gate_record: true, code_host: true }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Guardrails {
    threshold_percent: f64,
    decisions_standard_errors: f64,
    decisions_minimum: u32,
}

impl Default for Guardrails {
    fn default() -> Self {
        Self { threshold_percent: 50., decisions_standard_errors: 2., decisions_minimum: 30 }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RuleSettings {
    mode: Option<Mode>,
    window_days: Option<u16>,
    threshold: Option<Threshold>,
    mute: Option<Mute>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mute {
    reason: String,
    until: i64,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Settings {
    level: Level,
    pub tiers: Tiers,
    pub budgets: Budgets,
    pub charter: Charter,
    pub detectors: BTreeMap<String, RuleSettings>,
    probes: Probes,
    guardrails: Guardrails,
    language: Option<String>,
}

impl Settings {
    pub(super) fn investigation_room(&self) -> bool {
        self.budgets.investigation_room()
    }

    pub(super) fn investigation_charter(&self) -> [&[String]; 2] {
        [&self.charter.never, &self.charter.priorities]
    }

    pub(super) fn enabled_indicators(&self) -> [bool; 7] {
        let i = &self.charter.indicators;
        [i.involvement, i.stability, i.lead_time, i.releases, i.backlog, i.cognitive_load, i.operations]
    }

    pub fn mode(&self, rule: Rule) -> Mode {
        self.detectors.get(rule.id()).and_then(|change| change.mode).unwrap_or(Mode::Live)
    }

    pub fn detector(&self, rule: Rule) -> Detector {
        let mut detector = Detector::for_rule(rule);
        let change = self.detectors.get(rule.id());
        detector.mode = self.mode(rule);
        if let Some(days) = change.and_then(|s| s.window_days) {
            detector.window_days = days;
        }
        if let Some(threshold) = change.and_then(|s| s.threshold) {
            detector.threshold = threshold;
        }
        if rule == Rule::Allocation {
            detector.allocation = self.charter.allocation.clone();
        }
        detector
    }

    pub fn muted(&self, rule: Rule, at: i64) -> bool {
        self.detectors.get(rule.id()).and_then(|change| change.mute.as_ref()).is_some_and(|mute| at < mute.until)
    }

    fn validate(&self) -> Result<(), String> {
        if !self.budgets.valid() {
            return Err("Invalid Supervisor budget.".into());
        }
        if !(0.0..=100.0).contains(&self.guardrails.threshold_percent)
            || !self.guardrails.decisions_standard_errors.is_finite()
            || self.guardrails.decisions_standard_errors <= 0.
            || self.guardrails.decisions_minimum < 30
        {
            return Err("Invalid Supervisor guardrail.".into());
        }
        if !self.detector(Rule::Allocation).valid() {
            return Err("Allocation shares must use known task types and add up to 1.".into());
        }
        for (name, change) in &self.detectors {
            let rule = Rule::named(name)?;
            if !rule.configurable() {
                return Err("The collector heartbeat watchdog uses a fixed threshold.".into());
            }
            if rule.id() != name || !self.detector(rule).valid() {
                return Err(format!("{name}: invalid Supervisor detector settings."));
            }
            if change.mute.as_ref().is_some_and(|m| m.reason.trim().is_empty() || m.until <= 0) {
                return Err(format!("{name}: a detector mute needs a reason and an end time."));
            }
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub(super) struct Saved {
    pub revision: i64,
    pub settings: Settings,
}

/// Read only the live authorization level. A missing field in an older pack uses the default.
pub(crate) fn level(tx: &Transaction, project: &str) -> Result<Level, String> {
    let value: Option<String> = tx.query_row("SELECT pack -> 'level' FROM supervisor_settings WHERE project = ?1",
        [project], |r| r.get(0)).optional().map_err(sql)?.flatten();
    value.map(|json| serde_json::from_str(&json).map_err(|_| "Invalid saved Supervisor level.".into()))
        .transpose().map(Option::unwrap_or_default)
}

pub(super) fn revision(tx: &Transaction, project: &str) -> Result<i64, String> {
    tx.query_row("SELECT revision FROM supervisor_settings WHERE project = ?1", [project], |row| row.get(0))
        .optional().map(|revision| revision.unwrap_or_default()).map_err(sql)
}

pub(super) fn read(tx: &Transaction, project: &str) -> Result<Saved, String> {
    let saved: Option<(i64, String)> = tx.query_row(
        "SELECT revision, pack FROM supervisor_settings WHERE project = ?1",
        [project], |r| Ok((r.get(0)?, r.get(1)?)),
    ).optional().map_err(sql)?;
    let Some((revision, json)) = saved else {
        return Ok(Saved { revision: 0, settings: Settings::default() });
    };
    let mut settings: Settings = serde_json::from_str(&json).map_err(|_| "Invalid saved Supervisor settings.")?;
    settings.tiers.retain_project(tx, project, false)?;
    Ok(Saved { revision, settings })
}

/// Imports preserve the destination's agent choices and its switch. A copied pack can't select a pool.
pub(super) fn write(
    tx: &Transaction, project: &str, revision: i64, mut settings: Settings, imported: bool,
) -> Result<Saved, String> {
    let previous = read(tx, project)?;
    if revision != previous.revision {
        return Err("Supervisor settings changed. Reload them and try again.".into());
    }
    if imported {
        settings.tiers = previous.settings.tiers.clone();
        settings.level = previous.settings.level;
    } else {
        settings.tiers.retain_project(tx, project, true)?;
    }
    settings.validate()?;
    let attached = tx.query_row("SELECT EXISTS(SELECT 1 FROM project_supervisor
        WHERE project = ?1 AND (attachment IS NOT NULL OR enabled = 1))", [project], |r| r.get::<_, bool>(0)).map_err(sql)?;
    if attached {
        let mut suspended = Vec::new();
        for &rule in Rule::ALL {
            let old = previous.settings.mode(rule);
            let new = settings.mode(rule);
            if old != Mode::Off && new == Mode::Off {
                suspended.push(rule);
            }
            if new != Mode::Off && (old == Mode::Off || (imported && old == Mode::Shadow)) {
                settings.detectors.entry(rule.id().into()).or_default().mode = Some(Mode::Shadow);
            }
        }
        super::attach::retire_live(tx, project, &suspended)?;
    }
    let revision = revision + 1;
    let pack = serde_json::to_string(&settings).map_err(|_| "Could not encode Supervisor settings.")?;
    if pack.len() > 64 * 1024 {
        return Err("Supervisor preferences must fit in 64 KiB.".into());
    }
    tx.execute("INSERT INTO supervisor_settings (project, revision, pack) VALUES (?1, ?2, ?3)
        ON CONFLICT (project) DO UPDATE SET revision = excluded.revision, pack = excluded.pack",
        (project, revision, pack)).map_err(sql)?;
    Ok(Saved { revision, settings })
}
