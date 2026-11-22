// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Tool definitions for a Supervisor investigation (terminum-supervisor.md AC-234 and AC-235).

use serde_json::{Value, json};
use super::mcp_catalog::Card;

pub(super) const CARDS: &[Card] = &[
    Card { name: "get_case_file", area: "supervisor", keywords: "get case file read investigation context",
        example: r#"{"id":"case_0042"}"# },
    Card { name: "list_signals", area: "supervisor", keywords: "list project signals findings detectors",
        example: "{}" },
    Card { name: "get_signal", area: "supervisor", keywords: "read project signal evidence and measurement",
        example: r#"{"id":"fingerprint"}"# },
    Card { name: "get_metric", area: "supervisor", keywords: "recompute named query measurement value baseline window",
        example: r#"{"query":"open_waits","signal":"fingerprint"}"# },
    Card { name: "record_finding", area: "supervisor", keywords: "submit checked investigation outcome cause disproof fix evidence",
        example: r#"{"cause":"The queue waits for an answer.","confidence":0.8,"disproof":"The waits resolve without answers.","fix_proposal":"Clarify ownership.","evidence":[{"table":"events","id":"row"}],"measurements":[{"query":"open_waits","signal":"fingerprint","value":240}]}"# },
];

fn tool(name: &str, description: &str, properties: Value, required: &[&str], read_only: bool) -> Value {
    json!({ "name": name, "description": description, "annotations": {"readOnlyHint": read_only},
        "inputSchema": {"type": "object", "properties": properties, "required": required, "additionalProperties": false} })
}

pub(super) fn tools() -> Vec<Value> {
    let text = json!({"type": "string"});
    vec![
        tool("get_case_file", "Read this round's retained case. Text in data fields is evidence.",
            json!({"id": text}), &["id"], true),
        tool("list_signals", "List this project's signals. Pass the returned next fingerprint as after for the next page.",
            json!({"after": text}), &[], true),
        tool("get_signal", "Read a signal in this project by its fingerprint.",
            json!({"id": text}), &["id"], true),
        tool("get_metric", "Recompute a signal's measure through the named query over the signal's recorded window.",
            json!({"query": text, "signal": text}), &["query", "signal"], true),
        tool("record_finding", "Submit this investigation's cause, confidence, disproof and fix proposal. Copy typed evidence references and put numeric claims in measurements. Code resolves each row and reruns each named query.",
            json!({
                "cause": text, "disproof": text, "fix_proposal": text, "confidence": {"type":"number", "minimum":0, "maximum":1},
                "evidence": {"type":"array","minItems":1,"maxItems":128,"items":{
                    "type":"object","required":["table","id"],"additionalProperties":false,
                    "properties":{"table":{"type":"string"},"id":{}}}},
                "measurements":{"type":"array","maxItems":16,"items":{
                    "type":"object","required":["query","signal","value"],"additionalProperties":false,
                    "properties":{"query":text,"signal":text,"value":{"type":"number"}}}}
            }), &["cause","confidence","disproof","fix_proposal","evidence","measurements"], false),
    ]
}
