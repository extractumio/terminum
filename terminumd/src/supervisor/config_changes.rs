// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-253 and AC-254: the proposed diff cannot grant its own authority.

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::tasks::{comments, ops::{self, NewTask, rows, sql}, store::HUMAN};

use super::{DAY, Mode, Rule, Threshold, config_replay::{self, Receipt}, rounds, settings::{self, Mute, RuleSettings, Saved}, signals};

pub(crate) const PREVIEW: &str = "replay_config_change";
pub(crate) const PROPOSE: &str = "propose_config_change";
const MAX_MUTE: i64 = 14 * DAY;
const STALE: &str = "Supervisor settings changed. Replay the configuration change again.";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Proposal {
    detector: String,
    action: Change,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Change {
    Tune { threshold: Option<Threshold>, window_days: Option<u16> },
    Enable,
    Promote,
    Mute { mode: Quiet, reason: String, until: i64 },
    Remove,
    OwnerReview { target: OwnerTarget },
    Engine { requirement: EngineRequirement },
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Quiet { Off, Shadow, Digest }

impl From<Quiet> for Mode {
    fn from(mode: Quiet) -> Self {
        match mode { Quiet::Off => Self::Off, Quiet::Shadow => Self::Shadow, Quiet::Digest => Self::Digest }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum OwnerTarget { Page, Charter, SurfaceMap }

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EngineRequirement { Query, Tool, ProbeKind, RecordedField }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Submission {
    change: Proposal,
    replay: Receipt,
}

#[derive(PartialEq)]
enum Class { Tune, Extend, Owner, Engine }

impl Class {
    fn label(&self) -> &'static str {
        match self { Self::Tune => "tune", Self::Extend => "extend", Self::Owner => "owner", Self::Engine => "engine" }
    }
}

pub(super) fn rule(name: &str) -> Result<Rule, String> {
    let rule = Rule::named(name)?;
    if !rule.configurable() || rule.id() != name {
        return Err("Choose a configurable Supervisor detector ID.".into());
    }
    Ok(rule)
}

fn near(value: f64, anchor: f64, fraction: f64) -> bool {
    (value - anchor).abs() <= anchor * fraction
}

fn in_band(rule: Rule, before: &super::Detector, after: &super::Detector, band: f64) -> bool {
    let default = super::Detector::for_rule(rule);
    let threshold = if before.threshold == after.threshold { true } else {
        match (default.threshold, after.threshold) {
            (Threshold::Absolute(a), Threshold::Absolute(b)) |
            (Threshold::Change { by: a }, Threshold::Change { by: b }) => near(b, a, band),
            (Threshold::Baseline { percentile: a, floor: x }, Threshold::Baseline { percentile: b, floor: y }) =>
                near(f64::from(b), f64::from(a), band) && near(y, x, band),
            _ => false,
        }
    };
    threshold && (before.window_days == after.window_days
        || near(f64::from(after.window_days), f64::from(default.window_days), band))
}

struct Prepared {
    rule: Rule,
    before: Option<RuleSettings>,
    next: Option<RuleSettings>,
    class: Class,
    receipt: Receipt,
    revision: i64,
}

fn prepare(tx: &Transaction, project: &str, author: &str, change: &Proposal, as_of: i64, at: i64) -> Result<Prepared, String> {
    let rule = rule(&change.detector)?;
    let mut saved = settings::read(tx, project)?;
    let before = saved.settings.detectors.get(rule.id()).cloned();
    let mut next = before.clone().unwrap_or_default();
    let previous_detector = saved.settings.detector(rule);
    let class = match &change.action {
        Change::Tune { threshold, window_days } => {
            if previous_detector.mode == Mode::Off { return Err("Enable the detector in shadow before tuning it.".into()); }
            if threshold.is_none_or(|t| t == previous_detector.threshold) &&
                window_days.is_none_or(|days| days == previous_detector.window_days)
            {
                return Err("Change the detector threshold or window.".into());
            }
            if let Some(threshold) = threshold { next.threshold = Some(*threshold); }
            if let Some(days) = window_days { next.window_days = Some(*days); }
            Class::Tune
        }
        Change::Promote => return Err("Use the owner key to promote a judged shadow detector.".into()),
        Change::Enable => {
            if previous_detector.mode != Mode::Off { return Err("Only an off detector can start a new shadow trial.".into()); }
            next.mode = Some(Mode::Shadow);
            next.owner_live = false;
            next.mute = None;
            Class::Extend
        }
        Change::Mute { mode, reason, until } => {
            if reason.is_empty() || reason.len() > 1000 || *until <= at || *until > at.saturating_add(365 * DAY) {
                return Err("Give the mute a reason and an end time in the next year.".into());
            }
            if previous_detector.mode != Mode::Live {
                return Err("Only a live detector can have its delivery muted.".into());
            }
            let renewal = next.mute.as_ref().is_some_and(|mute| mute.until > at - MAX_MUTE);
            next.mute = Some(Mute { reason: reason.clone(), until: *until, owner: Some(author.into()), mode: Some((*mode).into()) });
            if renewal || *until - as_of > MAX_MUTE {
                Class::Owner
            } else {
                Class::Tune
            }
        }
        Change::Remove => {
            if previous_detector.mode == Mode::Off { return Err("That detector is already off.".into()); }
            next.mode = Some(Mode::Off);
            next.owner_live = false;
            next.mute = None;
            Class::Owner
        }
        Change::OwnerReview { .. } => Class::Owner,
        Change::Engine { .. } => Class::Engine,
    };
    let next = match change.action {
        Change::OwnerReview { .. } | Change::Engine { .. } => before.clone(),
        _ => Some(next),
    };
    if let Some(config) = &next {
        saved.settings.detectors.insert(rule.id().into(), config.clone());
    }
    let mut next_detector = saved.settings.detector(rule);
    if matches!(change.action, Change::Mute { mode: Quiet::Off, .. }) { next_detector.mode = Mode::Off; }
    if !next_detector.valid() { return Err("Invalid Supervisor threshold or measurement window.".into()); }
    let receipt = config_replay::run(tx, project, as_of, saved.revision, change, &previous_detector, &next_detector)?;

    let class = match class {
        Class::Tune | Class::Extend if open_breaker(tx, project, rule)? ||
            super::config_lifecycle::protected(tx, project, rule, &saved.settings)? || receipt.quiets() ||
            matches!(change.action, Change::Mute { mode: Quiet::Off, .. }) ||
            (matches!(change.action, Change::Tune { .. }) && paces_by_samples(rule)) ||
            (!receipt.measured() && matches!(change.action, Change::Mute { .. })) ||
            !in_band(rule, &previous_detector, &next_detector, saved.settings.threshold_band()) => Class::Owner,
        class => class,
    };
    Ok(Prepared { rule, before, next, class, receipt, revision: saved.revision })
}

fn paces_by_samples(rule: Rule) -> bool {
    matches!(rule, Rule::QueueGrowth | Rule::LimitNear | Rule::StalePlan | Rule::Loop | Rule::Subagents)
}

fn normalized(mut proposal: Proposal) -> Proposal {
    if let Change::Mute { reason, .. } = &mut proposal.action {
        *reason = crate::plans::rules::clean_with(reason, false);
    }
    proposal
}

fn open_breaker(tx: &Transaction, project: &str, rule: Rule) -> Result<bool, String> {
    tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_breakers
        WHERE project = ?1 AND detector = ?2 AND closed_at IS NULL)", (project, rule.id()), |r| r.get(0)).map_err(sql)
}

pub(super) fn preview(tx: &Transaction, project: &str, author: &str, change: Proposal, at: i64) -> Result<Value, String> {
    let change = normalized(change);
    let p = prepare(tx, project, author, &change, at, at)?;
    Ok(json!({"class":p.class.label(),"replay":p.receipt}))
}

pub(super) fn verify(tx: &Transaction, project: &str, author: &str, input: Submission, at: i64) -> Result<Verified, String> {
    let as_of = input.replay.at;
    if as_of < at - DAY || as_of > at {
        return Err("Replay the proposal again. The receipt must be from the last 24 hours.".into());
    }
    let change = normalized(input.change);
    let prepared = prepare(tx, project, author, &change, as_of, at)?;
    if prepared.receipt != input.replay {
        return Err("The proposed Supervisor replay does not reproduce. Replay the change again.".into());
    }
    if !prepared.receipt.measured() && !matches!(change.action, Change::Engine { .. } | Change::OwnerReview { .. } | Change::Mute { .. } | Change::Remove | Change::Enable) {
        return Err("This change has no measurable retained replay inputs.".into());
    }
    let generation = rounds::enabled(tx, project)?.ok_or("The Supervisor is disabled.")?;
    Ok(Verified { prepared, change, generation })
}

pub(super) struct Verified {
    prepared: Prepared,
    change: Proposal,
    generation: i64,
}

#[derive(Debug, Serialize)]
pub(super) struct Submitted {
    pub change: i64,
    class: String,
    pub task: Option<String>,
}

pub(super) fn submit(tx: &Transaction, project: &str, author: &str, verified: Verified, at: i64) -> Result<Submitted, String> {
    let Verified { prepared: mut p, change, generation } = verified;
    if settings::revision(tx, project)? != p.revision || rounds::enabled(tx, project)? != Some(generation) {
        return Err(STALE.into());
    }
    if settings::level(tx, project)? == settings::Level::Observer {
        return Err("Observer cannot submit Supervisor configuration changes.".into());
    }
    let proposal = signals::encode(&change)?;
    if let Some(existing) = tx.query_row("SELECT c.id, c.class, t.slug FROM supervisor_config_changes c
        LEFT JOIN tasks t ON t.id = c.task WHERE c.project = ?1 AND c.author = ?2 AND c.generation = ?3
        AND c.proposal = ?4 AND c.revision = ?5 AND c.decision IS NULL",
        (project, author, generation, &proposal, p.revision),
        |r| Ok((r.get(0)?, r.get::<_, String>(1)?, r.get(2)?))).optional().map_err(sql)?
    {
        return Ok(Submitted { change: existing.0, class: existing.1, task: existing.2 });
    }
    let n: i64 = tx.query_row("SELECT count(*) FROM supervisor_config_changes WHERE project = ?1 AND author = ?2",
        (project, author), |r| r.get(0)).map_err(sql)?;
    if n >= 8 { return Err("Submit at most eight configuration proposals per round.".into()); }
    if matches!(p.class, Class::Tune | Class::Extend)
        && (open_breaker(tx, project, p.rule)?
            || super::config_lifecycle::protected(tx, project, p.rule, &settings::read(tx, project)?.settings)?)
    {
        p.class = Class::Owner;
    }
    if let Change::Mute { until, .. } = &change.action && *until <= at {
        return Err("This mute already expired. Propose a new end date.".into());
    }
    let class = p.class.label();
    let id: i64 = tx.query_row("INSERT INTO supervisor_config_changes
        (project, detector, class, author, at, generation, revision, proposal, replay, previous, next)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) RETURNING id",
        (project, p.rule.id(), class, author, at, generation, p.revision, proposal,
            signals::encode(&p.receipt)?, signals::encode(&p.before)?, signals::encode(&p.next)?), |r| r.get(0)).map_err(sql)?;
    let mut task = None;
    match p.class {
        Class::Tune | Class::Extend => {
            let revision = apply(tx, project, p.rule, p.next, p.revision, at)?;
            applied(tx, id, revision, at)?;
        }
        Class::Engine => {
            let Change::Engine { requirement } = change.action else { unreachable!() };
            if let Some(board) = settings::read(tx, project)?.settings.engine_board {
                task = Some(super::config_engine::create(tx, &board, p.rule, requirement, id, at)?);
            } else {
                task = Some(card(tx, project, id, &change, &p.receipt, at)?);
            }
        }
        Class::Owner => { task = Some(card(tx, project, id, &change, &p.receipt, at)?); }
    }
    Ok(Submitted { change: id, class: class.into(), task })
}

fn apply(tx: &Transaction, project: &str, rule: Rule, next: Option<RuleSettings>, revision: i64, at: i64) -> Result<i64, String> {
    let Saved { mut settings, .. } = settings::read(tx, project)?;
    match next {
        Some(next) => { settings.detectors.insert(rule.id().into(), next); }
        None => { settings.detectors.remove(rule.id()); }
    }
    if settings.muted(rule, at) {
        super::config_lifecycle::quiet_signals(tx, project, rule)?;
    }
    Ok(settings::write_at(tx, project, revision, settings, at)?.revision)
}

fn applied(tx: &Transaction, id: i64, revision: i64, at: i64) -> Result<(), String> {
    tx.execute("UPDATE supervisor_config_changes SET applied_at = ?2, applied_revision = ?3, decision = 'applied' WHERE id = ?1",
        (id, at, revision)).map(drop).map_err(sql)
}

fn card(tx: &Transaction, project: &str, id: i64, change: &Proposal, receipt: &Receipt, at: i64) -> Result<String, String> {
    let by = crate::roles::supervisor::author(tx, project)?;
    let (title, action, label) = match &change.action {
        Change::Engine { .. } => ("Supervisor engine change", "review", "Review engine request"),
        Change::OwnerReview { .. } => ("Supervisor configuration decision", "review", "Review owner settings"),
        _ => ("Supervisor configuration decision", "apply", "Apply this change"),
    };
    let proposal = signals::encode(change)?;
    let words = format!("Why you are needed:\nThe Supervisor needs your decision on a configuration change.\n\nRecommended:\n1. Keep the current configuration until you have reviewed the replay.\n\nDetails:\nChange {id}. {proposal}\n{}", receipt.summary());
    let task = ops::create(tx, &by, NewTask { title: title.into(), text: words.clone(), project: Some(project.into()),
        severity: Some("P2".into()), ..Default::default() }, at)?;
    super::task_policy::mark_card(tx, &task)?;
    let choices = json!({"kind":"strategic","recommended":"keep","options":[
        {"id":"keep","label":"Keep current configuration"},{"id":action,"label":label}]});
    let question = comments::wait_for_owner(tx, &by, &task, &words, &choices, at)?;
    super::outcomes::unattributed(tx, project, super::outcomes::Kind::Card, &question, &task.key, at)?;
    tx.execute("UPDATE supervisor_config_changes SET task = ?2, question = ?3 WHERE id = ?1",
        (id, &task.key, question)).map_err(sql)?;
    Ok(task.id)
}

/// Called only from the signed-in task choice handler, in its transaction.
pub(crate) fn owner_choice(tx: &Transaction, task: &ops::Task, question: &str, choice: &str, at: i64) -> Result<(), String> {
    let found = tx.query_row("SELECT id, detector, generation, revision, next, decision, proposal FROM supervisor_config_changes
        WHERE project = ?1 AND task = ?2 AND question = ?3", (&task.project, &task.key, question),
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?,
            signals::decode::<Option<RuleSettings>>(r, 4)?, r.get::<_, Option<String>>(5)?, signals::decode::<Proposal>(r, 6)?))).optional().map_err(sql)?;
    let Some((id, name, generation, revision, mut next, decision, proposal)) = found else { return Ok(()) };
    if decision.is_some() { return Err("This Supervisor configuration decision was already recorded.".into()); }
    if choice == "apply" {
        if rounds::enabled(tx, &task.project)? != Some(generation) || settings::revision(tx, &task.project)? != revision {
            return Err(STALE.into());
        }
        if matches!(proposal.action, Change::Mute { .. })
            && let Some(mute) = next.as_mut().and_then(|config| config.mute.as_mut())
        {
            if mute.until <= at { return Err("This mute already expired. Propose a new end date.".into()); }
            mute.owner = Some(HUMAN.into());
            tx.execute("UPDATE supervisor_config_changes SET next = ?2 WHERE id = ?1", (id, signals::encode(&next)?)).map_err(sql)?;
        }
        let revision = apply(tx, &task.project, rule(&name)?, next, revision, at)?;
        applied(tx, id, revision, at)?;
    } else {
        tx.execute("UPDATE supervisor_config_changes SET decision = ?2 WHERE id = ?1", (id, choice)).map_err(sql)?;
    }
    Ok(())
}

pub(super) fn revert(tx: &Transaction, project: &str, id: i64, at: i64) -> Result<(), String> {
    let change = tx.query_row("SELECT detector, previous, next, applied_revision, reverted_at FROM supervisor_config_changes
        WHERE project = ?1 AND id = ?2 AND applied_at IS NOT NULL", (project, id),
        |r| Ok((r.get::<_, String>(0)?, signals::decode::<Option<RuleSettings>>(r, 1)?,
            signals::decode::<Option<RuleSettings>>(r, 2)?, r.get::<_, i64>(3)?, r.get::<_, Option<i64>>(4)?)))
        .optional().map_err(sql)?.ok_or("That configuration change was not applied in this project.")?;
    let (name, before, after, revision, reverted) = change;
    if reverted.is_some() { return Ok(()); }
    let later = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_config_changes
        WHERE project = ?1 AND detector = ?2 AND applied_revision > ?3 AND reverted_at IS NULL)",
        (project, &name, revision), |r| r.get::<_, bool>(0)).map_err(sql)?;
    let saved = settings::read(tx, project)?;
    if later || saved.settings.detectors.get(&name) != after.as_ref() {
        return Err("This detector changed since then. Revert the newer change first or review its settings.".into());
    }
    apply(tx, project, rule(&name)?, before, saved.revision, at)?;
    tx.execute("UPDATE supervisor_config_changes SET reverted_at = ?2 WHERE id = ?1", (id, at)).map(drop).map_err(sql)
}

fn ready(tx: &Transaction, project: &str, name: &str, id: i64) -> Result<Option<(i64, String)>, String> {
    let record = tx.query_row("SELECT id, applied_at, next FROM supervisor_config_changes
        WHERE project = ?1 AND detector = ?2 AND applied_at IS NOT NULL AND reverted_at IS NULL ORDER BY applied_revision DESC LIMIT 1",
        (project, name), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))).optional().map_err(sql)?;
    match record {
        Some((latest, since, next)) if latest == id && super::config_lifecycle::shadow_ready(tx, project, name, since)? =>
            Ok(Some((since, next))),
        _ => Ok(None),
    }
}

pub(super) fn promote(tx: &Transaction, project: &str, id: i64, at: i64) -> Result<(), String> {
    let name: String = tx.query_row("SELECT detector FROM supervisor_config_changes WHERE project = ?1 AND id = ?2",
        (project, id), |r| r.get(0)).optional().map_err(sql)?.ok_or("That shadow trial does not belong to this project.")?;
    let rule = rule(&name)?;
    let saved = settings::read(tx, project)?;
    let previous = saved.settings.detectors.get(&name);
    let (since, trial) = ready(tx, project, &name, id)?.ok_or("Rate three distinct shadow findings before promoting the detector.")?;
    if saved.settings.mode(rule) != Mode::Shadow || signals::encode(&previous)? != trial {
        return Err("The shadow detector changed. Review its current settings.".into());
    }
    let generation = rounds::enabled(tx, project)?.ok_or("The Supervisor is disabled.")?;
    let change = Proposal { detector: name.clone(), action: Change::Promote };
    let mut next = previous.cloned().unwrap_or_default();
    next.mode = Some(Mode::Live);
    next.owner_live = true;
    let revision = apply(tx, project, rule, Some(next.clone()), saved.revision, at)?;
    tx.execute("INSERT INTO supervisor_config_changes
        (project, detector, class, author, at, generation, revision, proposal, replay, previous, next, applied_at, applied_revision, decision)
        SELECT project, detector, 'owner', ?4, ?5, ?6, ?7, ?8, replay, ?9, ?10, ?5, ?11, 'applied'
        FROM supervisor_config_changes WHERE project = ?1 AND id = ?2 AND applied_at = ?3",
        (project, id, since, HUMAN, at, generation, saved.revision, signals::encode(&change)?,
            signals::encode(&previous)?, signals::encode(&Some(next))?, revision)).map(drop).map_err(sql)
}

#[derive(Serialize)]
pub(super) struct Entry {
    id: i64,
    detector: String,
    class: String,
    author: String,
    at: i64,
    previous: Value,
    next: Value,
    replay: String,
    applied: bool,
    reverted: bool,
    task: Option<String>,
    decision: Option<String>,
    promote: bool,
}

pub(super) fn list(tx: &Transaction, project: &str, after: i64) -> Result<Vec<Entry>, String> {
    let saved = settings::read(tx, project)?;
    let mut entries = rows(tx, "SELECT c.id, c.detector, c.class, c.author, c.at, c.previous, c.next, c.replay,
        c.applied_at IS NOT NULL, c.reverted_at IS NOT NULL, t.slug, c.decision
        FROM supervisor_config_changes c LEFT JOIN tasks t ON t.id = c.task
        WHERE c.project = ?1 AND (c.at >= ?2 OR c.reverted_at >= ?2 OR c.applied_at >= ?2)
        ORDER BY c.id DESC LIMIT 50", (project, after), |r| Ok(Entry {
        id: r.get(0)?, detector: r.get(1)?, class: r.get(2)?, author: r.get(3)?, at: r.get(4)?,
        previous: signals::decode(r, 5)?, next: signals::decode(r, 6)?, replay: signals::decode::<Receipt>(r, 7)?.summary(),
        applied: r.get(8)?, reverted: r.get(9)?, task: r.get(10)?, decision: r.get(11)?, promote: false,
    }))?;
    for entry in &mut entries {
        if entry.applied && !entry.reverted && rule(&entry.detector).is_ok_and(|rule| saved.settings.mode(rule) == Mode::Shadow) {
            entry.promote = ready(tx, project, &entry.detector, entry.id)?.is_some();
        }
    }
    Ok(entries)
}

pub(super) fn weekly(tx: &Transaction, project: &str, at: i64) -> Result<String, String> {
    let mut lines = list(tx, project, at - super::WEEK)?.into_iter().map(|entry| {
        let status = if entry.reverted { "reverted" } else { entry.decision.as_deref().unwrap_or("awaiting a decision") };
        format!("- Change {} · {} · {} · {}. {}", entry.id, entry.detector, entry.class, status, entry.replay)
    }).collect::<Vec<_>>();
    lines.extend(super::config_lifecycle::status(tx, project)?.into_iter().map(|entry| {
        let status = if entry.removal { "proposed for removal" } else { "weekly review only" };
        format!("- {} · {status}.", entry.detector)
    }));
    Ok(lines.join("\n"))
}

pub(super) fn for_reflection(tx: &Transaction, project: &str, run: &str, id: i64) -> Result<(String, String), String> {
    tx.query_row("SELECT proposal, replay FROM supervisor_config_changes WHERE project = ?1 AND author = ?2 AND id = ?3",
        (project, run, id), |r| Ok((r.get(0)?, signals::decode::<Receipt>(r, 1)?.summary())))
        .optional().map_err(sql)?.ok_or("Cite a configuration change submitted by this reflection.".into())
}
