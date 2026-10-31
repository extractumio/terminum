// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A coding agent's bookmarks and reminders (AC-98): what it hands the user through
//! add_notes_for_user (a final report, a next step, a gap seen elsewhere, deferred work, a
//! question) instead of printing it into the terminal's scroll. Each is a bookmark of its pane
//! (page/bookmarks.rs) marked `by` the agent, scoped as the user's (its project, else its pane),
//! its title the note and its body the text, found again by its title (the agent's transcript
//! of the call shows it). Agents' records have limits of their own, never the user's 300. The
//! notes of 0.23 (notes.json) become such records once.

use std::path::Path;

use serde_json::Value;

use crate::page::bookmarks::{Item, MarkError, NewMark, Store};
use crate::plans::rules;

/// Agents' bookmarks and open reminders in one scope: beyond them the oldest of that kind go to
/// make room (the owner: never a refusal; told at the turn's end). Past ALL_MOST records of
/// agents the oldest orphaned ones go (no scope shows them; a live scope's only by its own).
pub const BOOKMARKS_MOST: usize = 50;
pub const REMINDERS_MOST: usize = 20;
pub const ALL_MOST: usize = 500;
/// A record's title and text, in characters after `rules::clean`.
pub const TITLE: usize = 120;
pub const TEXT: usize = 2000;
pub const RECORDS_MOST: usize = 20;
/// The file a record is about, in characters.
pub const FILE: usize = 300;
pub const KINDS: [&str; 5] = ["bug", "deferred", "followup", "question", "report"];
pub const STATUSES: [&str; 2] = ["open", "done"];
/// Said with each reminder made, so the agent knows what it costs the user.
pub const COMES_UP: &str = "it comes up at each turn end until the user resolves it";

/// Where records are made: the pane, its project, and its row's title, host and folder.
#[derive(Clone, Debug, Default)]
pub struct Place {
    pub key: String,
    pub project: Option<String>,
    pub title: String,
    pub host: Option<String>,
    pub dir: Option<String>,
}

/// A record as the agent gave it (checked by `terminum mcp`, cleaned here).
pub struct Record {
    pub title: String,
    pub text: String,
    pub remind: bool,
    pub kind: Option<String>,
    pub file: Option<String>,
}

/// What changed, for every page: a record new or changed, or one gone.
#[derive(Debug, PartialEq)]
pub enum Told {
    Mark(Box<Item>),
    Unmark(String),
    /// agents' bookmarks (false) or reminders (true) of the caller's scope went for room
    Full(bool),
}

fn agent_of(by: &str) -> String {
    if crate::tmux::hookstate::agent(by).is_some() { by.to_string() } else { "agent".into() }
}

/// Whether `i` is an agent's record shown in `p`'s scope (orphaned ones are no scope's).
/// The pane's own made before it joined its project count too, as the user sees them there.
fn in_scope(i: &Item, p: &Place) -> bool {
    let own = i.project.is_none() && i.key.as_deref() == Some(p.key.as_str());
    i.by.is_some() && i.orphaned.is_none() && match &p.project {
        Some(project) => i.project.as_ref() == Some(project) || own,
        None => own,
    }
}

fn open(i: &Item) -> bool {
    i.resolved.is_none()
}

/// Whether `i` is an agent's record kept in `p`'s own scope (its project, else its session):
/// what a limit counts and room is made from (`in_scope` also shows a pane its records from
/// before it joined its project).
fn own_scope(i: &Item, p: &Place) -> bool {
    i.by.is_some() && i.orphaned.is_none() && match &p.project {
        Some(project) => i.project.as_ref() == Some(project),
        None => i.project.is_none() && i.key.as_deref() == Some(p.key.as_str()),
    }
}

/// The oldest (the file's order is the order they were made) open records of agents that go so
/// that `adding` new ones of each kind fit in `at`'s scope, never `keep`; then orphaned ones past
/// ALL_MOST. With whether each kind (bookmarks, reminders) went in the scope.
fn room(store: &Store, at: &Place, adding: [usize; 2], keep: &str) -> (Vec<String>, [bool; 2]) {
    let (mut gone, mut full) = (Vec::new(), [false; 2]);
    for (remind, most) in [(false, BOOKMARKS_MOST), (true, REMINDERS_MOST)] {
        let here: Vec<&Item> = store.all().iter().filter(|i| own_scope(i, at) && open(i) && i.remind == remind).collect();
        let over = (here.len() + adding[usize::from(remind)]).saturating_sub(most);
        gone.extend(here.iter().filter(|i| i.id != keep).take(over).map(|i| i.id.clone()));
        full[usize::from(remind)] = over > 0;
    }
    let agents = store.all().iter().filter(|i| i.by.is_some() && !gone.contains(&i.id)).count();
    let over = (agents + adding[0] + adding[1]).saturating_sub(ALL_MOST);
    gone.extend(store.all().iter().filter(|i| i.by.is_some() && i.orphaned.is_some()).take(over).map(|i| i.id.clone()));
    (gone, full)
}

/// What room made says: to the agent (the ids that went) and to the pages.
fn made_room(text: &mut String, told: &mut Vec<Told>, gone: Vec<String>, full: [bool; 2]) {
    if gone.is_empty() {
        return;
    }
    text.push_str(&format!(" To make room ({BOOKMARKS_MOST} bookmarks and {REMINDERS_MOST} open reminders of agents a scope) these oldest records were deleted: {}; the user is told.", gone.join(", ")));
    told.extend(gone.into_iter().map(Told::Unmark));
    told.extend([false, true].into_iter().filter(|k| full[usize::from(*k)]).map(Told::Full));
}

fn cleaned(s: &str, what: &str, max: usize) -> Result<String, String> {
    let c = rules::clean(s);
    match c.chars().count() {
        0 => Err(format!("{what} is empty")),
        n if n > max => Err(format!("{what} is over {max} characters")),
        _ => Ok(c),
    }
}

/// The records kept (an open one of the same title in the scope stands for its repeat), and the
/// answer to the agent.
pub fn add(store: &mut Store, at: &Place, by: &str, records: Vec<Record>) -> Result<(String, Vec<Told>), String> {
    let mut new = Vec::new();
    let mut repeats = Vec::new();
    let same = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
    for (i, r) in records.into_iter().enumerate() {
        let title = cleaned(&r.title, &format!("notes[{i}].title"), TITLE)?;
        let text = cleaned(&r.text, &format!("notes[{i}].text"), TEXT)?;
        if let Some(old) = store.all().iter().find(|x| in_scope(x, at) && open(x) && same(&x.note, &title)) {
            repeats.push(format!("{} \"{title}\" is already kept (not added again)", old.id));
            continue;
        }
        if new.iter().any(|n: &NewMark| same(&n.note, &title)) {
            continue;
        }
        new.push(NewMark {
            key: Some(at.key.clone()), n: -1, text, note: title, title: at.title.clone(), host: at.host.clone(), remind: r.remind,
            dir: at.dir.clone(), project: at.project.clone(), by: Some(agent_of(by)), kind: r.kind, file: r.file.map(|f| rules::clean(&f)),
            ..NewMark::default()
        });
    }
    let adding = [false, true].map(|r| new.iter().filter(|n| n.remind == r).count());
    let (gone, full) = if new.is_empty() { (Vec::new(), [false; 2]) } else { room(store, at, adding, "") };
    let made = if new.is_empty() { Vec::new() } else { store.replace_all(&gone, new).map_err(|e: MarkError| e.to_string())? };
    let mut said: Vec<String> = made.iter().map(|m| if m.remind {
        format!("reminder {} \"{}\" ({COMES_UP})", m.id, m.note)
    } else {
        format!("bookmark {} \"{}\"", m.id, m.note)
    }).collect();
    said.extend(repeats);
    let mut text = format!("Kept for the user: {}.", said.join("; "));
    let mut told: Vec<Told> = Vec::new();
    made_room(&mut text, &mut told, gone, full);
    told.extend(made.into_iter().map(|m| Told::Mark(Box::new(m))));
    Ok((text, told))
}

/// The change `update_notes_for_user` asks for: status done (a reminder resolved, a bookmark
/// deleted) or open (a resolved reminder open again), a new title, text or remind; only an
/// agent's record of `at`'s scope.
pub fn update(store: &mut Store, at: &Place, args: &Value) -> Result<(String, Vec<Told>), String> {
    let id = args["id"].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or("id is required: a record's id as add_notes_for_user answered it")?;
    let Some(item) = store.get(id).filter(|i| in_scope(i, at)).cloned() else {
        return Err(format!("No record {id} of an agent here: ids are as add_notes_for_user answered them."));
    };
    let title = args["title"].as_str().map(|t| cleaned(t, "title", TITLE)).transpose()?;
    let text = args["text"].as_str().map(|t| cleaned(t, "text", TEXT)).transpose()?;
    let (remind, status) = (args["remind"].as_bool(), args["status"].as_str());
    if let Some(s) = status.filter(|s| !STATUSES.contains(s)) {
        return Err(format!("status {s} is not one of {}", STATUSES.join(", ")));
    }
    if title.is_none() && text.is_none() && remind.is_none() && status.is_none() {
        return Err("nothing to change: give status (open, done), title, text or remind".into());
    }
    // done: a reminder resolved, a bookmark (what it is after this change) deleted
    if status == Some("done") && !remind.unwrap_or(item.remind) {
        store.remove(id).map_err(|e| e.to_string())?;
        return Ok((format!("Bookmark {id} deleted."), vec![Told::Unmark(id.to_string())]));
    }
    let now = crate::auth::store::unix_now() as i64;
    let changed = store.change(id, |i| {
        if let Some(t) = title { i.note = t; }
        if let Some(t) = text { i.text = t; }
        if let Some(on) = remind.filter(|on| *on != i.remind) { (i.remind, i.due, i.resolved) = (on, false, None); }
        match status {
            Some("done") => { i.due = false; i.resolved.get_or_insert(now); }
            Some("open") => i.resolved = None,
            _ => {}
        }
    }).map_err(|e| e.to_string())?;
    let what = if changed.remind { "Reminder" } else { "Bookmark" };
    let state = if changed.resolved.is_some() { " (resolved)" } else { "" };
    let mut text = format!("{what} {id} updated{state}.");
    let mut told = vec![Told::Mark(Box::new(changed))];
    // reopened, or made the other kind: its kind's oldest make room, never itself
    let (gone, full) = room(store, at, [0, 0], id);
    if !gone.is_empty() {
        store.replace_all(&gone, Vec::new()).map_err(|e| e.to_string())?;
        made_room(&mut text, &mut told, gone, full);
    }
    Ok((text, told))
}

/// The open notes of 0.23's notes.json in `dir` made agents' records once; the file is kept
/// as notes.json.imported. Errors go to the log: the notes stay where they were.
pub fn import_notes(store: &mut Store, dir: &Path) {
    let path = dir.join("notes.json");
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let notes = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["notes"].as_array().cloned()).unwrap_or_default();
    // a note already kept (an import whose file could not be set aside) is not kept twice
    let kept = |n: &NewMark| store.all().iter().any(|i| i.by.is_some() && i.key == n.key && i.project == n.project && i.note.to_lowercase() == n.note.to_lowercase());
    let room = ALL_MOST.saturating_sub(store.all().iter().filter(|i| i.by.is_some() && open(i)).count());
    let new: Vec<NewMark> = notes.iter().filter(|n| n["status"] == "open" && n["orphaned"] != true).filter_map(|n| {
        let body = rules::clean(n["text"].as_str()?);
        let title: String = body.lines().next().unwrap_or("").chars().take(TITLE).collect();
        (!title.is_empty()).then(|| NewMark {
            key: n["key"].as_str().map(str::to_string), n: -1, note: title, text: body.chars().take(TEXT).collect(),
            project: n["project"].as_str().map(str::to_string), by: Some("agent".into()),
            kind: n["kind"].as_str().map(str::to_string), file: n["file"].as_str().map(str::to_string),
            ..NewMark::default()
        })
    }).filter(|n| !kept(n)).collect();
    if new.len() > room {
        tracing::warn!(event = "notes.import", notes = new.len(), room, "more notes than agents' records may be kept: the newest are left in notes.json.imported");
    }
    let new: Vec<NewMark> = new.into_iter().take(room).collect();
    let n = new.len();
    if n > 0 && let Err(e) = store.add_all(new) {
        tracing::warn!(event = "notes.import", error = %e, "the notes of 0.23 were not imported");
        return;
    }
    match std::fs::rename(&path, dir.join("notes.json.imported")) {
        Ok(()) => tracing::info!(event = "notes.import", notes = n, "the notes of 0.23 are agents' bookmarks now"),
        Err(e) => tracing::warn!(event = "notes.import", error = %e, "notes.json was not set aside"),
    }
}

#[cfg(test)]
#[path = "agentmarks_tests.rs"]
mod tests;
