// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-247: quarantine only the detector and response that made the wrong decisions.

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::tasks::{comments, ops::{self, NewTask, rows, sql}};
use super::{Detector, Mode, Rule, Signal, Threshold, flow, outcomes, replay, settings, signals};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct Config {
    mode: Mode,
    window_days: u16,
    threshold: Threshold,
    allocation: std::collections::BTreeMap<String, f64>,
    muted: bool,
}

impl Config {
    fn of(d: &Detector, muted: bool) -> Self {
        Self { mode: d.mode, window_days: d.window_days, threshold: d.threshold, allocation: d.allocation.clone(), muted }
    }
}

const OPEN: &str = "SELECT id FROM supervisor_breakers WHERE project = ?1 AND detector = ?2 AND response = ?3 AND closed_at IS NULL";

pub(super) fn open_id(tx: &Transaction, project: &str, detector: &str, response: &str) -> Result<Option<i64>, String> {
    tx.query_row(OPEN, (project, detector, response), |r| r.get(0)).optional().map_err(sql)
}

pub(super) fn wrong(tx: &Transaction, project: &str, action: &outcomes::Action, alert: bool, at: i64) -> Result<(), String> {
    if action.response.is_empty() { return Ok(()); }
    if open_id(tx, project, &action.detector, &action.response)?.is_some() { return Ok(()); }
    let cutoff: i64 = tx.query_row("SELECT coalesce(max(reset_after_action), 0) FROM supervisor_breakers
        WHERE project = ?1 AND detector = ?2 AND response = ?3",
        (project, &action.detector, &action.response), |r| r.get(0)).map_err(sql)?;
    if !alert && action.id <= cutoff { return Ok(()); }
    if !alert {
        let wrong: i64 = tx.query_row("SELECT count(*) FROM (
            SELECT a.id FROM supervisor_actions a WHERE a.project = ?1 AND a.detector = ?2 AND a.response = ?3
              AND a.id > ?4 AND EXISTS (SELECT 1 FROM supervisor_outcomes o WHERE o.action = a.id AND o.verdict = 'wrong')
            LIMIT 2)", (project, &action.detector, &action.response, cutoff), |r| r.get(0)).map_err(sql)?;
        if wrong < 2 { return Ok(()); }
    }
    let config = Rule::named(&action.detector).ok().map(|rule| {
        let saved = settings::read(tx, project)?.settings;
        Ok::<_, String>(Config::of(&saved.detector(rule), saved.muted(rule, at)))
    }).transpose()?;
    tx.execute("INSERT INTO supervisor_breakers (project, detector, response, opened_at, opened_by, config)
        VALUES (?1,?2,?3,?4,?5,?6)", (project, &action.detector, &action.response, at, action.id, signals::encode(&config)?))
        .map(drop).map_err(sql)
}

#[derive(Debug, Serialize)]
pub(crate) struct Card {
    pub id: i64,
    pub task: String,
}

/// The blocked action is an owner question. The key deduplicates repeated requests
/// without treating different targets under the same rule as the same decision.
pub(super) fn divert(tx: &Transaction, s: &Signal, kind: &str, target: &str, description: &str, at: i64)
    -> Result<Option<Card>, String>
{
    divert_rule(tx, s, s.response, kind, target, description, at)
}

pub(super) fn divert_rule(tx: &Transaction, s: &Signal, response: &str, kind: &str, target: &str, description: &str, at: i64)
    -> Result<Option<Card>, String>
{
    let Some(breaker) = open_id(tx, &s.project, &s.detector, response)? else { return Ok(None) };
    let prior = tx.query_row("SELECT c.id, t.slug FROM supervisor_breaker_cards c JOIN tasks t ON t.id = c.task
        WHERE c.breaker = ?1 AND c.kind = ?2 AND c.fingerprint = ?3 AND c.target = ?4",
        (breaker, kind, &s.fingerprint, target), |r| Ok(Card { id: r.get(0)?, task: r.get(1)? }))
        .optional().map_err(sql)?;
    if prior.is_some() { return Ok(prior); }
    let by = crate::roles::supervisor::author(tx, &s.project)?;
    let words = format!("Why you are needed:\nAutonomy is paused for {} / {}.\n\nRecommended:\n1. Leave this action paused until you review the evidence.\n\nDetails:\n{description}\nSignal {}. This card records the proposed action. Use the rule's replay card on the Supervisor screen to restore autonomy.", s.detector, response, s.fingerprint);
    let task = ops::create(tx, &by, NewTask { project: Some(s.project.clone()), title: format!("Supervisor decision for {} / {}", s.detector, response),
        severity: Some("P1".into()), text: words.clone(), ..NewTask::default() }, at)?;
    super::task_policy::mark_card(tx, &task)?;
    let question = comments::wait_for_owner(tx, &by, &task, &words, &json!({
        "kind": "operational", "recommended": "leave-paused",
        "options": [{"id": "leave-paused", "label": "Leave paused"}, {"id": "handle-manually", "label": "Handle this action manually"}],
    }), at)?;
    let id = tx.query_row("INSERT INTO supervisor_breaker_cards (breaker, kind, fingerprint, target, task)
        VALUES (?1,?2,?3,?4,?5) RETURNING id", (breaker, kind, &s.fingerprint, target, &task.key), |r| r.get(0)).map_err(sql)?;
    outcomes::record_rule(tx, (s, response), outcomes::Kind::Card, &question, &task.key, "", at)?;
    Ok(Some(Card { id, task: task.id }))
}

#[derive(Debug, Serialize)]
pub(super) struct Open {
    id: i64,
    detector: String,
    response: String,
    opened_at: i64,
    cards: Vec<String>,
}

pub(super) fn list(tx: &Transaction, project: &str) -> Result<Vec<Open>, String> {
    let mut open = rows(tx, "SELECT id, detector, response, opened_at FROM supervisor_breakers
        WHERE project = ?1 AND closed_at IS NULL ORDER BY id DESC", [project], |r|
        Ok(Open { id: r.get(0)?, detector: r.get(1)?, response: r.get(2)?, opened_at: r.get(3)?, cards: Vec::new() }))?;
    for b in &mut open {
        b.cards = rows(tx, "SELECT t.slug FROM supervisor_breaker_cards c JOIN tasks t ON t.id = c.task
            WHERE c.breaker = ?1 AND t.project = ?2 AND t.archived_at IS NULL ORDER BY c.id DESC LIMIT 20",
            (b.id, project), |r| r.get(0))?;
    }
    Ok(open)
}

#[derive(Deserialize, Serialize)]
pub(super) struct Review {
    pub revision: i64,
    before: Config,
    after: Config,
    first: i64,
    as_of: i64,
    before_firings: usize,
    after_firings: usize,
}

pub(super) fn review(tx: &Transaction, project: &str, id: i64, at: i64) -> Result<Review, String> {
    let (name, old) = tx.query_row("SELECT detector, config FROM supervisor_breakers
        WHERE project = ?1 AND id = ?2 AND closed_at IS NULL", (project, id),
        |r| Ok((r.get::<_, String>(0)?, signals::decode::<Option<Config>>(r, 1)?)))
        .optional().map_err(sql)?.ok_or("That rule's breaker is no longer open.")?;
    let rule = Rule::named(&name)?;
    let before = old.ok_or("The previous rule configuration is unavailable.")?;
    let saved = settings::read(tx, project)?;
    let current = saved.settings.detector(rule);
    let after = Config::of(&current, saved.settings.muted(rule, at));
    if after.mode != Mode::Live || after.muted {
        return Err("Set this detector to Live and clear its mute before reviewing its replay.".into());
    }
    if (before.window_days, before.threshold, &before.allocation) == (after.window_days, after.threshold, &after.allocation) {
        return Err("Change this detector's measurement settings before reviewing its replay.".into());
    }
    if !rule.has_historical_query() { return Err("The detector has no retained historical replay.".into()); }
    let (first, _) = flow::history_bounds(tx, project, at)?;
    let as_of = at;
    if first <= 0 || as_of - first < 2 * super::WEEK {
        return Err("There is not enough retained history to replay this rule.".into());
    }
    let mut previous = current.clone();
    previous.mode = Mode::Live;
    previous.window_days = before.window_days;
    previous.threshold = before.threshold;
    previous.allocation.clone_from(&before.allocation);
    let (before_firings, after_firings) = replay::compare_rule(tx, project, first, as_of, [&previous, &current])?;
    Ok(Review { revision: saved.revision, before, after, first, as_of, before_firings, after_firings })
}

pub(super) fn save_review(tx: &Transaction, project: &str, id: i64, review: &Review) -> Result<String, String> {
    let still_open: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_breakers
        WHERE project = ?1 AND id = ?2 AND closed_at IS NULL)", (project, id), |r| r.get(0)).map_err(sql)?;
    if !still_open || settings::revision(tx, project)? != review.revision {
        return Err("The breaker or its settings changed. Review the replay again.".into());
    }
    let text = signals::encode(review)?;
    tx.execute("INSERT INTO supervisor_breaker_reviews VALUES (?1,?2,?3)
        ON CONFLICT (breaker) DO UPDATE SET revision = excluded.revision, review = excluded.review",
        (id, review.revision, &text)).map_err(sql)?;
    Ok(proof(id, &text))
}

/// Called only by the signed-in owner route, with proof from the displayed review card.
pub(super) fn restore(tx: &Transaction, project: &str, id: i64, key: &str, at: i64) -> Result<(), String> {
    let (revision, raw, detector) = tx.query_row("SELECT r.revision, r.review, b.detector
        FROM supervisor_breaker_reviews r JOIN supervisor_breakers b ON b.id = r.breaker
        WHERE b.project = ?1 AND b.id = ?2 AND b.closed_at IS NULL", (project, id),
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
        .optional().map_err(sql)?.ok_or("Review this open breaker's replay before restoring autonomy.")?;
    let saved = settings::read(tx, project)?;
    let review: Review = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let rule = Rule::named(&detector)?;
    let current = Config::of(&saved.settings.detector(rule), saved.settings.muted(rule, at));
    if saved.revision != revision || current != review.after || proof(id, &raw) != key {
        return Err("The breaker or its settings changed. Review the replay again.".into());
    }
    tx.execute("UPDATE supervisor_breakers SET closed_at = ?3,
        reset_after_action = (SELECT coalesce(max(id),0) FROM supervisor_actions WHERE project = ?1)
        WHERE project = ?1 AND id = ?2 AND closed_at IS NULL", (project, id, at)).map(drop).map_err(sql)
}

fn proof(breaker: i64, review: &str) -> String {
    crate::auth::crypto::sha256_hex(format!("{breaker}
{review}").as_bytes())
}

/// Return cards inserted by a transaction so its caller can notify the open board after commit.
pub(crate) fn with_new_cards<T>(tx: &Transaction, act: impl FnOnce() -> Result<T, String>) -> Result<(T, Vec<String>), String> {
    let since: i64 = tx.query_row("SELECT coalesce(max(id),0) FROM supervisor_breaker_cards", [], |r| r.get(0)).map_err(sql)?;
    let result = act()?;
    let cards = rows(tx, "SELECT t.slug FROM supervisor_breaker_cards c JOIN tasks t ON t.id = c.task
        WHERE c.id > ?1 ORDER BY c.id", [since], |r| r.get(0))?;
    Ok((result, cards))
}

pub(crate) fn notify(app: &crate::app::Shared, cards: Vec<String>) {
    for task in cards { crate::tasks::api::told(app, &task); }
}
