// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Dispatcher (`docs/specs/terminum-dispatcher.md`): terminum's system user. It starts a task's
//! session on a human's RUN, watches it by hooks' events and a walk on a fixed beat, moves the task
//! as the session lives, and closes what it started. All its clocks and facts are in the store.

mod acts;
mod answer;
pub(crate) mod after;
pub mod auto;
pub mod fire;
pub mod api;
mod goes_on;
pub(crate) mod letgo;
mod press;
mod recent;
mod carry;
mod places;
mod firstprompt;
mod queue;
mod resume;
mod gate;
pub(crate) mod giveup;
mod hooks;
pub mod plan;
pub mod pool;
pub mod usage;
pub mod usageask;
mod usageread;
pub mod procs;
pub mod runs;
pub mod screen;
pub mod subscriptions;
pub(crate) mod start;
mod walk;
pub(crate) mod supervisor_rounds;
pub(crate) mod supervisor_talk;
pub mod waits;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::tasks::api::store;
use crate::tasks::ops;
use crate::tasks::store::DISPATCHER;

pub use hooks::{good_id, heard, hearing};
pub(crate) use hooks::taking;
/// A red batch's integration task pressed on its agent (terminum-integration.md AC-194).
pub(crate) use api::press_integration;

/// The integration queue's words for the agent of the live run of Dispatcher's that holds task `id`
/// (terminum-integration.md AC-192): through the gate while it idles, else with its next hook. False when no such run
/// holds it: the caller hands the words to the task's session's next hook alone.
pub(crate) async fn tell_task_run(app: &crate::app::Shared, task: &str, text: String) -> bool {
    let key = task.to_string();
    let Ok(Some(run)) = store(app, move |tx| Ok(runs::last_of(tx, &key)?.filter(runs::Run::holds_slot))).await else { return false };
    let Some((server, pane)) = run.place(app) else { return false };
    acts::tell(app, &run, &server, &pane, text, "Its return from integration").await;
    true
}
pub use start::resume_words;
pub use gate::read as read_screen;
pub use walk::run as walking;

/// The walk's beat; a gap over GAP adds nothing to a clock (AC-131).
const BEAT: Duration = Duration::from_secs(30);
const GAP: i64 = 120;
/// What terminum typed is known for this long, in seconds: the hook's prompt is matched to it (§9).
const TYPED_FOR: u64 = 300;

#[derive(Default)]
pub struct Dispatcher {
    /// panes with activity since the last walk
    active: Mutex<HashSet<String>>,
    /// panes with a live run: the others' hook events cost nothing here; changed only with the store's lock
    /// held, so a walk's read and a window's opening each see the other whole
    panes: Mutex<HashMap<String, LivePane>>,
    /// those of them a + Session's run holds: their sessions are their projects' managers, which the list's rows
    /// say (terminum-agents.md AC-174); and whether they changed since the list was last asked to be read again
    managers: Mutex<HashSet<String>>,
    managers_changed: std::sync::atomic::AtomicBool,
    /// hooks' events, in the order they came (AC-131)
    events: hooks::Events,
    /// what terminum typed, by pane
    typed: Typed<TYPED_FOR>,
    /// runs whose start is being watched
    starting: Mutex<HashSet<String>>,
    /// runs whose End is under way (acts.rs)
    ending: Mutex<HashSet<String>>,
    /// Tasks and runs with a Run press preparing or delivering a prompt.
    pressing: Mutex<HashSet<String>>,
    /// runs with deferred walk actions still in flight
    deferred: Mutex<HashSet<String>>,
    /// restart recovery releases or reserves a project's slots in turn (AC-205)
    restart_recovery: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// a walk is due now (a session ended)
    soon: Notify,
    /// what a walk or a hook learned of each pane it watches
    watch: Mutex<HashMap<String, PaneWatch>>,
    /// what pages were last told of tasks' sessions (`taskusage`)
    usage: Mutex<String>,
    /// beats so far: a host with no run is read less often
    beat: AtomicU64,
    /// the agent's session each run's last counted report was of (usage.rs `heard`); a restart learns it anew
    counted_in: Mutex<HashMap<String, String>>,
    /// each kind's last subscription reading (AC-168)
    pub subscriptions: subscriptions::Readings,
    /// asking each kind's usage command (AC-168)
    asking: usageask::Asking,
}

struct LivePane {
    run: String,
    project: String,
}

/// What is known of a watched pane beyond the store; a restart learns it anew.
#[derive(Default)]
struct PaneWatch {
    /// its tree's CPU seconds at the last reading, and when
    cpu: Option<(f64, Instant)>,
    /// walks in a row over a memory limit (AC-128)
    over: (u32, u32),
    /// its agent was not under it at the last walk, with no exit recorded: the wrapper may not have got there
    agent_missed: bool,
    /// its subagents as its hooks said (AC-127); bounded: a hook's fields are data
    subs: HashMap<String, Sub>,
    /// When the watchdog saw this run stop working (AC-127).
    subs_paused_at: Option<Instant>,
    /// the tokens of its last turn's end; null: not known (AC-129)
    tokens: Option<serde_json::Value>,
    /// the last line pages were told of it: a host read less often keeps it
    line: Option<serde_json::Value>,
}

impl PaneWatch {
    fn clear_subagents(&mut self) {
        self.subs.clear();
        self.subs_paused_at = None;
    }
}

/// A subagent as its hooks said.
#[derive(Clone, Debug)]
struct Sub {
    kind: String,
    last: Instant,
    told: plan::SubStep,
}

/// A run's watch or End under way, by its id in one of Dispatcher's sets: one at a time; gone with it, a panic too.
struct Held(crate::app::Shared, fn(&Dispatcher) -> &Mutex<HashSet<String>>, String);

impl Held {
    /// Run `id` in `set`, unless it is there already.
    fn take(app: &crate::app::Shared, set: fn(&Dispatcher) -> &Mutex<HashSet<String>>, id: &str) -> Option<Held> {
        set(&app.dispatcher).lock().insert(id.to_string()).then(|| Held(app.clone(), set, id.to_string()))
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        (self.1)(&self.0.dispatcher).lock().remove(&self.2);
    }
}

/// Subagents kept per pane: a made-up hook adds no more.
const MOST_SUBS: usize = 64;

impl Dispatcher {
    /// A walk is due now: a setting changed what it would do.
    pub fn soon(&self) {
        self.soon.notify_one();
    }

    /// Activity in pane `key` (§9): a hook's event, a character a human typed.
    pub fn touched(&self, key: &str) {
        if self.has_run(key) {
            self.active.lock().insert(key.to_string());
        }
    }

    fn take_active(&self, key: &str) -> bool {
        self.active.lock().remove(key)
    }

    pub fn has_run(&self, key: &str) -> bool {
        self.panes.lock().contains_key(key)
    }

    /// The project of a pane Dispatcher currently holds.
    pub(crate) fn pane_project(&self, key: &str) -> Option<String> {
        self.panes.lock().get(key).map(|pane| pane.project.clone())
    }

    fn track_pane(&self, key: &str, run: &str, project: &str) {
        self.panes.lock().insert(key.to_owned(), LivePane { run: run.to_owned(), project: project.to_owned() });
    }

    fn release_pane(&self, key: &str, run: &str) {
        let mut panes = self.panes.lock();
        if panes.get(key).is_some_and(|pane| pane.run == run) {
            panes.remove(key);
        }
    }

    /// Whether pane `key`'s session is its project's manager, as the last read of the runs says: for what a
    /// page shows. What a caller may run is read from the store at its call (roles/mod.rs).
    pub fn manages(&self, key: &str) -> bool {
        self.managers.lock().contains(key)
    }

    /// Whether the managers' panes changed since this was last asked: the list is read again then.
    pub fn managers_changed(&self) -> bool {
        self.managers_changed.swap(false, std::sync::atomic::Ordering::Relaxed)
    }

    /// Pane `key` holds `run` from now on, in the transaction that says so (AC-131).
    fn pane_opened(&self, tx: &rusqlite::Transaction, run: &str, key: &str) -> Result<(runs::Run, bool), String> {
        let r = runs::change(tx, run, |r| r.pane_key = Some(key.to_string()))?;
        let released = crate::supervisor::holds::run_launched(tx, run, ops::now())?;
        self.track_pane(key, &r.id, &r.project);
        if r.manages() && self.managers.lock().insert(key.to_string()) {
            self.managers_changed.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        Ok((r, released))
    }

    /// The runs a walk goes by, and the panes they hold, in one transaction (AC-131).
    pub(crate) fn read_runs(&self, tx: &rusqlite::Transaction) -> Result<Vec<runs::Run>, String> {
        let all = runs::live(tx)?;
        *self.panes.lock() = all.iter().filter_map(|r| {
            r.pane_key.clone().map(|key| (key, LivePane { run: r.id.clone(), project: r.project.clone() }))
        }).collect();
        self.hold_managers(crate::roles::managers(tx, &all)?);
        Ok(all)
    }

    fn hold_managers(&self, managers: HashSet<String>) {
        let mut held = self.managers.lock();
        if *held != managers {
            *held = managers;
            self.managers_changed.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// The managers read again at once, a project's members having changed: a session the human put in a project
    /// shows its crown without waiting for the walk (AC-174).
    pub fn members_changed(app: &crate::app::Shared) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
        let a = app.clone();
        rt.spawn(async move {
            let b = a.clone();
            let read = store(&a, move |tx| crate::roles::managers(tx, &runs::live(tx)?).map(|m| b.dispatcher.hold_managers(m))).await;
            if read.is_ok() && a.dispatcher.managers_changed() {
                a.dirty.notify_one();                    // the rows say which sessions are managers
            }
        });
    }

    /// A hook's facts of a watched pane: a new agent starts clean; its subagents, shape-checked (AC-127).
    fn heard_facts(&self, key: &str, ev: &serde_json::Value) {
        let mut watch = self.watch.lock();
        let pane = watch.entry(key.to_string()).or_default();
        if ev["event"] == "SessionStart" {
            pane.clear_subagents();
        }
        let Some(id) = ev["sub"].as_str().filter(|id| id.len() <= 100 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')) else { return };
        if ev["event"] == "SubagentStop" {
            pane.subs.remove(id);
        } else if pane.subs.contains_key(id) || pane.subs.len() < MOST_SUBS {
            pane.subs.entry(id.to_string()).or_insert_with(|| Sub {
                kind: crate::plans::rules::clean(ev["subtype"].as_str().unwrap_or("")).chars().take(60).collect(),
                last: Instant::now(),
                told: plan::SubStep::Nothing,
            }).last = Instant::now();
        }
    }

    /// A hook's four integers, or null when they are not (AC-129); its model goes on to the store only.
    pub fn heard_tokens(&self, key: &str, ev: &serde_json::Value) {
        let kept = usage::Totals::of(&ev["tokens"]).map_or(serde_json::Value::Null, |t| t.shown());
        let known = !kept.is_null();
        self.watch.lock().entry(key.to_string()).or_default().tokens = Some(kept);
        // what grew goes to the store, in its turn with the run's other events (AC-129)
        if known && self.has_run(key) {
            let _ = self.events.sender.send((key.to_string(), ev.clone()));
        }
    }

    fn remember_typed(&self, key: &str, text: &str) {
        self.typed.remember(key, text);
    }

    /// Whether `prompt` in pane `key` is what terminum typed there (§9): once.
    fn was_typed(&self, key: &str, prompt: &str) -> bool {
        self.typed.has(key, prompt, true)
    }

    /// Whether Dispatcher typed `prompt` into pane `key` (its message's sender, AC-147), leaving
    /// it for `was_typed`.
    pub fn typed_it(&self, key: &str, prompt: &str) -> bool {
        self.typed.has(key, prompt, false)
    }
}

/// What terminum typed into panes and no hook has said yet as a prompt, each known for SECS seconds: (pane, text, when).
#[derive(Default)]
pub(crate) struct Typed<const SECS: u64>(Mutex<Vec<(String, String, Instant)>>);

impl<const SECS: u64> Typed<SECS> {
    pub(crate) fn remember(&self, key: &str, text: &str) {
        let mut all = self.0.lock();
        all.retain(|(_, _, at)| at.elapsed().as_secs() < SECS);
        all.push((key.to_string(), text.to_string(), Instant::now()));
    }

    /// Whether a hook's `prompt` in pane `key` is what was typed there (`same_prompt`, §9); `take`: once.
    pub(crate) fn has(&self, key: &str, prompt: &str, take: bool) -> bool {
        let mut all = self.0.lock();
        all.retain(|(_, _, at)| at.elapsed().as_secs() < SECS);
        let found = all.iter().position(|(k, text, _)| k == key && same_prompt(text, prompt));
        if let (Some(at), true) = (found, take) {
            all.remove(at);
        }
        found.is_some()
    }
}

/// Whether a hook's prompt `b` is the text `a` typed into its pane: their first 20,000 characters, blanks aside.
pub(crate) fn same_prompt(a: &str, b: &str) -> bool {
    a.chars().filter(|c| !c.is_whitespace()).take(20_000).eq(b.chars().filter(|c| !c.is_whitespace()).take(20_000))
}

/// A test's own time (§9 Times): only with a test's own app folder.
fn of_test(name: &str) -> Option<i64> {
    std::env::var("TM_APP_DIR").ok().and_then(|_| std::env::var(name).ok()).and_then(|s| s.parse().ok()).filter(|s: &i64| *s > 0)
}

/// The test clock.
pub fn scale() -> i64 {
    of_test("TM_TEST_CLOCK_SCALE").unwrap_or(1)
}

/// How long after DONE or FAILED a session closes; a test that acts on a finished session gives it longer.
pub fn close_after() -> i64 {
    of_test("TM_TEST_CLOSE_AFTER").unwrap_or(plan::CLOSE)
}

/// Moves one task and says why, as Dispatcher; one already there says nothing.
/// Dispatcher never fails nor stops a task whose commit is queued: its session's end, failure, Give up or a manager's
/// stop is not its commit's, which lands as any (terminum-integration.md AC-187); a batch's integration task is in no
/// queue and moves as any.
fn move_one(tx: &rusqlite::Transaction, id: &str, to: &str, words: &str) -> Result<(), String> {
    let t = ops::get(tx, id)?;
    if t.status == to || (matches!(to, "FAILED" | "TODO") && t.status == ops::INTEGRATION && crate::tasks::integration_queue::entry_of(tx, &t.key)?.is_some()) {
        return Ok(());
    }
    let at = ops::now();
    ops::move_to(tx, DISPATCHER, id, to, at)?;
    ops::log(tx, &t.key, DISPATCHER, "why", words, None, at).map(|_| ())
}

/// `run`'s open tasks FAILED, `words` their comment; none open: `words` only said.
fn fail(tx: &rusqlite::Transaction, run: &runs::Run, words: &str) -> Result<(), String> {
    if plan::finished(&run.status) { say(tx, run, words) } else { move_task(tx, run, "FAILED", words) }
}

/// Moves `run`'s tasks still open: what is done stays done (AC-138).
fn move_task(tx: &rusqlite::Transaction, run: &runs::Run, to: &str, words: &str) -> Result<(), String> {
    run.open.iter().try_for_each(|id| move_one(tx, id, to, words))
}

/// The agent of `run` asked a human on `asked` (AC-185): its other tasks in progress wait with it, so its session
/// closes on one question. Their IDs.
pub(crate) fn wait_with(tx: &rusqlite::Transaction, run: &runs::Run, asked: &str) -> Result<Vec<String>, String> {
    let mut moved = Vec::new();
    for id in run.open.iter().filter(|id| *id != asked) {
        if ops::get(tx, id)?.status == "IN PROGRESS" {
            move_one(tx, id, "WAIT FOR HUMAN", &format!("It waits with {asked}: its session's agent asked the human there."))?;
            moved.push(id.clone());
        }
    }
    Ok(moved)
}

/// Moves all `run`'s tasks: its agent took them.
fn move_all(tx: &rusqlite::Transaction, run: &runs::Run, to: &str, words: &str) -> Result<(), String> {
    run.ids.iter().try_for_each(|id| move_one(tx, id, to, words))
}

/// Says `words` on what `run` is about now, as Dispatcher.
fn say(tx: &rusqlite::Transaction, run: &runs::Run, words: &str) -> Result<(), String> {
    run.targets().iter().try_for_each(|id| ops::log(tx, &ops::key_of(tx, id)?, DISPATCHER, "told", words, None, ops::now()).map(|_| ()))
}

/// Every page hears of `run`'s tasks.
fn told_all(app: &crate::app::Shared, run: &runs::Run) {
    for id in &run.ids {
        crate::tasks::api::told(app, id);
    }
}

#[cfg(test)]
mod typed_tests {
    #[test]
    fn ac218_hook_scope_drops_a_released_run_without_erasing_its_replacement() {
        let dispatcher = super::Dispatcher::default();
        dispatcher.track_pane("pane", "old", "api");
        dispatcher.release_pane("pane", "old");
        assert_eq!(dispatcher.pane_project("pane"), None);
        dispatcher.track_pane("pane", "new", "web");
        dispatcher.release_pane("pane", "old");
        assert_eq!(dispatcher.pane_project("pane").as_deref(), Some("web"));
    }

    /// A prompt Dispatcher typed is its message (AC-147) and still its own for the gate (§9), once.
    #[tokio::test]
    async fn ac147_a_prompt_dispatcher_typed_is_told_apart_and_left_for_its_gate() {
        let (root, app) = crate::testutil::app("dispatcher-typed");
        let d = &app.dispatcher;
        d.remember_typed("k1", "T-0412: do it");
        assert!(d.typed_it("k1", "T-0412:  do it") && !d.typed_it("k2", "T-0412: do it") && !d.typed_it("k1", "my own words"));
        assert!(d.typed_it("k1", "T-0412: do it"), "asking takes nothing");
        assert!(d.was_typed("k1", "T-0412: do it"));
        assert!(!d.was_typed("k1", "T-0412: do it") && !d.typed_it("k1", "T-0412: do it"), "the gate's once");
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}
