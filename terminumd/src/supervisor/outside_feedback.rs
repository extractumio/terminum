// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-262: a local reading pass projects outside prose into closed categories.

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use crate::tasks::{attrs::Attrs, ops::{self, NewTask, sql}};

use super::{Evidence, Measurements, Query, Reading, impact::Source, probe_command::Failure, task_policy};

pub(super) const MAX_REPORTS: usize = 32;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct ReferenceBase(String);

impl ReferenceBase {
    pub(super) fn valid(&self) -> bool {
        let Some((host, path)) = self.0.strip_prefix("https://").and_then(|url| url.split_once('/')) else {
            return false;
        };
        self.0.len() <= 512 && host.len() <= 253 &&
            crate::sent::redact(&self.0) == self.0 &&
            super::dns_labels(host) &&
            (path.is_empty() || path.strip_suffix('/').is_some_and(|prefix| prefix.split('/').all(|part| {
                !matches!(part, "." | "..") && super::probes::identifier(part, 80)
            })))
    }

    pub(super) fn from_saved(base: String) -> Result<Self, String> {
        let base = Self(base);
        if base.valid() { Ok(base) } else { Err("Invalid saved outside feedback reference base.".into()) }
    }

    pub(super) fn as_str(&self) -> &str { &self.0 }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    reference: u64,
    outside_text: String,
}

#[derive(Debug, PartialEq)]
enum Theme { Usability, Reliability, Performance, Billing, Other }
#[derive(Debug, PartialEq)]
enum Kind { Problem, Request, Question, Other }
#[derive(Debug, PartialEq)]
enum Sentiment { Positive, Negative, Neutral }

#[derive(Debug, PartialEq)]
pub(super) struct Report {
    reference: u64,
    theme: Theme,
    kind: Kind,
    sentiment: Sentiment,
    reports: u8,
}

/// A single local pass. Its only input is text already bounded by the probe.
pub(super) fn read(input: Input) -> Result<Report, Failure> {
    if input.reference == 0 || input.outside_text.trim().is_empty() { return Err(Failure::Invalid); }
    let text = input.outside_text.to_ascii_lowercase();
    let has = |words: &[&str]| text.split(|c: char| !c.is_ascii_alphabetic()).any(|word| words.contains(&word));
    let reliability = has(&["crash", "crashed", "crashes", "error", "broken", "fails"]);
    let theme = if reliability { Theme::Reliability }
        else if has(&["slow", "latency", "lag", "laggy"]) { Theme::Performance }
        else if has(&["invoice", "refund", "billing", "charged"]) { Theme::Billing }
        else if has(&["navigation", "confusing", "unreadable", "accessibility"]) { Theme::Usability }
        else { Theme::Other };
    let kind = if reliability || has(&["bug"]) { Kind::Problem }
        else if has(&["feature", "request", "wish", "suggestion"]) { Kind::Request }
        else if text.contains('?') { Kind::Question }
        else { Kind::Other };
    let negative = has(&["bad", "broken", "frustrating", "unusable", "disappointed"]);
    let positive = has(&["good", "great", "helpful", "excellent", "love"]);
    let sentiment = match (positive, negative) {
        (true, false) => Sentiment::Positive,
        (false, true) => Sentiment::Negative,
        _ => Sentiment::Neutral,
    };
    Ok(Report { reference: input.reference, theme, kind, sentiment, reports: 1 })
}

pub(super) const MAX_UNANSWERED: usize = 256;

/// AC-212, AC-262: a feedback probe's typed list of the items still unanswered, by reference and
/// the time each was opened; it carries no outside prose.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Unanswered {
    unanswered: Vec<Open>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Open {
    reference: u64,
    since: i64,
}

impl Unanswered {
    pub(super) fn valid(&self, at: i64) -> bool {
        let mut references = std::collections::BTreeSet::new();
        self.unanswered.len() <= MAX_UNANSWERED && self.unanswered.iter().all(|open| {
            open.reference > 0 && (1..=at).contains(&open.since) && references.insert(open.reference)
        })
    }
}

/// A retained line of a feedback probe's sample: its unanswered list, or a metric this rule ignores.
#[derive(Deserialize)]
#[serde(untagged)]
enum Kept {
    Unanswered(Unanswered),
    Other(serde::de::IgnoredAny),
}

const UNSIZED: Source = Source::Unsized("Outside feedback is in no weekly task, owner or spend total.");

/// `product.feedback_age`: each unanswered outside item's age in minutes at its probe's latest fresh sample.
pub(super) fn age(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let sources = super::probes::feedback_samples(q.tx, q.project, q.window.end)?;
    if sources.is_empty() { return Ok(Measurements::missing("No approved probe has an outside feedback reference base.")); }
    let mut out = Measurements::default();
    for (base, sample) in sources {
        let Some((id, at, data)) = sample else {
            out.unavailable.push("Feedback probe history is unavailable for this window.");
            continue;
        };
        let lines: Vec<Kept> = serde_json::from_str(&data).map_err(|_| "Invalid retained Supervisor probe data.")?;
        let Some(list) = lines.into_iter().find_map(|line| match line { Kept::Unanswered(list) => Some(list), Kept::Other(_) => None }) else {
            out.unavailable.push("The feedback probe reports no unanswered list.");
            continue;
        };
        out.readings.extend(list.unanswered.into_iter().map(|open| {
            Reading::new("outside_feedback", format!("{}{}", base.as_str(), open.reference), "unanswered_minutes",
                (at - open.since) as f64 / 60.0, vec![Evidence::SupervisorProbes(id)], UNSIZED)
        }));
    }
    Ok(out)
}

pub(super) fn publish(tx: &Transaction, project: &str, base: &ReferenceBase, report: Report, at: i64)
    -> Result<Option<String>, String>
{
    let link = format!("{}{}", base.as_str(), report.reference);
    if tx.execute("INSERT OR IGNORE INTO supervisor_outside_references (project, link) VALUES (?1, ?2)",
        (project, &link)).map_err(sql)? == 0
    {
        return Ok(None);
    }
    let author = crate::roles::supervisor::author(tx, project)?;
    let task = ops::create(tx, &author, NewTask {
        title: "Review outside feedback".into(),
        text: format!("Theme: {:?}\nKind: {:?}\nSentiment: {:?}\nReports: {}",
            report.theme, report.kind, report.sentiment, report.reports),
        attrs: Attrs { external_link: Some(link), ..Default::default() },
        project: Some(project.into()), ..Default::default()
    }, at)?;
    task_policy::mark_outside(tx, &task)?;
    Ok(Some(task.id))
}
