// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The user's Files decisions on a host (AC-83): Enable, Not now, Remove. An enable or a removal
//! runs for seconds to minutes over ssh; a later decision (another page, `terminum hosts`) wins.
//! Each long one records its result only while it is still the host's last decision, judged
//! under settings.json's write lock, where Not now and Remove record theirs too.

use std::collections::HashMap;

use parking_lot::Mutex;
use serde_json::{Map, Value};

use crate::app::Shared;

/// Said by an enable a later Not now or Remove overtook.
pub(super) const OVERTAKEN: &str = "Not now or Remove came for this host meanwhile; nothing was recorded";

/// Said by an Enable while the helper's removal runs.
pub(super) const REMOVING: &str = "its helper is being removed; try again when that ends";

/// The Files operation a host's last decision started, while it runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Decision {
    Enable,
    Remove,
}

/// Each host's last decision and its ticket: an operation's end takes only its own.
#[derive(Default)]
pub struct Decisions(Mutex<(u64, HashMap<String, (Decision, u64)>)>);

impl Decisions {
    /// `d` is the host's last decision; its ticket. Enable is refused while a Remove runs (its
    /// copy would race the removal on the host); a second Enable keeps a running one going.
    pub(super) fn make(&self, host: &str, d: Decision) -> Option<u64> {
        let mut g = self.0.lock();
        if d == Decision::Enable && g.1.get(host).is_some_and(|&(was, _)| was == Decision::Remove) {
            return None;
        }
        g.0 += 1;
        let ticket = g.0;
        g.1.insert(host.to_string(), (d, ticket));
        Some(ticket)
    }

    pub(super) fn is(&self, host: &str, d: Decision) -> bool {
        self.0.lock().1.get(host).is_some_and(|&(was, _)| was == d)
    }

    /// The operation of `ticket` ended: its decision goes, unless a later one replaced it;
    /// whether it was still the last.
    pub(super) fn done(&self, host: &str, ticket: u64) -> bool {
        let mut g = self.0.lock();
        let last = g.1.get(host).is_some_and(|&(_, t)| t == ticket);
        if last {
            g.1.remove(host);
        }
        last
    }

    /// Not now, a removal without the helper's, the host removed: whatever runs stops at its
    /// next step.
    pub(super) fn withdraw(&self, host: &str) {
        self.0.lock().1.remove(host);
    }
}

/// Not now (or a removal while agents keep the helper): recorded as the host's last decision.
pub(super) fn record(decisions: &Decisions, h: &mut Map<String, Value>, host: &str, files: Option<&str>) {
    if let Some(r) = h.get_mut(host) {
        decisions.withdraw(host);
        r["files"] = files.map_or(Value::Null, Value::from);
    }
}

/// Records `files` for `host` (Not now, or Files removed while agents keep the helper).
pub fn decide(app: &Shared, host: &str, files: Option<&str>) -> Result<(), String> {
    super::save(app, |h| record(&app.hosts.files, h, host, files))
}

/// An enable's steps that reach the host or settings.json, over the recipe read at its start.
pub(crate) trait Steps {
    /// The host's platform, or why it has no helper.
    async fn probe(&self) -> Result<&'static str, String>;
    /// This build's helper for `platform` copied.
    async fn copy(&self, platform: &'static str) -> Result<(), String>;
    /// A trial connection through the forward, answered by this build's helper.
    async fn trial(&self) -> Result<(), String>;
    fn save(&self, f: &mut dyn FnMut(&mut Map<String, Value>)) -> Result<(), String>;
    fn recipe(&self) -> &[String];
}

/// Enable (AC-83) after the user's click or tick: "files": "enabled" recorded only while Enable is
/// still the last decision and the recipe the one it started with; a step after a later Not now
/// or Remove is not taken.
pub(crate) async fn enable_with(steps: &impl Steps, decisions: &Decisions, host: &str) -> Result<(), String> {
    let still = || if decisions.is(host, Decision::Enable) { Ok(()) } else { Err(OVERTAKEN.to_string()) };
    let platform = steps.probe().await?;
    still()?;
    steps.copy(platform).await?;
    still()?;
    steps.trial().await?;
    still()?;
    let (mut ours, mut saved) = (false, false);
    steps.save(&mut |h| {
        let Some(r) = h.get_mut(host).filter(|r| r["ssh"] == Value::from(steps.recipe())) else { return };
        ours = true;
        if decisions.is(host, Decision::Enable) {
            r["files"] = Value::from("enabled");
            saved = true;
        }
    })?;
    match (ours, saved) {
        (false, _) => Err(format!("{host} changed while it was set up")),
        (true, false) => Err(OVERTAKEN.into()),
        (true, true) => Ok(()),
    }
}

/// A ticked enabling that failed: "wanted" cleared, unless a later decision replaced it; whether
/// it was (the question then comes back with the reason).
pub(super) fn unwant(app: &Shared, host: &str) -> Result<bool, String> {
    let mut cleared = false;
    super::save(app, |h| {
        if let Some(r) = h.get_mut(host).filter(|r| r["files"] == "wanted") {
            r["files"] = Value::Null;
            cleared = true;
        }
    })?;
    Ok(cleared)
}

/// Enable (AC-83): the platform, the copy, a first connection; then "files": "enabled", unless
/// a later decision came. Every end leaves "enabling", so Enable can be tried again.
pub async fn enable(app: &Shared, host: &str) -> Result<(), String> {
    if app.tmux.host(host).is_some_and(|s| matches!(*s.status.lock(), crate::tmux::server::Status::Off)) {
        return Err(format!("Could not enable {host}: it is off; turn it on first"));
    }
    let recipe = super::recipe_of(app, host).ok_or_else(|| format!("{host} is not attached"))?;
    let decisions = &app.hosts.files;
    let ticket = decisions.make(host, Decision::Enable).ok_or_else(|| format!("Could not enable {host}: {REMOVING}"))?;
    let Some(before) = begin(app, host) else { return Ok(()) };
    let result = enable_with(&Live { app, host, recipe }, decisions, host).await;
    // ended before its decision goes: a click after it starts an enable of its own
    let result = finish(app, host, before, result);
    decisions.done(host, ticket);
    result
}

/// The state and note an enable found.
type Before = (String, String);

/// Marks `host` "enabling"; none when an enable already runs (checked and set at once: the tick
/// and a click may start it together).
fn begin(app: &Shared, host: &str) -> Option<Before> {
    let before = {
        let mut g = app.tunnels.0.lock();
        let a = g.entry(host.to_string()).or_default();
        if a.state == "enabling" {
            return None;
        }
        let state = std::mem::replace(&mut a.state, "enabling".into());
        (state, std::mem::replace(&mut a.note, format!("setting up {host}…")))
    };
    crate::lists::host_changed(app, host);
    Some(before)
}

/// An enable's end, whichever step it failed at: never left "enabling".
fn finish(app: &Shared, host: &str, (state, note): Before, result: Result<(), String>) -> Result<(), String> {
    let Err(e) = result else {
        let _ = super::set_helper(app, host, crate::agent::AGENT_ID);
        super::tunnel::stop(app, host); // an agents-only connection restarts with Files
        super::tunnel::reconcile(app, host);
        return Ok(());
    };
    let mut g = app.tunnels.0.lock();
    match g.get_mut(host) {
        // an agents-only connection keeps running, in the state it was in unless it said a newer one
        Some(a) if a.task.as_ref().is_some_and(|t| !t.is_finished()) => {
            if a.state == "enabling" {
                (a.state, a.note) = (state, note);
            }
        }
        _ => drop(g.remove(host)),
    }
    drop(g);
    crate::lists::host_changed(app, host);
    Err(format!("Could not enable {host}: {e}"))
}

/// The running terminumd's enable steps for `host`.
pub(super) struct Live<'a> {
    pub app: &'a Shared,
    pub host: &'a str,
    pub recipe: Vec<String>,
}

impl Steps for Live<'_> {
    async fn probe(&self) -> Result<&'static str, String> {
        super::tunnel::probe(&self.recipe).await?.0
    }
    async fn copy(&self, platform: &'static str) -> Result<(), String> {
        super::tunnel::install(&self.recipe, &super::tunnel::binary_for(platform)?, crate::agent::AGENT_ID).await
    }
    async fn trial(&self) -> Result<(), String> {
        let mut trial = super::tunnel::connect(self.app, self.host, &self.recipe, None, true).await?;
        let _ = trial.child.kill().await;
        if !super::tunnel::current(&trial.agent_id) {
            return Err(format!("the helper on {} answers as build {:?}", self.host, trial.agent_id));
        }
        crate::remote::register(self.app, self.host, None);
        Ok(())
    }
    fn save(&self, f: &mut dyn FnMut(&mut Map<String, Value>)) -> Result<(), String> {
        super::save(self.app, f)
    }
    fn recipe(&self) -> &[String] {
        &self.recipe
    }
}

/// "Remove helper from <host>" (AC-83): no connection starts while it runs; at its end "files"
/// is cleared, unless a later decision came; when it fails, Files stays enabled and connects
/// again.
pub async fn remove(app: &Shared, host: &str) -> Result<(), String> {
    let recipe = super::recipe_of(app, host).ok_or_else(|| format!("{host} is not attached"))?;
    let files = &app.hosts.files;
    let ticket = files.make(host, Decision::Remove).expect("a removal is never refused");
    super::tunnel::stop(app, host);
    let saved = match super::tunnel::remove_helper(&recipe).await {
        Ok(()) => super::save(app, |h| {
            if let Some(r) = h.get_mut(host).filter(|_| files.done(host, ticket)) {
                r["files"] = Value::Null;
            }
        }),
        Err(e) => Err(format!("Could not remove the helper from {host}: {e}")),
    };
    files.done(host, ticket);
    if saved.is_err() {
        super::tunnel::reconcile(app, host);
    }
    saved
}

#[cfg(test)]
#[path = "consent_tests.rs"]
mod tests;
