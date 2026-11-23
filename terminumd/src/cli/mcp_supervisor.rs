// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Tool definitions for a Supervisor investigation (terminum-supervisor.md AC-234 and AC-235).

use serde_json::{Value, json};
use super::mcp_catalog::Card;

pub(super) const CARDS: &[Card] = &[
    Card { name: "get_supervisor_review", area: "supervisor", keywords: "read weekly review context register decisions verdicts", example: "{}" },
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
    Card { name: "open_problem", area: "supervisor", keywords: "open problem register cluster fingerprints hypothesis",
        example: r#"{"title":"Recurring queue waits","lens":"flow","hypothesis":"Tasks have no answer owner.","disproof":"Waits with an owner take as long.","signals":["fingerprint"],"confidence":0.8,"effort":1,"fix_options":[{"layer":"check","action":"Require an answer owner."}]}"# },
    Card { name: "update_problem", area: "supervisor", keywords: "update problem hypothesis disproof fix options observation weeks",
        example: r#"{"id":42,"confidence":0.9,"observation_weeks":6}"# },
    Card { name: "post_incident_answer", area: "supervisor", keywords: "answer owner comment incident thread",
        example: r#"{"text":"The investigation confirmed the repeated failures."}"# },
    Card { name: "propose_decision", area: "supervisor", keywords: "propose decision card owner choice evidence weekly",
        example: r#"{"problem":42,"kind":"fix_options","why":"The change needs an owner decision.","recommended":"wait","recommend_why":"Reversible","waiting":"Another week of delays","cost":2,"options":[{"id":"wait","label":"Wait"},{"id":"act","label":"Act"}]}"# },
    Card { name: "propose_rank", area: "supervisor", keywords: "propose register problem rank to Auto-Dispatcher advisory task priority",
        example: r#"{"problem":42,"task":"API-41"}"# },
];

fn tool(name: &str, description: &str, properties: Value, required: &[&str], read_only: bool) -> Value {
    json!({ "name": name, "description": description, "annotations": {"readOnlyHint": read_only},
        "inputSchema": {"type": "object", "properties": properties, "required": required, "additionalProperties": false} })
}

pub(super) fn tools() -> Vec<Value> {
    let text = json!({"type": "string"});
    let id = json!({"type":"integer","minimum":1});
    let confidence = json!({"type":"number","minimum":0,"maximum":1});
    let effort = json!({"type":"integer","minimum":1,"maximum":4});
    let options = json!({"type":"array","maxItems":6,"items":{
        "type":"object","required":["layer","action"],"additionalProperties":false,
        "properties":{"layer":{"enum":["check","test","hook","setting","documentation","memory"]},"action":text}
    }});
    vec![
        tool("get_supervisor_review", "Read the measured context for the live Supervisor weekly review task or your own project as its manager.", json!({}), &[], true),
        tool("open_problem", "Open a problem with recorded project signals. Code derives impact and the yardstick.",
            json!({"title":text,"lens":{"enum":["flow","architecture","product","owner"]},"hypothesis":text,"disproof":text,
                "fix_options":options,"confidence":confidence,"effort":effort,"signals":{"type":"array","minItems":1,"maxItems":16,"items":text}}),
            &["title","lens","hypothesis","disproof","fix_options","confidence","effort","signals"], false),
        tool("update_problem", "Update an open problem. The detector yardstick may be lengthened. Code and the owner close problems.",
            json!({"id":id,"hypothesis":text,"disproof":text,"fix_options":options,"confidence":confidence,"effort":effort,
                "state":{"enum":["suspected","confirmed","fix_in_work","watching"]},"observation_weeks":{"type":"integer","minimum":4,"maximum":52}}),
            &["id"], false),
        tool("post_incident_answer", "Post the answer on this live session's incident thread and queue its fixed notice.",
            json!({"text":text}), &["text"], false),
        tool("propose_decision", "Before submitting the checked finding, propose an owner choice on its register problem. Code sends high-impact owner choices to the board and groups lesser decisions in the weekly review.",
            json!({"problem":id,"kind":{"enum":["fix_options","standing_rule","budget","design_signoff","hold","open_breaker","routine"]},
                "why":text,"recommend_why":text,"waiting":text,"cost":{"type":"number","minimum":0},"recommended":text,
                "options":{"type":"array","minItems":2,"maxItems":12,"items":{"type":"object",
                    "required":["id","label"],"properties":{"id":text,"label":text},"additionalProperties":false}}}),
            &["problem","kind","why","recommend_why","waiting","cost","recommended","options"], false),
        tool("propose_rank", "Make this problem's measured rank available to Auto-Dispatcher for a related task.",
            json!({"problem":id,"task":text}), &["problem","task"], false),
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
