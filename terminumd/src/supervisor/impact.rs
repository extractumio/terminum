// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213: score disjoint, project-scoped additive observations.

use serde::{Deserialize, Serialize};

use super::Scores;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Unit {
    FailureEvents,
    ScheduleEvents,
    RunEvents,
    OwnerHours,
    TaskDaysOfWait,
    TaskDaysToDone,
    /// Open P0 and P1 task-days, by the tasks' current priority.
    PriorityTaskDays,
    Tasks,
    Switches,
    Operations,
    StrategicActs,
    Tokens,
    Spend,
}

/// Every reading names its additive quantity, or why it has none (AC-213).
#[derive(Clone, Copy, Debug)]
pub(super) enum Source {
    Count(Unit),
    WaitDays,
    /// A task's days past its age limit, up to the measurement.
    OverdueDays,
    BlockingLinks,
    BacklogGrowth,
    /// A share of the week's spend above a target share.
    SpendAbove(f64),
    RunUsage,
    LeadDays,
    ExcessOwnerHours,
    ExcessOffHours,
    ExcessSwitches,
    ExcessOperations,
    Present,
    /// A rate above its band, sized as the excess count over that many observations.
    RateExcess(Unit, f64),
    /// A metric with no additive project quantity; the reason is recorded instead of a share.
    Unsized(&'static str),
}

pub(super) const OWN_HEALTH: &str = "The Supervisor's own health is in no weekly project total.";

impl Unit {
    pub const ALL: [Self; 13] = [
        Self::FailureEvents, Self::ScheduleEvents, Self::RunEvents, Self::OwnerHours, Self::TaskDaysOfWait,
        Self::TaskDaysToDone, Self::PriorityTaskDays, Self::Tasks, Self::Switches, Self::Operations, Self::StrategicActs,
        Self::Tokens, Self::Spend,
    ];

    fn counted_events(self) -> bool {
        matches!(self, Self::FailureEvents | Self::ScheduleEvents | Self::RunEvents | Self::Switches | Self::Operations
            | Self::StrategicActs)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Share {
    pub week: i64,
    pub complete: bool,
    pub unit: Unit,
    pub amount: f64,
    pub total: f64,
    pub noise: f64,
}

impl Share {
    pub fn fraction(&self) -> f64 {
        self.amount / self.total
    }

    fn enough_events(&self) -> bool {
        !self.unit.counted_events() || self.amount >= 3.0
    }

    fn high(&self) -> bool {
        let share = self.fraction();
        self.complete && self.enough_events() && share >= 0.10 && (self.noise <= 0.10 || share > self.noise || share >= 0.25)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Assessment {
    pub ranked: bool,
    pub shares: Vec<Share>,
    /// Why no share was measured, so an unsized signal is said, never silently level 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_share: Option<String>,
}

impl Assessment {
    pub fn peak_share(&self) -> f64 {
        self.shares.iter().map(Share::fraction).fold(0.0, f64::max)
    }

    pub fn score(&self, short_history: bool, charter_breach: bool) -> u8 {
        if charter_breach {
            return 3;
        }
        if !self.ranked {
            return 0;
        }
        let mut high_weeks = std::collections::BTreeMap::<Unit, usize>::new();
        let mut qualifying = std::collections::BTreeMap::<i64, std::collections::BTreeSet<Unit>>::new();
        for share in &self.shares {
            if share.high() {
                *high_weeks.entry(share.unit).or_default() += 1;
            }
            if share.complete && share.enough_events() && share.fraction() >= 0.05 {
                qualifying.entry(share.week).or_default().insert(share.unit);
            }
        }
        // Tokens and dollars measure the same use. Two independent quantities must qualify.
        let paired = qualifying.values().any(|units| {
            units.len() > 2 || (units.len() == 2 && !(units.contains(&Unit::Tokens) && units.contains(&Unit::Spend)))
        });
        if !short_history && (paired || high_weeks.values().any(|weeks| *weeks >= 3)) {
            return 2;
        }
        u8::from(self.peak_share() >= 0.03)
    }
}

/// Compare weekly values at their own scale. The median is robust against one outlier.
pub(super) fn absolute_noise(series: &[Option<f64>]) -> f64 {
    let mut steps: Vec<_> = series.windows(2).filter_map(|p| Some((p[1]? - p[0]?).abs())).collect();
    super::percentile(&mut steps, 50).unwrap_or(0.0)
}

pub(super) fn relative_noise(series: &[Option<f64>]) -> f64 {
    let mut values: Vec<_> = series.iter().flatten().copied().collect();
    let typical = super::percentile(&mut values, 50).unwrap_or(0.0);
    let noise = absolute_noise(series);
    if typical > 0.0 { noise / typical } else if noise > 0.0 { 1.0 } else { 0.0 }
}

impl Scores {
    pub(super) fn response(self, repeated: bool) -> &'static str {
        match self.level() {
            0 if repeated => "R1",
            0 => "R0",
            1 => "R2",
            2 => "R4",
            _ => "R5",
        }
    }

    pub(super) fn check_proposed_level(self, proposed: u8) -> Result<(), String> {
        if proposed < self.level() || proposed > 3 {
            return Err("A Supervisor round can only raise the signal's level, up to 3.".into());
        }
        // A proposal alone does not change the measured impact.
        Ok(())
    }
}
