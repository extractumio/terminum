// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! `~/.terminum/settings.json` (AC-19, AC-12, AC-13, AC-15; hosts records of AC-87): every key
//! optional with its documented default. A wrong value keeps the default for that key and says
//! so in a message the page shows as a toast; an unknown key is reported once; a file that does
//! not parse keeps every default. `save_hosts` rewrites only the `hosts` value in the text, so
//! the user's order, spacing and other keys stay as written. Also the JSON locator the theme
//! and hot keys files use to name the line of a bad value.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{Map, Value, json};

/// Theme, settings and hot keys files larger than this are refused (spec: limits).
pub const MAX_BYTES: usize = 64 * 1024;
const FILE: &str = "settings.json";

/// The sizing policy (AC-13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Latest,
    Largest,
    Manual,
}

impl Size {
    pub fn as_str(self) -> &'static str {
        match self {
            Size::Latest => "latest",
            Size::Largest => "largest",
            Size::Manual => "manual",
        }
    }
}

/// Theme rules (AC-12): by session name, by host name (`*`: any remote host), else `default`.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeRules {
    pub default: String,
    pub hosts: BTreeMap<String, String>,
    pub sessions: BTreeMap<String, String>,
}

/// What a browser with no choice of its own starts with (AC-19).
#[derive(Clone, Debug, PartialEq)]
pub struct PageSettings {
    pub text_size: u32,
    pub lines: String,
    pub screen_mode: String,
    pub widgets: bool,
    pub folds: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub port: u16,
    pub tmux_socket: String,
    pub tmux_history: u32,
    pub size: Size,
    pub restore: bool,
    pub themes: ThemeRules,
    pub page: PageSettings,
    /// Host records as written (`ssh`, `files`, `helper`, `theme`): the hosts module judges them.
    pub hosts: Map<String, Value>,
    /// The messages of the last parse, for the page (`page_json`).
    pub messages: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            port: 47900,
            tmux_socket: "terminum".into(),
            tmux_history: 12000,
            size: Size::Latest,
            restore: true,
            themes: ThemeRules {
                default: "local".into(),
                hosts: BTreeMap::from([("*".to_string(), "remote".to_string())]),
                sessions: BTreeMap::new(),
            },
            page: PageSettings { text_size: 13, lines: "wrap".into(), screen_mode: "auto".into(), widgets: true, folds: true },
            hosts: Map::new(),
            messages: Vec::new(),
        }
    }
}

impl Settings {
    /// The `settings` message's body: the page defaults and what was wrong with the file.
    pub fn page_json(&self) -> Value {
        let p = &self.page;
        json!({"textSize": p.text_size, "lines": p.lines, "screenMode": p.screen_mode,
               "widgets": p.widgets, "folds": p.folds, "messages": self.messages})
    }
}

/// A file name or a socket name: letters, digits, `-`, `_`, `.`, not starting with a dot.
pub fn is_name(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && !s.starts_with('.')
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// serde_json's complaint in the words of the spec ("expected a value"), without its position.
pub fn syntax_message(e: &serde_json::Error) -> String {
    let full = e.to_string();
    let msg = full.rsplit_once(" at line ").map_or(full.as_str(), |(m, _)| m);
    if msg == "expected value" || msg == "key must be a string" {
        "expected a value".into()
    } else if msg.starts_with("EOF while parsing") {
        "the file ends too early".into()
    } else {
        msg.to_string()
    }
}

/// Messages without repeats, each prefixed with the file name.
#[derive(Default)]
struct Msgs(Vec<String>);

impl Msgs {
    fn bad(&mut self, what: impl AsRef<str>) {
        let m = format!("{FILE}: {}", what.as_ref());
        if !self.0.contains(&m) {
            self.0.push(m);
        }
    }
}

/// The members of an object setting; anything else is reported and yields none.
fn members<'a>(v: &'a Value, name: &str, m: &mut Msgs) -> Vec<(&'a str, &'a Value)> {
    match v {
        Value::Object(o) => o.iter().map(|(k, v)| (k.as_str(), v)).collect(),
        _ => {
            m.bad(format!("{name} must be an object"));
            Vec::new()
        }
    }
}

fn word(v: &Value, allowed: &[&str]) -> Option<String> {
    v.as_str().filter(|s| allowed.contains(s)).map(str::to_string)
}

fn boolean(v: &Value, name: &str, slot: &mut bool, m: &mut Msgs) {
    match v.as_bool() {
        Some(b) => *slot = b,
        None => m.bad(format!("{name} must be true or false")),
    }
}

fn rules(v: &Value, name: &str, into: &mut BTreeMap<String, String>, m: &mut Msgs) {
    for (k, t) in members(v, name, m) {
        match t.as_str().filter(|t| is_name(t)) {
            Some(t) => {
                into.insert(k.to_string(), t.to_string());
            }
            None => m.bad(format!("{name}.{k} must be a theme name")),
        }
    }
}

/// Settings from the file's text, and what was wrong with it.
pub fn parse(text: &str) -> (Settings, Vec<String>) {
    let mut s = Settings::default();
    let mut m = Msgs::default();
    if text.len() > MAX_BYTES {
        m.bad("larger than 64 KB, not read");
    } else if !text.trim().is_empty() {
        match serde_json::from_str::<Value>(text) {
            Err(e) => m.bad(format!("line {}: {}", e.line().max(1), syntax_message(&e))),
            Ok(Value::Object(top)) => {
                for (k, v) in &top {
                    apply(&mut s, k, v, &mut m);
                }
            }
            Ok(_) => m.bad("expected an object"),
        }
    }
    s.messages = m.0.clone();
    (s, m.0)
}

fn apply(s: &mut Settings, key: &str, v: &Value, m: &mut Msgs) {
    match key {
        "port" => match v.as_u64() {
            Some(p @ 1..=65535) => s.port = p as u16,
            _ => m.bad("port must be 1–65535"),
        },
        "tmux" => {
            for (k, v) in members(v, "tmux", m) {
                match k {
                    "socket" => match v.as_str().filter(|n| is_name(n)) {
                        Some(n) => s.tmux_socket = n.to_string(),
                        None => m.bad("tmux.socket must be a name of letters, digits, -, _ or ."),
                    },
                    "history" => match v.as_u64() {
                        Some(n @ 1000..=1_000_000) => s.tmux_history = n as u32,
                        _ => m.bad("tmux.history must be 1000–1000000"),
                    },
                    _ => m.bad(format!("unknown key tmux.{k}")),
                }
            }
        }
        "size" => match v.as_str() {
            Some("latest") => s.size = Size::Latest,
            Some("largest") => s.size = Size::Largest,
            Some("manual") => s.size = Size::Manual,
            _ => m.bad("size must be latest, largest or manual"),
        },
        "restore" => boolean(v, "restore", &mut s.restore, m),
        "themes" => {
            for (k, v) in members(v, "themes", m) {
                match k {
                    "default" => match v.as_str().filter(|t| is_name(t)) {
                        Some(t) => s.themes.default = t.to_string(),
                        None => m.bad("themes.default must be a theme name"),
                    },
                    "hosts" => rules(v, "themes.hosts", &mut s.themes.hosts, m),
                    "sessions" => rules(v, "themes.sessions", &mut s.themes.sessions, m),
                    _ => m.bad(format!("unknown key themes.{k}")),
                }
            }
        }
        "page" => {
            for (k, v) in members(v, "page", m) {
                let p = &mut s.page;
                match k {
                    "textSize" => match v.as_f64().filter(|n| n.fract() == 0.0 && (8.0..=40.0).contains(n)) {
                        Some(n) => p.text_size = n as u32,
                        None => m.bad("page.textSize must be 8–40"),
                    },
                    "lines" => match word(v, &["wrap", "grid", "fit"]) {
                        Some(w) => p.lines = w,
                        None => m.bad("page.lines must be wrap, grid or fit"),
                    },
                    "screenMode" => match word(v, &["auto", "always", "never"]) {
                        Some(w) => p.screen_mode = w,
                        None => m.bad("page.screenMode must be auto, always or never"),
                    },
                    "widgets" => boolean(v, "page.widgets", &mut p.widgets, m),
                    "folds" => boolean(v, "page.folds", &mut p.folds, m),
                    _ => m.bad(format!("unknown key page.{k}")),
                }
            }
        }
        "hosts" => match v {
            Value::Object(o) => s.hosts = o.clone(),
            Value::Null => {}
            _ => m.bad("hosts must be an object"),
        },
        _ => m.bad(format!("unknown key {key}")),
    }
}

/// A user's file of terminum (settings, hot keys, a theme): None when it is missing, why not
/// when it is larger than 64 KB or cannot be read.
pub fn read_capped(path: &Path) -> Result<Option<String>, String> {
    match fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(md) if md.len() > MAX_BYTES as u64 => return Err("larger than 64 KB, not read".into()),
        _ => {}
    }
    fs::read_to_string(path).map(Some).map_err(|e| format!("cannot be read: {e}"))
}

/// The file at `path`: defaults when it is missing; a file that cannot be read says so.
pub fn load(path: &Path) -> (Settings, Vec<String>) {
    match read_capped(path) {
        Ok(None) => (Settings::default(), Vec::new()),
        Ok(Some(text)) => parse(&text),
        Err(why) => {
            let s = Settings { messages: vec![format!("{FILE}: {why}")], ..Settings::default() };
            let m = s.messages.clone();
            (s, m)
        }
    }
}

/// Changes the `hosts` object with `f` and writes the file atomically, replacing only the
/// text of that value (or adding the key): the rest of the file stays byte for byte. A file
/// that does not parse is left alone: its line is reported for the user to fix first.
pub fn save_hosts(path: &Path, f: impl FnOnce(&mut Map<String, Value>)) -> Result<(), String> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{FILE}: cannot be read: {e}")),
    };
    if text.len() > MAX_BYTES {
        return Err(format!("{FILE}: larger than 64 KB, not read"));
    }
    let top: Value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&text)
            .map_err(|e| format!("{FILE}: line {}: {}; fix it first", e.line().max(1), syntax_message(&e)))?
    };
    let Some(obj) = top.as_object() else { return Err(format!("{FILE}: expected an object")) };
    let mut hosts = match obj.get("hosts") {
        Some(Value::Object(h)) => h.clone(),
        None | Some(Value::Null) => Map::new(),
        Some(_) => return Err(format!("{FILE}: hosts must be an object")),
    };
    f(&mut hosts);
    let rendered = serde_json::to_string_pretty(&Value::Object(hosts)).map_err(|e| e.to_string())?.replace('\n', "\n  ");
    let out = if text.trim().is_empty() {
        format!("{{\n  \"hosts\": {rendered}\n}}\n")
    } else if let Some(sp) = locate(&text, &[Seg::Key("hosts")]) {
        format!("{}{rendered}{}", &text[..sp.start], &text[sp.end..])
    } else {
        let end = text.rfind('}').ok_or_else(|| format!("{FILE}: expected an object"))?;
        let head = text[..end].trim_end();
        let comma = if obj.is_empty() { "" } else { "," };
        format!("{head}{comma}\n  \"hosts\": {rendered}\n{}", &text[end..])
    };
    write_atomic(path, out.as_bytes()).map_err(|e| format!("{FILE}: cannot be written: {e}"))
}

/// Write beside the file, keep its permissions, then rename over it.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    crate::config::write_replacing(path, data, 0o666, fs::metadata(path).ok().map(|m| m.permissions()))
}

/// A step of a path into a JSON document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg<'a> {
    Key(&'a str),
    Index(usize),
}

/// Where a value sits in the text: byte range and 1-based line of its first byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
}

/// The value at `path` in a JSON text that parses (the first of duplicate keys), or None.
pub fn locate(text: &str, path: &[Seg]) -> Option<Span> {
    let mut sc = Scan { t: text, i: 0 };
    let (start, end) = sc.find(path)?;
    Some(Span { start, end, line: 1 + text.as_bytes()[..start].iter().filter(|&&b| b == b'\n').count() })
}

/// The line of the value at `path`, or 1 when it cannot be found.
pub fn line_of(text: &str, path: &[Seg]) -> usize {
    locate(text, path).map_or(1, |s| s.line)
}

struct Scan<'t> {
    t: &'t str,
    i: usize,
}

impl Scan<'_> {
    fn peek(&self) -> Option<u8> {
        self.t.as_bytes().get(self.i).copied()
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.i += 1;
        }
    }

    fn eat(&mut self, b: u8) -> Option<()> {
        self.ws();
        (self.peek()? == b).then(|| self.i += 1)
    }

    /// A string token starting here, decoded.
    fn string(&mut self) -> Option<String> {
        self.ws();
        let start = self.i;
        if self.peek()? != b'"' {
            return None;
        }
        self.i += 1;
        loop {
            match self.peek()? {
                b'\\' => self.i += 2,
                b'"' => break,
                _ => self.i += 1,
            }
        }
        self.i += 1;
        serde_json::from_str(&self.t[start..self.i]).ok()
    }

    /// Skips one value; its byte range.
    fn skip(&mut self) -> Option<(usize, usize)> {
        self.ws();
        let start = self.i;
        match self.peek()? {
            b'"' => {
                self.string()?;
            }
            b'{' | b'[' => {
                let mut depth = 0usize;
                loop {
                    match self.peek()? {
                        b'"' => {
                            self.string()?;
                            continue;
                        }
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => depth -= 1,
                        _ => {}
                    }
                    self.i += 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            _ => {
                while !matches!(self.peek(), None | Some(b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n')) {
                    self.i += 1;
                }
            }
        }
        Some((start, self.i))
    }

    fn find(&mut self, path: &[Seg]) -> Option<(usize, usize)> {
        let Some((seg, rest)) = path.split_first() else { return self.skip() };
        self.ws();
        match (seg, self.peek()?) {
            (Seg::Key(want), b'{') => {
                self.i += 1;
                loop {
                    self.ws();
                    if self.peek()? == b'}' {
                        return None;
                    }
                    let key = self.string()?;
                    self.eat(b':')?;
                    if key == *want {
                        return self.find(rest);
                    }
                    self.skip()?;
                    self.ws();
                    if self.peek()? == b',' {
                        self.i += 1;
                    }
                }
            }
            (Seg::Index(want), b'[') => {
                self.i += 1;
                for n in 0.. {
                    self.ws();
                    if self.peek()? == b']' {
                        return None;
                    }
                    if n == *want {
                        return self.find(rest);
                    }
                    self.skip()?;
                    self.ws();
                    if self.peek()? == b',' {
                        self.i += 1;
                    }
                }
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
