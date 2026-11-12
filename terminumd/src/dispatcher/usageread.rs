// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What a kind's usage command says (`terminum-dispatcher.md` AC-168), read into a subscription's windows:
//! Claude Code's `claude -p /usage --output-format json`, its `usage_report.rate_limits.limits`; Codex's
//! `codex app-server`, its answer to `account/rateLimits/read`. Neither is a model call: no tokens are spent.

use serde_json::Value;

use super::subscriptions::{Window, FIVE_HOURS, WEEK};

/// What terminumd writes to `codex app-server`: its handshake, then the question, answered as id 2.
pub const CODEX_ASK: &str = concat!(
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"terminum","version":"1"}}}"#, "\n",
    r#"{"jsonrpc":"2.0","method":"initialized"}"#, "\n",
    r#"{"jsonrpc":"2.0","id":2,"method":"account/rateLimits/read"}"#, "\n",
);

/// A window as a command says it, judged as an event's; a use past its limit is 100%.
fn window(minutes: u64, used: Option<f64>, resets_at: Option<i64>) -> Option<Window> {
    super::subscriptions::window(Some(minutes), used.map(|u| u.min(100.0)), &resets_at.into())
}

/// The first line of `text` that is not blank, without what would hide or act in it, cut to `most` characters.
pub fn first_line(text: &str, most: usize) -> Option<String> {
    text.lines().find(|l| !l.trim().is_empty()).map(|l| crate::termtext::clean(l, false).chars().take(most).collect())
}

/// A meter's `resets_at`: null is not known; a time not read leaves the meter out, as an event's would be.
fn resets(v: &Value) -> Option<Option<i64>> {
    if v.is_null() { Some(None) } else { v.as_str().and_then(iso_seconds).map(Some) }
}

/// Claude Code's meters by kind: `session` is the five hours, `weekly_all` the week of every model. A model's own
/// week (`weekly_scoped`) and kinds it may add are left out: the board's two stripes are the account's.
fn meter(kind: &str) -> Option<u64> {
    match kind {
        "session" => Some(FIVE_HOURS),
        "weekly_all" => Some(WEEK),
        _ => None,
    }
}

/// `claude -p /usage --output-format json`'s output: its messages as one JSON array (or one object, or a line
/// each); the windows of the first `usage_report` that has any. Err says why there are none.
pub fn of_claude(out: &str) -> Result<Vec<Window>, String> {
    let values: Vec<Value> = match serde_json::from_str::<Value>(out) {
        Ok(Value::Array(all)) => all,
        Ok(one) => vec![one],
        Err(_) => out.lines().filter_map(|l| serde_json::from_str(l).ok()).collect(),
    };
    if values.is_empty() {
        return Err("its output is not JSON".into());
    }
    for v in &values {
        let Some(limits) = v["usage_report"]["rate_limits"]["limits"].as_array() else { continue };
        let found: Vec<Window> = limits.iter()
            .filter(|l| l["scope"].is_null())
            .filter_map(|l| window(meter(l["kind"].as_str()?)?, l["percent"].as_f64(), resets(&l["resets_at"])?))
            .collect();
        if !found.is_empty() {
            return Ok(found);
        }
    }
    // an API key, Bedrock or Vertex: /usage has no subscription to say
    let said = values.iter().find_map(|v| v["result"].as_str().or(v["message"]["content"][0]["text"].as_str())).unwrap_or("");
    Err(match first_line(said, 120) {
        Some(line) => format!("no subscription limits in its answer: {line}"),
        None => "no subscription limits in its answer".into(),
    })
}

/// `codex app-server`'s answer to `account/rateLimits/read` (the line with id 2): `rateLimits.primary` and
/// `secondary`, each `usedPercent`, `windowDurationMins`, `resetsAt`.
pub fn of_codex(answer: &Value) -> Result<Vec<Window>, String> {
    if let Some(e) = answer["error"]["message"].as_str() {
        return Err(format!("codex app-server: {}", first_line(e, 120).unwrap_or_default()));
    }
    let limits = &answer["result"]["rateLimits"];
    let found: Vec<Window> = ["primary", "secondary"].iter()
        .filter_map(|k| window(limits[*k]["windowDurationMins"].as_u64()?, limits[*k]["usedPercent"].as_f64(), limits[*k]["resetsAt"].as_i64()))
        .collect();
    if found.is_empty() { Err("codex app-server said no rate limits (an API key has none)".into()) } else { Ok(found) }
}

/// `2026-11-16T02:00:00.472314+00:00` (or `Z`) as Unix seconds; None for anything else.
pub fn iso_seconds(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> { s.get(from..from + len).filter(|t| t.bytes().all(|c| c.is_ascii_digit()))?.parse().ok() };
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (y, mo, d, h, mi, sec) = (num(0, 4)?, num(5, 2)?, num(8, 2)?, num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    // the offset: after any fraction of a second
    let mut i = 19;
    if b.get(i) == Some(&b'.') {
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) { i += 1; }
    }
    let offset = match (b.get(i), s.len() - i) {
        (Some(b'Z'), 1) => 0,
        (Some(sign @ (b'+' | b'-')), 6) if b[i + 3] == b':' => {
            let o = num(i + 1, 2)? * 3600 + num(i + 4, 2)? * 60;
            if *sign == b'+' { o } else { -o }
        }
        _ => return None,
    };
    Some(crate::dashboard::days_of(y, mo, d) * 86400 + h * 3600 + mi * 60 + sec - offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ac168_iso_times() {
        assert_eq!(iso_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_seconds("2026-11-16T02:00:00.472314+00:00"), Some(1794794400));
        assert_eq!(iso_seconds("2026-11-16T03:00:00+01:00"), Some(1794794400), "an offset");
        assert_eq!(iso_seconds("2024-02-29T12:00:00Z"), Some(1709208000), "a leap day");
        for bad in ["", "soon", "2026-13-01T00:00:00Z", "2026-11-16 02:00:00Z", "2026-11-16T02:00:00", "2026-11-16T02:00:00+0100"] {
            assert_eq!(iso_seconds(bad), None, "{bad}");
        }
    }

    /// Claude Code 2.1's /usage output, with what is not numbers cut down.
    fn claude_out(limits: Value) -> String {
        json!([{"type": "system", "subtype": "init", "cwd": "/Users/alex/usage"},
               {"type": "assistant", "message": {"content": [{"type": "text", "text": "You are currently using your subscription"}]},
                "usage_report": {"session": {"total_cost_usd": 0}, "rate_limits": {"limits": limits}}},
               {"type": "result", "total_cost_usd": 0, "usage": {"input_tokens": 0, "output_tokens": 0}}]).to_string()
    }

    #[test]
    fn ac168_claude_s_usage_command() {
        let out = claude_out(json!([
            {"kind": "session", "group": "session", "percent": 34, "resets_at": "2026-11-12T14:00:00+00:00", "scope": null},
            {"kind": "weekly_all", "group": "weekly", "percent": 52, "resets_at": "2026-11-16T02:00:00.472314+00:00", "scope": null},
            {"kind": "weekly_scoped", "group": "weekly", "percent": 58, "resets_at": "2026-11-16T02:00:00+00:00", "scope": {"model": {"display_name": "Fable"}}}]));
        assert_eq!(of_claude(&out).unwrap(), vec![Window { minutes: FIVE_HOURS, used: 34.0, resets_at: Some(1794492000) }, Window { minutes: WEEK, used: 52.0, resets_at: Some(1794794400) }]);
        let week_only = claude_out(json!([{"kind": "weekly_all", "percent": 52, "resets_at": null, "scope": null}]));
        assert_eq!(of_claude(&week_only).unwrap(), vec![Window { minutes: WEEK, used: 52.0, resets_at: None }], "no five-hour meter: one");
    }

    #[test]
    fn ac168_claude_s_usage_command_with_no_subscription() {
        let api_key = json!([{"type": "result", "result": "You are using an API key\nno limits"}]).to_string();
        assert_eq!(of_claude(&api_key).unwrap_err(), "no subscription limits in its answer: You are using an API key");
        let over = of_claude(&claude_out(json!([{"kind": "weekly_all", "percent": 104, "resets_at": null, "scope": null}]))).unwrap();
        assert_eq!(over[0].used, 100.0, "past its limit: full, not left out");
        assert!(of_claude(&claude_out(json!([{"kind": "weekly_all", "percent": -1, "scope": null}]))).is_err(), "out of range");
        assert!(of_claude(&claude_out(json!([{"kind": "weekly_all", "percent": 5, "resets_at": "Monday", "scope": null}]))).is_err(), "a reset time not read: left out, said");
        assert_eq!(of_claude("Error: not logged in").unwrap_err(), "its output is not JSON");
        assert_eq!(of_claude("").unwrap_err(), "its output is not JSON");
    }

    #[test]
    fn ac168_codex_s_usage_command() {
        let answer = json!({"id": 2, "result": {"rateLimits": {"limitId": "codex", "primary": {"usedPercent": 0, "windowDurationMins": 10080, "resetsAt": 1795078866},
            "secondary": null, "credits": {"balance": "75000"}, "planType": "prolite"}, "accountId": "a5d0"}});
        assert_eq!(of_codex(&answer).unwrap(), vec![Window { minutes: WEEK, used: 0.0, resets_at: Some(1795078866) }]);
        assert!(of_codex(&json!({"id": 2, "result": {"rateLimits": {"primary": null, "secondary": null}}})).is_err());
        assert_eq!(of_codex(&json!({"id": 2, "error": {"code": -1, "message": "not logged in"}})).unwrap_err(), "codex app-server: not logged in");
    }
}
