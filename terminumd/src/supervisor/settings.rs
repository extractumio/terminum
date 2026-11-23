// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-220: the project pack contains preferences. Command authority lives separately.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::tasks::ops::sql;
use super::{Detector, Mode, Rule, Threshold, outside_feedback::ReferenceBase, outside_indicators, probes};

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

    pub(super) fn reflection_agent(&self, tx: &Transaction, project: &str) -> Result<String, String> {
        let pool = crate::tasks::agents::of_project(tx, project)?;
        let own = [self.small.as_deref(), self.medium.as_deref(), self.strong.as_deref()];
        let independent = |id: &str| !own.contains(&Some(id));
        let agent = pool.iter().find(|a| Some(a.id.as_str()) == self.independent.as_deref() && independent(&a.id))
            .ok_or("Choose an independent pool agent that does not run the Supervisor's other tiers.")?;
        let supervisor = own.into_iter().rev().flatten().find_map(|id| pool.iter().find(|a| a.id == id));
        if let Some(supervisor) = supervisor
            && agent.kind == supervisor.kind
            && pool.iter().any(|a| independent(&a.id) && a.kind != supervisor.kind)
        {
            return Err("Choose an independent pool agent from a different model family.".into());
        }
        Ok(agent.id.clone())
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
    pub(super) priority_review_at: Option<i64>,
    pub allocation: BTreeMap<String, f64>,
    pub(super) allocation_review_at: Option<i64>,
    indicators: Indicators,
    pub(super) outside_indicators: BTreeMap<String, outside_indicators::Setting>,
    pub(super) surfaces: super::surfaces::Surfaces,
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
    git_file_line_cap: Option<u32>,
    gate_record: bool,
    code_host: bool,
    feedback: BTreeMap<String, ReferenceBase>,
}

impl Default for Probes {
    fn default() -> Self {
        Self { git: true, git_file_line_cap: None, gate_record: true, code_host: true, feedback: BTreeMap::new() }
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

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RuleSettings {
    pub(super) mode: Option<Mode>,
    pub(super) window_days: Option<u16>,
    pub(super) threshold: Option<Threshold>,
    pub(super) mute: Option<Mute>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(super) owner_live: bool,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Mute {
    pub(super) reason: String,
    pub(super) until: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) mode: Option<Mode>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Settings {
    pub(super) level: Level,
    pub tiers: Tiers,
    pub budgets: Budgets,
    pub charter: Charter,
    pub detectors: BTreeMap<String, RuleSettings>,
    probes: Probes,
    guardrails: Guardrails,
    language: Option<String>,
    pub(super) engine_board: Option<String>,
}

impl Settings {
    pub(super) fn threshold_band(&self) -> f64 {
        self.guardrails.threshold_percent / 100.0
    }

    pub(super) fn owner_live(&self, rule: Rule) -> bool {
        self.detectors.get(rule.id()).is_some_and(|config| config.owner_live)
    }

    pub(super) fn delivery(&self, rule: Rule, at: i64) -> Option<Mode> {
        self.detectors.get(rule.id()).and_then(|change| change.mute.as_ref())
            .filter(|mute| at < mute.until).map(|mute| mute.mode.unwrap_or(Mode::Off))
    }

    #[cfg(test)]
    pub(super) fn expired_since(&self, rule: Rule, previous: i64, at: i64) -> bool {
        self.detectors.get(rule.id()).and_then(|change| change.mute.as_ref())
            .is_some_and(|mute| previous < mute.until && mute.until <= at)
    }

    pub(super) fn decision_band(&self) -> (u32, f64) {
        (self.guardrails.decisions_minimum, self.guardrails.decisions_standard_errors)
    }

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
        self.delivery(rule, at).is_some()
    }

    fn validate(&self) -> Result<(), String> {
        if self.probes.git_file_line_cap == Some(0) { return Err("The Git source file line limit must be positive.".into()); }
        outside_indicators::validate(&self.charter.outside_indicators)?;
        if self.probes.feedback.len() > 32 || self.probes.feedback.iter().any(|(probe, base)| {
            !probes::probe_id(probe) || !base.valid()
        }) {
            return Err("Use up to 32 feedback probes, each with a probe ID and an HTTPS reference base ending in a slash.".into());
        }
        self.charter.surfaces.validate()?;
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
            if change.mute.as_ref().is_some_and(|m| m.reason.trim().is_empty() || m.until <= 0 || m.mode == Some(Mode::Live)) {
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

pub(super) fn recall_due(tx: &Transaction, project: &str, previous: i64, at: i64) -> Result<(i64, bool), String> {
    tx.query_row("SELECT revision, EXISTS (SELECT 1 FROM json_each(pack, '$.detectors') AS detector
        WHERE json_extract(detector.value, '$.mute.until') > ?2 AND json_extract(detector.value, '$.mute.until') <= ?3)
        FROM supervisor_settings WHERE project = ?1", (project, previous, at),
        |r| Ok((r.get(0)?, r.get(1)?))).optional().map(|row| row.unwrap_or_default()).map_err(sql)
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
    if settings.engine_board.as_deref().is_some_and(|id| crate::tasks::ops::check_project(tx, id).is_err()) {
        settings.engine_board = None;
    }
    Ok(Saved { revision, settings })
}

/// Imports preserve the destination's agent choices and its switch. A copied pack can't select a pool.
pub(super) fn write(
    tx: &Transaction, project: &str, revision: i64, settings: Settings, imported: bool,
) -> Result<Saved, String> {
    let source = if imported { SaveSource::Import } else { SaveSource::Owner };
    save(tx, project, revision, settings, source, crate::tasks::ops::now())
}

pub(super) fn write_at(
    tx: &Transaction, project: &str, revision: i64, settings: Settings, at: i64,
) -> Result<Saved, String> {
    save(tx, project, revision, settings, SaveSource::ConfigChange, at)
}

#[derive(Clone, Copy, PartialEq)]
enum SaveSource { Owner, Import, ConfigChange }

fn save(
    tx: &Transaction, project: &str, revision: i64, mut settings: Settings, source: SaveSource, at: i64,
) -> Result<Saved, String> {
    let previous = read(tx, project)?;
    if revision != previous.revision {
        return Err("Supervisor settings changed. Reload them and try again.".into());
    }
    let imported = source == SaveSource::Import;
    if imported {
        settings.tiers = previous.settings.tiers.clone();
        settings.level = previous.settings.level;
        settings.engine_board = previous.settings.engine_board.clone();
    } else {
        settings.tiers.retain_project(tx, project, true)?;
    }
    let attached = tx.query_row("SELECT EXISTS(SELECT 1 FROM project_supervisor
        WHERE project = ?1 AND (attachment IS NOT NULL OR enabled = 1))", [project], |r| r.get::<_, bool>(0)).map_err(sql)?;
    if source != SaveSource::ConfigChange {
        for (name, rule) in &mut settings.detectors {
            let prior = previous.settings.detectors.get(name);
            rule.owner_live = !imported && prior.is_some_and(|prior| prior.owner_live);
            if let Some(mute) = &mut rule.mute {
                let former = prior.and_then(|p| p.mute.as_ref());
                if imported || former != Some(mute) {
                    mute.owner = Some(crate::tasks::store::HUMAN.into());
                }
            }
        }
    }
    if let Some(engine) = &settings.engine_board {
        crate::tasks::ops::check_project(tx, engine)?;
    }
    settings.validate()?;
    let mut suspended = Vec::new();
    for &rule in Rule::ALL {
        let old = previous.settings.mode(rule);
        let mut new = settings.mode(rule);
        if attached && source != SaveSource::ConfigChange && new != Mode::Off
            && (old == Mode::Off || (imported && old == Mode::Shadow))
        {
            new = Mode::Shadow;
            let config = settings.detectors.entry(rule.id().into()).or_default();
            config.mode = Some(new);
            config.owner_live = false;
        }
        if old != new {
            if attached && source != SaveSource::ConfigChange && matches!((old, new), (Mode::Off, _) | (_, Mode::Off)) {
                suspended.push(rule);
            }
            if source == SaveSource::ConfigChange && matches!((old, new), (Mode::Off, _) | (_, Mode::Off)) {
                if new != Mode::Live { super::config_lifecycle::quiet_signals(tx, project, rule)?; }
            } else {
                super::config_lifecycle::mode_changed(tx, project, rule, new, at)?;
            }
        }
    }
    super::attach::retire_live(tx, project, &suspended)?;
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
