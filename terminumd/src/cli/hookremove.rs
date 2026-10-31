// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Remove only terminum's commands from Codex hook entries, keeping shared matcher metadata.

use serde_json::Value;

/// `hooks.json` without terminum's commands; an entry or event emptied by removal goes too.
pub fn codex_remove(v: &Value, cmd: &str) -> Value {
    let mut v = v.clone();
    if let Some(hooks) = v.get_mut("hooks").and_then(Value::as_object_mut) {
        for entries in hooks.values_mut().filter_map(Value::as_array_mut) {
            entries.retain_mut(|entry| {
                let Some(commands) = entry.get_mut("hooks").and_then(Value::as_array_mut) else { return true };
                let before = commands.len();
                commands.retain(|hook| hook["command"] != cmd);
                commands.len() == before || !commands.is_empty()
            });
        }
        hooks.retain(|_, entries| entries.as_array().is_none_or(|a| !a.is_empty()));
    }
    v
}
