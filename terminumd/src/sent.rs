// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Sent history (AC-90): the messages and commands the user sent, per project (a host and a
//! folder), in `~/.terminum/sessions/<id>/sent.jsonl` (0600; `meta.json` names the project),
//! newest last, at most KEEP of them. A coding agent's prompt comes exactly from its hooks
//! (`terminum hook`); otherwise the page says what stood at the cursor when Enter went. Kept
//! out: a line starting with a space (a shell's "not in history"), an empty one, the same text
//! again from the same pane within AGAIN, values of words named like secrets and secrets' known
//! shapes (replaced by "[redacted]").

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};

/// Entries kept per project; the file is cut back to it once it holds TRIM, or to its newest
/// BYTES / 2 once it is larger than BYTES.
pub const KEEP: usize = 2000;
const TRIM: usize = 2500;
const BYTES: usize = 4 << 20;
/// The most characters of one entry.
pub const TEXT: usize = 20000;
const AGAIN: Duration = Duration::from_secs(10);

pub struct Sent {
    root: PathBuf,
    /// per pane: the last text kept and when (one Enter seen twice is kept once)
    last: Mutex<HashMap<String, (String, Instant)>>,
    write: Mutex<()>,
}

/// A project's folder name: hex of the hash of its host and path.
pub fn id(host: &str, path: &str) -> String {
    crate::auth::crypto::hex(&crate::auth::crypto::sha256(format!("{host}\0{path}").as_bytes())[..8])
}

const SECRET_WORD: [&str; 6] = ["token", "secret", "password", "passwd", "apikey", "api_key"];
const SECRET_SHAPE: [&str; 9] = ["sk-", "ghp_", "gho_", "github_pat_", "glpat-", "xoxb-", "xoxp-", "AKIA", "eyJ"];

/// `text` with secrets' values left out: `NAME=value`, `NAME: value`, `--name value` for a
/// name like a secret's, `Bearer value`, words of a known secret's shape and private key blocks.
pub fn redact(text: &str) -> String {
    let text = pem_out(text);
    let secretish = |w: &str| { let w = w.trim_start_matches('-').to_ascii_lowercase(); SECRET_WORD.iter().any(|s| w.contains(s)) };
    let mut out = String::with_capacity(text.len());
    let mut hide_next = false;
    for piece in text.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end_matches(char::is_whitespace);
        let gap = &piece[word.len()..];
        let bare = word.trim_matches(|c: char| "'\"`(),;".contains(c));
        if word.is_empty() {
            out.push_str(piece);
            continue;
        }
        let hidden = if hide_next && !bare.starts_with('-') {
            hide_next = false;
            "[redacted]".to_string()
        } else if let Some((k, _)) = word.split_once('=').filter(|(k, v)| !v.is_empty() && secretish(k)) {
            format!("{k}=[redacted]")
        } else if SECRET_SHAPE.iter().any(|s| bare.starts_with(s) && bare.len() >= s.len() + 12) {
            "[redacted]".to_string()
        } else {
            let key = bare.trim_end_matches(':').trim_matches('"');
            hide_next = (word.starts_with("--") && !word.contains('=') && secretish(word)) || (bare.ends_with(':') && secretish(key))
                || bare.eq_ignore_ascii_case("bearer");
            word.to_string()
        };
        out.push_str(&hidden);
        out.push_str(gap);
    }
    out
}

/// `text` with every "-----BEGIN … PRIVATE KEY-----" block replaced by "[redacted]".
fn pem_out(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("-----BEGIN ") {
        let Some(end) = rest[i..].find("PRIVATE KEY-----").map(|e| i + e) else { break };
        let close = rest[end + 16..].find("PRIVATE KEY-----").map_or(rest.len(), |e| end + 16 + e + 16);
        out.push_str(&rest[..i]);
        out.push_str("[redacted]");
        rest = &rest[close..];
    }
    out.push_str(rest);
    out
}

impl Sent {
    pub fn new(root: PathBuf) -> Sent {
        Sent { root, last: Mutex::default(), write: Mutex::default() }
    }

    fn file(&self, host: &str, path: &str) -> PathBuf {
        self.root.join(id(host, path)).join("sent.jsonl")
    }

    /// Keeps `text` sent in pane `key` of project (`host`, `path`); Ok(false) when it is left
    /// out (see the module). `info`: {agent, title, via} kept with it. Blocking.
    pub fn add(&self, host: &str, path: &str, key: &str, text: &str, info: Value) -> Result<bool, String> {
        let text: String = text.trim_end().chars().take(TEXT).collect();
        if text.trim().is_empty() || text.starts_with(' ') || path.is_empty() {
            return Ok(false);
        }
        let text = redact(&text);
        {
            let mut last = self.last.lock();
            if last.get(key).is_some_and(|(t, at)| *t == text && at.elapsed() < AGAIN) {
                return Ok(false);
            }
            if last.len() > 1000 {
                last.clear();
            }
            last.insert(key.to_string(), (text.clone(), Instant::now()));
        }
        let mut entry = json!({"at": crate::auth::store::unix_now(), "text": text});
        for k in ["agent", "title", "via"] {
            if let Some(v) = info[k].as_str().filter(|v| !v.is_empty()) {
                entry[k] = json!(v);
            }
        }
        let _w = self.write.lock();
        let file = self.file(host, path);
        let dir = file.parent().expect("a folder");
        if !dir.is_dir() {
            crate::config::private_dir(&self.root);
            crate::config::private_dir(dir);
            let meta = serde_json::to_vec(&json!({"host": host, "path": path})).expect("json");
            crate::config::write_private(&dir.join("meta.json"), &meta).map_err(|e| e.to_string())?;
        }
        append(&file, &(entry.to_string() + "\n")).map_err(|e| format!("{}: {e}", file.display()))?;
        trim(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(true)
    }

    /// The `n`-th newest entry of a project (0: the last) and how many there are. Blocking.
    pub fn get(&self, host: &str, path: &str, n: usize) -> (usize, Option<Value>) {
        let text = std::fs::read_to_string(self.file(host, path)).unwrap_or_default();
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let item = lines.len().checked_sub(n + 1).and_then(|i| serde_json::from_str(lines[i]).ok());
        (lines.len(), item)
    }
}

fn append(file: &Path, line: &str) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(file)?;
    f.write_all(line.as_bytes())
}

/// Cuts the file back to its KEEP newest entries once it holds TRIM, and to its newest half of
/// BYTES once larger (a page reads it whole for each one it shows).
fn trim(file: &Path) -> std::io::Result<()> {
    let text = std::fs::read_to_string(file)?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < TRIM && text.len() <= BYTES {
        return Ok(());
    }
    let mut first = lines.len().saturating_sub(KEEP);
    let mut size: usize = lines[first..].iter().map(|l| l.len() + 1).sum();
    while text.len() > BYTES && size > BYTES / 2 && first + 1 < lines.len() {
        size -= lines[first].len() + 1;
        first += 1;
    }
    let kept = lines[first..].join("\n") + "\n";
    crate::config::write_private(file, kept.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (Sent, PathBuf) {
        let dir = std::env::temp_dir().join(format!("tm-sent-{}-{}", std::process::id(), rand::random::<u32>()));
        (Sent::new(dir.clone()), dir)
    }

    #[test]
    fn kept_per_project_newest_last_and_read_back_by_age() {
        let (s, dir) = store();
        let info = json!({"agent": "claude", "via": "hook"});
        assert!(s.add("local", "/Users/alex/api", "k1", "fix the tests\nand the lint", info.clone()).unwrap());
        assert!(s.add("local", "/Users/alex/api", "k1", "ls -la", json!({})).unwrap());
        assert!(s.add("devbox", "/Users/alex/api", "k2", "make", json!({})).unwrap());
        let (n, last) = s.get("local", "/Users/alex/api", 0);
        assert_eq!((n, last.unwrap()["text"].as_str()), (2, Some("ls -la")));
        let first = s.get("local", "/Users/alex/api", 1).1.unwrap();
        assert_eq!((first["text"].as_str(), first["agent"].as_str()), (Some("fix the tests\nand the lint"), Some("claude")));
        assert!(s.get("local", "/Users/alex/api", 2).1.is_none());
        assert_eq!(s.get("devbox", "/Users/alex/api", 0).0, 1, "another host is another project");
        let f = s.file("local", "/Users/alex/api");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        let meta: Value = serde_json::from_slice(&std::fs::read(f.with_file_name("meta.json")).unwrap()).unwrap();
        assert_eq!(meta, json!({"host": "local", "path": "/Users/alex/api"}));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn left_out_a_space_an_empty_line_the_same_again_and_secrets() {
        let (s, dir) = store();
        assert!(!s.add("local", "/p", "k", " secret command", json!({})).unwrap());
        assert!(!s.add("local", "/p", "k", "   ", json!({})).unwrap());
        assert!(s.add("local", "/p", "k", "make test", json!({})).unwrap());
        assert!(!s.add("local", "/p", "k", "make test", json!({})).unwrap(), "one Enter told by the hook and the page");
        assert!(s.add("local", "/p", "k2", "make test", json!({})).unwrap(), "another pane");
        assert_eq!(redact("export GITHUB_TOKEN=ghp_abc def"), "export GITHUB_TOKEN=[redacted] def");
        assert_eq!(redact("mysql --password hunter2 -u root"), "mysql --password [redacted] -u root");
        let key = format!("sk-{}", "ant-api03-abcdefghijklmnop");          // no secret's shape in the source
        assert_eq!(redact(&format!("curl -H 'x' {key}")), "curl -H 'x' [redacted]");
        assert_eq!(redact("ask about the token flow"), "ask about the token flow", "a word alone is no secret");
        assert_eq!(redact("git commit --author alex"), "git commit --author alex");
        assert_eq!(redact(&format!("note\n{key}\tok")), "note\n[redacted]\tok", "any whitespace, kept as it was");
        assert_eq!(redact(r#"{"api_key": "abc", "x": 1}"#), r#"{"api_key": [redacted] "x": 1}"#);
        assert_eq!(redact("curl -H 'Authorization: Bearer abc.def'"), "curl -H 'Authorization: Bearer [redacted]");
        let pem = ["-----BEGIN OPENSSH ", "PRIVATE KEY-----\nb3Blbn\n-----END OPENSSH ", "PRIVATE KEY-----"].concat();
        assert_eq!(redact(&format!("key:\n{pem}\ndone")), "key:\n[redacted]\ndone");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cut_back_to_its_newest() {
        let (s, dir) = store();
        let f = s.file("local", "/p");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        let many: String = (0..TRIM - 1).map(|i| format!("{{\"at\":1,\"text\":\"c{i}\"}}\n")).collect();
        std::fs::write(&f, many).unwrap();
        s.add("local", "/p", "k", "last", json!({})).unwrap();
        let (n, last) = s.get("local", "/p", 0);
        assert_eq!((n, last.unwrap()["text"].as_str()), (KEEP, Some("last")));
        assert_eq!(s.get("local", "/p", KEEP - 1).1.unwrap()["text"], format!("c{}", TRIM - KEEP));
        let _ = std::fs::remove_dir_all(dir);
    }
}
