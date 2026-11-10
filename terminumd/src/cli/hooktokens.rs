// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The tokens of a turn's session (`terminum-dispatcher.md` AC-129), summed beside the transcript on
//! its own machine within `terminum hook`'s time. Integers and models' names leave it, the whole and each
//! model's: no path, no text.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::dispatcher::subscriptions::{self, Window};
use crate::dispatcher::usage::{Totals, MOST_MODEL};

/// Past these it says nothing rather than overrun the hook's 3 s.
const MOST_BYTES: u64 = 256 << 20;
const MOST_TIME: Duration = Duration::from_millis(1500);
const CODEX_TAIL: u64 = 1 << 20;

/// Each model's totals, `""` for none named.
type Sum = BTreeMap<String, Totals>;

fn int(v: &Value, key: &str) -> i64 {
    v[key].as_u64().map_or(0, |n| n.min(i64::MAX as u64) as i64)
}

/// Each line holding one of `needles`, parsed, to `each`; past `until`: None.
fn each_line<T: DeserializeOwned>(mut reader: impl BufRead, until: Instant, needles: &[&str], mut each: impl FnMut(usize, T)) -> Option<()> {
    let (mut line, mut at) = (String::new(), 0);
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            return Some(());
        }
        if Instant::now() >= until {
            return None;
        }
        if needles.iter().any(|n| line.contains(n)) && let Ok(v) = serde_json::from_str(&line) {
            each(at, v);
        }
        at += 1;
    }
}

/// Of a Claude Code line only these are parsed: a message's text is skipped unread.
#[derive(Deserialize)]
struct ClaudeLine {
    message: ClaudeMessage,
}

#[derive(Deserialize)]
struct ClaudeMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<ClaudeUsage>,
}

#[derive(Deserialize)]
struct ClaudeUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_creation: Option<CacheCreation>,
}

/// Of `cache_creation_input_tokens`, those kept an hour.
#[derive(Deserialize)]
struct CacheCreation {
    ephemeral_1h_input_tokens: Option<u64>,
}

/// Claude Code's lines: `message.usage` per message, priced by its own model; a message comes in several lines,
/// the last stands.
fn claude(reader: impl BufRead, until: Instant) -> Option<Sum> {
    let mut by_message: HashMap<String, (String, Totals)> = HashMap::new();
    each_line(reader, until, &["\"usage\""], |at, line: ClaudeLine| {
        if let Some(u) = line.message.usage {
            let n = |v: Option<u64>| v.map_or(0, |n| n.min(i64::MAX as u64) as i64);
            let cache_write = n(u.cache_creation_input_tokens);
            let hour = n(u.cache_creation.and_then(|c| c.ephemeral_1h_input_tokens)).min(cache_write);
            by_message.insert(line.message.id.unwrap_or_else(|| at.to_string()), (line.message.model.map(|m| m.chars().take(MOST_MODEL).collect()).unwrap_or_default(),
                Totals { input: n(u.input_tokens), output: n(u.output_tokens), cache_write, cache_write_1h: hour, cache_read: n(u.cache_read_input_tokens) }));
        }
    })?;
    if by_message.is_empty() {
        return None;
    }
    let mut sum = Sum::new();
    for (m, t) in by_message.into_values() {
        let of = sum.entry(m).or_default();
        *of = *of + t;
    }
    Some(sum)
}

/// What Codex's lines say: the session's last total, its last turn's model, its account's last `rate_limits` (AC-168).
#[derive(Default)]
struct CodexRead {
    total: Option<Totals>,
    model: String,
    limits: Value,
}

/// Codex's lines: a `token_count` event holds the session's total, its input with the cached part in it, and its
/// account's `rate_limits`; a `turn_context` names the turn's model. The last of each: the whole is priced at the last model.
fn codex(reader: impl BufRead, until: Instant) -> Option<CodexRead> {
    let mut read = CodexRead::default();
    each_line(reader, until, &[r#""type":"token_count""#, r#""type":"turn_context""#], |_, mut v: Value| {
        if v["type"] == "turn_context" && let Some(m) = v["payload"]["model"].as_str() {
            read.model = m.chars().take(MOST_MODEL).collect();
        } else if v["payload"]["type"] == "token_count" {
            if let Some(u) = v.pointer("/payload/info/total_token_usage") {
                let cached = int(u, "cached_input_tokens");
                read.total = Some(Totals { input: (int(u, "input_tokens") - cached).max(0), output: int(u, "output_tokens"), cache_read: cached, ..Totals::default() });
            }
            // a count without them is no reset: the last that has them stands, read once at the end
            if v["payload"]["rate_limits"].is_object() {
                read.limits = v["payload"]["rate_limits"].take();
            }
        }
    })?;
    Some(read)
}

/// The transcript an event names.
fn transcript(input: &Value) -> Option<&str> {
    input.get("transcript_path").and_then(Value::as_str)
}

/// `path` opened when it is a regular file of a size the hook reads; its length.
fn opened(path: &str) -> Option<(std::fs::File, u64)> {
    // a regular file: a FIFO or a device named there would block past any limit
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file() && m.len() <= MOST_BYTES)?;
    Some((std::fs::File::open(path).ok()?, meta.len()))
}

/// Codex's account's windows as its log's tail last says them (AC-168): numbers alone; None when the tail has none.
pub fn codex_windows(input: &Value) -> Option<Vec<Window>> {
    let (mut file, len) = opened(transcript(input)?)?;
    file.seek(SeekFrom::Start(len.saturating_sub(CODEX_TAIL))).ok()?;
    subscriptions::of_codex(&codex(std::io::BufReader::new(&file), Instant::now() + MOST_TIME)?.limits)
}

/// What goes with a turn's end: the whole and each model's numbers, or `null` when they are not known (a new
/// agent version's format, a file gone, too long a read); nothing when the event names no transcript.
pub fn of_event(agent: &str, input: &Value) -> Option<Value> {
    let path = transcript(input)?;
    let until = Instant::now() + MOST_TIME;
    let read = || -> Option<Sum> {
        let (mut file, len) = opened(path)?;
        if agent == "codex" {
            // the last total and model: the end of the file holds them, at no cost as a session grows; what it
            // lacks is looked for before it, the tail's alone when that is past the time
            let start = len.saturating_sub(CODEX_TAIL);
            file.seek(SeekFrom::Start(start)).ok()?;
            let CodexRead { mut total, mut model, .. } = codex(std::io::BufReader::new(&file), until).unwrap_or_default();
            if start > 0 && (total.is_none() || model.is_empty()) {
                file.seek(SeekFrom::Start(0)).ok()?;
                if let Some(r) = codex(std::io::BufReader::new((&file).take(start)), until) {
                    total = total.or(r.total);
                    if model.is_empty() {
                        model = r.model;
                    }
                }
            }
            return Some(Sum::from([(model, total?)]));
        }
        claude(std::io::BufReader::new(file), until)
    };
    Some(read().map_or(Value::Null, |s| {
        let mut v = s.values().fold(Totals::default(), |a, t| a + *t).json();
        v["models"] = s.iter().map(|(m, t)| {
            let mut e = t.json();
            e["model"] = json!((!m.is_empty()).then_some(m));
            e
        }).collect();
        v
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(name: &str, text: &str) -> String {
        let path = crate::testutil::fresh_dir(name).join("t.jsonl");
        std::fs::write(&path, text).unwrap();
        path.display().to_string()
    }

    #[test]
    fn ac129_claude_a_message_counts_once_at_its_last_line() {
        let p = written("tokens-claude", concat!(
            r#"{"type":"user","message":{"content":"secret words"}}"#, "\n",
            r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":5,"cache_creation_input_tokens":2}}}"#, "\n",
            r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":10,"output_tokens":7,"cache_read_input_tokens":5,"cache_creation_input_tokens":2}}}"#, "\n",
            r#"{"type":"assistant","message":{"id":"m2","usage":{"input_tokens":3,"output_tokens":4}}}"#, "\n"));
        let got = of_event("claude", &json!({"transcript_path": p})).unwrap();
        let whole = json!({"input": 13, "output": 11, "cache_write": 2, "cache_write_1h": 0, "cache_read": 5});
        let mut one = whole.clone();
        one["model"] = Value::Null;
        let mut said = whole;
        said["models"] = json!([one]);
        assert_eq!(got, said, "integers only: no text, no path");
    }

    #[test]
    fn ac129_claude_a_line_is_read_for_its_id_and_usage_only() {
        let p = written("tokens-claude-typed", concat!(
            r#"{"type":"user","message":{"content":[{"text":"the word \"usage\" in a prompt"}]}}"#, "\n",
            r#"{"type":"summary","message":"\"usage\" in a string"}"#, "\n",
            r#"{"message":{"content":[{"type":"text","text":"long"}],"usage":{"input_tokens":2,"output_tokens":1,"cache_read_input_tokens":null,"cache_creation":{"ephemeral_5m_input_tokens":9}}}}"#, "\n",
            r#"{"message":{"usage":{"input_tokens":3,"output_tokens":1}}}"#, "\n"));
        let got = of_event("claude", &json!({"transcript_path": p})).unwrap();
        assert_eq!((got["input"].as_i64(), got["output"].as_i64(), got["cache_write"].as_i64()), (Some(5), Some(2), Some(0)), "no id: each line its own; no usage: not counted");
    }

    #[test]
    fn ac129_codex_its_last_total_stands() {
        let p = written("tokens-codex", concat!(
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2}}}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":9,"cached_input_tokens":3,"output_tokens":4}}}}"#, "\n"));
        let got = of_event("codex", &json!({"transcript_path": p})).unwrap();
        assert_eq!((got["input"].as_i64(), got["output"].as_i64(), got["cache_read"].as_i64()), (Some(6), Some(4), Some(3)), "its input holds the cached part: counted once");
        assert_eq!(got["models"], json!([{"model": null, "input": 6, "output": 4, "cache_write": 0, "cache_write_1h": 0, "cache_read": 3}]), "no turn names a model");
    }

    #[test]
    fn ac129_codex_its_last_turn_s_model_prices_the_whole() {
        let p = written("tokens-codex-model", concat!(
            r#"{"type":"session_meta","payload":{"id":"s1","model_provider":"openai"}}"#, "\n",
            r#"{"type":"turn_context","payload":{"turn_id":"t1","model":"gpt-6.1-terra","cwd":"/Users/alex/api"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"message","content":[{"type":"output_text","text":"a turn_context in its words"}]}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2}}}}"#, "\n",
            r#"{"type":"turn_context","payload":{"turn_id":"t2","model":"gpt-6.1-sol"}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":9,"cached_input_tokens":3,"output_tokens":4}}}}"#, "\n"));
        let got = of_event("codex", &json!({"transcript_path": p})).unwrap();
        assert_eq!(got["models"], json!([{"model": "gpt-6.1-sol", "input": 6, "output": 4, "cache_write": 0, "cache_write_1h": 0, "cache_read": 3}]), "no path, no text");
    }

    #[test]
    fn ac129_codex_a_model_named_before_the_tail_is_found() {
        let mut text = String::from(r#"{"type":"turn_context","payload":{"model":"gpt-6.1-sol"}}"#);
        text.push('\n');
        let filler = format!("{{\"type\":\"response_item\",\"payload\":{{\"text\":\"{}\"}}}}\n", "x".repeat(1000));
        text.push_str(&filler.repeat((CODEX_TAIL as usize / filler.len()) + 2));
        text.push_str(r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":9,"cached_input_tokens":3,"output_tokens":4}}}}"#);
        let got = of_event("codex", &json!({"transcript_path": written("tokens-codex-far", &text)})).unwrap();
        assert_eq!(got["models"][0]["model"], "gpt-6.1-sol");
    }

    #[test]
    fn ac168_codex_s_last_windows_are_read_from_its_tail() {
        let p = written("limits-codex", concat!(
            r#"{"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":20.0,"window_minutes":10080,"resets_at":1794428830},"secondary":null}}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"message","content":[{"type":"output_text","text":"secret words"}]}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":9,"cached_input_tokens":3,"output_tokens":4}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":21.0,"window_minutes":10080,"resets_at":1794428830},"secondary":{"used_percent":3.5,"window_minutes":300,"resets_at":1794350000},"plan_type":"plus"}}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":null}}"#, "\n"));
        let got = codex_windows(&json!({"transcript_path": p})).unwrap();
        assert_eq!(got, vec![Window { minutes: 10080, used: 21.0, resets_at: Some(1794428830) }, Window { minutes: 300, used: 3.5, resets_at: Some(1794350000) }],
                   "the last that has them stands: a count without them is no reset");
        assert_eq!(codex_windows(&json!({"transcript_path": written("limits-none", "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"}}\n")})), None, "none in the tail: nothing, not 0%");
        assert_eq!(codex_windows(&json!({"transcript_path": "/dev/zero"})), None);
        assert_eq!(codex_windows(&json!({})), None);
    }

    #[test]
    fn ac129_what_cannot_be_read_is_said_as_not_known() {
        assert_eq!(of_event("claude", &json!({})), None, "no transcript named: nothing to say");
        assert_eq!(of_event("claude", &json!({"transcript_path": "/no/such/file"})), Some(Value::Null), "the file gone");
        assert_eq!(of_event("claude", &json!({"transcript_path": "/dev/zero"})), Some(Value::Null), "no regular file: never read");
        let p = written("tokens-odd", "{\"another\":\"format\"}\nnot json at all\n");
        assert_eq!(of_event("claude", &json!({"transcript_path": p.clone()})), Some(Value::Null), "a new version's format");
        assert_eq!(of_event("codex", &json!({"transcript_path": p})), Some(Value::Null));
    }

    #[test]
    fn ac129_claude_its_model_goes_with_the_numbers() {
        let p = written("tokens-model", concat!(
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":10,"output_tokens":1}}}"#, "\n"));
        assert_eq!(of_event("claude", &json!({"transcript_path": p})).unwrap()["models"][0]["model"], "claude-opus-5-5");
    }

    #[test]
    fn ac129_claude_each_model_and_its_hour_s_cache_apart() {
        let p = written("tokens-models", concat!(
            r#"{"message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":10,"output_tokens":1,"cache_creation_input_tokens":30,"cache_creation":{"ephemeral_5m_input_tokens":10,"ephemeral_1h_input_tokens":20}}}}"#, "\n",
            r#"{"message":{"id":"m2","model":"claude-haiku-5-5","usage":{"input_tokens":3,"output_tokens":2}}}"#, "\n",
            r#"{"message":{"id":"m3","model":"claude-opus-5-5","usage":{"input_tokens":5,"output_tokens":1}}}"#, "\n"));
        let got = of_event("claude", &json!({"transcript_path": p})).unwrap();
        assert_eq!((got["input"].as_i64(), got["cache_write_1h"].as_i64()), (Some(18), Some(20)), "the whole");
        assert_eq!(got["models"], json!([
            {"model": "claude-haiku-5-5", "input": 3, "output": 2, "cache_write": 0, "cache_write_1h": 0, "cache_read": 0},
            {"model": "claude-opus-5-5", "input": 15, "output": 2, "cache_write": 30, "cache_write_1h": 20, "cache_read": 0}]));
    }
}
