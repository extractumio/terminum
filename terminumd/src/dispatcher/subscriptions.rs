// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A subscription's windows (`terminum-dispatcher.md` AC-168): how much of each, five hours and a week,
//! its agent's account has used, as a session of that kind last read them. Claude Code says them only to a
//! status line command (`terminum statusline`), Codex in its log's token counts (`terminum hook`); both send
//! numbers alone, as a `Limits` event. Each kind's usage command also says them (usageask.rs). Kept per kind,
//! in a file of the state folder: a restart shows the last reading, faded once old.

use std::collections::BTreeMap;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};

/// The most windows one reading holds; past them a made-up event is not kept.
const MOST_WINDOWS: usize = 4;
/// The longest window a reading may name: a month of 31 days, in minutes.
const LONGEST: u64 = 31 * 24 * 60;
/// The two windows the plans have, in minutes.
pub const FIVE_HOURS: u64 = 5 * 60;
pub const WEEK: u64 = 7 * 24 * 60;
/// A reading of the same numbers moves its time no oftener: a status line redraws far more often.
const FRESH: i64 = 600;

/// One window of a reading: its length, the percent used, when it starts anew (Unix seconds).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Window {
    pub minutes: u64,
    pub used: f64,
    pub resets_at: Option<i64>,
}

/// A window's name by its length, as the board says it: 5h, week, 1d.
pub fn label(minutes: u64) -> String {
    if minutes == WEEK { "week".into() } else if minutes.is_multiple_of(1440) { format!("{}d", minutes / 1440) } else { format!("{}h", (minutes as f64 / 60.0).round()) }
}

/// A time as Unix seconds, as both agents say it; a number past 10^11 is taken for milliseconds.
fn seconds(v: &Value) -> Option<i64> {
    let n = v.as_f64()?;
    let n = if n > 1e11 { n / 1000.0 } else { n };
    (n.is_finite() && n > 0.0 && n < 1e10).then_some(n as i64)
}

/// A window as an event says it; None when any number is out of its range.
fn window(minutes: Option<u64>, used: Option<f64>, resets: &Value) -> Option<Window> {
    let (minutes, used) = (minutes.filter(|m| (1..=LONGEST).contains(m))?, used.filter(|u| u.is_finite() && (0.0..=100.0).contains(u))?);
    let resets_at = if resets.is_null() { None } else { Some(seconds(resets)?) };
    Some(Window { minutes, used, resets_at })
}

/// The windows of a `Limits` event (`[{minutes, used, resets_at}]`); None when it holds none, too many, or one out of range:
/// the whole event is dropped then.
pub fn of_event(ev: &Value) -> Option<Vec<Window>> {
    let list = ev["windows"].as_array().filter(|l| !l.is_empty() && l.len() <= MOST_WINDOWS)?;
    list.iter().map(|w| window(w["minutes"].as_u64(), w["used"].as_f64(), &w["resets_at"])).collect()
}

/// Those of `found` that are windows; None when none is.
fn some(found: impl Iterator<Item = Option<Window>>) -> Option<Vec<Window>> {
    let found: Vec<Window> = found.flatten().collect();
    (!found.is_empty()).then_some(found)
}

/// Claude Code's status line input: `rate_limits.five_hour` and `seven_day`, each `used_percentage` and `resets_at`.
/// None when it has neither (an API key, Bedrock, Vertex, before its first answer).
pub fn of_claude(input: &Value) -> Option<Vec<Window>> {
    let limits = &input["rate_limits"];
    some([("five_hour", FIVE_HOURS), ("seven_day", WEEK)].iter().map(|(k, minutes)| window(Some(*minutes), limits[*k]["used_percentage"].as_f64(), &limits[*k]["resets_at"])))
}

/// Codex's `rate_limits` of a token count: `primary` and `secondary`, each `used_percent`, `window_minutes`, `resets_at`.
pub fn of_codex(limits: &Value) -> Option<Vec<Window>> {
    some(["primary", "secondary"].iter().map(|k| window(limits[*k]["window_minutes"].as_u64(), limits[*k]["used_percent"].as_f64(), &limits[*k]["resets_at"])))
}

/// One kind's last reading: when it came, its windows, and what said it: `session` or `usage` (its usage command).
#[derive(Clone, Debug, PartialEq)]
struct Reading {
    at: i64,
    windows: Vec<Window>,
    via: String,
}

/// The last reading of each kind, and why its usage command last failed, if it did: by kind, with when.
#[derive(Default)]
pub struct Readings {
    last: Mutex<BTreeMap<String, Reading>>,
    failed: Mutex<BTreeMap<String, (i64, String)>>,
}

impl Readings {
    /// A reading of `kind` came at `at` from `via`: it replaces the last when its numbers differ or the last is
    /// `FRESH` old. Whether it did: pages are told then. A usage command's reading clears its failure.
    pub fn heard(&self, kind: &str, windows: Vec<Window>, at: i64, via: &str) -> bool {
        let cleared = via == "usage" && self.failed.lock().remove(kind).is_some();
        let mut all = self.last.lock();
        if all.get(kind).is_some_and(|r| r.windows == windows && at - r.at < FRESH) {
            return cleared;
        }
        all.insert(kind.to_string(), Reading { at, windows, via: via.to_string() });
        true
    }

    /// `kind`'s usage command failed at `at`, for `why`: the last reading stands, and its window says why.
    pub fn failed(&self, kind: &str, why: String, at: i64) {
        self.failed.lock().insert(kind.to_string(), (at, why));
    }

    /// As pages show them: by kind, `{at, windows, via}`, and `{failed: {at, why}}` when its usage command failed.
    pub fn shown(&self) -> Value {
        let mut out = serde_json::Map::new();
        for (k, r) in self.last.lock().iter() {
            out.insert(k.clone(), json!({"at": r.at, "windows": r.windows, "via": r.via}));
        }
        for (k, (at, why)) in self.failed.lock().iter() {
            let e = out.entry(k.clone()).or_insert_with(|| json!({}));
            e["failed"] = json!({"at": at, "why": why});
        }
        out.into()
    }

    /// The readings as the state file keeps them: failures are not kept.
    pub fn kept(&self) -> Value {
        self.last.lock().iter().map(|(k, r)| (k.clone(), json!({"at": r.at, "windows": r.windows, "via": r.via}))).collect::<serde_json::Map<_, _>>().into()
    }

    /// The readings of a state file (`kept`), each judged as an event's: one out of range, or of a kind terminum
    /// does not know, is left out.
    pub fn load(&self, file: &Value) {
        let Some(all) = file.as_object() else { return };
        let mut last = self.last.lock();
        for (kind, r) in all.iter().filter(|(k, _)| crate::tmux::hookstate::agent(k).is_some()) {
            let (Some(at), Some(windows)) = (r["at"].as_i64().filter(|a| *a > 0), of_event(r)) else { continue };
            let via = if r["via"] == "usage" { "usage" } else { "session" };
            last.entry(kind.clone()).or_insert(Reading { at, windows, via: via.into() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac168_claude_s_two_windows() {
        let input = json!({"session_id": "s", "cwd": "/Users/alex/api", "model": {"id": "claude-opus-5-5"},
            "rate_limits": {"five_hour": {"used_percentage": 34.5, "resets_at": 1794350000}, "seven_day": {"used_percentage": 21, "resets_at": 1794600000}}});
        let got = of_claude(&input).unwrap();
        assert_eq!(got, vec![Window { minutes: 300, used: 34.5, resets_at: Some(1794350000) }, Window { minutes: 10080, used: 21.0, resets_at: Some(1794600000) }]);
        assert!(!json!(got).to_string().contains("alex"), "numbers alone leave the session");
    }

    #[test]
    fn ac168_claude_without_limits_is_no_reading() {
        assert_eq!(of_claude(&json!({"model": {"id": "x"}})), None, "an API key: nothing");
        assert_eq!(of_claude(&json!({"rate_limits": null})), None);
        assert_eq!(of_claude(&json!({"rate_limits": {"five_hour": null, "seven_day": null}})), None);
        assert_eq!(of_claude(&json!({"rate_limits": {"five_hour": {"used_percentage": 140}}})), None, "out of range");
        let one = of_claude(&json!({"rate_limits": {"seven_day": {"used_percentage": 5, "resets_at": null}}})).unwrap();
        assert_eq!(one, vec![Window { minutes: 10080, used: 5.0, resets_at: None }], "the one it has");
    }

    #[test]
    fn ac168_codex_s_windows_as_its_log_says_them() {
        let limits = json!({"limit_id": "codex", "primary": {"used_percent": 21.0, "window_minutes": 10080, "resets_at": 1794428830}, "secondary": null,
            "credits": {"has_credits": true, "balance": "75000"}, "plan_type": "prolite"});
        assert_eq!(of_codex(&limits).unwrap(), vec![Window { minutes: 10080, used: 21.0, resets_at: Some(1794428830) }], "no five-hour window: one");
        let two = json!({"primary": {"used_percent": 3, "window_minutes": 300, "resets_at": 1794350000}, "secondary": {"used_percent": 40, "window_minutes": 10080, "resets_at": 1794600000}});
        assert_eq!(of_codex(&two).unwrap().len(), 2);
        assert_eq!(of_codex(&json!({"primary": null, "secondary": null})), None);
        assert_eq!(of_codex(&json!({"primary": {"used_percent": 3, "window_minutes": 0}})), None, "a window of no length");
    }

    #[test]
    fn ac168_an_event_out_of_range_is_dropped_whole() {
        let ok = json!({"windows": [{"minutes": 300, "used": 34, "resets_at": 1794350000}, {"minutes": 10080, "used": 21, "resets_at": null}]});
        assert_eq!(of_event(&ok).map(|w| w.len()), Some(2));
        for bad in [json!({"windows": []}), json!({}), json!({"windows": [{"minutes": 300, "used": -1}]}), json!({"windows": [{"minutes": 300, "used": 101}]}),
                    json!({"windows": [{"minutes": 99999, "used": 1}]}), json!({"windows": [{"minutes": 300, "used": 1, "resets_at": "soon"}]}),
                    json!({"windows": [{"minutes": 300, "used": 1}, {"minutes": 300, "used": 1}, {"minutes": 300, "used": 1}, {"minutes": 300, "used": 1}, {"minutes": 300, "used": 1}]}),
                    json!({"windows": [{"minutes": 300, "used": 1}, {"minutes": 300, "used": "x"}]})] {
            assert_eq!(of_event(&bad), None, "{bad}");
        }
    }

    #[test]
    fn ac168_the_same_numbers_move_its_time_only_when_old() {
        let (r, w) = (Readings::default(), vec![Window { minutes: WEEK, used: 1.0, resets_at: None }]);
        assert!(r.heard("claude", w.clone(), 1000, "session"));
        assert!(!r.heard("claude", w.clone(), 1000 + FRESH - 1, "session"));
        assert!(r.heard("claude", w, 1000 + FRESH, "session"), "an old reading's time moves: its stripes do not fade while it is said");
        assert_eq!(r.shown()["claude"]["at"], 1000 + FRESH);
    }

    #[test]
    fn ac168_a_window_s_name() {
        assert_eq!([label(FIVE_HOURS), label(WEEK), label(1440), label(90)], ["5h", "week", "1d", "2h"]);
    }

    #[test]
    fn ac168_reset_times_in_seconds_or_milliseconds() {
        let w = |r: Value| window(Some(300), Some(1.0), &r).map(|w| w.resets_at);
        assert_eq!(w(json!(1794350000)), Some(Some(1794350000)));
        assert_eq!(w(json!(1794350000123u64)), Some(Some(1794350000)), "milliseconds");
        assert_eq!(w(json!("2026-11-11T12:00:00Z")), None, "a text: no reading");
        assert_eq!(w(Value::Null), Some(None), "not said: not known");
        assert_eq!(w(json!(-5)), None);
    }

    #[test]
    fn ac168_the_last_reading_of_each_kind_stands() {
        let r = Readings::default();
        assert!(r.heard("claude", vec![Window { minutes: 300, used: 10.0, resets_at: None }], 100, "session"), "the first: news");
        assert!(r.heard("codex", vec![Window { minutes: 10080, used: 21.0, resets_at: Some(5) }], 110, "usage"));
        assert!(!r.heard("codex", vec![Window { minutes: 10080, used: 21.0, resets_at: Some(5) }], 115, "usage"), "the same numbers soon after: no news");
        assert!(r.heard("claude", vec![Window { minutes: 300, used: 12.0, resets_at: None }], 120, "session"));
        assert_eq!(r.shown(), json!({"claude": {"at": 120, "windows": [{"minutes": 300, "used": 12.0, "resets_at": null}], "via": "session"},
                                     "codex": {"at": 110, "windows": [{"minutes": 10080, "used": 21.0, "resets_at": 5}], "via": "usage"}}));
    }

    #[test]
    fn ac168_a_failed_usage_command_is_said_and_the_last_reading_stands() {
        let (r, w) = (Readings::default(), vec![Window { minutes: WEEK, used: 52.0, resets_at: None }]);
        r.failed("codex", "codex: not found".into(), 50);
        assert_eq!(r.shown(), json!({"codex": {"failed": {"at": 50, "why": "codex: not found"}}}), "never a made-up 0%");
        assert!(r.heard("claude", w.clone(), 100, "usage"));
        r.failed("claude", "timed out".into(), 200);
        assert_eq!(r.shown()["claude"]["windows"][0]["used"], 52.0, "the last reading stands");
        assert_eq!(r.shown()["claude"]["failed"]["why"], "timed out");
        assert!(r.heard("claude", w, 210, "usage"), "the same numbers that clear a failure are news");
        assert!(r.shown()["claude"]["failed"].is_null());
        assert_eq!(r.kept().get("codex"), None, "a failure is not kept");
    }

    #[test]
    fn ac168_the_state_file_brings_back_the_last_readings() {
        let r = Readings::default();
        r.heard("claude", vec![Window { minutes: WEEK, used: 52.0, resets_at: Some(1795000000) }], 100, "usage");
        let again = Readings::default();
        again.load(&r.kept());
        assert_eq!(again.shown(), r.shown());
        let bad = json!({"claude": {"at": 5, "windows": [{"minutes": 300, "used": 140}]}, "gemini": {"at": 5, "windows": [{"minutes": 300, "used": 1}]},
                         "codex": {"at": -1, "windows": [{"minutes": 300, "used": 1}]}});
        let none = Readings::default();
        none.load(&bad);
        none.load(&json!("not an object"));
        assert_eq!(none.shown(), json!({}), "out of range, an unknown kind, no time: left out");
    }
}
