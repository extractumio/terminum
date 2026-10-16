// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! HTTP to terminumd over its private socket ~/.terminum/state/terminumd.sock (AC-09): the
//! installer and the `terminum` command never talk to whatever holds the TCP port and never
//! carry a sign-in. One HTTP/1.0 request a connection, the answer read to its end.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

/// terminumd does not answer on its socket.
#[derive(Debug)]
pub struct Down;

/// (status, parsed JSON or None) of one request; a POST always carries a JSON body.
pub fn call(sock: &Path, method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<(u16, Option<Value>), Down> {
    let down = |_| Down;
    let mut s = UnixStream::connect(sock).map_err(down)?;
    s.set_read_timeout(Some(timeout)).map_err(down)?;
    s.set_write_timeout(Some(timeout)).map_err(down)?;
    let mut req = format!("{method} {path} HTTP/1.0\r\nHost: localhost\r\n");
    let data = if method == "GET" { None } else { Some(body.cloned().unwrap_or_else(|| json!({})).to_string()) };
    if let Some(d) = &data {
        req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", d.len()));
    }
    req.push_str("\r\n");
    req.push_str(data.as_deref().unwrap_or(""));
    s.write_all(req.as_bytes()).map_err(down)?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(down)?;
    let (status, body) = parse(&raw).ok_or(Down)?;
    if body.is_empty() {
        return Ok((status, None));
    }
    match serde_json::from_slice(&body) {
        Ok(v) => Ok((status, Some(v))),
        Err(_) => {
            let text: String = String::from_utf8_lossy(&body).chars().take(200).collect();
            Ok((status, Some(json!({"error": "bad_answer", "message": text}))))
        }
    }
}

/// The status and body of an HTTP/1.x answer (Content-Length, chunked or to the end).
fn parse(raw: &[u8]) -> Option<(u16, Vec<u8>)> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let mut chunked = false;
    let mut length = None;
    for line in lines {
        let (k, v) = line.split_once(':')?;
        match k.trim().to_ascii_lowercase().as_str() {
            "transfer-encoding" => chunked = v.trim().eq_ignore_ascii_case("chunked"),
            "content-length" => length = v.trim().parse::<usize>().ok(),
            _ => {}
        }
    }
    let rest = &raw[end + 4..];
    let body = if chunked {
        dechunk(rest)?
    } else {
        rest[..length.unwrap_or(rest.len()).min(rest.len())].to_vec()
    };
    Some((status, body))
}

fn dechunk(mut rest: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = rest.windows(2).position(|w| w == b"\r\n")?;
        let size = usize::from_str_radix(std::str::from_utf8(&rest[..eol]).ok()?.split(';').next()?.trim(), 16).ok()?;
        rest = &rest[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(rest.get(..size)?);
        rest = rest.get(size + 2..)?;
    }
}

/// The text of an error answer {"error", "message"}.
pub fn message(answer: Option<&Value>, status: u16) -> String {
    let field = |k: &str| answer.and_then(|a| a.get(k)).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    field("message").or_else(|| field("error")).unwrap_or_else(|| format!("HTTP {status}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_by_length_chunks_or_end() {
        assert_eq!(parse(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}xx"), Some((200, b"{}".to_vec())));
        assert_eq!(parse(b"HTTP/1.1 409 Conflict\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n"), Some((409, b"{}".to_vec())));
        assert_eq!(parse(b"HTTP/1.0 200 OK\r\n\r\n[1]"), Some((200, b"[1]".to_vec())));
        assert_eq!(parse(b"garbage"), None);
    }

    #[test]
    fn the_message_of_an_error() {
        assert_eq!(message(Some(&json!({"error": "web", "message": "no"})), 409), "no");
        assert_eq!(message(Some(&json!({"error": "web"})), 409), "web");
        assert_eq!(message(None, 500), "HTTP 500");
    }

    #[test]
    fn nothing_listening_is_down() {
        assert!(call(Path::new("/nonexistent/terminumd.sock"), "GET", "/health", None, Duration::from_secs(1)).is_err());
    }
}
