// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Notes a coding agent leaves for the user (bugs, deferred work, follow-ups, questions;
//! AC-97): scoped as bookmarks are (page/bookmarks_scope.rs), to the project the pane's session
//! is in, else to that pane (its own notes stay with it once it joins a project); at most 100
//! open per scope, the 500 newest done kept. A tmux server started anew hides its panes' notes
//! (kept). state/notes.json, 0600, written atomically; a file that cannot be read is set aside
//! as notes.json.bak, never written over.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;

const FILE: &str = "notes.json";
const VERSION: u64 = 1;
pub const OPEN_MOST: usize = 100;
/// Done notes kept at most, the newest.
pub const DONE_MOST: usize = 500;
/// Notes kept at most in all: past it the oldest done go first, then the oldest unseen.
pub const ALL_MOST: usize = 2000;
/// An open note shown nowhere (orphaned, or its pane gone outside a project) goes this old.
pub const UNSEEN_DAYS: u64 = 30;
pub use crate::plans::rules::{clean, KINDS};
use crate::plans::rules::{NOTE_FILE, NOTES_MOST, NOTE_TEXT};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub kind: String,
    pub text: String,
    pub file: Option<String>,
    /// open | done
    pub status: String,
    pub at: u64,
    pub done_at: Option<u64>,
    /// the pane key it was made in
    pub key: String,
    pub session: String,
    pub project: Option<String>,
    /// its tmux server started anew (pane ids begin again at %0): kept, shown nowhere
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub orphaned: bool,
}

/// Where a note shows: the project's sessions, else the pane alone.
#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    pub key: String,
    pub project: Option<String>,
}

impl Scope {
    fn holds(&self, n: &Note) -> bool {
        match &self.project {
            // the pane's own notes made before its session joined a project stay with it
            Some(p) if n.project.as_deref() == Some(p) => true,
            _ => n.project.is_none() && n.key == self.key && !n.orphaned,
        }
    }
}

/// A note as the agent gave it: kind, text, file.
pub struct New {
    pub kind: String,
    pub text: String,
    pub file: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Inner {
    next: u64,
    notes: Vec<Note>,
    /// each tmux server's identity by its keys' start, as plans.json keeps it
    #[serde(default)]
    servers: HashMap<String, String>,
}

impl Inner {
    /// Keeps the DONE_MOST newest done notes, drops open notes shown nowhere (orphaned, or
    /// their pane not in `live` and in no project) once UNSEEN_DAYS old, and holds ALL_MOST in
    /// all (oldest done first, then oldest unseen). `live`: None when the panes are not known.
    fn prune(&mut self, now: u64, live: Option<&HashSet<String>>) {
        let unseen = |n: &Note| n.status == "open" && (n.orphaned || (n.project.is_none() && live.is_some_and(|l| !l.contains(&n.key))));
        let old = now.saturating_sub(UNSEEN_DAYS * 86_400);
        self.notes.retain(|n| !(unseen(n) && n.at < old));
        let mut done: Vec<(u64, u64)> = self.notes.iter().filter(|n| n.status == "done").map(|n| (n.done_at.unwrap_or(n.at), n.id)).collect();
        done.sort_unstable_by(|a, b| b.cmp(a));
        let mut gone: HashSet<u64> = done.iter().skip(DONE_MOST).map(|(_, id)| *id).collect();
        let over = |gone: &HashSet<u64>| (self.notes.len() - gone.len()).saturating_sub(ALL_MOST);
        let kept_done: Vec<u64> = done.iter().take(DONE_MOST).rev().map(|(_, id)| *id).collect();
        let n = over(&gone);
        gone.extend(kept_done.into_iter().take(n));
        let mut hidden: Vec<(u64, u64)> = self.notes.iter().filter(|n| unseen(n)).map(|n| (n.at, n.id)).collect();
        hidden.sort_unstable();
        let n = over(&gone);
        gone.extend(hidden.into_iter().take(n).map(|(_, id)| id));
        if !gone.is_empty() {
            self.notes.retain(|n| !gone.contains(&n.id));
        }
    }
}

pub struct Notes {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

fn kind_ok(k: &str) -> Result<(), String> {
    KINDS.contains(&k).then_some(()).ok_or_else(|| format!("kind {k} is not one of {}", KINDS.join(", ")))
}

fn text_ok(t: &str, what: &str, max: usize) -> Result<(), String> {
    match t.chars().count() {
        0 => Err(format!("{what} is empty")),
        n if n > max => Err(format!("{what} is over {max} characters")),
        _ => Ok(()),
    }
}

impl Notes {
    /// The notes a terminumd before this one kept; none without a file. `None`: kept in memory.
    pub fn load(dir: Option<&Path>) -> Notes {
        let path = dir.map(|d| d.join(FILE));
        let inner = path.as_deref().and_then(|p| crate::auth::store::read_or_set_aside(p, |v| {
            if v["version"] != VERSION {
                return Err(format!("version {} is not {VERSION}", v["version"]));
            }
            serde_json::from_value::<Inner>(v).map_err(|e| e.to_string())
        })).unwrap_or_default();
        Notes { path, inner: Mutex::new(inner) }
    }

    fn save(&self, inner: &Inner) -> Result<(), String> {
        let Some(p) = &self.path else { return Ok(()) };
        let v = json!({"version": VERSION, "next": inner.next, "notes": inner.notes, "servers": inner.servers});
        crate::auth::store::write_private(p, &v).map_err(|e| format!("notes.json: {e}"))
    }

    /// Adds 1–20 notes to `scope`; their ids, or why none was added.
    pub fn add(&self, scope: &Scope, session: &str, new: Vec<New>, now: u64) -> Result<Vec<u64>, String> {
        if new.is_empty() || new.len() > NOTES_MOST {
            return Err(format!("give 1–{NOTES_MOST} notes, not {}", new.len()));
        }
        let mut made = Vec::new();
        for (i, n) in new.into_iter().enumerate() {
            let at = format!("notes[{i}].");
            kind_ok(&n.kind).map_err(|e| format!("{at}{e}"))?;
            let text = clean(&n.text);
            text_ok(&text, &format!("{at}text"), NOTE_TEXT)?;
            let file = n.file.map(|f| clean(&f)).filter(|f| !f.is_empty());
            if let Some(f) = &file {
                text_ok(f, &format!("{at}file"), NOTE_FILE)?;
            }
            made.push((n.kind, text, file));
        }
        let mut inner = self.inner.lock();
        let open = inner.notes.iter().filter(|n| n.status == "open" && scope.holds(n)).count();
        if open + made.len() > OPEN_MOST {
            return Err(format!("This session has {open} open notes; at most {OPEN_MOST} may be open. Mark finished ones done with update_notes_for_user."));
        }
        let (next, len) = (inner.next, inner.notes.len());
        let mut ids = Vec::new();
        for (kind, text, file) in made {
            inner.next += 1;
            let id = inner.next;
            inner.notes.push(Note { id, kind, text, file, status: "open".into(), at: now, done_at: None, key: scope.key.clone(),
                                    session: session.to_string(), project: scope.project.clone(), orphaned: false });
            ids.push(id);
        }
        // kept only once saved: the agent's retry would make them twice
        let before = inner.notes.clone();
        inner.prune(now, None);
        if let Err(e) = self.save(&inner) {
            inner.notes = before;
            inner.notes.truncate(len);
            inner.next = next;
            return Err(format!("The notes were not saved ({e}); none was added."));
        }
        Ok(ids)
    }

    /// The agent changes note `id` of its `scope`: status, text, kind.
    pub fn update(&self, scope: &Scope, id: u64, status: Option<&str>, text: Option<&str>, kind: Option<&str>, now: u64) -> Result<(), String> {
        if let Some(s) = status.filter(|s| *s != "open" && *s != "done") {
            return Err(format!("status {s} is not one of open, done"));
        }
        if let Some(k) = kind {
            kind_ok(k)?;
        }
        let text = text.map(clean);
        if let Some(t) = &text {
            text_ok(t, "text", NOTE_TEXT)?;
        }
        let mut inner = self.inner.lock();
        let reopen = status == Some("open") && inner.notes.iter().any(|n| n.id == id && n.status == "done");
        if reopen && inner.notes.iter().filter(|n| n.status == "open" && scope.holds(n)).count() >= OPEN_MOST {
            return Err(format!("At most {OPEN_MOST} notes may be open."));
        }
        let n = inner.notes.iter_mut().find(|n| n.id == id && scope.holds(n)).ok_or_else(|| format!("This session has no note {id}."))?;
        if let Some(s) = status {
            set_status(n, s, now);
        }
        if let Some(t) = text {
            n.text = t;
        }
        if let Some(k) = kind {
            n.kind = k.to_string();
        }
        inner.prune(now, None);
        self.save(&inner)
    }

    /// The user's action from the Page on note `id`: done, open or delete.
    pub fn user(&self, id: u64, action: &str, now: u64) -> Result<(), String> {
        let mut inner = self.inner.lock();
        let i = inner.notes.iter().position(|n| n.id == id).ok_or_else(|| format!("No note {id}."))?;
        match action {
            "delete" => { inner.notes.remove(i); }
            "done" | "open" => set_status(&mut inner.notes[i], action, now),
            other => return Err(format!("unknown action {other}: done, open or delete")),
        }
        inner.prune(now, None);
        self.save(&inner)
    }

    /// The notes of `scope`: open first, newest first.
    pub fn list(&self, scope: &Scope) -> Vec<Note> {
        let mut v: Vec<Note> = self.inner.lock().notes.iter().filter(|n| scope.holds(n)).cloned().collect();
        v.sort_by(|a, b| (a.status != "open").cmp(&(b.status != "open")).then(b.id.cmp(&a.id)));
        v
    }
}

impl Notes {
    /// How many notes of `scope` are open (as a row counts them).
    #[cfg(test)]
    pub fn open(&self, scope: &Scope) -> usize {
        self.open_counts().of(scope)
    }

    /// Open notes counted once for every row: (by project, by pane outside a project).
    pub fn open_counts(&self) -> OpenCounts {
        let mut c = OpenCounts::default();
        for n in self.inner.lock().notes.iter().filter(|n| n.status == "open") {
            match &n.project {
                Some(p) => *c.project.entry(p.clone()).or_default() += 1,
                None if !n.orphaned => *c.pane.entry(n.key.clone()).or_default() += 1,
                None => {}
            }
        }
        c
    }

    /// The panes of the servers read now are `live` (keys starting with one of `prefixes`):
    /// a gone pane's old open notes outside a project go (UNSEEN_DAYS). True when any went.
    pub fn sweep(&self, prefixes: &[String], live: &HashSet<String>, now: u64) -> Result<bool, String> {
        let mut inner = self.inner.lock();
        let len = inner.notes.len();
        // panes of servers not read now count as live
        let mut known: HashSet<String> = live.clone();
        known.extend(inner.notes.iter().filter(|n| !prefixes.iter().any(|p| n.key.starts_with(p.as_str()))).map(|n| n.key.clone()));
        inner.prune(now, Some(&known));
        if inner.notes.len() == len {
            return Ok(false);
        }
        self.save(&inner)?;
        Ok(true)
    }

    /// The scope note `id` shows in.
    pub fn scope(&self, id: u64) -> Option<Scope> {
        self.inner.lock().notes.iter().find(|n| n.id == id).map(|n| Scope { key: n.key.clone(), project: n.project.clone() })
    }

    /// A server's list was read (`prefix` its keys' start): when it is another identity than
    /// its notes were made in, their panes are not the notes' panes any more and the notes go
    /// out of sight (kept). True when anything changed (then saved).
    pub fn seen(&self, prefix: &str, identity: &str) -> Result<bool, String> {
        let mut inner = self.inner.lock();
        let was = inner.servers.insert(prefix.to_string(), identity.to_string());
        if was.as_deref() == Some(identity) {
            return Ok(false);
        }
        if was.is_some() {
            inner.notes.iter_mut().filter(|n| n.key.starts_with(prefix)).for_each(|n| n.orphaned = true);
        }
        self.save(&inner)?;
        Ok(true)
    }

    /// The notes of a project not in `projects` (it went) show with their own pane again; a
    /// gone pane's stay stored. True when any changed (then saved).
    pub fn rescope(&self, projects: &[&str]) -> Result<bool, String> {
        let mut inner = self.inner.lock();
        let mut changed = false;
        for n in inner.notes.iter_mut().filter(|n| n.project.as_deref().is_some_and(|p| !projects.contains(&p))) {
            n.project = None;
            changed = true;
        }
        if changed {
            self.save(&inner)?;
        }
        Ok(changed)
    }
}

/// `Notes::open_counts`: a scope's open notes are its project's plus its pane's own.
#[derive(Default)]
pub struct OpenCounts {
    project: HashMap<String, usize>,
    pane: HashMap<String, usize>,
}

impl OpenCounts {
    pub fn of(&self, scope: &Scope) -> usize {
        let own = self.pane.get(&scope.key).copied().unwrap_or(0);
        own + scope.project.as_ref().and_then(|p| self.project.get(p)).copied().unwrap_or(0)
    }
}

fn set_status(n: &mut Note, s: &str, now: u64) {
    n.done_at = (s == "done").then_some(n.done_at.filter(|_| n.status == "done").unwrap_or(now));
    n.status = s.to_string();
}

/// The scope of pane `key`: the project its session is in (projects.rs), else the pane.
pub fn scope_of(app: &crate::app::Shared, key: &str) -> Scope {
    let project = app.projects.lock().list().iter().find(|p| p.members.iter().any(|m| m == key)).map(|p| p.id.clone());
    Scope { key: key.to_string(), project }
}

/// Each pane's project, by key, read once (for every row of a list).
pub fn projects_of(app: &crate::app::Shared) -> HashMap<String, String> {
    let mut by = HashMap::new();
    for p in app.projects.lock().list().iter() {
        for m in &p.members {
            by.entry(m.clone()).or_insert_with(|| p.id.clone());
        }
    }
    by
}

#[cfg(test)]
#[path = "notes_tests.rs"]
mod tests;
