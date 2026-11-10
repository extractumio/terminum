// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The Dashboard's numbers (terminum.md AC-152, AC-171, AC-172), one tab an answer: `you` (how much the
//! human works here and how loaded), `system` (Dispatcher's sessions, failures, the agents' load), `costs`
//! (tokens and dollars). All in this Mac's local time, over a period cut into buckets, each figure against
//! the same span before. `GET /api/dashboard?tab=&period=&project=`; reads only, off the async workers.

pub mod costs;
pub mod system;
#[cfg(test)]
mod tests;
pub mod you;

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use rusqlite::Transaction;
use serde_json::{json, Value};

use crate::app::Shared;
use crate::tasks::api::{answer, store};
use crate::tasks::ops::{rows, sql};

pub const DAY: i64 = 86_400_000;
pub const HOUR: i64 = 3_600_000;
/// Projects with their own color in a stacked chart; the rest are Other.
pub const STACKED: usize = 6;

/// SQL of the local time in ms of unix ms `col` (the zone's offset at that moment, DST too).
pub fn local(col: &str) -> String {
    format!("({col} + (CAST(strftime('%s', ({col}) / 1000, 'unixepoch', 'localtime') AS INTEGER) - ({col}) / 1000) * 1000)")
}

/// The periods the page offers: the last 24 hours by the hour, 14 and 30 days by the day, 13 weeks by the
/// week, 12 calendar months by the month.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Period { Day, TwoWeeks, Month, Quarter, Year }

impl Period {
    /// As the address says it; anything else is the month.
    pub fn of(s: &str) -> Period {
        match s {
            "day" => Period::Day,
            "2w" => Period::TwoWeeks,
            "quarter" => Period::Quarter,
            "year" => Period::Year,
            _ => Period::Month,
        }
    }

    /// Its days: how far back the span before lies.
    fn days(self) -> i64 {
        match self { Period::Day => 1, Period::TwoWeeks => 14, Period::Month => 30, Period::Quarter => 91, Period::Year => 365 }
    }
}

/// A span of local time up to `until`, cut into buckets that begin at `edges`.
#[derive(Clone, Debug)]
pub struct Span {
    /// what a bucket is: hour, day, week, month
    pub unit: &'static str,
    pub edges: Vec<i64>,
    pub until: i64,
}

impl Span {
    /// The span of `period` that ends at `now` (local ms).
    pub fn of(period: Period, now: i64) -> Span {
        let today = now.div_euclid(DAY);
        let n = period.days();
        let days = |step: i64| (0..n / step).map(|i| (today - n + 1 + i * step) * DAY).collect();
        let (unit, edges) = match period {
            Period::Day => ("hour", (0..24).map(|i| (now.div_euclid(HOUR) - 23 + i) * HOUR).collect()),
            Period::TwoWeeks | Period::Month => ("day", days(1)),
            Period::Quarter => ("week", days(7)),
            Period::Year => {
                let (y, m, _) = civil_of(today);
                ("month", (0..12).map(|i| { let k = y * 12 + m - 1 - 11 + i; days_of(k.div_euclid(12), k.rem_euclid(12) + 1, 1) * DAY }).collect())
            }
        };
        Span { unit, edges, until: now }
    }

    /// The same span, `period`'s length earlier, up to the same time.
    pub fn before(&self, period: Period) -> Span {
        let by = period.days() * DAY;
        Span { unit: self.unit, edges: self.edges.iter().map(|e| e - by).collect(), until: self.until - by }
    }

    pub fn by_hour(&self) -> bool {
        self.unit == "hour"
    }

    pub fn by_day(&self) -> bool {
        self.unit == "day"
    }

    pub fn lo(&self) -> i64 {
        self.edges[0]
    }

    pub fn len(&self) -> usize {
        self.edges.len()
    }

    pub fn holds(&self, t: i64) -> bool {
        t >= self.lo() && t <= self.until
    }

    /// The bucket of local time `t`; None outside the span.
    pub fn index(&self, t: i64) -> Option<usize> {
        self.holds(t).then(|| self.edges.partition_point(|e| *e <= t) - 1)
    }

    /// Where bucket `i` ends: the next one's start, the last one's at `until`.
    pub fn end(&self, i: usize) -> i64 {
        self.edges.get(i + 1).copied().unwrap_or(self.until + 1)
    }

    /// The local days it touches.
    pub fn days(&self) -> i64 {
        self.until.div_euclid(DAY) - self.lo().div_euclid(DAY) + 1
    }

    /// Each bucket for the page: its first and last local day, an hour's hour.
    fn json(&self) -> Value {
        (0..self.len()).map(|i| {
            let from = self.edges[i];
            json!({"from": civil(from.div_euclid(DAY)), "to": civil((self.end(i) - 1).div_euclid(DAY)), "hour": self.by_hour().then(|| from.rem_euclid(DAY) / HOUR)})
        }).collect()
    }
}

/// What a tab is asked: the span and, when what is kept reaches its start, the one before it (a part of a
/// span compared with a whole one would read as a rise); the unix ms no row before which matters; one
/// project's only (`""`: none's); the session list's colors.
pub struct Ask<'a> {
    pub span: &'a Span,
    pub before: Option<&'a Span>,
    pub since: i64,
    pub project: Option<&'a str>,
    pub colors: &'a HashMap<String, usize>,
}

impl Ask<'_> {
    /// The figures of the span before as `tiles` works them out, or null when it is not compared.
    pub fn earlier(&self, tiles: impl FnOnce(&Span) -> Result<Value, String>) -> Result<Value, String> {
        self.before.map_or(Ok(Value::Null), tiles)
    }

    /// A tasks' store project of the filter: no project's rows are the default project's there.
    pub fn task_project(&self) -> Option<&str> {
        self.project.map(|p| if p.is_empty() { "default" } else { p })
    }

    /// A project as the page draws it: id (`""` none), name, the session list's color.
    pub fn project_json(&self, id: &str, name: &str) -> Value {
        json!({"id": id, "name": name, "color": self.colors.get(id)})
    }
}

/// The tasks' store's projects' names; the default project is no project (`""`).
pub fn project_names(tx: &Transaction) -> Result<HashMap<String, String>, String> {
    Ok(rows(tx, "SELECT CASE WHEN is_default THEN '' ELSE id END, CASE WHEN is_default THEN '' ELSE name END FROM projects", [], |r| Ok((r.get(0)?, r.get(1)?)))?.into_iter().collect())
}

/// The first `STACKED` of `ranked` ids (most first): those a stack colors apart.
pub fn stacked<'a>(ranked: impl Iterator<Item = &'a str>) -> Vec<String> {
    ranked.take(STACKED).map(str::to_string).collect()
}

/// `id` when a stack colors it apart, else `other`.
pub fn stack_key(stacked: &[String], id: &str) -> String {
    if stacked.iter().any(|s| s == id) { id.to_string() } else { "other".into() }
}

pub fn median(mut v: Vec<i64>) -> Option<i64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

/// The numbers of `tab` over `period` up to unix ms `now_ms`.
pub fn numbers(tx: &Transaction, now_ms: i64, tab: &str, period: Period, project: Option<&str>, colors: &HashMap<String, usize>) -> Result<Value, String> {
    let now: i64 = tx.query_row(&format!("SELECT {}", local("?1")), [now_ms], |r| r.get(0)).map_err(sql)?;
    let span = Span::of(period, now);
    let before = span.before(period);
    // what the tab's rows still hold: the input log is trimmed by age and by rows, the reports are not
    let oldest = if tab == "costs" { "SELECT min(at) * 1000 AS at FROM usage" } else { "SELECT min(at) AS at FROM inputs" };
    let kept_from: Option<i64> = tx.query_row(&format!("SELECT {} FROM ({oldest}) WHERE at IS NOT NULL", local("at")), [], |r| r.get(0)).ok();
    let before = Some(&before).filter(|b| kept_from.is_some_and(|k| k <= b.lo()));
    // a day more: the zone's offset may have been another then
    let ask = Ask { span: &span, before, since: now_ms - (now - before.unwrap_or(&span).lo()) - DAY, project, colors };
    let mut v = match tab {
        "system" => system::numbers(tx, &ask, now)?,
        "costs" => costs::numbers(tx, &ask, now_ms / 1000)?,
        _ => you::numbers(tx, &ask)?,
    };
    v["unit"] = json!(span.unit);
    v["buckets"] = span.json();
    v["kept_from"] = json!(kept_from.filter(|k| *k > span.lo()).map(|k| civil(k.div_euclid(DAY))));
    Ok(v)
}

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/dashboard", get(dashboard))
}

async fn dashboard(State(app): State<Shared>, Query(q): Query<HashMap<String, String>>) -> Response {
    let period = Period::of(q.get("period").map_or("", String::as_str));
    let tab = q.get("tab").cloned().unwrap_or_default();
    // as the Message history's filter: absent or empty, every project; NONE, no project
    let project = q.get("project").filter(|p| !p.is_empty()).map(|p| if p == crate::messages::search::NONE { String::new() } else { p.clone() });
    let colors = crate::messages::api::colors(&app);
    answer(store(&app, move |tx| numbers(tx, crate::inputs::now_ms(), &tab, period, project.as_deref(), &colors)).await)
}

/// `YYYY-MM-DD` of day `z` since 1970-01-01.
pub fn civil(z: i64) -> String {
    let (y, m, d) = civil_of(z);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Year, month, day of day `z` since 1970-01-01 (Howard Hinnant's civil_from_days).
pub fn civil_of(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The day since 1970-01-01 of a date (Howard Hinnant's days_from_civil).
pub fn days_of(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
