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

/// The Files operation a host's last decision started, while it runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Decision {
    Enable,
    Remove,
}

#[derive(Default)]
pub struct Decisions(Mutex<HashMap<String, Decision>>);

impl Decisions {
    /// `d` is the host's last decision (a second Enable while one runs keeps that one going).
    pub(super) fn make(&self, host: &str, d: Decision) {
        self.0.lock().insert(host.to_string(), d);
    }

    pub(super) fn is(&self, host: &str, d: Decision) -> bool {
        self.0.lock().get(host) == Some(&d)
    }

    /// The operation ended: its decision goes, unless a later one replaced it.
    pub(super) fn done(&self, host: &str, d: Decision) {
        let mut g = self.0.lock();
        if g.get(host) == Some(&d) {
            g.remove(host);
        }
    }

    /// Not now, or a removal without the helper's: whatever runs stops at its next step.
    fn withdraw(&self, host: &str) {
        self.0.lock().remove(host);
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
    /// The platform read, this build's helper copied.
    async fn install(&self) -> Result<(), String>;
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
    steps.install().await?;
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
    // also when one runs: this later click keeps it going
    decisions.make(host, Decision::Enable);
    let Some(before) = begin(app, host) else { return Ok(()) };
    let result = enable_with(&Live { app, host, recipe }, decisions, host).await;
    decisions.done(host, Decision::Enable);
    finish(app, host, before, result)
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
    async fn install(&self) -> Result<(), String> {
        super::tunnel::update(&self.recipe).await
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
    files.make(host, Decision::Remove);
    super::tunnel::stop(app, host);
    let removed = super::tunnel::remove_helper(&recipe).await;
    let saved = match removed {
        Ok(()) => super::save(app, |h| {
            if files.is(host, Decision::Remove) {
                record(files, h, host, None);
            }
        }),
        Err(e) => Err(format!("Could not remove the helper from {host}: {e}")),
    };
    files.done(host, Decision::Remove);
    if saved.is_err() {
        super::tunnel::reconcile(app, host);
    }
    saved
}

#[cfg(test)]
#[path = "consent_tests.rs"]
mod tests;
