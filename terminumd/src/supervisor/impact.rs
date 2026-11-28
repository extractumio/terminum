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
    /// An age in minutes since this time: the task's days past its limit, up to the measurement.
    AgeSince(i64),
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

pub(super) const OWN_HEALTH: Source = Source::Unsized("The Supervisor's own health is in no weekly project total.");

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
    /// The latest measured week's largest share; a verification weighs waiting by it (AC-233).
    pub fn latest_share(&self) -> Option<f64> {
        let week = self.shares.iter().map(|s| s.week).max()?;
        self.shares.iter().filter(|s| s.week == week && s.total > 0.0).map(Share::fraction).reduce(f64::max)
    }

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

pub(super) fn percentile(values: &mut [f64], p: u8) -> Option<f64> {
    values.sort_unstable_by(f64::total_cmp);
    values.get((values.len() * usize::from(p)).div_ceil(100).saturating_sub(1)).copied()
}

/// AC-212: `percentile` of sorted `values` with one `own` value left out, so a subject is ranked
/// against the project's other subjects; none left gives `None`, and the floor decides.
pub(super) fn percentile_of_others(values: &[f64], own: f64, p: u8) -> Option<f64> {
    let own_at = values.partition_point(|v| v.total_cmp(&own).is_le()).checked_sub(1)?;
    let at = ((values.len() - 1) * usize::from(p)).div_ceil(100).checked_sub(1)?;
    values.get(if at < own_at { at } else { at + 1 }).copied()
}

/// Compare weekly values at their own scale. The median is robust against one outlier.
pub(super) fn absolute_noise(series: &[Option<f64>]) -> f64 {
    let mut steps: Vec<_> = series.windows(2).filter_map(|p| Some((p[1]? - p[0]?).abs())).collect();
    percentile(&mut steps, 50).unwrap_or(0.0)
}

pub(super) fn relative_noise(series: &[Option<f64>]) -> f64 {
    let mut values: Vec<_> = series.iter().flatten().copied().collect();
    let typical = percentile(&mut values, 50).unwrap_or(0.0);
    let noise = absolute_noise(series);
    if typical > 0.0 { noise / typical } else if noise > 0.0 { 1.0 } else { 0.0 }
}

/// A response level's word, the only way a level reaches a human: bare numbers read as severities P0–P3,
/// which run the other way (§9, AC-213).
pub(super) const fn level_word(level: u8) -> &'static str {
    match level {
        0 => "Record",
        1 => "Digest",
        2 => "Card",
        _ => "Incident",
    }
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
