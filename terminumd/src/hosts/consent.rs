// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A host's Files set up or turned off (AC-83): the enable that runs by itself once the host attaches or
//! at the user's Enable, and the user's Not now and Remove, which it never undoes. An enable or a removal
//! runs for seconds to minutes over ssh, in the host's turn at its helper (turns.rs); a later
//! decision (another page, `terminum hosts`) wins, and a later Enable joins the one running.
//! Each long one records its result only while it is still the host's last decision, judged
//! under settings.json's write lock, where Not now and Remove record theirs too. A host is
//! "enabling" exactly while its Enable runs, so every end of an enable leaves it.

use std::collections::HashMap;

use parking_lot::Mutex;
use serde_json::{Map, Value};
use tokio::sync::watch;

use crate::app::Shared;

/// Said by an enable a later Not now or Remove overtook.
const OVERTAKEN: &str = "Not now or Remove came for this host meanwhile; nothing was recorded";

/// The Files operation a host's last decision started, while it runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Decision {
    Enable,
    Remove,
}

/// Said to an Enable that joined one stopped before its end (its host's connection replaced).
const STOPPED: &str = "the setup running was stopped; try again";

/// An enable's result once it ended, for the Enables that joined it.
type Outcome = Option<Result<(), String>>;

/// What an Enable does.
enum Start {
    /// runs, with its ticket and where its result goes
    Run(u64, watch::Sender<Outcome>),
    /// keeps the enable running going and takes its result
    Join(watch::Receiver<Outcome>),
    /// refused while a removal runs or waits: the user asked for the helper to go
    Refused,
}

/// A host's last decision.
struct Last {
    decision: Decision,
    ticket: u64,
    /// an Enable's: where its result comes
    result: Option<watch::Receiver<Outcome>>,
}

#[derive(Default)]
struct Table {
    issued: u64,
    last: HashMap<String, Last>,
}

impl Table {
    fn issue(&mut self, host: &str, decision: Decision, result: Option<watch::Receiver<Outcome>>) -> u64 {
        self.issued += 1;
        self.last.insert(host.to_string(), Last { decision, ticket: self.issued, result });
        self.issued
    }
}

/// The decisions running, by host: an operation's end takes only its own.
#[derive(Default)]
pub struct Decisions(Mutex<Table>);

impl Decisions {
    fn enable(&self, host: &str) -> Start {
        let mut t = self.0.lock();
        match t.last.get(host) {
            Some(Last { decision: Decision::Remove, .. }) => Start::Refused,
            Some(Last { result: Some(result), .. }) => Start::Join(result.clone()),
            _ => {
                let (to, result) = watch::channel(None);
                Start::Run(t.issue(host, Decision::Enable, Some(result)), to)
            }
        }
    }

    fn remove(&self, host: &str) -> u64 {
        self.0.lock().issue(host, Decision::Remove, None)
    }

    pub(super) fn is(&self, host: &str, d: Decision) -> bool {
        self.0.lock().last.get(host).is_some_and(|l| l.decision == d)
    }

    /// The operation of `ticket` ended: its decision goes, unless a later one replaced it;
    /// whether it was still the last.
    fn done(&self, host: &str, ticket: u64) -> bool {
        let mut t = self.0.lock();
        let last = t.last.get(host).is_some_and(|l| l.ticket == ticket);
        if last {
            t.last.remove(host);
        }
        last
    }

    /// Not now, a removal without the helper's, Files unticked, the host removed: whatever runs
    /// stops at its next step.
    pub(super) fn withdraw(&self, host: &str) {
        self.0.lock().last.remove(host);
    }
}

/// Not now (or a removal while agents keep the helper) recorded as the host's last decision;
/// whether Files was enabled.
fn record(decisions: &Decisions, h: &mut Map<String, Value>, host: &str, files: Option<&str>) -> bool {
    let Some(r) = h.get_mut(host) else { return false };
    decisions.withdraw(host);
    std::mem::replace(&mut r["files"], files.map_or(Value::Null, Value::from)) == "enabled"
}

/// Records `files` for `host` (Not now, or Files removed while agents keep the helper); whether
/// Files was enabled.
pub fn decide(app: &Shared, host: &str, files: Option<&str>) -> Result<bool, String> {
    let mut was = false;
    super::save(app, |h| was = record(&app.hosts.decisions, h, host, files))?;
    Ok(was)
}

/// An enable's steps that reach the host or settings.json, over the recipe read at its start.
trait Steps {
    /// The host's platform, or why it has no helper.
    async fn probe(&self) -> Result<&'static str, String>;
    /// This build's helper for `platform` copied.
    async fn copy(&self, platform: &'static str) -> Result<(), String>;
    /// A trial connection through the forward, answered by this build's helper.
    async fn trial(&self) -> Result<(), String>;
    fn save(&self, f: &mut dyn FnMut(&mut Map<String, Value>)) -> Result<(), String>;
    /// The helper copied for nothing taken off the host again, unless Files or agents use it.
    async fn discard(&self);
    fn recipe(&self) -> &[String];
}

/// Enable (AC-83) after the user's click or tick: "files": "enabled" and the helper's build
/// recorded only while Enable is still the last decision and the recipe the one it started with;
/// a step after a later Not now or Remove is not taken, the first neither when one came while
/// the enable waited for its turn. An enable that fails after its copy leaves no helper behind.
async fn enable_with(steps: &impl Steps, decisions: &Decisions, host: &str) -> Result<(), String> {
    let still = || if decisions.is(host, Decision::Enable) { Ok(()) } else { Err(OVERTAKEN.to_string()) };
    still()?;
    let platform = steps.probe().await?;
    still()?;
    steps.copy(platform).await?;
    let result = record_after_copy(steps, host, still).await;
    if result.is_err() {
        steps.discard().await;
    }
    result
}

/// An enable's steps after its copy.
async fn record_after_copy(steps: &impl Steps, host: &str, still: impl Fn() -> Result<(), String>) -> Result<(), String> {
    still()?;
    steps.trial().await?;
    still()?;
    let mut result = Err(format!("{host} changed while it was set up"));
    steps.save(&mut |h| {
        if let Some(r) = h.get_mut(host).filter(|r| r["ssh"] == Value::from(steps.recipe())) {
            result = still().map(|()| {
                r["files"] = Value::from("enabled");
                r["helper"] = Value::from(crate::agent::AGENT_ID);
            });
        }
    })?;
    result
}

/// The result of the enable running, for an Enable that joined it.
async fn joined(mut result: watch::Receiver<Outcome>) -> Result<(), String> {
    match result.wait_for(Option::is_some).await {
        Ok(got) => got.clone().unwrap_or_else(|| Err(STOPPED.into())),
        Err(_) => Err(STOPPED.into()),
    }
}

/// An enable running: its end takes its decision, so the host leaves "enabling", also when it is
/// dropped mid-step (its task stopped with the host's connection).
struct Running<'a> {
    app: &'a Shared,
    host: &'a str,
    ticket: u64,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.app.hosts.decisions.done(self.host, self.ticket);
        crate::lists::host_changed(self.app, self.host);
    }
}

/// Enable (AC-83), in the host's turn at its helper: the platform, the copy, a first connection;
/// then "files": "enabled", unless a later decision came, and its agents by themselves (AC-99).
/// An Enable while one runs joins it and gets its result.
pub async fn enable(app: &Shared, host: &str) -> Result<(), String> {
    if app.tmux.host(host).is_some_and(|s| matches!(*s.status.lock(), crate::tmux::server::Status::Off)) {
        return Err(format!("Could not enable {host}: it is off; turn it on first"));
    }
    if super::recipe_of(app, host).is_none() {
        return Err(format!("{host} is not attached"));
    }
    let decisions = &app.hosts.decisions;
    let (ticket, to) = match decisions.enable(host) {
        Start::Run(ticket, to) => (ticket, to),
        // the setup by itself and a click together, or a click again: one runs
        Start::Join(result) => return joined(result).await,
        Start::Refused => return Err(format!("Could not enable {host}: its helper is being removed; try again when that ends")),
    };
    crate::lists::host_changed(app, host);
    let running = Running { app, host, ticket };
    let result = app.helper_turns.hold(host, async {
        // read in its turn: an earlier operation may have changed the record
        let recipe = super::recipe_of(app, host).ok_or_else(|| format!("{host} is not attached"))?;
        enable_with(&Live { app, host, recipe }, decisions, host).await
    }).await.map_err(|e| format!("Could not enable {host}: {e}"));
    drop(running);
    if result.is_ok() {
        super::tunnel::stop(app, host); // an agents-only connection restarts with Files
        super::tunnel::reconcile(app, host);
        // its agents after its files: the helper is there
        super::agents::auto(app, host);
    }
    let _ = to.send(Some(result.clone()));
    result
}

/// The running terminumd's enable steps for `host`.
struct Live<'a> {
    app: &'a Shared,
    host: &'a str,
    recipe: Vec<String>,
}

impl Steps for Live<'_> {
    async fn probe(&self) -> Result<&'static str, String> {
        super::tunnel::probe(&self.recipe).await?.0
    }
    async fn copy(&self, platform: &'static str) -> Result<(), String> {
        super::tunnel::copy(&self.recipe, platform).await
    }
    async fn trial(&self) -> Result<(), String> {
        super::tunnel::trial(self.app, self.host, &self.recipe).await
    }
    fn save(&self, f: &mut dyn FnMut(&mut Map<String, Value>)) -> Result<(), String> {
        super::save(self.app, f)
    }
    async fn discard(&self) {
        let used = super::link::wanted(self.app, self.host) || super::files_state(self.app, self.host).as_deref() == Some("enabled");
        if !used && let Err(why) = super::tunnel::remove_helper(&self.recipe).await {
            tracing::warn!(event = "agent.discard_failed", host = self.host, why);
        }
    }
    fn recipe(&self) -> &[String] {
        &self.recipe
    }
}

/// "Remove helper from <host>" (AC-83), in the host's turn at its helper: while agents are on
/// they keep the helper and only Files goes; else no connection starts while it is removed. At
/// its end "files" is "declined", so nothing sets it up again by itself, unless a later decision
/// came; when it fails, Files stays enabled and connects again.
pub async fn remove(app: &Shared, host: &str) -> Result<(), String> {
    let decisions = &app.hosts.decisions;
    let ticket = decisions.remove(host);
    let result = app.helper_turns.hold(host, async {
        // judged in its turn: no agents on or off runs beside it
        let kept = super::link::wanted(app, host);
        if !kept {
            let recipe = super::recipe_of(app, host).ok_or_else(|| format!("{host} is not attached"))?;
            super::tunnel::stop(app, host);
            super::tunnel::remove_helper(&recipe).await.map_err(|e| format!("Could not remove the helper from {host}: {e}"))?;
        }
        super::update(app, host, |r| {
            if decisions.done(host, ticket) {
                r["files"] = Value::from("declined");
            }
        })?;
        if kept {
            super::link::changed(app, host);
        }
        Ok(())
    }).await;
    // where the save's check never ran: a failure, the host's record gone
    decisions.done(host, ticket);
    if result.is_err() {
        super::tunnel::reconcile(app, host);
    }
    result
}

#[cfg(test)]
#[path = "consent_tests.rs"]
mod tests;
