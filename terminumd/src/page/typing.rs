// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Typing and pasting into the shown pane (AC-03). Text goes as `send-keys -l`, special keys
//! by tmux's names (tmux encodes them for the pane's mode), never raw control characters.
//! Keys that arrive while earlier ones are on their way go together in the next round
//! trip, in order: a held key comes out at its repeat rate. Mouse reports (AC-06) share the
//! queue, so a trackpad's wheel on a far host is one `send-keys -H` a round trip, in order
//! with the keys, and never holds up the page's next messages. A bracketed paste goes through
//! a tmux buffer (`paste-buffer -p`), so a program that asked for it gets its markers.

use std::collections::VecDeque;
use std::sync::Arc;

use super::client::error;
use super::Page;
use crate::app::Shared;
use crate::tmux::keys::{PASTE_END, PASTE_START};
use crate::tmux::{args, keys, Server};

/// Largest paste (AC-03 limits).
const PASTE_MAX: usize = 1 << 20;
/// Most bytes of one set-buffer command (a host's paste).
const BUFFER_PART: usize = 64 << 10;

/// Typed data not yet handed to tmux.
pub struct Typed {
    server: Arc<Server>,
    pane: String,
    key: String,
    data: String,
    /// a mouse report's bytes (`data` empty)
    mouse: Vec<u8>,
    k: Option<super::trace::KeyRec>,
}

#[derive(Default)]
pub struct Queue {
    items: VecDeque<Typed>,
    running: bool,
}

/// The page typed `data` into its shown pane.
pub fn typed(app: &Shared, page: &Arc<Page>, data: String, k: Option<u64>) {
    let st = page.state.lock();
    let Some(s) = &st.shown else {
        drop(st);
        error(page, "Open a session first.", None);
        return;
    };
    let (server, pane, key) = (s.server.clone(), s.pane.clone(), s.key.clone());
    drop(st);
    let k = super::timing::arrived(page, k);
    queue(app, page, Typed { server, pane, key, data, mouse: Vec::new(), k });
}

/// Mouse report bytes for the shown pane `pane` (state key `key`) of `server` (AC-06).
pub fn mouse(app: &Shared, page: &Arc<Page>, server: Arc<Server>, pane: String, key: String, bytes: Vec<u8>) {
    queue(app, page, Typed { server, pane, key, data: String::new(), mouse: bytes, k: None });
}

fn queue(app: &Shared, page: &Arc<Page>, item: Typed) {
    let mut st = page.state.lock();
    st.queue.items.push_back(item);
    if st.queue.running {
        return;
    }
    st.queue.running = true;
    drop(st);
    tokio::spawn(run(app.clone(), page.clone()));
}

/// A paste the page sent as such (`paste` message).
pub fn paste(app: &Shared, page: &Arc<Page>, data: String, bracket: bool) {
    let wrapped = if bracket { format!("{PASTE_START}{data}{PASTE_END}") } else { data };
    typed(app, page, wrapped, None);
}

async fn run(app: Shared, page: Arc<Page>) {
    loop {
        let mut batch: Vec<Typed> = {
            let mut st = page.state.lock();
            let Some(first) = st.queue.items.front() else {
                st.queue.running = false;
                return;
            };
            let pane = first.pane.clone();
            let mut out = Vec::new();
            while st.queue.items.front().is_some_and(|t| t.pane == pane) {
                out.push(st.queue.items.pop_front().expect("checked"));
            }
            out
        };
        let (server, pane, key) = (batch[0].server.clone(), batch[0].pane.clone(), batch[0].key.clone());
        let keys = batch.iter().any(|t| t.mouse.is_empty());
        let ks: Vec<Option<super::trace::KeyRec>> = batch.iter_mut().map(|t| t.k.take()).collect();
        let Some(client) = server.client(&pane) else {
            let what = if keys { "the keys were not sent" } else { "the mouse event was not sent" };
            error(&page, format!("Not connected: tmux on {} does not answer — {what}", server.label()), Some(&key));
            continue;
        };
        let mut result = Ok(());
        for run in runs(&batch) {
            result = match run {
                Run::Keys(data) => send(&app, &client, &server, &pane, &data).await,
                Run::Mouse(bytes) => client.run(mouse_line(&pane, &bytes)).await.map(drop).map_err(|e| format!("Mouse event not sent: {e}")),
            };
            if result.is_err() {
                break;
            }
        }
        super::timing::typed(&page, ks, "keys");
        if let Err(e) = result {
            error(&page, e, Some(&key));
            continue;
        }
        {
            let st = page.state.lock();
            if let Some(s) = st.shown.as_ref().filter(|s| s.pane == pane) {
                s.ctl.wake.notify_one();
            }
        }
        if keys {
            super::sizing::typed(&app, &page).await;
        }
    }
}

/// A batch in order as runs of one kind: keys joined, mouse reports joined; a run is a round
/// trip of its own (typing while scrolling on a host: one each, rare and short).
enum Run {
    Keys(String),
    Mouse(Vec<u8>),
}

fn runs(batch: &[Typed]) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for t in batch {
        match (out.last_mut(), t.mouse.is_empty()) {
            (Some(Run::Keys(d)), true) => d.push_str(&t.data),
            (Some(Run::Mouse(b)), false) => b.extend_from_slice(&t.mouse),
            (_, true) => out.push(Run::Keys(t.data.clone())),
            (_, false) => out.push(Run::Mouse(t.mouse.clone())),
        }
    }
    out
}

/// Mouse reports as one tmux command: their bytes in hex.
fn mouse_line(pane: &str, bytes: &[u8]) -> String {
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("send-keys -t {pane} -H {}", hex.join(" "))
}

/// Hands `data` to tmux: bracketed pastes through a buffer, the rest as keys.
async fn send(app: &Shared, client: &crate::tmux::control::Control, server: &Server, pane: &str, data: &str) -> Result<(), String> {
    let mut rest = data;
    while !rest.is_empty() {
        let (before, pasted, after) = match rest.find(PASTE_START) {
            Some(i) => {
                let inner = &rest[i + PASTE_START.len()..];
                match inner.find(PASTE_END) {
                    Some(j) => (&rest[..i], Some(&inner[..j]), &inner[j + PASTE_END.len()..]),
                    None => (&rest[..i], Some(inner), ""),
                }
            }
            None => (rest, None, ""),
        };
        if !before.is_empty() {
            let parts = keys::parts(before);
            let extended = !keys::needs_extended(&parts) || server.extended_keys(client).await;
            let cmds = keys::commands(pane, &parts, extended);
            for a in client.run_all(cmds).await {
                a.map_err(|e| format!("tmux did not take the keys: {e}"))?;
            }
        }
        if let Some(text) = pasted {
            paste_buffer(app, client, server, pane, text).await?;
        }
        rest = after;
    }
    Ok(())
}

/// A paste through a tmux buffer: a 0600 file and `load-buffer` on this Mac, `set-buffer`
/// in parts on a host; then `paste-buffer -p` (bracketed, the buffer deleted).
async fn paste_buffer(app: &Shared, client: &crate::tmux::control::Control, server: &Server, pane: &str, text: &str) -> Result<(), String> {
    if text.len() > PASTE_MAX {
        return Err("Paste is larger than 1 MB".into());
    }
    if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r')) {
        return Err("The text holds control characters and was not sent".into());
    }
    let name = format!("tm-{}", crate::auth::crypto::random_hex::<6>());
    if server.is_local() {
        let path = app.cfg.dir.join(format!("paste-{name}"));
        crate::config::write_private(&path, text.as_bytes()).map_err(|e| format!("Paste not sent: {e}"))?;
        let loaded = client.run(format!("load-buffer -b {name} {}", args::quote(&path.display().to_string()))).await;
        let _ = std::fs::remove_file(&path);
        loaded.map_err(|e| format!("tmux did not take the paste: {e}"))?;
    } else {
        let mut first = true;
        for part in chunks(text, BUFFER_PART) {
            let append = if first { "" } else { "-a " };
            first = false;
            // `--`: a part that starts with "-" is text, not a flag (tmux stops at the first word)
            client.run(format!("set-buffer {append}-b {name} -- {}", double_quoted(part))).await.map_err(|e| format!("tmux did not take the paste: {e}"))?;
        }
    }
    client.run(format!("paste-buffer -p -d -b {name} -t {pane}")).await.map_err(|e| format!("tmux did not take the paste: {e}"))?;
    Ok(())
}

/// `s` cut at character boundaries into parts of at most `max` bytes.
fn chunks(s: &str, max: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while rest.len() > max {
        let mut at = max;
        while !rest.is_char_boundary(at) {
            at -= 1;
        }
        out.push(&rest[..at]);
        rest = &rest[at..];
    }
    out.push(rest);
    out
}

/// Text in tmux's double quotes, where `\n`, `\t`, `\r` escape and `$` would expand.
pub fn double_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(data: &str, mouse: &[u8]) -> Typed {
        let link = crate::tmux::server::Link::Local { tmux: "tmux".into(), socket: "tm-test".into(), conf: "/dev/null".into() };
        let server = Server::new("local", "tm-test", link, Arc::new(tokio::sync::Notify::new()));
        Typed { server, pane: "%1".into(), key: String::new(), data: data.into(), mouse: mouse.to_vec(), k: None }
    }

    #[test]
    fn a_batch_keeps_its_order_and_joins_each_kind() {
        let batch = [item("a", b""), item("b", b""), item("", b"\x1b[<64;1;1M"), item("", b"\x1b[<64;1;1M"), item("c", b"")];
        let got: Vec<String> = runs(&batch).into_iter().map(|r| match r {
            Run::Keys(d) => format!("keys:{d}"),
            Run::Mouse(b) => format!("mouse:{}", String::from_utf8_lossy(&b)),
        }).collect();
        assert_eq!(got, ["keys:ab", "mouse:\x1b[<64;1;1M\x1b[<64;1;1M", "keys:c"]);
        assert_eq!(mouse_line("%3", b"\x1b["), "send-keys -t %3 -H 1b 5b");
    }

    #[test]
    fn parts_never_split_a_character() {
        let s = "ab€cd";
        let parts = chunks(s, 3);
        assert_eq!(parts.concat(), s);
        assert!(parts.iter().all(|p| p.len() <= 3));
    }

    #[test]
    fn double_quotes_escape_what_tmux_would_read() {
        assert_eq!(double_quoted("a\nb $HOME \"q\" \\ \t"), r#""a\nb \$HOME \"q\" \\ \t""#);
    }
}
