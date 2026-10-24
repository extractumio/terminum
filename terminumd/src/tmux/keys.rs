// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Typed bytes to `send-keys` commands (AC-03; AS-05, AS-10; spec §9 "The control client",
//! Typing). The page sends keys as xterm encodes them (`web/static/keys.js`, `input.js`); a
//! pane gets them from tmux either as literal text (`send-keys -l`) or by tmux's key names,
//! which tmux encodes for the pane's mode (application cursor keys in vim and mc). No control
//! character ever reaches a command line: each C0 byte and DEL becomes its key name.
//!
//! Decisions: CR is `Enter`, LF is `C-j` (the page sends Enter as CR and turns pasted newlines
//! into CR, so an LF is a typed ⌃J, which Claude Code and readline take as their own key);
//! Shift+Enter and Ctrl+Enter (CSI u, `ESC [ 13 ; 2 u`) go by name (`S-Enter`, `C-Enter`):
//! with `extended-keys on` tmux sends CSI u to a program that asked for extended keys (Claude
//! Code) and Enter to one that did not (a shell), AS-10; to a server with it off, as bytes; C1 characters in text are dropped
//! (no keyboard types them, and a command line carries no control characters).

use super::args::{quote, word};

/// One run of typed input, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// Text for `send-keys -l`.
    Text(String),
    /// tmux key names for one `send-keys`.
    Keys(Vec<String>),
}

const MODS: [&str; 7] = ["S-", "M-", "M-S-", "C-", "C-S-", "C-M-", "C-M-S-"];
/// Keys in one `send-keys` at most. The terminal hands a program its input in parts of its
/// buffer (1 KB on macOS), which may end inside a key's bytes; a program busy drawing then
/// takes a lone ESC for Escape and the rest for text ("[D" in a coding agent's input), so a
/// longer run goes in parts, each once the program read the last (typing.rs).
pub const KEY_PART: usize = 100;

/// Keys (not text) in `parts`.
pub fn key_count(parts: &[Part]) -> usize {
    parts.iter().map(|p| if let Part::Keys(k) = p { k.len() } else { 0 }).sum()
}

/// The bracketed-paste markers the page wraps a paste in when the menu's switch is on.
pub const PASTE_START: &str = "\x1b[200~";
pub const PASTE_END: &str = "\x1b[201~";

/// tmux's names for the special keys the page can send, besides single characters.
const NAMES: &[&str] = &["Up", "Down", "Left", "Right", "BSpace", "BTab", "DC", "End", "Enter",
    "Escape", "Home", "IC", "NPage", "PageDown", "PgDn", "PPage", "PageUp", "PgUp", "Space", "Tab",
    "Delete", "Insert"];

/// The key of a final byte of `CSI A`…`CSI S` or `SS3 A`…`SS3 S`.
fn base(c: u8) -> Option<&'static str> {
    Some(match c {
        b'A' => "Up", b'B' => "Down", b'C' => "Right", b'D' => "Left", b'H' => "Home", b'F' => "End",
        b'P' => "F1", b'Q' => "F2", b'R' => "F3", b'S' => "F4",
        _ => return None,
    })
}

/// The key of `CSI <n> ~`.
fn tilde(n: u32) -> Option<&'static str> {
    Some(match n {
        2 => "IC", 3 => "DC", 5 => "PPage", 6 => "NPage", 15 => "F5", 17 => "F6", 18 => "F7",
        19 => "F8", 20 => "F9", 21 => "F10", 23 => "F11", 24 => "F12",
        _ => return None,
    })
}

/// The modifier prefix of xterm's parameter 2..=8.
fn modifier(m: u32) -> Option<&'static str> {
    (2..=8).contains(&m).then(|| MODS[(m - 2) as usize])
}

/// tmux's name for a C0 byte or DEL.
fn control_name(b: u8) -> String {
    match b {
        b'\r' => "Enter".into(),
        b'\n' => "C-j".into(),
        b'\t' => "Tab".into(),
        0x7f => "BSpace".into(),
        0x1b => "Escape".into(),
        0x00 => "C-Space".into(),
        0x01..=0x1a => format!("C-{}", (b'a' + b - 1) as char),
        0x1c => "C-\\".into(),
        0x1d => "C-]".into(),
        0x1e => "C-^".into(),
        _ => "C-_".into(),
    }
}

/// Digits at the start of `s`: their value and count (at most `max` digits, at least one).
fn number(s: &[u8], max: usize) -> Option<(u32, usize)> {
    let n = s.iter().take(max + 1).take_while(|b| b.is_ascii_digit()).count();
    if n == 0 || n > max {
        return None;
    }
    Some((std::str::from_utf8(&s[..n]).ok()?.parse().ok()?, n))
}

/// A recognized sequence after `ESC [`: the part and the bytes it took after the bracket.
fn csi(s: &[u8]) -> Option<(Part, usize)> {
    let key = |name: String| Part::Keys(vec![name]);
    match *s.first()? {
        b'Z' => return Some((key("BTab".into()), 1)),
        c @ (b'A' | b'B' | b'C' | b'D' | b'H' | b'F') => return Some((key(base(c)?.into()), 1)),
        _ => {}
    }
    let (n, len) = number(s, 7)?;
    let mut i = len;
    let mut m = None;
    if s.get(i) == Some(&b';') {
        let (v, l) = number(&s[i + 1..], 3)?;
        m = Some(v);
        i += 1 + l;
    }
    let fin = *s.get(i)?;
    let part = match fin {
        b'~' => key(format!("{}{}", m.map_or(Some(""), modifier)?, tilde(n)?)),
        b'u' => {
            let name = match n { 13 => "Enter", 9 => "Tab", 27 => "Escape", 127 => "BSpace", 32 => "Space", _ => return None };
            key(format!("{}{name}", m.map_or(Some(""), modifier)?))
        }
        c if n == 1 && base(c).is_some() => key(format!("{}{}", modifier(m?)?, base(c)?)),
        _ => return None,
    };
    Some((part, i + 1))
}

/// The part an escape sequence at the start of `s` (which starts with ESC) stands for, and its
/// length in bytes.
fn escape(s: &str) -> (Part, usize) {
    let b = s.as_bytes();
    let alone = (Part::Keys(vec!["Escape".into()]), 1);
    match b.get(1) {
        None | Some(0x1b) => alone,
        Some(b'[') => match csi(&b[2..]) {
            Some((p, n)) => (p, n + 2),
            None => alone,                         // unknown: Escape, then the rest as text
        },
        Some(b'O') => match b.get(2).and_then(|&c| base(c)) {
            Some(name) => (Part::Keys(vec![name.into()]), 3),
            None => alone,
        },
        Some(&c) if c < 0x20 || c == 0x7f => (Part::Keys(vec![format!("M-{}", control_name(c))]), 2),
        Some(_) => {
            let ch = s[1..].chars().next().unwrap_or(' ');
            if ('\u{80}'..='\u{9f}').contains(&ch) {
                return alone;
            }
            let name = if ch == ' ' { "M-Space".to_string() } else { format!("M-{ch}") };
            (Part::Keys(vec![name]), 1 + ch.len_utf8())   // Option as Meta
        }
    }
}

/// Appends `p`, merging it with the last part when both are text or both keys.
fn push(out: &mut Vec<Part>, p: Part) {
    match (out.last_mut(), p) {
        (Some(Part::Text(a)), Part::Text(b)) => a.push_str(&b),
        (Some(Part::Keys(a)), Part::Keys(b)) => a.extend(b),
        (_, p) => out.push(p),
    }
}

/// `data` in order as runs of text and of key names.
pub fn parts(data: &str) -> Vec<Part> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let b = data.as_bytes()[i];
        if b == 0x1b {
            let (p, n) = escape(&data[i..]);
            push(&mut out, p);
            i += n;
        } else if b < 0x20 || b == 0x7f {
            push(&mut out, Part::Keys(vec![control_name(b)]));
            i += 1;
        } else {
            let ch = data[i..].chars().next().unwrap_or(' ');
            if !('\u{80}'..='\u{9f}').contains(&ch) {
                push(&mut out, Part::Text(ch.to_string()));
            }
            i += ch.len_utf8();
        }
    }
    out
}

/// The CSI u bytes of a key tmux encodes as CSI u only with `extended-keys` on.
fn csi_u(name: &str) -> Option<&'static [u8]> {
    match name {
        "S-Enter" => Some(b"\x1b[13;2u"),
        "C-Enter" => Some(b"\x1b[13;5u"),
        _ => None,
    }
}

/// Whether `parts` hold a key whose sending depends on the server's `extended-keys`.
pub fn needs_extended(parts: &[Part]) -> bool {
    parts.iter().any(|p| matches!(p, Part::Keys(k) if k.iter().any(|n| csi_u(n).is_some())))
}

/// The `send-keys` commands for `parts`, in order, for pane `target` (`%3`). A server with
/// `extended-keys off` (a host's own tmux by default) would send S-Enter as Enter, so there
/// Shift+Enter and Ctrl+Enter go as their CSI u bytes (`-H`), as iTerm2 sends them (AS-10).
pub fn commands(target: &str, parts: &[Part], extended: bool) -> Vec<String> {
    let t = word(target);
    let names = |k: &[&String]| format!("send-keys -t {t} {}", k.iter().map(|n| word(n)).collect::<Vec<_>>().join(" "));
    let mut out = Vec::new();
    for p in parts {
        match p {
            Part::Text(s) if s.is_empty() => {}
            // "--": text starting with "-" is not read as flags (tmux 3.6: "unknown flag")
            Part::Text(s) => out.push(format!("send-keys -t {t} -l -- {}", quote(s))),
            Part::Keys(k) => {
                let mut run: Vec<&String> = Vec::new();
                for n in k {
                    match csi_u(n).filter(|_| !extended) {
                        Some(bytes) => {
                            if !run.is_empty() {
                                out.push(names(&run));
                                run.clear();
                            }
                            out.push(format!("send-keys -t {t} -H {}", bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")));
                        }
                        None => run.push(n),
                    }
                    if run.len() == KEY_PART {
                        out.push(names(&run));
                        run.clear();
                    }
                }
                if !run.is_empty() {
                    out.push(names(&run));
                }
            }
        }
    }
    out
}

/// Whether tmux knows `name` as a key (hot keys in `hotkeys.json`, AC-18): `C-`, `M-`, `S-`
/// or `^` prefixes, then a special key name, `F1`…`F12` or one printable character.
pub fn names_ok(name: &str) -> bool {
    let mut rest = name;
    loop {
        let b = rest.as_bytes();
        if b.len() > 2 && b[1] == b'-' && matches!(b[0].to_ascii_uppercase(), b'C' | b'M' | b'S') {
            rest = &rest[2..];
        } else if b.len() > 1 && b[0] == b'^' {
            rest = &rest[1..];
        } else {
            break;
        }
    }
    let mut chars = rest.chars();
    match (chars.next(), chars.next()) {
        (None, _) => false,
        (Some(c), None) => !c.is_control() && c != ' ',
        _ => {
            NAMES.iter().any(|n| n.eq_ignore_ascii_case(rest))
                || rest.strip_prefix(['F', 'f']).and_then(|n| n.parse::<u8>().ok())
                    .is_some_and(|n| (1..=12).contains(&n) && !rest[1..].starts_with('0'))
        }
    }
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
