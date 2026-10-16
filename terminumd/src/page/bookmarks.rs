// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Bookmarks of the page's terminal (AC-33): text a user selected, with a note, kept on the
//! Mac for every signed-in client (`web-bookmarks.json`, 0600, version 2). A bookmark is found
//! again by its text, nearest the line it was on: tmux numbers lines anew when it drops them
//! and when the scrollback is cleared. A port of iterm-enhancer's bookmarks.py: the store, the
//! checks of the page's messages and the pure search; reading the rows from tmux is the
//! stream's. The store writes synchronously: call it off the async workers.

use serde::Serialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

pub const MAX_ITEMS: usize = 300;
/// Characters of selected text kept (a longer one is cut).
pub const MAX_TEXT: usize = 2000;
/// Characters of a note (a longer one is refused).
pub const MAX_NOTE: usize = 1000;
/// Letters and digits of the text it is looked for by.
pub const PRINT: usize = 200;
/// Then by parts of it this long, in turn.
pub const PARTS: [usize; 2] = [48, 20];
/// Lines either side of a bookmark's line read first.
pub const NEAR: i64 = 1000;
/// Saves remembered by the page's ref (a save sent again is kept once).
pub const REFS: usize = 100;
pub const GONE: &str = "These lines are no longer in this session's history.";
pub const DELETED: &str = "This bookmark was deleted meanwhile.";
pub const NOT_SHOWN: &str = "This session is no longer the one shown: select the text again.";
pub const CLOSED: &str = "This session has closed.";
pub const ELSEWHERE: &str = "Its session is not the one shown.";

/// Why a bookmark was not saved; the page shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkError(pub String);

impl std::fmt::Display for MarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MarkError {}

fn err<T>(why: impl Into<String>) -> Result<T, MarkError> {
    Err(MarkError(why.into()))
}

/// A bookmark as kept in the file. `key` is the pane's key, `created` its session's creation
/// time (OQ-31; unknown for a version 1 file), `at` when it was made (Unix seconds).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Item {
    pub id: String,
    pub key: Option<String>,
    pub created: Option<i64>,
    pub n: i64,
    pub text: String,
    pub note: String,
    pub title: String,
    pub host: Option<String>,
    pub at: i64,
}

impl Item {
    /// As the page gets it: the fields and `sid`, the pane's key the page shows panes by.
    pub fn wire(&self) -> Value {
        let mut v = serde_json::to_value(self).expect("an item serializes");
        v["sid"] = json!(self.key);
        v
    }
}

/// A new bookmark's fields, stamped by terminumd but its line, text and note.
#[derive(Clone, Debug, Default)]
pub struct NewMark {
    pub key: Option<String>,
    pub created: Option<i64>,
    pub n: i64,
    pub text: String,
    pub note: String,
    pub title: String,
    pub host: Option<String>,
}

/// Where a bookmark's text is: the line where its letters and digits [start..start+size] begin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub n: i64,
    pub start: usize,
    pub size: usize,
}

// ---------- found again by its text ----------

/// Only letters and digits, lowercase: the same in a wrapped row, a re-joined line and
/// rendered Markdown. The page's marktext.js has the same rule.
pub fn loose(text: &str) -> String {
    text.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// The rows' loose text joined without breaks, with each row's first byte and character.
struct Joined {
    text: String,
    bytes: Vec<usize>,
    chars: Vec<usize>,
}

impl Joined {
    fn row_at(&self, byte: usize) -> usize {
        self.bytes.partition_point(|&s| s <= byte).max(1) - 1
    }

    fn char_at(&self, byte: usize) -> usize {
        let r = self.row_at(byte);
        self.chars[r] + self.text[self.bytes[r]..byte].chars().count()
    }

    /// The needle nearest row `k`'s start, as (character, byte): the last one before it and the
    /// first one from it are the only candidates (two searches, however often it is there).
    fn nearest(&self, needle: &str, k: usize) -> Option<(usize, usize)> {
        let (at_b, at_c) = (self.bytes.get(k).copied().unwrap_or(0), self.chars.get(k).copied().unwrap_or(0));
        let end = self.text.floor_char_boundary((at_b + needle.len() - 1).min(self.text.len()));
        let before = self.text[..end].rfind(needle).map(|b| (self.char_at(b), b));
        let after = self.text[at_b..].find(needle).map(|b| (self.char_at(at_b + b), at_b + b));
        match (before, after) {
            (None, None) => None,
            (None, Some(a)) => Some(a),
            (Some(b), Some(a)) if a.0 - at_c < at_c - b.0 => Some(a),
            (Some(b), _) => Some(b),
        }
    }
}

/// Where `text` is in rows (line, row text): the place nearest `hint` when it is there more
/// than once; None when it is not there. Rows are joined without breaks, so wrapping does not
/// matter. The whole print wins wherever it is; else the part found nearest `hint`, as a
/// selection in rendered Markdown lacks what the source has (a link's address, a table's bars).
pub fn find<S: AsRef<str>>(rows: &[(i64, S)], text: &str, hint: i64) -> Option<Found> {
    let whole: Vec<char> = loose(text).chars().take(PRINT).collect();
    if whole.is_empty() {
        return rows.iter().any(|(n, _)| *n == hint).then_some(Found { n: hint, start: 0, size: 0 });
    }
    let mut j = Joined { text: String::new(), bytes: Vec::new(), chars: Vec::new() };
    let mut count = 0;
    for (_, t) in rows {
        let s = loose(t.as_ref());
        j.bytes.push(j.text.len());
        j.chars.push(count);
        count += s.chars().count();
        j.text.push_str(&s);
    }
    let k = rows.iter().position(|(n, _)| *n >= hint).unwrap_or(rows.len().saturating_sub(1)); // the hint's row
    let at = j.chars.get(k).copied().unwrap_or(0);
    let found = |b: usize, start, size| Some(Found { n: rows[j.row_at(b)].0, start, size });
    if let Some((_, b)) = j.nearest(&whole.iter().collect::<String>(), k) {
        return found(b, 0, whole.len());
    }
    let mut best: Option<((usize, usize), usize, usize)> = None;
    for size in PARTS.into_iter().filter(|&s| s < whole.len()) {
        let mut starts: Vec<usize> = (0..=whole.len() - size).step_by(size).collect();
        if !starts.contains(&(whole.len() - size)) {
            starts.push(whole.len() - size);
        }
        for start in starts {
            let part: String = whole[start..start + size].iter().collect();
            if let Some(i) = j.nearest(&part, k)
                && best.is_none_or(|(b, _, _)| i.0.abs_diff(at) < b.0.abs_diff(at))
            {
                best = Some((i, start, size));
            }
        }
        if best.is_some() {
            break;
        }
    }
    best.and_then(|((_, b), start, size)| found(b, start, size))
}

/// Lines first..end (end exclusive) read first: NEAR either side of a bookmark's line.
pub fn near(first: i64, end: i64, around: i64) -> (i64, i64) {
    (first.max(around - NEAR), end.min(around + NEAR))
}

/// Whether the whole history is to be read after the lines near the bookmark's: its whole
/// print was not found there.
pub fn needs_whole(found: Option<&Found>, text: &str) -> bool {
    found.is_none_or(|f| f.size < loose(text).chars().take(PRINT).count())
}

/// {t: located, id, sid, n, start, size}, or {.., n: null, why: GONE} when it was not found.
pub fn located(id: &str, sid: &str, found: Option<&Found>) -> Value {
    match found {
        Some(f) => json!({"t": "located", "id": id, "sid": sid, "n": f.n, "start": f.start, "size": f.size}),
        None => json!({"t": "located", "id": id, "sid": sid, "n": null, "why": GONE}),
    }
}

/// {t: located, id, n: null, why}: DELETED, ELSEWHERE or "The history could not be read: …".
pub fn unlocated(id: &str, why: &str) -> Value {
    json!({"t": "located", "id": id, "n": null, "why": why})
}

// ---------- the page's messages ----------

/// The note of a mark message: text up to MAX_NOTE characters, trimmed.
pub fn note_of(msg: &Value) -> Result<String, MarkError> {
    let note = match msg.get("note") {
        None => "",
        Some(Value::String(s)) => s.as_str(),
        Some(_) => return err("The note must be text."),
    };
    if note.chars().count() > MAX_NOTE {
        return err(format!("The note is longer than {MAX_NOTE} characters."));
    }
    Ok(super::folds::py_strip(note).to_string())
}

/// A new bookmark's (text cut to MAX_TEXT, note, line) from {text, note, n}, checked in the
/// order the bridge checked them; the pane's checks (NOT_SHOWN, CLOSED) come first.
pub fn mark_of(msg: &Value) -> Result<(String, String, i64), MarkError> {
    let note = note_of(msg)?;
    let text = match msg.get("text") {
        Some(Value::String(t)) if !super::folds::py_strip(t).is_empty() => t.chars().take(MAX_TEXT).collect(),
        _ => return err("Select some text first."),
    };
    match msg.get("n").and_then(py_int) {
        Some(n) => Ok((text, note, n)),
        None => err("The selection's line is not known: select the text again."),
    }
}

/// Python's int() of a JSON value: an integer, a float cut, a bool, an integer's text.
fn py_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f.trunc() as i64)),
        Value::Bool(b) => Some(*b as i64),
        Value::String(s) => super::folds::py_strip(s).parse().ok(),
        _ => None,
    }
}

/// Python's str() of a JSON value.
fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "None".into(),
        Value::Bool(b) => (if *b { "True" } else { "False" }).into(),
        other => other.to_string(),
    }
}

/// `x or None`: a falsy value (null, false, 0, "", [], {}) is None.
fn truthy(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Null | Value::Bool(false) => None,
        Value::String(s) if s.is_empty() => None,
        Value::Number(n) if n.as_f64() == Some(0.0) => None,
        Value::Array(a) if a.is_empty() => None,
        Value::Object(o) if o.is_empty() => None,
        other => Some(py_str(other)),
    }
}

/// An item of the file, version 1 (sid dropped, created unknown) or 2.
fn clean_stored(it: &Value) -> Result<Item, String> {
    let field = |k: &str| it.get(k).ok_or_else(|| format!("an item has no {k:?}"));
    let int = |k: &str, v: &Value| py_int(v).ok_or_else(|| format!("{k:?} is not a number: {v}"));
    let opt = |k: &str| it.get(k).filter(|v| !v.is_null());
    Ok(Item {
        id: py_str(field("id")?),
        key: truthy(it.get("key")),
        created: opt("created").map(|v| int("created", v)).transpose()?,
        n: int("n", field("n")?)?,
        text: py_str(field("text")?).chars().take(MAX_TEXT).collect(),
        note: opt("note").map(py_str).unwrap_or_default().chars().take(MAX_NOTE).collect(),
        title: opt("title").map(py_str).unwrap_or_default(),
        host: truthy(it.get("host")),
        at: opt("at").map(|v| int("at", v)).transpose()?.unwrap_or(0),
    })
}

// ---------- the store ----------

/// The bookmarks, in the order they were made. A file that cannot be read is kept as it is:
/// nothing is saved over it, and every save says why.
pub struct Store {
    path: Option<PathBuf>,
    broken: Option<String>,
    items: Vec<Item>,
    refs: VecDeque<(String, String)>, // the page's ref of a save -> the bookmark it made
}

impl Store {
    /// A store kept in memory only.
    pub fn memory() -> Self {
        Store { path: None, broken: None, items: Vec::new(), refs: VecDeque::new() }
    }

    /// The bookmarks of `path`; none when it does not exist; broken when it cannot be read.
    pub fn load(path: PathBuf) -> Self {
        let mut s = Store { path: Some(path.clone()), ..Store::memory() };
        let read = match fs::read_to_string(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return s,
            Err(e) => Err(os_message(&e)),
            Ok(text) => Self::parse(&text),
        };
        match read {
            Ok(items) => s.items = items,
            Err(why) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                s.broken = Some(format!("{name} cannot be read ({why}); move it away to keep new bookmarks"));
                tracing::warn!("bookmarks: {}", s.broken.as_deref().unwrap_or_default());
            }
        }
        s
    }

    fn parse(text: &str) -> Result<Vec<Item>, String> {
        let data: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let list = data.get("items").and_then(Value::as_array).ok_or("no list of items")?;
        let mut items: Vec<Item> = Vec::new();
        for it in list {
            let item = clean_stored(it)?;
            match items.iter_mut().find(|x| x.id == item.id) {
                Some(same) => *same = item, // the last of an id, in the first one's place
                None => items.push(item),
            }
        }
        Ok(items)
    }

    /// Why nothing can be saved (the file cannot be read), if so.
    pub fn broken(&self) -> Option<&str> {
        self.broken.as_deref()
    }

    pub fn all(&self) -> &[Item] {
        &self.items
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    fn save(&self) -> Result<(), MarkError> {
        if let Some(b) = &self.broken {
            return err(format!("Not saved: {b}."));
        }
        let Some(path) = &self.path else { return Ok(()) };
        let data = json!({"version": 2, "items": self.items});
        write_atomic(path, data.to_string().as_bytes()).or_else(|e| err(format!("Not saved: {}.", os_message(&e))))
    }

    /// A new bookmark, saved; nothing is kept when it cannot be saved.
    pub fn add(&mut self, new: NewMark) -> Result<Item, MarkError> {
        if let Some(b) = &self.broken {
            return err(format!("Not saved: {b}."));
        }
        if self.items.len() >= MAX_ITEMS {
            return err(format!("{MAX_ITEMS} bookmarks are kept at most: delete some first (Bookmarks, below the sessions)."));
        }
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let item = Item {
            id: new_id(),
            key: new.key,
            created: new.created,
            n: new.n,
            text: new.text.chars().take(MAX_TEXT).collect(),
            note: new.note,
            title: new.title,
            host: new.host,
            at,
        };
        self.items.push(item.clone());
        if let Err(e) = self.save() {
            self.items.pop();
            return Err(e);
        }
        Ok(item)
    }

    fn change(&mut self, id: &str, edit: impl FnOnce(&mut Item)) -> Result<Item, MarkError> {
        let Some(k) = self.items.iter().position(|i| i.id == id) else { return err(DELETED) };
        let old = self.items[k].clone();
        edit(&mut self.items[k]);
        if let Err(e) = self.save() {
            self.items[k] = old;
            return Err(e);
        }
        Ok(self.items[k].clone())
    }

    /// The bookmark with its note changed, saved.
    pub fn set_note(&mut self, id: &str, note: String) -> Result<Item, MarkError> {
        self.change(id, |i| i.note = note)
    }

    /// The bookmark at the line its text was found on (looked for there first next time).
    pub fn set_n(&mut self, id: &str, n: i64) -> Result<Item, MarkError> {
        self.change(id, |i| i.n = n)
    }

    /// False when there was no such bookmark.
    pub fn remove(&mut self, id: &str) -> Result<bool, MarkError> {
        let Some(k) = self.items.iter().position(|i| i.id == id) else { return Ok(false) };
        let old = self.items.remove(k);
        if let Err(e) = self.save() {
            self.items.insert(k, old);
            return Err(e);
        }
        Ok(true)
    }

    /// The bookmark a save with this ref made (a save sent again after no answer came).
    pub fn ref_id(&self, r: &str) -> Option<&str> {
        self.refs.iter().find(|(x, _)| x == r).map(|(_, id)| id.as_str())
    }

    /// Remember the bookmark a save with this ref made; the oldest of more than REFS go.
    pub fn remember(&mut self, r: &str, id: &str) {
        if r.is_empty() || self.ref_id(r).is_some() {
            return;
        }
        self.refs.push_back((r.to_string(), id.to_string()));
        while self.refs.len() > REFS {
            self.refs.pop_front();
        }
    }
}

/// 12 random characters [A-Za-z0-9].
fn new_id() -> String {
    use rand::Rng;
    rand::rng().sample_iter(&rand::distr::Alphanumeric).take(12).map(char::from).collect()
}

/// An OS error's text without Rust's " (os error N)".
fn os_message(e: &io::Error) -> String {
    let s = e.to_string();
    match s.rfind(" (os error ") {
        Some(k) if e.raw_os_error().is_some() => s[..k].to_string(),
        _ => s,
    }
}

/// `bytes` into `path` whole or not at all: a 0600 file beside it, synced, renamed over it.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(path.parent().unwrap_or(Path::new(".")))?;
    crate::config::write_private(path, bytes)
}

#[cfg(test)]
#[path = "bookmarks_tests.rs"]
mod tests;
