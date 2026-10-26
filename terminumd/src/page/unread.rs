// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Unread (AC-93): a pane whose coding agent ended a turn while no page in sight showed it
//! is unread until a page shows it, and the list marks its row. Kept in state/unread.json with
//! the identity of each tmux server the marks were made in, so a restart of terminumd (an
//! upgrade) keeps them while a tmux server started anew (a reboot, a killed tmux: its pane ids
//! start again at %0) takes its marks with it. A change marks the list as last sent and tells
//! the pages (no new read of the servers).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{json, Value};

use crate::app::Shared;

const FILE: &str = "unread.json";

/// The unread panes' keys, and each server's identity (`@terminum-id`) by its keys' start.
#[derive(Default)]
pub struct Marks {
    keys: HashSet<String>,
    servers: HashMap<String, String>,
}

impl Marks {
    /// The marks a terminumd before this one kept; none when there is no file (one that cannot
    /// be read is set aside).
    pub fn load(dir: &Path) -> Marks {
        crate::auth::store::read_or_set_aside(&dir.join(FILE), |v| {
            let keys = serde_json::from_value(v["unread"].clone()).map_err(|e| e.to_string())?;
            let servers = serde_json::from_value(v["servers"].clone()).map_err(|e| e.to_string())?;
            Ok(Marks { keys, servers })
        }).unwrap_or_default()
    }

    /// A server's list was read (`prefix` its keys' start): a server with another identity than
    /// the marks were made in started anew, and its marks go. True when anything changed.
    fn seen(&mut self, prefix: &str, identity: &str) -> bool {
        let was = self.servers.insert(prefix.to_string(), identity.to_string());
        if was.as_deref() == Some(identity) {
            return false;
        }
        if was.is_some() {
            self.keys.retain(|k| !k.starts_with(prefix));
        }
        true
    }

    fn json(&self) -> Value {
        let mut keys: Vec<&String> = self.keys.iter().collect();
        keys.sort();
        json!({"unread": keys, "servers": self.servers})
    }
}

/// One write of unread.json at a time.
static WRITING: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// unread.json with the marks as they are when it is written.
fn write(app: &Shared) {
    let _one = WRITING.lock();
    let v = app.unread.lock().json();
    if let Err(e) = crate::auth::store::write_private(&app.cfg.dir.join(FILE), &v) {
        tracing::warn!(event = "unread.unsaved", error = %e);
    }
}

/// unread.json written off the async workers, one write at a time: the last one holds the last
/// change (and `flush` the marks a stop came with).
fn save(app: &Shared) {
    if app.cfg.agent {
        return;
    }
    let a = app.clone();
    match tokio::runtime::Handle::try_current() {
        Ok(rt) => drop(rt.spawn_blocking(move || write(&a))),
        Err(_) => write(&a),
    }
}

/// Before terminumd stops: the marks as they are now on disk, whether or not a write
/// `save` started has run yet (the process ends before it would).
pub fn flush(app: &Shared) {
    if !app.cfg.agent {
        write(app);
    }
}

/// A server's list was read: its marks stay only while it is the server they were made in.
pub async fn seen(app: &Shared, server: &crate::tmux::Server) {
    let Some(identity) = server.identity().await else { return };
    if app.unread.lock().seen(&server.key(""), &identity) {
        save(app);
    }
}

/// The panes whose agent ended a turn: those no page in sight shows are unread now.
pub fn ended(app: &Shared, keys: &[String]) {
    let looking: Vec<String> = app.pages.all().iter().filter_map(|p| {
        let st = p.state.lock();
        st.shown.as_ref().filter(|_| !st.paused).map(|s| s.key.clone())
    }).collect();
    let mut unread = app.unread.lock();
    let more: Vec<&String> = keys.iter().filter(|k| !looking.contains(k) && unread.keys.insert((*k).clone())).collect();
    drop(unread);
    if !more.is_empty() {
        save(app);
        relist(app);
    }
}

/// The list as last sent takes the marks as they are now, and every page gets it.
fn relist(app: &Shared) {
    let told = {
        let mut layout = app.layout.write();
        if let Some(groups) = layout.as_array_mut() {
            mark(app, groups, false);
        }
        json!({"t": "layout", "groups": *layout}).to_string()
    };
    app.pages.tell(&told);
}

/// The user's Mark as Unread (`on`) or Mark as Read in a row's menu, for a pane the list has.
pub fn set(app: &Shared, key: &str, on: bool) {
    if app.tmux.resolve(key).is_none() {
        return;
    }
    let changed = if on { app.unread.lock().keys.insert(key.to_string()) } else { app.unread.lock().keys.remove(key) };
    if changed {
        save(app);
        relist(app);
    }
}

/// A page in sight shows pane `key`: it is read.
pub fn read(app: &Shared, key: &str) {
    if app.unread.lock().keys.remove(key) {
        save(app);
        relist(app);
    }
}

/// The list's rows of unread panes say so, but not while busy (working, waiting, its subagents
/// running: its work goes on, and a list sent before a hook's Stop may still say so). When the
/// list is whole, panes gone are forgotten.
pub fn mark(app: &Shared, groups: &mut [Value], whole: bool) {
    let mut unread = app.unread.lock();
    let mut listed = HashSet::new();
    for it in groups.iter_mut().flat_map(|g| g["items"].as_array_mut().into_iter().flatten()) {
        let Some(id) = it["id"].as_str() else { continue };
        let on = unread.keys.contains(id) && !crate::page::turns::busy(it);
        listed.insert(id.to_string());
        match it.as_object_mut() {
            Some(o) if on => { o.insert("unread".into(), json!(true)); }
            Some(o) => { o.remove("unread"); }
            None => {}
        }
    }
    let before = unread.keys.len();
    if whole {
        unread.keys.retain(|k| listed.contains(k));
    }
    if unread.keys.len() != before {
        drop(unread);
        save(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "tmux:local:terminum:";

    #[test]
    fn marks_come_back_from_the_file_and_a_broken_one_is_set_aside() {
        let dir = std::env::temp_dir().join(format!("tm-unread-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(Marks::load(&dir).keys.is_empty(), "no file: none");
        let mut m = Marks::default();
        m.keys.insert(format!("{A}%3"));
        m.servers.insert(A.into(), "id1".into());
        crate::auth::store::write_private(&dir.as_path().join(FILE), &m.json()).unwrap();
        let back = Marks::load(dir.as_path());
        assert_eq!(back.keys, m.keys);
        assert_eq!(back.servers, m.servers);
        std::fs::write(dir.as_path().join(FILE), "{not json").unwrap();
        assert!(Marks::load(dir.as_path()).keys.is_empty(), "broken: starts empty");
        assert!(dir.join("unread.json.bak").is_file(), "and the broken file is kept aside");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_server_started_anew_takes_its_marks_the_same_one_keeps_them() {
        let mut m = Marks::default();
        m.keys.extend([format!("{A}%3"), "tmux:devbox.example:default:%3".to_string()]);
        assert!(m.seen(A, "id1"), "first sight: its identity is kept");
        assert_eq!(m.keys.len(), 2, "and nothing goes");
        assert!(!m.seen(A, "id1"), "the same server (terminumd restarted): nothing changes");
        assert_eq!(m.keys.len(), 2);
        assert!(m.seen(A, "id2"), "started anew: %3 is another pane now");
        assert_eq!(m.keys.iter().collect::<Vec<_>>(), vec!["tmux:devbox.example:default:%3"], "only that server's marks go");
    }
}
