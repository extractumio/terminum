// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! HTTP to terminumd over its private socket ~/.terminum/state/terminumd.sock (AC-09): the
//! installer and the `terminum` command never talk to whatever holds the TCP port and never
//! carry a sign-in. One HTTP/1.0 request a connection, the answer read to its end.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// terminumd does not answer on its socket.
#[derive(Debug)]
pub struct Down;

/// (status, parsed JSON or None) of one request; a POST always carries a JSON body. `timeout`
/// is the whole call's: connect, write and every read share it.
pub fn call(sock: &Path, method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<(u16, Option<Value>), Down> {
    let end = Instant::now() + timeout;
    let mut s = crate::runlink::connect_within(sock, timeout).map_err(|_| Down)?;
    let mut req = format!("{method} {path} HTTP/1.0\r\nHost: localhost\r\n");
    let data = if method == "GET" { None } else { Some(body.cloned().unwrap_or_else(|| json!({})).to_string()) };
    if let Some(d) = &data {
        req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", d.len()));
    }
    req.push_str("\r\n");
    req.push_str(data.as_deref().unwrap_or(""));
    let mut out = req.as_bytes();
    while !out.is_empty() {
        left(&s, end)?;
        match s.write(out) {
            Ok(0) | Err(_) => return Err(Down),
            Ok(n) => out = &out[n..],
        }
    }
    let mut raw = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        left(&s, end)?;
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(Down),
        }
    }
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

/// Sets the socket's read and write timeouts to what is left until `end`; none left is Down.
/// macOS refuses the setting (EINVAL) once the other end closed: the next call then ends at once.
fn left(s: &UnixStream, end: Instant) -> Result<(), Down> {
    let rest = end.saturating_duration_since(Instant::now());
    if rest.is_zero() {
        return Err(Down);
    }
    let _ = s.set_read_timeout(Some(rest)).and_then(|()| s.set_write_timeout(Some(rest)));
    Ok(())
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

    /// A server on a fresh socket that accepts and then sends `trickle` one byte every 40 ms
    /// (empty: never answers), kept open 3 s.
    fn stalled(tag: &str, trickle: &'static [u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("tms-{}-{tag}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let l = std::os::unix::net::UnixListener::bind(&p).unwrap();
        std::thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                for b in trickle {
                    std::thread::sleep(Duration::from_millis(40));
                    let _ = c.write_all(&[*b]);
                }
                std::thread::sleep(Duration::from_secs(3));
            }
        });
        p
    }

    #[test]
    fn a_stalled_or_trickling_server_is_down_within_the_budget() {
        let budget = Duration::from_millis(150);
        for (tag, trickle) in [("mute", &b""[..]), ("drip", &b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\n{}"[..])] {
            let p = stalled(tag, trickle);
            let began = Instant::now();
            assert!(call(&p, "POST", "/hook", Some(&json!({})), budget).is_err(), "{tag}");
            let took = began.elapsed();
            assert!(took >= budget && took < budget + Duration::from_millis(100), "{tag}: {took:?}");
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn nothing_listening_is_down() {
        assert!(call(Path::new("/nonexistent/terminumd.sock"), "GET", "/health", None, Duration::from_secs(1)).is_err());
    }
}
