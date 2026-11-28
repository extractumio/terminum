// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use super::{Measurements, Query, Threshold, flow, integration_rules, outside_feedback, outside_indicators, people, probe_rules,
    report_rules, review_rules, samples, spend};

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
    Stuck, "flow.stuck", "stuck_tasks", Absolute(1440.0), flow::stuck;
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
    Returned, "flow.returned", "integration_returns", change(0.0), integration_rules::returned;
    VerifyRed, "drift.verify_red", "verify_failures", Absolute(0.0), integration_rules::verify;
    DiffOutlier, "drift.diff_outlier", "diff_outliers", baseline(95, 100.0), integration_rules::diffs;
    MainRed, "flow.main_red", "target_red", Absolute(0.0), integration_rules::target_red;
    SurfaceTouched, "arch.surface_touched", "surface_touches", Absolute(1.0), review_rules::surface_touched;
    ReviewMissing, "arch.review_missing", "verify_without_review", Absolute(0.0), review_rules::review_missing;
    RubberStamp, "owner.rubber_stamp", "recommended_streak", Absolute(5.0), people::rubber_stamp;
    Misrouted, "drift.misrouted", "misrouted_tasks", Absolute(0.0), integration;
    Flaky, "flow.flaky", "gate_failures", Absolute(0.0), probe_rules::flaky;
    Hotspot, "arch.hotspot", "git_hotspots", baseline(95, 1000000.0), probe_rules::hotspots;
    Dependency, "arch.dependency", "git_dependencies", Absolute(0.0), probe_rules::dependencies;
    MigrationEdited, "arch.migration_edited", "git_migrations", Absolute(0.0), probe_rules::migrations;
    BugCluster, "arch.bug_cluster", "git_bug_clusters", baseline(95, 3.0), probe_rules::bug_clusters;
    ReleaseTime, "improve.release_time", "task_release_lead_time", change(1.0), probe_rules::release_time;
    Releases, "improve.releases", "release_cadence", change(0.0), probe_rules::releases;
    External, "product.external", "outside_metric_moves", change(0.0), outside_indicators::external;
    FeedbackAge, "product.feedback_age", "unanswered_feedback", Absolute(4320.0), outside_feedback::age;
    ProbeFailed, "self.probe_failed", "probe_failures", Absolute(0.0), probe_rules::failures;
    QueueGrowth, "flow.queue_growth", "queue_growth", Absolute(0.0), samples::snapshots;
    LimitNear, "flow.limit_near", "subscription_limits", Absolute(100.0), samples::snapshots;
    StalePlan, "drift.stale_plan", "stale_plans", Absolute(0.0), samples::snapshots;
    Loop, "drift.loop", "tool_loops", baseline(90, 2.0), samples::counters;
    Subagents, "drift.subagents", "subagent_counts", baseline(90, 2.0), samples::counters;
    CollectorDead, "self.collector_dead", "collector_heartbeat", Absolute(3.0), heartbeat;
    LineDead, "self.line_dead", "line_health", Absolute(0.0), line_health;
    ReportCritical, "reports.critical", "critical_reports", Absolute(0.0), report_rules::critical;
    UnreadReports, "reports.unread_age", "unread_reports", Absolute(1440.0), report_rules::unread_age;
    FollowUps, "reports.follow_up_age", "unanswered_replies", Absolute(1440.0), report_rules::follow_up_age;
}

impl Rule {
    pub(super) fn probe_source(self) -> Option<&'static str> {
        use super::probes::{ARCH, GATE, GIT};
        match self {
            Self::Hotspot | Self::Dependency | Self::MigrationEdited | Self::BugCluster | Self::DiffOutlier => Some(ARCH),
            Self::ReleaseTime | Self::Releases => Some(GIT),
            Self::Flaky => Some(GATE),
            _ => None,
        }
    }

    pub(super) fn configurable(self) -> bool {
        !matches!(self, Self::CollectorDead | Self::LineDead | Self::ProbeFailed | Self::ReportCritical)
    }

    /// A rule whose finding its author declared, a critical report (terminum-reports.md AC-276): its level is
    /// theirs, never sized by the project's weeks; its incident makes no report of its own and names its reports.
    pub(super) fn authored(self) -> bool {
        self == Self::ReportCritical
    }

    pub(super) fn fixed_impact(self) -> Option<u8> {
        self.authored().then_some(3)
    }

    /// A detector whose firing is a counted fact, in the owner's words: only such a detector can break
    /// a charter never the owner ties to it (AC-213). A statistic, or a fact that fires most weeks in an
    /// active project (a lockfile gaining a package), is not one.
    pub(super) fn checked_fact(self) -> Option<&'static str> {
        match self {
            Self::MigrationEdited => Some("An existing migration file is edited"),
            Self::MainRed => Some("The target is red: steps a Verify failed fail on it too"),
            _ => None,
        }
    }

    /// A rule that measures only the present state, with why its history is unavailable.
    pub(super) fn present_only(self) -> Option<&'static str> {
        match self {
            Self::Starved | Self::UserBugAge => Some("Historical slot or priority state is unavailable."),
            Self::MainRed => Some("Red-target history keeps only the open Bug task."),
            Self::Stuck => Some("Whether an agent works on a task is sampled only at present."),
            Self::ReportCritical | Self::UnreadReports | Self::FollowUps => Some("A report's read state keeps no history."),
            _ => None,
        }
    }

    pub(super) fn needs_weekly_samples(self) -> bool {
        self.present_only().is_none() && !matches!(self, Self::WaitHuman | Self::CostOutlier | Self::QueueGrowth | Self::LimitNear | Self::StalePlan | Self::Loop | Self::Subagents | Self::CollectorDead | Self::LineDead | Self::ProbeFailed)
    }

    pub(super) fn has_historical_query(self) -> bool {
        self.present_only().is_none() && !matches!(self, Self::Misrouted
            | Self::CollectorDead | Self::LineDead)
    }

    pub(super) fn supports(self, threshold: Threshold) -> bool {
        if self == Self::Returned && matches!(threshold, Threshold::Absolute(_)) { return true; }
        match self.defaults() {
            Threshold::Change { .. } => matches!(threshold, Threshold::Change { .. }),
            Threshold::Baseline { .. } => !matches!(threshold, Threshold::Change { .. }),
            Threshold::Absolute(_) => matches!(threshold, Threshold::Absolute(_)),
        }
    }
}

pub(super) fn integration_on(tx: &rusqlite::Transaction, project: &str) -> Result<bool, String> {
    use rusqlite::OptionalExtension;
    use crate::tasks::ops::sql;

    Ok(tx.query_row("SELECT is_on FROM project_integration WHERE project = ?1", [project], |r| r.get(0))
        .optional().map_err(sql)?.unwrap_or(false))
}

fn integration(q: &Query<'_, '_>) -> Result<Measurements, String> {
    Ok(Measurements::missing(if integration_on(q.tx, q.project)? { "Integration detector is unavailable in the store catalog." } else { "Project integration is off." }))
}

fn heartbeat(_: &Query<'_, '_>) -> Result<Measurements, String> {
    Ok(Measurements::missing("The independent watchdog owns collector heartbeat findings."))
}

const LINE_UNAVAILABLE: &str = "The critical line records its own delivery health.";

fn line_health(_: &Query<'_, '_>) -> Result<Measurements, String> {
    Ok(Measurements::missing(LINE_UNAVAILABLE))
}

pub(super) const UNAVAILABLE: &[(&str, &str)] = &[
    ("drift.rule_break", "Lint probe history is unavailable."),
    ("drift.scope", "Scope judgments require a recorded case."),
    ("arch.spec_drift", "Git probe history is unavailable."),
    ("process.handoff_overdue", "Indexed handoff observations are unavailable."),
    ("process.rules_growth", "Git probe history is unavailable."),
    ("process.rule_recurs", "Recorded rule judgments are unavailable."),
    ("self.budget", "Supervisor budget history is unavailable."),
    ("self.line_dead", LINE_UNAVAILABLE),
    ("self.fights", "Supervisor override history is unavailable."),
    ("self.unjudged", "Supervisor verdict history is unavailable."),
];
