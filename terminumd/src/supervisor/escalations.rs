// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213: a checked round's critical escalation. It raises blast radius or reversibility to 3, never the measured
//! impact, and stands while its signal keeps the evidence the round judged.

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::tasks::ops::sql;

use super::{Scores, Signal};

const CRITICAL: u8 = 3;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Score { Blast, Reversibility }

impl Score {
    fn name(self) -> &'static str {
        match self { Self::Blast => "blast", Self::Reversibility => "reversibility" }
    }

    pub(super) fn read(name: &str) -> Option<Self> {
        match name { "blast" => Some(Self::Blast), "reversibility" => Some(Self::Reversibility), _ => None }
    }

    /// The scores a standing escalation gives its signal's level and response.
    pub(super) fn raise(self, mut scores: Scores) -> Scores {
        match self {
            Self::Blast => scores.blast = CRITICAL,
            Self::Reversibility => scores.reversibility = CRITICAL,
        }
        scores
    }
}

/// A round's typed request, inside its finding.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub score: Score,
    pub level: u8,
    pub reason: String,
}

impl Request {
    /// Only a raise to level 3, on a finding code checked, at the confidence of a strong heuristic.
    pub(super) fn check(&self, signal: &Signal, confidence: f64) -> Result<(), String> {
        signal.scores.check_proposed_level(self.level)?;
        if self.level != CRITICAL || signal.level >= CRITICAL {
            return Err("A round's escalation raises a signal below level 3 to level 3, the critical level.".into());
        }
        if confidence < super::STRONG {
            return Err("A critical escalation needs a finding with confidence of at least 0.7.".into());
        }
        Ok(())
    }
}

/// The score a standing escalation raises: only while the signal holds the evidence the round judged.
pub(super) fn standing(tx: &Transaction, fingerprint: &str, evidence: &str) -> Result<Option<Score>, String> {
    let score: Option<String> = tx.prepare_cached("SELECT score FROM supervisor_escalations
        WHERE fingerprint = ?1 AND evidence = ?2").and_then(|mut q| q.query_row((fingerprint, evidence), |r| r.get(0))
        .optional()).map_err(sql)?;
    Ok(score.as_deref().and_then(Score::read))
}

/// Record the escalation and raise the signal's incident at once, as a level 3 signal would.
pub(super) fn record(tx: &Transaction, signal: &Signal, round: &str, request: &Request, at: i64) -> Result<(), String> {
    let evidence: String = tx.query_row("SELECT evidence FROM supervisor_signals WHERE fingerprint = ?1",
        [&signal.fingerprint], |r| r.get(0)).map_err(sql)?;
    tx.execute("INSERT INTO supervisor_escalations (fingerprint, project, round, score, evidence, reason, at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT (fingerprint) DO UPDATE SET round = excluded.round,
            score = excluded.score, evidence = excluded.evidence, reason = excluded.reason, at = excluded.at",
        (&signal.fingerprint, &signal.project, round, request.score.name(), &evidence, &request.reason, at)).map_err(sql)?;
    let raised = super::signals::get(tx, &signal.project, &signal.fingerprint)?;
    super::incidents::escalate(tx, &raised, at)
}
