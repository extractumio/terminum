// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A subscription's windows (`terminum-dispatcher.md` AC-167): how much of each, five hours and a week,
//! its agent's account has used, as a session of that kind last read them. Claude Code says them only to a
//! status line command (`terminum statusline`), Codex in its log's token counts (`terminum hook`); both send
//! numbers alone, as a `Limits` event. Kept per kind, in memory: the next reading comes with the next session.

use std::collections::BTreeMap;

use parking_lot::Mutex;
use serde_json::{json, Value};

/// The most windows one reading holds; past them a made-up event is not kept.
const MOST_WINDOWS: usize = 4;
/// The longest window a reading may name: a month of 31 days, in minutes.
const LONGEST: u64 = 31 * 24 * 60;

/// One window of a reading: its length, the percent used, when it starts anew (Unix seconds).
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub minutes: u64,
    pub used: f64,
    pub resets_at: Option<i64>,
}

impl Window {
    fn json(&self) -> Value {
        json!({"minutes": self.minutes, "used": self.used, "resets_at": self.resets_at})
    }
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

/// Claude Code's status line input: `rate_limits.five_hour` and `seven_day`, each `used_percentage` and `resets_at`.
/// None when it has neither (an API key, Bedrock, Vertex, before its first answer).
pub fn of_claude(input: &Value) -> Option<Vec<Window>> {
    let limits = &input["rate_limits"];
    let found: Vec<Window> = [("five_hour", 300), ("seven_day", 7 * 24 * 60)].iter()
        .filter_map(|(k, minutes)| limits.get(*k).filter(|w| w.is_object()).and_then(|w| window(Some(*minutes), w["used_percentage"].as_f64(), &w["resets_at"])))
        .collect();
    (!found.is_empty()).then_some(found)
}

/// Codex's `rate_limits` of a token count: `primary` and `secondary`, each `used_percent`, `window_minutes`, `resets_at`.
pub fn of_codex(limits: &Value) -> Option<Vec<Window>> {
    let found: Vec<Window> = ["primary", "secondary"].iter()
        .filter_map(|k| limits.get(*k).filter(|w| w.is_object()).and_then(|w| window(w["window_minutes"].as_u64(), w["used_percent"].as_f64(), &w["resets_at"])))
        .collect();
    (!found.is_empty()).then_some(found)
}

/// Windows as an event carries them.
pub fn json_of(windows: &[Window]) -> Value {
    windows.iter().map(Window::json).collect()
}

/// The last reading of each kind, and when it came.
#[derive(Default)]
pub struct Readings(Mutex<BTreeMap<String, (i64, Vec<Window>)>>);

impl Readings {
    /// A reading of `kind` came at `at`: it replaces the last. Whether its numbers differ from the last's.
    pub fn heard(&self, kind: &str, windows: Vec<Window>, at: i64) -> bool {
        self.0.lock().insert(kind.to_string(), (at, windows.clone())).is_none_or(|(_, was)| was != windows)
    }

    /// As pages show them: by kind, `{at, windows}`.
    pub fn shown(&self) -> Value {
        self.0.lock().iter().map(|(k, (at, w))| (k.clone(), json!({"at": at, "windows": json_of(w)}))).collect::<serde_json::Map<_, _>>().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac167_claude_s_two_windows() {
        let input = json!({"session_id": "s", "cwd": "/Users/alex/api", "model": {"id": "claude-opus-5-5"},
            "rate_limits": {"five_hour": {"used_percentage": 34.5, "resets_at": 1794350000}, "seven_day": {"used_percentage": 21, "resets_at": 1794600000}}});
        let got = of_claude(&input).unwrap();
        assert_eq!(got, vec![Window { minutes: 300, used: 34.5, resets_at: Some(1794350000) }, Window { minutes: 10080, used: 21.0, resets_at: Some(1794600000) }]);
        assert_eq!(json_of(&got).to_string().contains("alex"), false, "numbers alone leave the session");
    }

    #[test]
    fn ac167_claude_without_limits_is_no_reading() {
        assert_eq!(of_claude(&json!({"model": {"id": "x"}})), None, "an API key: nothing");
        assert_eq!(of_claude(&json!({"rate_limits": null})), None);
        assert_eq!(of_claude(&json!({"rate_limits": {"five_hour": null, "seven_day": null}})), None);
        assert_eq!(of_claude(&json!({"rate_limits": {"five_hour": {"used_percentage": 140}}})), None, "out of range");
        let one = of_claude(&json!({"rate_limits": {"seven_day": {"used_percentage": 5, "resets_at": null}}})).unwrap();
        assert_eq!(one, vec![Window { minutes: 10080, used: 5.0, resets_at: None }], "the one it has");
    }

    #[test]
    fn ac167_codex_s_windows_as_its_log_says_them() {
        let limits = json!({"limit_id": "codex", "primary": {"used_percent": 21.0, "window_minutes": 10080, "resets_at": 1794428830}, "secondary": null,
            "credits": {"has_credits": true, "balance": "75000"}, "plan_type": "prolite"});
        assert_eq!(of_codex(&limits).unwrap(), vec![Window { minutes: 10080, used: 21.0, resets_at: Some(1794428830) }], "no five-hour window: one");
        let two = json!({"primary": {"used_percent": 3, "window_minutes": 300, "resets_at": 1794350000}, "secondary": {"used_percent": 40, "window_minutes": 10080, "resets_at": 1794600000}});
        assert_eq!(of_codex(&two).unwrap().len(), 2);
        assert_eq!(of_codex(&json!({"primary": null, "secondary": null})), None);
        assert_eq!(of_codex(&json!({"primary": {"used_percent": 3, "window_minutes": 0}})), None, "a window of no length");
    }

    #[test]
    fn ac167_an_event_out_of_range_is_dropped_whole() {
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
    fn ac167_reset_times_in_seconds_or_milliseconds() {
        let w = |r: Value| window(Some(300), Some(1.0), &r).map(|w| w.resets_at);
        assert_eq!(w(json!(1794350000)), Some(Some(1794350000)));
        assert_eq!(w(json!(1794350000123u64)), Some(Some(1794350000)), "milliseconds");
        assert_eq!(w(json!("2026-11-11T12:00:00Z")), None, "a text: no reading");
        assert_eq!(w(Value::Null), Some(None), "not said: not known");
        assert_eq!(w(json!(-5)), None);
    }

    #[test]
    fn ac167_the_last_reading_of_each_kind_stands() {
        let r = Readings::default();
        assert!(r.heard("claude", vec![Window { minutes: 300, used: 10.0, resets_at: None }], 100), "the first: news");
        assert!(r.heard("codex", vec![Window { minutes: 10080, used: 21.0, resets_at: Some(5) }], 110));
        assert!(!r.heard("codex", vec![Window { minutes: 10080, used: 21.0, resets_at: Some(5) }], 115), "the same numbers: no news, a later time");
        assert!(r.heard("claude", vec![Window { minutes: 300, used: 12.0, resets_at: None }], 120));
        assert_eq!(r.shown(), json!({"claude": {"at": 120, "windows": [{"minutes": 300, "used": 12.0, "resets_at": null}]},
                                     "codex": {"at": 115, "windows": [{"minutes": 10080, "used": 21.0, "resets_at": 5}]}}));
    }
}
