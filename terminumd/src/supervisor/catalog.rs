// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use super::{Measurements, Query, Threshold, flow, people, samples, spend};

macro_rules! catalog {
    ($($name:ident, $id:literal, $query:literal, $threshold:expr, $run:path;)+) => {
        #[derive(Clone, Copy, Debug, PartialOrd, Ord, PartialEq, Eq)]
        pub(crate) enum Rule { $($name),+ }

        impl Rule {
            pub(crate) const ALL: &'static [Self] = &[$(Self::$name),+];

            pub(crate) fn named(name: &str) -> Result<Self, String> {
                match name {
                    $($id | $query => Ok(Self::$name),)+
                    _ => Err("Unknown Supervisor query.".into()),
                }
            }

            pub(crate) fn id(self) -> &'static str {
                match self { $(Self::$name => $id),+ }
            }

            pub(super) fn defaults(self) -> Threshold {
                match self { $(Self::$name => $threshold),+ }
            }

            pub(super) fn query(self, context: &Query<'_, '_>) -> Result<Measurements, String> {
                match self { $(Self::$name => $run(context)),+ }
            }
        }
    }
}

use Threshold::Absolute;
const fn baseline(percentile: u8, floor: f64) -> Threshold { Threshold::Baseline { percentile, floor } }
const fn change(by: f64) -> Threshold { Threshold::Change { by } }

catalog! {
    WaitHuman, "flow.wait_human_age", "open_waits", baseline(90, 240.0), flow::wait_human;
    FailedWithoutReason, "flow.failed_no_reason", "reasonless_failures", baseline(90, 1.0), flow::reasonless;
    FailedRepeat, "flow.failed_repeat", "repeated_failures", Absolute(2.0), flow::failed_repeat;
    RerunAfterDone, "flow.rerun_after_done", "reruns_after_done", Absolute(0.0), flow::rerun;
    Starved, "flow.starved", "starved_tasks", Absolute(1440.0), flow::starved;
    LeadTime, "flow.lead_time_shift", "lead_time_shift", change(60.0), flow::lead_time;
    BacklogGrowth, "flow.backlog_growth", "backlog_growth", Absolute(3.0), flow::backlog;
    CostOutlier, "flow.cost_outlier", "run_cost_outliers", baseline(95, 1.0), spend::outliers;
    ScheduleHealth, "flow.schedule_health", "schedule_health", Absolute(0.0), flow::schedule;
    AskRate, "drift.ask_rate", "agent_ask_rate", baseline(90, 0.5), spend::ask_rate;
    AgentScorecard, "drift.agent_scorecard", "agent_scorecard", baseline(90, 0.1), spend::scorecard;
    Involvement, "improve.involvement", "features_per_owner_hour", change(0.1), people::involvement;
    Operations, "improve.operations", "operations_per_done", change(0.5), people::operations;
    GuardBroken, "improve.guard_broken", "store_guards", change(0.0), people::guard_broken;
    Allocation, "product.allocation", "effort_allocation", Absolute(0.15), spend::allocation;
    UserBugAge, "product.user_bug_age", "open_priority_bugs", Absolute(1440.0), flow::bugs;
    AnswerLatency, "owner.answer_latency", "answer_latency", change(60.0), people::answer_latency;
    PriorityChurn, "owner.priority_churn", "priority_churn", baseline(90, 1.0), people::churn;
    OwnerLoad, "owner.load", "owner_load", change(0.0), people::load;
    Returned, "flow.returned", "integration_returns", Absolute(0.0), integration;
    VerifyRed, "drift.verify_red", "verify_failures", Absolute(0.0), integration;
    DiffOutlier, "drift.diff_outlier", "diff_outliers", Absolute(0.0), integration;
    Misrouted, "drift.misrouted", "misrouted_tasks", Absolute(0.0), integration;
    QueueGrowth, "flow.queue_growth", "queue_growth", Absolute(0.0), samples::snapshots;
    LimitNear, "flow.limit_near", "subscription_limits", Absolute(100.0), samples::snapshots;
    StalePlan, "drift.stale_plan", "stale_plans", Absolute(0.0), samples::snapshots;
    Loop, "drift.loop", "tool_loops", baseline(90, 2.0), samples::counters;
    Subagents, "drift.subagents", "subagent_counts", baseline(90, 2.0), samples::counters;
    CollectorDead, "self.collector_dead", "collector_heartbeat", Absolute(2.0), heartbeat;
    LineDead, "self.line_dead", "line_health", Absolute(0.0), line_health;
}

impl Rule {
    pub(super) fn configurable(self) -> bool {
        !matches!(self, Self::CollectorDead | Self::LineDead)
    }

    pub(super) fn present_only(self) -> bool {
        matches!(self, Self::Starved | Self::UserBugAge)
    }

    pub(super) fn needs_weekly_samples(self) -> bool {
        !self.present_only() && !matches!(self, Self::WaitHuman | Self::CostOutlier | Self::QueueGrowth | Self::LimitNear | Self::StalePlan | Self::Loop | Self::Subagents | Self::CollectorDead | Self::LineDead)
    }

    pub(super) fn has_historical_query(self) -> bool {
        !self.present_only() && !matches!(self, Self::Returned | Self::VerifyRed | Self::DiffOutlier | Self::Misrouted
            | Self::CollectorDead | Self::LineDead)
    }

    pub(super) fn supports(self, threshold: Threshold) -> bool {
        match self.defaults() {
            Threshold::Change { .. } => matches!(threshold, Threshold::Change { .. }),
            Threshold::Baseline { .. } => !matches!(threshold, Threshold::Change { .. }),
            Threshold::Absolute(_) => matches!(threshold, Threshold::Absolute(_)),
        }
    }
}

fn integration(q: &Query<'_, '_>) -> Result<Measurements, String> {
    use rusqlite::OptionalExtension;
    use crate::tasks::ops::sql;

    let on = q.tx.query_row("SELECT is_on FROM project_integration WHERE project = ?1", [q.project], |r| r.get::<_, bool>(0))
        .optional().map_err(sql)?.unwrap_or(false);
    Ok(Measurements::missing(if on { "Integration detector is unavailable in the store catalog." } else { "Project integration is off." }))
}

fn heartbeat(_: &Query<'_, '_>) -> Result<Measurements, String> {
    Ok(Measurements::missing("The independent watchdog owns collector heartbeat findings."))
}

const LINE_UNAVAILABLE: &str = "The critical line records its own delivery health.";

fn line_health(_: &Query<'_, '_>) -> Result<Measurements, String> {
    Ok(Measurements::missing(LINE_UNAVAILABLE))
}

pub(super) const UNAVAILABLE: &[(&str, &str)] = &[
    ("flow.flaky", "Nightly and gate probe history is unavailable."),
    ("flow.main_red", "CI probe history is unavailable."),
    ("drift.rule_break", "Lint probe history is unavailable."),
    ("drift.scope", "Scope judgments require a recorded case."),
    ("arch.surface_touched", "Historical integration and git surface data are unavailable."),
    ("arch.migration_edited", "Git probe history is unavailable."),
    ("arch.dependency", "Git probe history is unavailable."),
    ("arch.hotspot", "Git probe history is unavailable."),
    ("arch.spec_drift", "Git probe history is unavailable."),
    ("arch.bug_cluster", "Task-to-commit history is unavailable."),
    ("arch.review_missing", "Historical Verify review records are unavailable."),
    ("improve.release_time", "Task commits and release tags are unavailable."),
    ("improve.releases", "Release tags are unavailable."),
    ("product.external", "No historical external metric probe is available."),
    ("product.feedback_age", "No historical feedback probe is available."),
    ("process.handoff_overdue", "Indexed handoff observations are unavailable."),
    ("process.rules_growth", "Git probe history is unavailable."),
    ("owner.rubber_stamp", "Structured question choices are unavailable."),
    ("process.rule_recurs", "Recorded rule judgments are unavailable."),
    ("self.budget", "Supervisor budget history is unavailable."),
    ("self.line_dead", LINE_UNAVAILABLE),
    ("self.round_failed", "Supervisor round history is unavailable."),
    ("self.fights", "Supervisor override history is unavailable."),
    ("self.unjudged", "Supervisor verdict history is unavailable."),
];
