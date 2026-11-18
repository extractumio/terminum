// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The agent's own turns (terminum-terminal.md AC-90, terminum-dispatcher.md AC-122): prompts its hooks
//! speak though no one typed them. Its notices begin as `super::injected` says; a turn it scheduled
//! for itself (Claude Code's ScheduleWakeup, CronCreate, /loop) is the plain text it scheduled, known
//! by the hook's `source` where Claude Code says it, else by that text, which its PreToolUse spoke.
//! The texts outlive a restart of terminumd (each landing installs one): kept in a file of the state folder.

use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Claude Code's `source` of a prompt no one typed: a /loop's wakeup, a scheduled task's fire, a
/// machine's turn (another session's message, a task's end, a continuation), a host's event.
const SOURCES: [&str; 4] = ["loop_wakeup", "schedule_wakeup", "system", "poll_event"];
/// How long a scheduled prompt is known, in seconds: Claude Code's recurring tasks end after 7 days.
const KEPT: i64 = 7 * 24 * 3600;
/// Scheduled prompts known per pane at most, the newest.
const PER_PANE: usize = 20;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    pane: String,
    text: String,
    /// unix seconds it was scheduled last
    at: i64,
}

/// The prompts each pane's agent scheduled for itself, kept in `path` (none: in memory only).
#[derive(Default)]
pub struct Scheduled {
    all: Mutex<Vec<Entry>>,
    path: Option<PathBuf>,
    write: Mutex<()>,
}

impl Scheduled {
    /// The texts kept in `path`; a missing file is none, an unreadable one is set aside as the other state files are.
    pub fn load(path: PathBuf) -> Scheduled {
        let all = crate::auth::store::read_or_set_aside(&path, |v| serde_json::from_value(v).map_err(|e| e.to_string())).unwrap_or_default();
        Scheduled { all: Mutex::new(all), path: Some(path), write: Mutex::default() }
    }

    /// Pane `key`'s agent scheduled `text`: known again from now. `save` keeps it.
    pub fn remember(&self, key: &str, text: &str) {
        let mut all = self.all.lock();
        let at = crate::tasks::ops::now();
        all.retain(|e| at - e.at < KEPT && !(e.pane == key && e.text == text));
        if all.iter().filter(|e| e.pane == key).count() >= PER_PANE
            && let Some(oldest) = all.iter().position(|e| e.pane == key) {
            all.remove(oldest);
        }
        all.push(Entry { pane: key.to_string(), text: text.to_string(), at });
    }

    /// Writes what is known to its file, atomically. Blocking: off the async workers.
    pub fn save(&self) {
        let Some(path) = &self.path else { return };
        let _one = self.write.lock();
        let bytes = serde_json::to_vec(&*self.all.lock()).unwrap_or_default();
        if let Err(e) = crate::config::write_private(path, &bytes) {
            tracing::warn!(event = "scheduled.save", path = %path.display(), error = %e);
        }
    }

    /// Whether pane `key`'s agent scheduled `prompt` (blanks aside); kept: a cron fires again.
    pub fn has(&self, key: &str, prompt: &str) -> bool {
        let at = crate::tasks::ops::now();
        self.all.lock().iter().any(|e| e.pane == key && at - e.at < KEPT && crate::dispatcher::same_prompt(&e.text, prompt))
    }
}

/// The prompt a hook's tool event schedules for the agent itself (`terminum hook`'s `scheduled`).
pub fn scheduled(ev: &Value) -> Option<&str> {
    ev["scheduled"].as_str().filter(|s| ev["event"] == "PreToolUse" && !s.trim().is_empty())
}

/// Whether hook event `ev` of pane `key` is a prompt of the agent's own turn: a `source` Claude Code
/// gives decides ("user", "sdk": a human's even in words the agent scheduled), else, none or one not
/// known, the scheduled texts.
pub fn turn(scheduled: &Scheduled, key: &str, ev: &Value) -> bool {
    let Some(prompt) = ev["prompt"].as_str().filter(|_| ev["event"] == "UserPromptSubmit") else { return false };
    super::injected(prompt) || match ev["source"].as_str() {
        Some("user" | "sdk") => false,
        Some(s) if SOURCES.contains(&s) => true,
        _ => scheduled.has(key, prompt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn prompt(p: &str, source: Option<&str>) -> Value {
        let mut v = json!({"event": "UserPromptSubmit", "prompt": p});
        if let Some(s) = source {
            v["source"] = json!(s);
        }
        v
    }

    /// AC-90, AC-122: a wakeup the agent scheduled, a cron's fire and a machine's turn are its own; a human's is not.
    #[test]
    fn ac90_ac122_a_turn_the_agent_scheduled_is_its_own() {
        let s = Scheduled::default();
        let (a, b) = ("tmux:local:terminum:%1", "tmux:local:terminum:%2");
        let wake = "Check the landing of TERM-9, then report.";
        s.remember(a, "Check the landing of TERM-9,\nthen report.");
        assert!(turn(&s, a, &prompt(wake, None)), "blanks aside");
        assert!(turn(&s, a, &prompt(wake, None)), "a cron fires again");
        assert!(!turn(&s, b, &prompt(wake, None)), "another pane's agent did not schedule it");
        assert!(!turn(&s, a, &prompt(wake, Some("user"))), "a source Claude Code gives decides");
        assert!(turn(&s, a, &prompt(wake, Some("later_kind"))) && !turn(&s, a, &prompt("fix it", Some("later_kind"))), "one not known: the words do");
        assert!(!turn(&s, a, &prompt("fix the tests", None)), "a human's");
        for source in SOURCES {
            assert!(turn(&s, b, &prompt("go on", Some(source))), "{source}");
        }
        assert!(turn(&s, b, &prompt("<task-notification>\n<task-id>b1</task-id>", Some("user"))), "a notice is its own whatever it says");
        assert!(!turn(&s, a, &json!({"event": "PreToolUse", "prompt": wake})), "only a prompt");
    }

    #[test]
    fn a_pane_keeps_its_newest_scheduled_prompts_once_each() {
        let s = Scheduled::default();
        for i in 0..=PER_PANE {
            s.remember("k", &format!("wake {i}"));
        }
        s.remember("k", "wake 5");
        s.remember("other", "wake 0");
        assert!(!s.has("k", "wake 0"), "the oldest gave way");
        assert!(s.has("k", "wake 1") && s.has("k", "wake 5") && s.has("k", &format!("wake {PER_PANE}")));
        assert_eq!(s.all.lock().iter().filter(|e| e.pane == "k").count(), PER_PANE, "the same text scheduled again is one");
        assert!(s.has("other", "wake 0"));
        s.all.lock().iter_mut().for_each(|e| e.at -= KEPT);
        assert!(!s.has("other", "wake 0"), "7 days on it is no longer known");
    }

    /// AC-122: a wakeup scheduled before terminumd restarts (a landing installs it) is still the agent's own.
    #[test]
    fn ac122_the_scheduled_prompts_outlive_a_restart() {
        let dir = crate::testutil::fresh_dir("scheduled-restart");
        let path = dir.join("scheduled-prompts.json");
        let s = Scheduled::load(path.clone());
        s.remember("k", "land TERM-9 once its checks pass");
        s.save();
        assert!(Scheduled::load(path.clone()).has("k", "land TERM-9 once its checks pass"));
        std::fs::write(&path, "not json").unwrap();
        assert!(!Scheduled::load(path.clone()).has("k", "x"), "an unreadable file is none");
        assert!(crate::auth::store::backup_path(&path).exists(), "and set aside, not overwritten");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_a_tools_event_schedules_and_never_blanks() {
        assert_eq!(scheduled(&json!({"event": "PreToolUse", "scheduled": "wake"})), Some("wake"));
        assert_eq!(scheduled(&json!({"event": "PreToolUse", "scheduled": "  "})), None);
        assert_eq!(scheduled(&json!({"event": "UserPromptSubmit", "scheduled": "wake"})), None);
    }
}
