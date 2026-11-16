// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A task's attributes beside its title and text (terminum-kanban.md AC-199): its type, shown as the word before its
//! title; a link to an outside tracker; the release it is meant for; the tasks that block it and those that follow
//! it, each with its reason. The rules are here for humans and agents alike; each change is a line of its history.

use std::collections::HashSet;

use rusqlite::Transaction;
use serde_json::{json, Value};

use super::ops::{log, rows, sql, user_name};

/// Each type and the word its title is shown with.
pub const TYPES: [(&str, &str); 4] = [("Feature", "Feat"), ("Bug", "Bug"), ("Debt", "Debt"), ("Story", "Story")];
pub const LINK_MOST: usize = 2048;
pub const TARGET_MOST: usize = 64;
pub const REASON_MOST: usize = 1000;
/// Tasks in one list at most.
pub const LINKS_MOST: usize = 20;
/// The lists of other tasks, as kept and named in the API and the tools, and as a human reads them.
pub const LISTS: [(&str, &str); 2] = [("blocked_by", "Blocked by"), ("follow_ups", "Follow-ups")];

const TYPE_FAULT: &str = "Type: one of Feature, Bug, Debt, Story";

/// A type named by its name or its word, any case; none for an empty one.
pub fn check_type(given: &str) -> Result<Option<&'static str>, String> {
    let g = given.trim();
    if g.is_empty() {
        return Ok(None);
    }
    TYPES.iter().find(|(name, word)| name.eq_ignore_ascii_case(g) || word.eq_ignore_ascii_case(g)).map(|(name, _)| Some(*name)).ok_or_else(|| TYPE_FAULT.to_string())
}

/// The type a title's first word names ("Bug:", "feat:", "Feature:"), and the title after it.
pub fn split_title(title: &str) -> (Option<&'static str>, &str) {
    if let Some((head, rest)) = title.split_once(':') && let Ok(Some(kind)) = check_type(head) && !head.trim().is_empty() && head.trim() == head {
        return (Some(kind), rest.trim_start());
    }
    (None, title)
}

/// The word a type's title is shown with.
fn word(kind: &str) -> Option<&'static str> {
    TYPES.iter().find(|(name, _)| *name == kind).map(|(_, word)| *word)
}

/// The title as shown: its type's word, then the title.
pub fn shown(kind: Option<&str>, title: &str) -> String {
    match kind.and_then(word) {
        Some(w) => format!("{w}: {title}"),
        None => title.to_string(),
    }
}

/// A title as kept and the type it sets (`None`: the type stays): a type's word before it sets that type, `given`
/// sets its own (empty: none); both naming different types are refused.
pub fn titled(title: &str, given: Option<&str>) -> Result<(String, Option<Option<&'static str>>), String> {
    let (word, bare) = split_title(title);
    let given = given.map(check_type).transpose()?;
    match (word, given) {
        (Some(w), Some(g)) if Some(w) != g => Err(format!("The title says {}: and Type says {}: give one.", self::word(w).unwrap_or(w), g.unwrap_or("none"))),
        _ => Ok((bare.to_string(), given.or(word.map(Some)))),
    }
}

/// A link to an outside tracker: an http or https address; none for an empty one.
pub fn check_link(given: &str) -> Result<Option<String>, String> {
    let g = given.trim();
    if g.is_empty() {
        return Ok(None);
    }
    let lower = g.to_ascii_lowercase();
    let web = (lower.starts_with("https://") || lower.starts_with("http://")) && g.len() > 8;
    if !web || g.len() > LINK_MOST || g.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("External link: an http or https address, at most {LINK_MOST} characters"));
    }
    Ok(Some(g.to_string()))
}

/// The release it is meant for, any words; none for an empty one.
pub fn check_target(given: &str) -> Result<Option<String>, String> {
    let cleaned = crate::termtext::clean(given, false);
    let g = cleaned.trim();
    if g.chars().count() > TARGET_MOST {
        return Err(format!("Target: at most {TARGET_MOST} characters"));
    }
    Ok((!g.is_empty()).then(|| g.to_string()))
}

/// What may change beside the title and text; absent keeps, empty takes it off.
#[derive(Default, Clone, Debug)]
pub struct Attrs {
    pub external_link: Option<String>,
    pub target: Option<String>,
    /// each list given: its whole new content, tasks by ID with their reasons
    pub lists: Vec<(&'static str, Vec<(String, String)>)>,
}

impl Attrs {
    pub fn is_empty(&self) -> bool {
        self.external_link.is_none() && self.target.is_none() && self.lists.is_empty()
    }

    /// The fields of a request's JSON: `blocked_by` and `follow_ups` as [{id, reason}] or ["ID"].
    pub fn of_json(v: &Value) -> Result<Attrs, String> {
        let mut out = Attrs { external_link: v["external_link"].as_str().map(str::to_string), target: v["target"].as_str().map(str::to_string), ..Attrs::default() };
        for (list, said) in LISTS {
            let Some(items) = v.get(list).filter(|x| !x.is_null()) else { continue };
            let items = items.as_array().ok_or_else(|| format!("{said}: a list of tasks"))?;
            let tasks = items.iter().map(|i| match i {
                Value::String(id) => Some((id.clone(), String::new())),
                Value::Object(_) => Some((i["id"].as_str()?.to_string(), i["reason"].as_str().unwrap_or_default().to_string())),
                _ => None,
            }).collect::<Option<Vec<_>>>().ok_or_else(|| format!("{said}: each a task's ID, with its reason"))?;
            out.lists.push((list, tasks));
        }
        Ok(out)
    }
}

/// Sets the type (none: off), with its line of history.
pub fn set_type(tx: &Transaction, author: &str, key: &str, kind: Option<&str>, at: i64) -> Result<(), String> {
    if !changed(tx, key, "type", kind)? {
        return Ok(());
    }
    let who = user_name(tx, author)?;
    log(tx, key, author, "edit", &kind.map_or(format!("{who} took its type off"), |k| format!("{who} set its type to {k}")), None, at).map(|_| ())
}

/// Sets what `attrs` gives on task `key`, each change a line of its history.
pub fn apply(tx: &Transaction, author: &str, key: &str, attrs: &Attrs, at: i64) -> Result<(), String> {
    if attrs.is_empty() {
        return Ok(());
    }
    let who = user_name(tx, author)?;
    if let Some(given) = &attrs.external_link {
        let link = check_link(given)?;
        if changed(tx, key, "external_link", link.as_deref())? {
            log(tx, key, author, "edit", &if link.is_some() { format!("{who} set its external link") } else { format!("{who} took its external link off") }, None, at)?;
        }
    }
    if let Some(given) = &attrs.target {
        let target = check_target(given)?;
        if changed(tx, key, "target", target.as_deref())? {
            log(tx, key, author, "edit", &target.as_ref().map_or(format!("{who} took its target off"), |t| format!("{who} set its target to {t}")), None, at)?;
        }
    }
    for (list, tasks) in &attrs.lists {
        set_list(tx, author, key, list, tasks, at)?;
    }
    Ok(())
}

/// Whether `column` (one of this module's, never a name from outside) took a new value.
fn changed(tx: &Transaction, key: &str, column: &str, value: Option<&str>) -> Result<bool, String> {
    let was: Option<String> = tx.query_row(&format!("SELECT {column} FROM tasks WHERE id = ?1"), [key], |r| r.get(0)).map_err(sql)?;
    if was.as_deref() == value {
        return Ok(false);
    }
    tx.execute(&format!("UPDATE tasks SET {column} = ?1 WHERE id = ?2"), (value, key)).map_err(sql)?;
    Ok(true)
}

/// A list's name as a human reads it.
pub fn said(list: &str) -> &'static str {
    LISTS.iter().find(|(l, _)| *l == list).map_or("", |(_, s)| s)
}

/// A list of `key` becomes `tasks`, in their order: the tasks added, taken out and given another reason said in
/// its history. A task once, of its project, never itself; Blocked by never closes a ring (A blocked by B blocked by A).
pub fn set_list(tx: &Transaction, author: &str, key: &str, list: &str, tasks: &[(String, String)], at: i64) -> Result<(), String> {
    let name = said(list);
    if name.is_empty() {
        return Err("The lists are blocked_by and follow_ups.".into());
    }
    if tasks.len() > LINKS_MOST {
        return Err(format!("{name}: at most {LINKS_MOST} tasks"));
    }
    let project: String = tx.query_row("SELECT project FROM tasks WHERE id = ?1", [key], |r| r.get(0)).map_err(sql)?;
    let was: Vec<(String, String, String)> = rows(tx, "SELECT l.other, x.slug, l.reason FROM task_links l JOIN tasks x ON x.id = l.other WHERE l.task = ?1 AND l.kind = ?2 ORDER BY l.place", (key, list), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    tx.execute("DELETE FROM task_links WHERE task = ?1 AND kind = ?2", (key, list)).map_err(sql)?;
    let who = user_name(tx, author)?;
    let mut seen = HashSet::new();
    for (place, (id, reason)) in tasks.iter().enumerate() {
        let other = found(tx, id.trim())?;
        if other.key == key {
            return Err(format!("{name}: a task cannot name itself"));
        }
        // a session sees its project's tasks only: a list never shows it another's
        if other.project != project {
            return Err(format!("{name}: {} is in another project", other.id));
        }
        if !seen.insert(other.key.clone()) {
            return Err(format!("{name}: {} is there twice", other.id));
        }
        let reason = crate::termtext::clean(reason, true).trim().to_string();
        if reason.chars().count() > REASON_MOST {
            return Err(format!("{name}: a reason is at most {REASON_MOST} characters"));
        }
        if list == "blocked_by" && blocks(tx, &other.key, key)? {
            return Err(format!("{} waits for this task already: a task cannot wait for one that waits for it", other.id));
        }
        tx.execute("INSERT INTO task_links (task, kind, other, reason, place) VALUES (?1, ?2, ?3, ?4, ?5)", (key, list, &other.key, &reason, place as i64)).map_err(sql)?;
        match was.iter().find(|(k, _, _)| *k == other.key) {
            None => { log(tx, key, author, "edit", &format!("{who} added {} to {name}", other.id), None, at)?; }
            Some((_, _, r)) if *r != reason => { log(tx, key, author, "edit", &format!("{who} changed why {} is in {name}", other.id), None, at)?; }
            Some(_) => {}
        }
    }
    for (k, slug, _) in was {
        if !seen.contains(&k) {
            log(tx, key, author, "edit", &format!("{who} took {slug} out of {name}"), None, at)?;
        }
    }
    Ok(())
}

/// A task named in a list: its key, ID and project.
struct Named {
    key: String,
    id: String,
    project: String,
}

fn found(tx: &Transaction, id: &str) -> Result<Named, String> {
    let number = super::ops::number_of(tx, id)?;
    tx.query_row("SELECT id, slug, project FROM tasks WHERE number = ?1", [number], |r| Ok(Named { key: r.get(0)?, id: r.get(1)?, project: r.get(2)? })).map_err(sql)
}

/// Task `key` moved to another project: its lists keep only that project's tasks, and other projects' lists let it
/// go (a list holds its project's tasks only); each said in the history of the task whose list it left.
pub fn moved(tx: &Transaction, author: &str, key: &str, at: i64) -> Result<(), String> {
    let gone: Vec<(String, String, String, String)> = rows(tx, "SELECT l.task, l.kind, a.slug, b.slug FROM task_links l JOIN tasks a ON a.id = l.task JOIN tasks b ON b.id = l.other
        WHERE (l.task = ?1 OR l.other = ?1) AND a.project <> b.project", [key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    let who = user_name(tx, author)?;
    for (task, list, its, other) in gone {
        tx.execute("DELETE FROM task_links WHERE task = ?1 AND kind = ?2 AND other = (SELECT id FROM tasks WHERE slug = ?3)", (&task, &list, &other)).map_err(sql)?;
        log(tx, &task, author, "edit", &format!("{who} moved {} to another project: it left {}", if task == key { &other } else { &its }, said(&list)), None, at)?;
    }
    Ok(())
}

/// Whether task `waiting` is blocked by `key`, through any chain of Blocked by.
fn blocks(tx: &Transaction, waiting: &str, key: &str) -> Result<bool, String> {
    tx.query_row("WITH RECURSIVE up(k) AS (SELECT ?1 UNION SELECT l.other FROM task_links l JOIN up ON l.task = up.k WHERE l.kind = 'blocked_by')
                  SELECT EXISTS (SELECT 1 FROM up WHERE k = ?2)", (waiting, key), |r| r.get(0)).map_err(sql)
}

/// The tasks of each list of `key`: their IDs, shown titles, statuses and reasons, in order.
pub fn lists_of(tx: &Transaction, key: &str) -> Result<Value, String> {
    let mut out: serde_json::Map<String, Value> = LISTS.iter().map(|(list, _)| (list.to_string(), json!([]))).collect();
    let all = rows(tx, "SELECT l.kind, x.slug, x.shown, x.status, l.reason FROM task_links l JOIN tasks x ON x.id = l.other WHERE l.task = ?1 ORDER BY l.kind, l.place", [key],
                   |r| Ok((r.get::<_, String>(0)?, json!({"id": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?, "status": r.get::<_, String>(3)?, "reason": r.get::<_, String>(4)?}))))?;
    for (list, item) in all {
        if let Some(Value::Array(items)) = out.get_mut(&list) {
            items.push(item);
        }
    }
    Ok(Value::Object(out))
}

/// The list's tasks with their reasons, as `set_list` takes them, by their IDs.
pub fn list_of(tx: &Transaction, key: &str, list: &str) -> Result<Vec<(String, String)>, String> {
    rows(tx, "SELECT x.slug, l.reason FROM task_links l JOIN tasks x ON x.id = l.other WHERE l.task = ?1 AND l.kind = ?2 ORDER BY l.place", (key, list), |r| Ok((r.get(0)?, r.get(1)?)))
}

/// One task into a list or out of it: `reason` given or kept; out with `remove`.
pub struct Link<'a> {
    pub list: &'a str,
    pub task: &'a str,
    pub reason: Option<&'a str>,
    pub remove: bool,
}

/// Task `key`'s list with `change` made.
pub fn link(tx: &Transaction, author: &str, key: &str, change: &Link, at: i64) -> Result<(), String> {
    let Link { list, reason, remove, .. } = *change;
    let other = found(tx, change.task)?.id;
    let mut tasks = list_of(tx, key, list)?;
    let at_place = tasks.iter().position(|(id, _)| *id == other);
    match (at_place, remove) {
        (None, true) => return Err(format!("{other} is not in {}.", said(list))),
        (Some(i), true) => { tasks.remove(i); }
        (Some(i), false) => if let Some(r) = reason { tasks[i].1 = r.to_string() },
        (None, false) => tasks.push((other, reason.unwrap_or_default().to_string())),
    }
    set_list(tx, author, key, list, &tasks, at)
}

#[cfg(test)]
#[path = "attrs_tests.rs"]
mod tests;
