// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A host's coding agents' progress turned on or off (AC-99): by itself once the host attaches,
//! `terminum hosts agents <host> on|off` and the host menu. On: the helper copied (as Files does, without registering Files),
//! terminum's hooks and MCP server installed in the host's agents, `"agents": "enabled"` stored
//! only when one took them. Off: they are taken out, `"agents": "off"` stored so nothing turns them on
//! again by itself, the helper removed only
//! when Files does not use it. Each runs in the host's turn at its helper (turns.rs), so Files'
//! enable or removal never runs beside it. The ssh steps are behind [`Ops`] so the decisions are tested.

use serde_json::{Map, Value};

use crate::app::Shared;

type Got = Vec<(String, String)>;

/// Said when the host's record changed while an operation ran on the recipe it started with.
pub(super) const CHANGED: &str = "the host's record changed meanwhile; nothing was saved, try again";

/// The steps that reach the host or settings.json, all over the recipe read once at the start.
pub(crate) trait Ops {
    async fn install(&self) -> Result<Got, String>;
    async fn remove(&self) -> Result<Got, String>;
    async fn remove_helper(&self) -> Result<(), String>;
    fn save(&self, f: &mut dyn FnMut(&mut Map<String, Value>)) -> Result<(), String>;
    /// The link follows the stored setting (link::changed).
    fn changed(&self);
    /// The recipe this operation started with.
    fn recipe(&self) -> &[String];
}

/// How the refusal begins when a host has no coding agent to take the hooks: no failure of the
/// setup by itself.
const NO_AGENT: &str = "No coding agent on ";

/// Each agent's line: `<agent>: <result as shown>`.
pub fn lines(host: &str, got: &[(String, String)]) -> Vec<String> {
    got.iter().map(|(a, r)| format!("{a}: {}", super::agentsinstall::shown(host, r))).collect()
}

/// At least one agent took terminum's hooks (or uses the host's own terminum).
pub fn took(got: &[(String, String)]) -> bool {
    got.iter().any(|(_, r)| matches!(r.as_str(), "installed" | "needs-trust" | "skipped-own"))
}

/// Files uses the helper: it stays when agents go off.
fn files_use_helper(rec: &Value) -> bool {
    matches!(rec["files"].as_str(), Some("enabled" | "wanted"))
}

/// On: nothing is saved when the record changed during the ssh steps (a stale operation never
/// writes over a newer record). Off: saved and the link closed first, so /hook and /agent stop at
/// once; the host's entries go after that, bounded, with their own result.
pub(crate) async fn set_with(ops: &impl Ops, host: &str, on: bool) -> Result<Vec<String>, String> {
    if on {
        let got = ops.install().await.map_err(|e| format!("Could not set up coding agents' progress on {host}: {e}"))?;
        let shown = lines(host, &got);
        if !took(&got) {
            return Err(format!("{NO_AGENT}{host} took terminum's hooks: {}", shown.join("; ")));
        }
        // judged under the settings file's write lock: the record a newer edit left is kept
        let mut saved = false;
        ops.save(&mut |h| {
            if let Some(r) = h.get_mut(host).filter(|r| r["ssh"] == Value::from(ops.recipe())) {
                r["agents"] = Value::from("enabled");
                saved = true;
            }
        })?;
        if !saved {
            return Err(format!("Could not set up coding agents' progress on {host}: {CHANGED}"));
        }
        ops.changed();
        return Ok(shown);
    }
    let mut keep_helper = true;
    ops.save(&mut |h| {
        if let Some(r) = h.get_mut(host) {
            r["agents"] = Value::from("off");
            keep_helper = files_use_helper(r);
        }
    })?;
    ops.changed();
    let got = ops.remove().await.map_err(|e| format!("Turned off here; could not take terminum's hooks out of {host}: {e}"))?;
    let mut shown = lines(host, &got);
    if !keep_helper {
        match ops.remove_helper().await {
            Ok(()) => shown.push(format!("helper removed from {host}")),
            Err(e) => return Err(format!("Turned off here; {}; could not remove the helper from {host}: {e}", shown.join("; "))),
        }
    }
    Ok(shown)
}

/// The running terminumd's steps for `host`, over the recipe read when the operation began.
struct Live<'a> {
    app: &'a Shared,
    host: &'a str,
    recipe: Vec<String>,
}

impl Ops for Live<'_> {
    async fn install(&self) -> Result<Got, String> {
        let current = || super::recipe_of(self.app, self.host).is_some_and(|r| r == self.recipe);
        super::agentsinstall::install(&self.recipe, |recipe| async move { super::tunnel::update(&recipe).await }, current).await
    }
    async fn remove(&self) -> Result<Got, String> {
        super::agentsinstall::remove_on(&self.recipe).await
    }
    async fn remove_helper(&self) -> Result<(), String> {
        super::tunnel::remove_helper(&self.recipe).await
    }
    fn save(&self, f: &mut dyn FnMut(&mut Map<String, Value>)) -> Result<(), String> {
        super::save(self.app, f)
    }
    fn changed(&self) {
        super::link::changed(self.app, self.host);
    }
    fn recipe(&self) -> &[String] {
        &self.recipe
    }
}

/// `hosts agents <host> on|off`: the per-agent result lines.
pub async fn set_agents(app: &Shared, host: &str, on: bool) -> Result<Vec<String>, String> {
    if !super::records(app).contains_key(host) {
        return Err(format!("no host named {host}"));
    }
    app.helper_turns.run(host, async {
        // read in its turn: an earlier operation may have changed the record
        let recipe = super::recipe_of(app, host).ok_or_else(|| format!("{host} is not attached"))?;
        set_with(&Live { app, host, recipe }, host, on).await
    }).await
}

/// The host attached with no word of the user's on its agents: turned on by itself (AC-99), in
/// the host's turn after any on or off running, which it never stops, and only while the record
/// still says nothing. A host with no coding agent is done until terminumd starts anew (an
/// upgrade); another failure is shown as the agents' state and tried at the next attach. A helper
/// it copied for nothing goes again unless Files uses it.
pub fn auto(app: &Shared, host: &str) {
    if !undecided(app, host) || matches!(app.hosts.agents_auto.lock().get(host), Some(None)) {
        return;
    }
    let (app, host) = (app.clone(), host.to_string());
    tokio::spawn(async move {
        let got = app.helper_turns.after(&host, async {
            if !undecided(&app, &host) {
                return Ok(None);
            }
            let recipe = super::recipe_of(&app, &host).ok_or_else(|| format!("{host} is not attached"))?;
            let ops = Live { app: &app, host: &host, recipe };
            let got = set_with(&ops, &host, true).await;
            if got.is_err() && !super::records(&app).get(&host).is_some_and(files_use_helper) {
                let _ = ops.remove_helper().await;
            }
            got.map(Some)
        }).await;
        let failed = match got {
            Ok(None) => return,
            Ok(Some(lines)) => {
                tracing::info!(event = "agents.auto", host, result = lines.join("; "));
                None
            }
            Err(why) if why.starts_with(NO_AGENT) => {
                tracing::info!(event = "agents.auto_none", host, why);
                None
            }
            Err(why) => {
                tracing::warn!(event = "agents.auto_failed", host, why);
                Some(why)
            }
        };
        app.hosts.agents_auto.lock().insert(host.clone(), failed);
        crate::lists::host_changed(&app, &host);
    });
}

/// The host's record has no word of the user's on its agents: neither on nor off.
fn undecided(app: &Shared, host: &str) -> bool {
    app.looks.settings.read().hosts.get(host).is_some_and(|r| r["agents"].is_null())
}

/// Why turning the host's agents on by itself failed, while nothing turned them on or off since.
pub fn auto_failed(app: &Shared, host: &str) -> Option<String> {
    app.hosts.agents_auto.lock().get(host).cloned().flatten().filter(|_| undecided(app, host))
}

/// After AC-84 updated the helper: the host's agents' entries rewritten where they differ, in
/// the host's turn after any on or off running (agents still on then).
pub fn after_update(app: &Shared, host: &str) {
    if !super::link::wanted(app, host) {
        return;
    }
    let (app, host) = (app.clone(), host.to_string());
    tokio::spawn(async move {
        let refresh = async {
            if !super::link::wanted(&app, &host) {
                return Ok(Vec::new());
            }
            super::agentsinstall::refresh(&app, &host).await
        };
        match app.helper_turns.after(&host, refresh).await {
            Ok(got) => tracing::info!(event = "agents.refreshed", host, result = lines(&host, &got).join("; ")),
            Err(why) => tracing::warn!(event = "agents.refresh_failed", host, why),
        }
    });
}

/// The host is being removed with agents on, or while an operation at its helper runs for it
/// (an on's entries may be there already): an on or off stopped, terminum's entries taken out,
/// best effort.
pub fn removed(app: &Shared, host: &str) {
    let Some(recipe) = super::recipe_of(app, host).filter(|_| super::link::wanted(app, host) || app.helper_turns.busy(host)) else { return };
    let (app, host) = (app.clone(), host.to_string());
    tokio::spawn(async move {
        match app.helper_turns.run(&host, super::agentsinstall::remove_on(&recipe)).await {
            Ok(got) => tracing::info!(event = "agents.removed", host, result = lines(&host, &got).join("; ")),
            Err(why) => tracing::warn!(event = "agents.remove_failed", host, why),
        }
    });
}

#[cfg(test)]
#[path = "agents_tests.rs"]
mod tests;
