// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Where a bookmark shows (AC-33, AC-91): in every session of the project (AC-92) its session
//! was in when it was made, while the project lives (its own session may have closed), else in
//! that session alone. Once that project went (deleted, or its last session closed) or that
//! session closed it is orphaned (projects.rs says when), shown only in the list below the
//! sessions; a restored server's panes keep theirs. A file before version 3 scoped by folder: its bookmarks
//! take the project their session is in now, once.

use std::collections::HashMap;

use super::{Item, MarkError, Store};
use crate::app::Shared;

impl Store {
    /// A file before version 3: each bookmark takes the project its session is in now
    /// (`projects`: pane key -> project id), else stays its session's; saved.
    pub fn take_projects(&mut self, projects: &HashMap<String, String>) -> Result<(), MarkError> {
        if !self.legacy {
            return Ok(());
        }
        for i in self.items.iter_mut() {
            i.project = i.key.as_ref().and_then(|k| projects.get(k)).cloned();
        }
        self.save()?;
        self.legacy = false;
        Ok(())
    }

    /// Bookmarks `ids` orphaned now, saved; those it changed.
    pub fn orphan(&mut self, ids: &[String]) -> Result<Vec<Item>, MarkError> {
        let now = crate::auth::store::unix_now() as i64;
        self.edit_all(|i| {
            let gone = i.orphaned.is_none() && ids.contains(&i.id);
            if gone {
                i.orphaned = Some(now);
            }
            gone
        })
    }

    /// The server of `prefix` was made again from its checkpoint (AC-15): its bookmarks take
    /// their panes' new keys (`panes`: old id, new id); those of a pane not made again are
    /// orphaned, but a project's (it lives with its project). An orphaned one keeps its key (a
    /// pane id is reused after a restart).
    pub fn restarted(&mut self, prefix: &str, panes: &[(String, String)]) -> Result<Vec<Item>, MarkError> {
        let now = crate::auth::store::unix_now() as i64;
        self.edit_all(|i| {
            let Some(old) = i.key.as_deref().and_then(|k| k.strip_prefix(prefix)).filter(|_| i.orphaned.is_none()) else { return false };
            match panes.iter().find(|(o, _)| o == old) {
                Some((_, new)) if new != old => { i.key = Some(format!("{prefix}{new}")); true }
                Some(_) => false,
                None if i.project.is_none() => { i.orphaned = Some(now); true }
                None => false,
            }
        })
    }
}

/// At start: the bookmarks of a file before version 3 take their sessions' projects.
pub fn adopt(app: &Shared) {
    let members: HashMap<String, String> = app.projects.lock().list().iter()
        .flat_map(|p| p.members.iter().map(|m| (m.clone(), p.id.clone()))).collect();
    if let Err(e) = app.bookmarks.lock().take_projects(&members) {
        tracing::warn!(event = "bookmarks.unsaved", error = %e);
    }
}

/// Which of `items` lost their project, or (outside projects) their session: a project not in
/// `projects` (ids), a session `closed` says closed; those orphaned already are not named again.
pub fn lost(items: &[Item], projects: &[String], closed: impl Fn(&str) -> bool) -> Vec<String> {
    let gone = |i: &Item| match &i.project {
        Some(p) => !projects.contains(p),
        None => i.key.as_deref().is_some_and(&closed),
    };
    items.iter().filter(|i| i.orphaned.is_none() && gone(i)).map(|i| i.id.clone()).collect()
}
