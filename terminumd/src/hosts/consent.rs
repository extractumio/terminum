// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A host's Files set up or turned off (AC-83): the enable that runs by itself once the host attaches or
//! at the user's Enable, and the user's Not now and Remove, which it never undoes. An enable or a removal
//! runs for seconds to minutes over ssh; a later decision (another page, `terminum hosts`) wins.
//! Each long one records its result only while it is still the host's last decision, judged
//! under settings.json's write lock, where Not now and Remove record theirs too. A host is
//! "enabling" exactly while its Enable runs, so every end of an enable leaves it.

use std::collections::HashMap;

use parking_lot::Mutex;
use serde_json::{Map, Value};

use crate::app::Shared;

/// Said by an enable a later Not now or Remove overtook.
const OVERTAKEN: &str = "Not now or Remove came for this host meanwhile; nothing was recorded";

/// The Files operation a host's last decision started, while it runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Decision {
    Enable,
    Remove,
}

/// What an Enable does.
#[derive(PartialEq, Debug)]
enum Start {
    /// runs, with its ticket
    Run(u64),
    /// keeps the enable running going
    Join,
    /// waits for the removal running: its copy would race the removal on the host
    Refused,
}

#[derive(Default)]
struct Table {
    issued: u64,
    /// each host's last decision and its ticket
    last: HashMap<String, (Decision, u64)>,
}

impl Table {
    fn issue(&mut self, host: &str, d: Decision) -> u64 {
        self.issued += 1;
        self.last.insert(host.to_string(), (d, self.issued));
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
            Some((Decision::Remove, _)) => Start::Refused,
            Some((Decision::Enable, _)) => Start::Join,
            None => Start::Run(t.issue(host, Decision::Enable)),
        }
    }

    fn remove(&self, host: &str) -> u64 {
        self.0.lock().issue(host, Decision::Remove)
    }

    pub(super) fn is(&self, host: &str, d: Decision) -> bool {
        self.0.lock().last.get(host).is_some_and(|&(was, _)| was == d)
    }

    /// The operation of `ticket` ended: its decision goes, unless a later one replaced it;
    /// whether it was still the last.
    fn done(&self, host: &str, ticket: u64) -> bool {
        let mut t = self.0.lock();
        let last = t.last.get(host).is_some_and(|&(_, was)| was == ticket);
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
    fn recipe(&self) -> &[String];
}

/// Enable (AC-83) after the user's click or tick: "files": "enabled" and the helper's build
/// recorded only while Enable is still the last decision and the recipe the one it started with;
/// a step after a later Not now or Remove is not taken.
async fn enable_with(steps: &impl Steps, decisions: &Decisions, host: &str) -> Result<(), String> {
    let still = || if decisions.is(host, Decision::Enable) { Ok(()) } else { Err(OVERTAKEN.to_string()) };
    let platform = steps.probe().await?;
    still()?;
    steps.copy(platform).await?;
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

/// Enable (AC-83): the platform, the copy, a first connection; then "files": "enabled", unless
/// a later decision came.
pub async fn enable(app: &Shared, host: &str) -> Result<(), String> {
    if app.tmux.host(host).is_some_and(|s| matches!(*s.status.lock(), crate::tmux::server::Status::Off)) {
        return Err(format!("Could not enable {host}: it is off; turn it on first"));
    }
    let recipe = super::recipe_of(app, host).ok_or_else(|| format!("{host} is not attached"))?;
    let decisions = &app.hosts.decisions;
    let ticket = match decisions.enable(host) {
        Start::Run(ticket) => ticket,
        // the tick and a click together, or a click again: one runs
        Start::Join => return Ok(()),
        Start::Refused => return Err(format!("Could not enable {host}: its helper is being removed; try again when that ends")),
    };
    crate::lists::host_changed(app, host);
    let result = enable_with(&Live { app, host, recipe }, decisions, host).await;
    decisions.done(host, ticket);
    crate::lists::host_changed(app, host);
    match result {
        Ok(()) => {
            super::tunnel::stop(app, host); // an agents-only connection restarts with Files
            super::tunnel::reconcile(app, host);
            Ok(())
        }
        Err(e) => Err(format!("Could not enable {host}: {e}")),
    }
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
    fn recipe(&self) -> &[String] {
        &self.recipe
    }
}

/// "Remove helper from <host>" (AC-83): no connection starts while it runs; at its end "files"
/// is "declined", so nothing sets it up again by itself, unless a later decision came; when it fails, Files stays enabled and connects
/// again.
pub async fn remove(app: &Shared, host: &str) -> Result<(), String> {
    let recipe = super::recipe_of(app, host).ok_or_else(|| format!("{host} is not attached"))?;
    let decisions = &app.hosts.decisions;
    let ticket = decisions.remove(host);
    super::tunnel::stop(app, host);
    let result = super::tunnel::remove_helper(&recipe).await.map_err(|e| format!("Could not remove the helper from {host}: {e}"));
    let result = result.and_then(|()| super::update(app, host, |r| {
        if decisions.done(host, ticket) {
            r["files"] = Value::from("declined");
        }
    }));
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
