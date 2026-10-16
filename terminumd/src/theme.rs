// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Themes (AC-12): JSON files in `~/.terminum/themes/` (colors, ANSI palette, font), chosen
//! per session and host by the rules of settings.json, with an optional "light" variant used
//! while the system is light. A file that breaks keeps its last good version and is reported
//! once with the line of the bad value ("Theme work.json: line 7: expected a color"); a rule
//! naming a missing theme falls back to the default and says so. The two shipped themes are
//! written into the folder when missing and stand in when their files are gone.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value};

use crate::settings::{MAX_BYTES, Seg, Settings, is_name, line_of, syntax_message};

pub const LOCAL: &str = include_str!("../assets/themes/local.json");
pub const REMOTE: &str = include_str!("../assets/themes/remote.json");
const COLORS: [&str; 8] = ["bg", "fg", "cursor", "cursorText", "sel", "selfg", "bold", "link"];
const FONT_MAX: usize = 100;

/// The shipped theme of that name, parsed.
fn builtin(name: &str) -> Option<Value> {
    let text = match name {
        "local" => LOCAL,
        "remote" => REMOTE,
        _ => return None,
    };
    let mut v: Value = serde_json::from_str(text).expect("shipped themes parse");
    v.as_object_mut()?.remove("name");
    Some(v)
}

fn is_color(v: &Value) -> bool {
    v.as_str().is_some_and(|s| s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit()))
}

/// A font family name as the page puts it into CSS `local()`: no quotes, no punctuation of CSS.
fn is_font(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        (1..=FONT_MAX).contains(&s.len()) && s.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
    })
}

/// One field of a theme or of its light variant; Err names what was expected and where.
fn field(key: &str, v: &Value, text: &str, at: &[Seg]) -> Result<(), String> {
    let mut path = at.to_vec();
    let expected = match key {
        "ansi" => match v.as_array().filter(|a| a.len() == 16) {
            None => Some("16 colors"),
            Some(a) => a.iter().position(|c| !is_color(c)).map(|i| {
                path.push(Seg::Index(i));
                "a color"
            }),
        },
        "font" => (!is_font(v)).then_some("a font name"),
        "size" => (!v.as_f64().is_some_and(|n| (6.0..=72.0).contains(&n))).then_some("a size from 6 to 72"),
        "useBold" => (!v.is_boolean()).then_some("true or false"),
        _ => (!is_color(v)).then_some("a color"),
    };
    match expected {
        Some(what) => Err(format!("line {}: expected {what}", line_of(text, &path))),
        None => Ok(()),
    }
}

/// A theme file's text: the theme (missing fields from the shipped "local") and warnings about
/// unknown keys, or the reason it was refused. `file` is the file name used in messages.
pub fn validate(text: &str, file: &str) -> Result<(Value, Vec<String>), String> {
    let fail = |m: String| format!("Theme {file}: {m}");
    if text.len() > MAX_BYTES {
        return Err(fail("larger than 64 KB, not read".into()));
    }
    let v: Value = serde_json::from_str(text).map_err(|e| fail(format!("line {}: {}", e.line().max(1), syntax_message(&e))))?;
    let Value::Object(given) = v else { return Err(fail("expected an object".into())) };
    let mut out = builtin("local").and_then(|b| b.as_object().cloned()).unwrap_or_default();
    let mut warnings = Vec::new();
    for (k, v) in &given {
        match k.as_str() {
            "name" => continue,
            "light" => {
                out.insert("light".into(), light(v, text, file)?);
                continue;
            }
            k if COLORS.contains(&k) || matches!(k, "ansi" | "font" | "size" | "useBold") => {
                field(k, v, text, &[Seg::Key(k)]).map_err(fail)?;
            }
            k => {
                warnings.push(fail(format!("line {}: unknown key {k}", line_of(text, &[Seg::Key(k)]))));
                continue;
            }
        }
        out.insert(k.clone(), v.clone());
    }
    Ok((Value::Object(out), warnings))
}

/// The light variant: null, or an object of colors and a palette.
fn light(v: &Value, text: &str, file: &str) -> Result<Value, String> {
    let fail = |m: String| format!("Theme {file}: {m}");
    match v {
        Value::Null => Ok(Value::Null),
        Value::Object(o) => {
            for (k, v) in o {
                if !(COLORS.contains(&k.as_str()) || k == "ansi") {
                    return Err(fail(format!("line {}: unknown key light.{k}", line_of(text, &[Seg::Key("light"), Seg::Key(k)]))));
                }
                field(k, v, text, &[Seg::Key("light"), Seg::Key(k)]).map_err(fail)?;
            }
            Ok(v.clone())
        }
        _ => Err(fail(format!("line {}: expected an object or null", line_of(text, &[Seg::Key("light")])))),
    }
}

/// The theme the page draws with: the light fields over the theme when light is asked for and
/// the theme has them; "dark" says which variant it is.
pub fn variant(theme: &Value, name: &str, dark: bool) -> Value {
    let mut out: Map<String, Value> = theme.as_object().cloned().unwrap_or_default();
    let light = out.remove("light");
    let use_light = !dark && matches!(light, Some(Value::Object(_)));
    if use_light && let Some(Value::Object(l)) = light {
        out.extend(l);
    }
    out.insert("name".into(), Value::String(name.to_string()));
    out.insert("dark".into(), Value::Bool(!use_light));
    Value::Object(out)
}

/// What a pane wears.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub name: String,
    /// The theme for the page, with "name" and "dark".
    pub theme: Value,
    /// A rule named a theme that does not exist (the caller toasts each distinct one once).
    pub message: Option<String>,
}

/// The themes folder: the last good version of every theme file.
#[derive(Debug, Default)]
pub struct Themes {
    dir: PathBuf,
    good: BTreeMap<String, Value>,
    /// The last message told per file, so a broken file is reported once, not at every scan.
    told: BTreeMap<String, String>,
}

impl Themes {
    /// The folder's themes and what was wrong with them.
    pub fn load(dir: &Path) -> (Themes, Vec<String>) {
        let mut t = Themes { dir: dir.to_path_buf(), ..Themes::default() };
        let m = t.reload();
        (t, m)
    }

    /// Reads the folder again (the watcher calls this on a change): new messages only.
    pub fn reload(&mut self) -> Vec<String> {
        let mut seen = BTreeMap::new();
        let mut msgs = Vec::new();
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.good.clear();
                return msgs;
            }
            Err(e) => return vec![format!("Themes: {} cannot be read: {e}", self.dir.display())],
        };
        let mut files: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        files.sort();
        for path in files {
            let Some(file) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            let Some(name) = file.strip_suffix(".json").filter(|n| is_name(n)) else { continue };
            if fs::metadata(&path).is_ok_and(|md| !md.is_file()) {
                continue;
            }
            let result = match crate::settings::read_capped(&path) {
                Ok(None) => continue, // removed since it was listed
                Ok(Some(text)) => validate(&text, &file),
                Err(why) => Err(format!("Theme {file}: {why}")),
            };
            let report = match result {
                Ok((theme, warnings)) => {
                    seen.insert(name.to_string(), theme);
                    warnings.join("; ")
                }
                Err(e) => {
                    if let Some(old) = self.good.get(name) {
                        seen.insert(name.to_string(), old.clone());
                    }
                    e
                }
            };
            if report.is_empty() {
                self.told.remove(&file);
            } else if self.told.get(&file) != Some(&report) {
                self.told.insert(file, report.clone());
                msgs.extend(report.split("; ").map(str::to_string));
            }
        }
        self.told.retain(|f, _| self.dir.join(f).exists());
        self.good = seen;
        msgs
    }

    #[cfg(test)]
    pub fn names(&self) -> Vec<String> {
        self.good.keys().cloned().collect()
    }

    /// A theme by name: its file's last good version, else a shipped one of that name.
    pub fn get(&self, name: &str) -> Option<Value> {
        self.good.get(name).cloned().or_else(|| builtin(name))
    }

    /// The theme of a pane: `host` is None for this Mac. Sessions win over hosts, an exact host
    /// over `*` (remote hosts only), a host record's own "theme" after the exact rule, then
    /// the default; a missing theme falls back to the default, then to the shipped "local".
    pub fn resolve(&self, settings: &Settings, host: Option<&str>, session: &str, dark: bool) -> Resolved {
        let r = &settings.themes;
        let (wanted, why) = rule(settings, host, session);
        if let Some(t) = self.get(wanted) {
            return Resolved { name: wanted.to_string(), theme: variant(&t, wanted, dark), message: None };
        }
        let (name, t) = match self.get(&r.default) {
            Some(t) => (r.default.clone(), t),
            None => ("local".to_string(), builtin("local").expect("shipped")),
        };
        let message = Some(format!("Theme {wanted} (the {why} rule) does not exist: using {name}"));
        Resolved { theme: variant(&t, &name, dark), name, message }
    }

    /// Only the name `resolve` would give (the session list asks it for every session).
    pub fn name_of(&self, settings: &Settings, host: Option<&str>, session: &str) -> String {
        let has = |n: &str| self.good.contains_key(n) || matches!(n, "local" | "remote");
        let wanted = rule(settings, host, session).0;
        let name = if has(wanted) { wanted } else if has(&settings.themes.default) { &settings.themes.default } else { "local" };
        name.to_string()
    }
}

/// The theme a rule names for a pane, and which rule: sessions win over hosts, an exact host
/// over `*` (remote hosts only), a host record's own "theme" after the exact rule, then the default.
fn rule<'a>(settings: &'a Settings, host: Option<&'a str>, session: &'a str) -> (&'a str, Why<'a>) {
    let r = &settings.themes;
    let record = host.and_then(|h| settings.hosts.get(h)).and_then(|rec| rec.get("theme")).and_then(Value::as_str);
    if let Some(t) = r.sessions.get(session) {
        (t, Why::Session(session))
    } else if let Some((t, h)) = host.and_then(|h| r.hosts.get(h).map(|t| (t, h))) {
        (t, Why::Host(h))
    } else if let (Some(t), Some(h)) = (record, host) {
        (t, Why::Host(h))
    } else if let (Some(t), Some(_)) = (r.hosts.get("*"), host) {
        (t, Why::AnyHost)
    } else {
        (&r.default, Why::Default)
    }
}

enum Why<'a> {
    Session(&'a str),
    Host(&'a str),
    AnyHost,
    Default,
}

impl std::fmt::Display for Why<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Why::Session(s) => write!(f, "session {s}"),
            Why::Host(h) => write!(f, "host {h}"),
            Why::AnyHost => f.write_str("hosts *"),
            Why::Default => f.write_str("default"),
        }
    }
}

/// Writes the shipped themes into `dir` where their files are missing (first run).
pub fn ensure_defaults(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    for (file, text) in [("local.json", LOCAL), ("remote.json", REMOTE)] {
        let path = dir.join(file);
        if !path.exists() {
            crate::settings::write_atomic(&path, text.as_bytes())?;
        }
    }
    Ok(())
}

/// Whether macOS is in dark mode (`defaults read -g AppleInterfaceStyle` prints "Dark"; light
/// mode has no such key). False elsewhere or on failure. It runs a process: callers cache it.
pub fn system_dark() -> bool {
    Command::new("/usr/bin/defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "Dark")
}

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
