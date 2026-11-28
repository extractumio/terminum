// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213: code-owned subject scores and measured raises: scope rules raise blast radius, a query's own rule
//! (`waiting`) impact; otherwise impact remains the additive assessment's.

use serde::{Deserialize, Serialize};

use super::{Finding, Rule, Scores};

impl Rule {
    pub(super) fn scores(self) -> Scores {
        use Rule::*;
        let (blast, reversibility) = match self {
            // One task, session or report; a reply, retry or plan correction takes minutes.
            WaitHuman | RerunAfterDone | Starved | Stuck | CostOutlier | UserBugAge | FeedbackAge | StalePlan
                | Loop | Subagents | ReportCritical | FollowUps => (0, 0),
            // One task, with hours of diagnosis or rework.
            FailedRepeat | VerifyRed | DiffOutlier => (0, 1),
            // Several tasks or one process, code or measurement area; hours of correction.
            FailedWithoutReason | LeadTime | BacklogGrowth | ScheduleHealth | Returned | Flaky | QueueGrowth | LimitNear
                | AskRate | AgentScorecard | Misrouted | Involvement | Operations | GuardBroken
                | ReleaseTime | Releases | Allocation | External | AnswerLatency | PriorityChurn
                | OwnerLoad | RubberStamp | SurfaceTouched | ReviewMissing | Hotspot | Dependency
                | BugCluster | ProbeFailed => (1, 1),
            MainRed => (2, 1),
            MigrationEdited => (2, 2),
            UnreadReports => (1, 0),
            // Project-wide detection or paging; repair/restart takes minutes. Independent critical
            // evaluations still own their critical response.
            CollectorDead | LineDead => (2, 0),
        };
        Scores { blast, reversibility, ..Scores::default() }
    }
}

/// The score a measured rule raises.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Score { Impact, Blast }

impl Score {
    fn of(self, scores: &mut Scores) -> &mut u8 {
        match self { Self::Impact => &mut scores.impact, Self::Blast => &mut scores.blast }
    }
}

/// The finding's measurement and evidence establish this raise; `from` keeps the score code measured.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Raise {
    pub score: Score,
    pub from: u8,
    pub to: u8,
    pub rule: String,
    /// What the rule found, in words for the owner: what held a waiting P0 task (`waiting`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Applies the scope rules below and the query's own raises (`waiting`); each records the score it found as `from`,
/// and one that does not lift its score is dropped.
pub(super) fn apply(rule: Rule, finding: &mut Finding) {
    let scope = match (rule, finding.metric) {
        (Rule::Flaky, "nightly_reds") if finding.measure.value > 0.0 => Some(("nightly_reds", 2)),
        (Rule::WaitHuman, "blocked_tasks") => Some(("blocked_tasks", 1)),
        (Rule::CostOutlier, "failed_run_dollars") => Some(("failed_spend_area", 1)),
        // The query requires enough attempts in both windows and a failure-rate rise above its band.
        (Rule::VerifyRed, "own_verify_failure_share") => Some(("agent_verify_failures", 1)),
        _ => None,
    };
    if let Some((name, to)) = scope {
        finding.score_raises.push(Raise { score: Score::Blast, from: 0, to, rule: name.into(), reason: None });
    }
    let scores = &mut finding.scores;
    finding.score_raises.retain_mut(|raise| {
        let score = raise.score.of(scores);
        raise.from = *score;
        *score = (*score).max(raise.to);
        raise.to > raise.from
    });
}
