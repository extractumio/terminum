// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Tool definitions for a Supervisor investigation (terminum-supervisor.md AC-234 and AC-235).

use serde_json::{Value, json};
use super::mcp_catalog::Card;

pub(super) const CARDS: &[Card] = &[
    Card { name: "hold", area: "supervisor", keywords: "hold stop run pause auto dispatcher integration take out",
        example: r#"{"signal":"fingerprint","kind":"pause_integration","reason":"The target is red."}"# },
    Card { name: "release_hold", area: "supervisor", keywords: "release undo hold resume integration run",
        example: r#"{"id":42}"# },
    Card { name: "get_supervisor_audit", area: "supervisor", keywords: "read quarterly audit ledger signals misses register", example: r#"{"section":"ledger","offset":0}"# },
    Card { name: "get_supervisor_review", area: "supervisor", keywords: "read weekly review context register decisions verdicts", example: "{}" },
    Card { name: "get_case_file", area: "supervisor", keywords: "get case file read investigation context",
        example: r#"{"id":"case_0042"}"# },
    Card { name: "list_signals", area: "supervisor", keywords: "list project signals findings detectors",
        example: "{}" },
    Card { name: "get_signal", area: "supervisor", keywords: "read project signal evidence and measurement",
        example: r#"{"id":"fingerprint"}"# },
    Card { name: "get_metric", area: "supervisor", keywords: "recompute named query measurement value baseline window",
        example: r#"{"query":"open_waits","signal":"fingerprint"}"# },
    Card { name: crate::supervisor::reflection::TOOL, area: "supervisor", keywords: "record independent reflection timeline proposals",
        example: r#"{"missed":"Earlier evidence was absent.","noise":"The repeat alert added no evidence.","wrong":"The rule counted pending decisions.","proposals":[{"change_id":42,"evidence":[1]}]}"# },
    Card { name: crate::supervisor::config_changes::PREVIEW, area: "supervisor", keywords: "replay supervisor configuration change detector threshold mute shadow",
        example: r#"{"detector":"flow.failed_repeat","action":{"operation":"tune","threshold":{"absolute":2.5}}}"# },
    Card { name: crate::supervisor::config_changes::PROPOSE, area: "supervisor", keywords: "propose apply supervisor configuration change replay detector tune mute",
        example: r#"{"change":{"detector":"flow.failed_repeat","action":{"operation":"tune","threshold":{"absolute":2.5}}},"replay":{"at":1800000000,"first":1799000000,"source":"retained project history","before":{"firings":0,"disabled":false,"measured_cutoffs":2,"unavailable_cutoffs":0,"exact_right":0,"exact_wrong":0},"after":{"firings":0,"disabled":false,"measured_cutoffs":2,"unavailable_cutoffs":0,"exact_right":0,"exact_wrong":0},"labeled_weeks":[],"proof":"copy the returned proof"}}"# },
    Card { name: "record_finding", area: "supervisor", keywords: "submit checked investigation outcome cause disproof fix evidence",
        example: r#"{"cause":"The queue waits for an answer.","confidence":0.8,"disproof":"The waits resolve without answers.","fix_proposal":"Clarify ownership.","evidence":[{"table":"events","id":"row"}],"measurements":[{"query":"open_waits","signal":"fingerprint","value":240}]}"# },
    Card { name: "open_problem", area: "supervisor", keywords: "open problem register cluster fingerprints hypothesis",
        example: r#"{"title":"Recurring queue waits","lens":"flow","hypothesis":"Tasks have no answer owner.","disproof":"Waits with an owner take as long.","signals":["fingerprint"],"confidence":0.8,"effort":1,"fix_options":[{"layer":"check","action":"Require an answer owner."}]}"# },
    Card { name: "update_problem", area: "supervisor", keywords: "update problem hypothesis disproof fix options",
        example: r#"{"id":42,"confidence":0.9}"# },
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

pub(super) fn stop_tool() -> Value {
    let text = json!({"type":"string"});
    tool("stop_task", "Apply a stop_run hold to a marked task run using a level 2 or 3 signal.",
        json!({"id":text,"signal":text,"reason":text}), &["id","signal","reason"], false)
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
    let change = json!({"type":"object","required":["detector","action"],"additionalProperties":false,
        "properties":{"detector":text,"action":{"type":"object","required":["operation"],
            "properties":{"operation":{"enum":["tune","enable","mute","remove","owner_review","engine"]},
                "threshold":{"type":"object","description":"One of absolute, baseline (percentile and floor), or change (by)."},
                "window_days":{"type":"integer","minimum":1,"maximum":365},
                "mode":{"enum":["off","shadow","digest"]},"reason":text,"until":{"type":"integer","description":"UTC Unix seconds."},
                "target":{"enum":["page","charter","surface_map"]},
                "requirement":{"enum":["query","tool","probe_kind","recorded_field"]}},
            "additionalProperties":false}}});
    vec![
        tool(crate::supervisor::config_changes::PREVIEW, "Replay a detector change in the current project. Copy the returned replay to propose_config_change.",
            change["properties"].clone(), &["detector","action"], true),
        tool(crate::supervisor::config_changes::PROPOSE, "Submit a detector change with the unedited replay receipt. Terminum reproduces it and applies, shadows, or sends it to an owner decision according to its class.",
            json!({"change":change,"replay":{"type":"object","description":"Copy the exact replay object returned by replay_config_change."}}),
            &["change","replay"], false),
        tool("hold", "Apply one reversible hold in this project using a current level 2 or 3 signal. Shadow holds are recorded without changing work.",
            json!({"signal":text,"kind":{"enum":["stop_run","pause_dispatch","pause_integration","take_out"]},
                "task":text,"reason":text}), &["signal","kind","reason"], false),
        tool("release_hold", "Use the held action's undo in this project. The hold ID identifies its qualifying signal and saved target.",
            json!({"id":id}), &["id"], false),
        tool("get_supervisor_audit", "Page through evidence for your live independent quarterly audit.",
            json!({"section":{"enum":["ledger","signals","misses","register"]},"offset":{"type":"integer","minimum":0}}), &[], true),
        tool("get_supervisor_review", "Read the measured context for the live Supervisor weekly review task or your own project as its manager.", json!({}), &[], true),
        tool("open_problem", "Open a problem with recorded project signals. Code derives impact and the yardstick.",
            json!({"title":text,"lens":{"enum":["flow","architecture","product","owner"]},"hypothesis":text,"disproof":text,
                "fix_options":options,"confidence":confidence,"effort":effort,"signals":{"type":"array","minItems":1,"maxItems":16,"items":text}}),
            &["title","lens","hypothesis","disproof","fix_options","confidence","effort","signals"], false),
        tool("update_problem", "Update an open problem. Code owns its detector yardstick. Code and the owner close problems.",
            json!({"id":id,"hypothesis":text,"disproof":text,"fix_options":options,"confidence":confidence,"effort":effort,
                "state":{"enum":["suspected","confirmed","fix_in_work"]}}),
            &["id"], false),
        tool("post_incident_answer", "Post the answer on this live session's incident thread and queue its fixed notice.",
            json!({"text":text}), &["text"], false),
        tool("propose_decision", "Before submitting the checked finding, propose an owner choice on its register problem. Code sends high-impact owner choices to the board and groups lesser decisions in the weekly review. A standing rule gives recurring: the event ID of one asking of the repeated question it ends, from the problem's signal evidence.",
            json!({"problem":id,"kind":{"enum":["fix_options","standing_rule","budget","design_signoff","hold","open_breaker","routine"]},
                "why":text,"recommend_why":text,"waiting":text,"cost":{"type":"number","minimum":0},"recommended":text,
                "options":{"type":"array","minItems":2,"maxItems":12,"items":{"type":"object",
                    "required":["id","label"],"properties":{"id":text,"label":text},"additionalProperties":false}},
                "recurring":text}),
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
        tool(crate::supervisor::reflection::TOOL, "Record the independent review. Cite numbered timeline events and configuration change IDs returned by propose_config_change. Routing proposals become owner decision cards.",
            json!({"missed":text,"noise":text,"wrong":text,
                "proposals":{"type":"array","maxItems":8,"items":{"type":"object","additionalProperties":false,
                    "required":["change_id","evidence"],"properties":{"change_id":id,
                        "evidence":{"type":"array","minItems":1,"items":id}}}},
                "routing":{"type":"object","additionalProperties":false,"required":["agent","task_type","change","why"],
                    "properties":{"agent":text,"task_type":text,"change":text,"why":text}}
            }), &["missed","noise","wrong","proposals"], false),
        tool("record_finding", "Submit this investigation's cause, confidence, disproof and fix proposal. Copy typed evidence references and put numeric claims in measurements. Code resolves each row and reruns each named query. To ask for a critical escalation, add escalate with the score it raises to level 3 and why; it needs confidence of at least 0.7 and the recomputed measurement of the case's own signal.",
            json!({
                "cause": text, "disproof": text, "fix_proposal": text, "confidence": {"type":"number", "minimum":0, "maximum":1},
                "evidence": {"type":"array","minItems":1,"maxItems":128,"items":{
                    "type":"object","required":["table","id"],"additionalProperties":false,
                    "properties":{"table":{"type":"string"},"id":{}}}},
                "measurements":{"type":"array","maxItems":16,"items":{
                    "type":"object","required":["query","signal","value"],"additionalProperties":false,
                    "properties":{"query":text,"signal":text,"value":{"type":"number"}}}},
                "escalate":{"type":"object","required":["score","level","reason"],"additionalProperties":false,
                    "properties":{"score":{"enum":["blast","reversibility"]},"level":{"type":"integer","minimum":0,"maximum":3},"reason":text}}
            }), &["cause","confidence","disproof","fix_proposal","evidence","measurements"], false),
    ]
}
