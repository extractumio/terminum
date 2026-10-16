// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Who may ask what over TCP (AC-09, AC-10): the HTTP guard's decisions as pure functions, so
//! the table of refused requests is tested without a server. The guard computes `signed_in`
//! (the loopback sign-in from `signin_secret`, the web listener's from its cookie) and answers
//! a `Denial` as `{"error": code, "message": message}` with its status.
//!
//! Order, first refusal wins: `/internal/*` is 404 (served only on the CLI socket); a path
//! that could be judged as one thing and routed as another (dot segments; `%` in an API path)
//! is 400; the exact Host (DNS rebinding); on the web listener a sign-in through a loopback
//! name (its cookie would reach every local port) and what only the Mac may do (`not_here`);
//! the sign-in; the Origin; the Content-Type of a write.
//!
//! Host and session actions (`addhost`, `hostaction`, web on/off, `signout`) have no TCP
//! route at all: they are the CLI socket's, and the WebSocket refuses them off loopback.

use serde_json::{json, Value};

/// The listener a request came in on, with what its Host check needs.
#[derive(Clone, Copy, Debug)]
pub enum Listener<'a> {
    /// 127.0.0.1:<port>, the Mac's browser.
    Loopback { port: u16 },
    /// The network listener: `allow` from web.json's `allow_hosts`, `own_names` this Mac's
    /// names and addresses (host name, `<name>.local`, its network addresses).
    Web { allow: &'a [String], own_names: &'a [String] },
}

/// A refused request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Denial {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

impl Denial {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> Denial {
        Denial { status, code, message: message.into() }
    }

    pub fn body(&self) -> Value {
        json!({"error": self.code, "message": self.message})
    }
}

/// The name in a Host header: without its port and an IPv6 address's brackets, lowercase.
pub fn host_name(host: &str) -> String {
    let host = host.trim();
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else if host.matches(':').count() == 1 {
        host.split(':').next().unwrap_or("")
    } else {
        host // a name, or a bare IPv6 address
    };
    name.to_ascii_lowercase()
}

/// The loopback listener answers only to its own address: `127.0.0.1:<port>`, exactly.
pub fn host_ok_loopback(host: &str, port: u16) -> bool {
    host == format!("127.0.0.1:{port}")
}

/// The web listener answers to this Mac's own names and addresses, web.json's `allow_hosts` and
/// Tailscale names (`*.ts.net`); a page of another site whose DNS name points at this Mac sends
/// that name and is refused (DNS rebinding).
pub fn host_ok_web(host: &str, allow: &[String], own_names: &[String]) -> bool {
    let name = host_name(host);
    let known = |list: &[String]| list.iter().any(|n| n.eq_ignore_ascii_case(&name));
    !name.is_empty()
        && (matches!(name.as_str(), "localhost" | "127.0.0.1" | "::1")
            || known(allow)
            || known(own_names)
            || name.strip_suffix(".ts.net").is_some_and(|n| !n.is_empty() && !n.ends_with('.')))
}

/// An Origin is one of the page's own, exactly (browsers send it serialized: scheme, host, port).
pub fn origin_ok(origin: &str, expected: &[String]) -> bool {
    expected.iter().any(|e| e == origin)
}

/// The page's own origins for a request: the loopback page's, or on the web listener the Host
/// the browser asked for, over HTTP or behind an HTTPS proxy (`tailscale serve`).
pub fn expected_origins(listener: &Listener, host: &str) -> Vec<String> {
    match listener {
        Listener::Loopback { port } => vec![format!("http://127.0.0.1:{port}")],
        Listener::Web { .. } => vec![format!("http://{host}"), format!("https://{host}")],
    }
}

/// `path` without repeated slashes, so `//fb/api/os/x` is judged as `/fb/api/os/x`.
fn tidy(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if !(c == '/' && out.ends_with('/')) {
            out.push(c);
        }
    }
    out
}

/// `path` is `prefix` or below it.
fn under(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Requests that read or change terminals, sessions or files: everything but the page's own
/// files, `/api/hello`, `/open/<code>` and the web listener's sign-in.
pub fn needs_signin(path: &str) -> bool {
    let p = tidy(path);
    p == "/ws" || p == "/paste" || p == "/upload" || (under(&p, "/api") && p != "/api/hello") || under(&p, "/fb/api")
}

/// What only the Mac's own browser may do: Finder and default apps, typing a path into a
/// terminal (the spec's table names them as the Files API has them, `/fb/api/…`, and as fbd
/// had them, `/api/…`).
fn mac_only(path: &str) -> bool {
    let p = tidy(path);
    ["/fb/api/os", "/fb/api/terminal", "/api/os", "/api/terminal"].iter().any(|prefix| under(&p, prefix))
}

/// The web listener's sign-in.
fn is_signin(path: &str) -> bool {
    matches!(path, "/login" | "/auth/login")
}

/// A path whose meaning depends on who decodes it: `.` or `..` segments anywhere, `%` in an
/// API path (a router that decodes `/fb/api/%6fs/x` would serve `/fb/api/os/x`).
fn ambiguous(path: &str) -> bool {
    path.split('/').any(|seg| seg == "." || seg == "..")
        || (path.contains('%') && (under(path, "/api") || under(path, "/fb/api") || path.starts_with("/fb/api%")))
}

fn is_websocket(path: &str, header: &dyn Fn(&str) -> Option<String>) -> bool {
    let p = tidy(path);
    p == "/ws" || p == "/fb/api/ws" || header("upgrade").is_some_and(|u| u.to_ascii_lowercase().contains("websocket"))
}

/// The media type of a Content-Type header is `application/json` (parameters such as a charset
/// allowed).
fn is_json(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|c| c.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json"))
}

/// Allows or refuses a request before any handler runs. `header(name)` gives a request header
/// by its lowercase name; `signed_in` says whether the request carries a valid sign-in.
pub fn decide(listener: &Listener, method: &str, path: &str, header: impl Fn(&str) -> Option<String>,
              signed_in: bool) -> Result<(), Denial> {
    let p = tidy(path);
    if under(&p, "/internal") {
        return Err(Denial::new(404, "not_found", "Not found"));
    }
    if ambiguous(&p) {
        return Err(Denial::new(400, "bad_path", "The path has dot segments or escapes"));
    }
    let host = header("host").unwrap_or_default();
    let host_ok = match listener {
        Listener::Loopback { port } => host_ok_loopback(&host, *port),
        Listener::Web { allow, own_names } => host_ok_web(&host, allow, own_names),
    };
    if !host_ok {
        return Err(Denial::new(403, "bad_host", format!("Host {} is not allowed", clip(&host))));
    }
    let web = matches!(listener, Listener::Web { .. });
    if web && is_signin(&p) && matches!(host_name(&host).as_str(), "localhost" | "127.0.0.1" | "::1") {
        return Err(Denial::new(403, "bad_host", "Sign in with this Mac's network name: a cookie for a loopback name reaches every local port"));
    }
    if web && mac_only(&p) {
        return Err(Denial::new(403, "not_here", "Only the Mac's own browser can do this"));
    }
    if needs_signin(&p) && !signed_in {
        return Err(Denial::new(401, "bad_session", "Missing or expired sign-in"));
    }
    let write = !matches!(method, "GET" | "HEAD");
    let upload = write && (p == "/paste" || p == "/upload");
    // A browser names the page on every write and WebSocket; only these may lack an Origin:
    // a write on the loopback listener that comes from no page at all.
    let origin_needed = is_websocket(&p, &header) || upload || (write && matches!(listener, Listener::Web { .. }));
    match header("origin") {
        Some(origin) if !origin_ok(&origin, &expected_origins(listener, &host)) => {
            return Err(Denial::new(403, "bad_origin", format!("Origin {} is not allowed", clip(&origin))));
        }
        None if origin_needed => return Err(Denial::new(403, "bad_origin", "Origin is required")),
        _ => {}
    }
    if write && !upload && !is_json(header("content-type").as_deref()) {
        return Err(Denial::new(415, "bad_content_type", "Content-Type must be application/json"));
    }
    Ok(())
}

/// A header value as quoted in a message, at most 200 characters.
fn clip(s: &str) -> String {
    s.chars().take(200).collect()
}

/// The loopback sign-in a request presents: the `X-TM-Session` header; on a GET (an `<img>`) the
/// query's `t=`; on a WebSocket the second of the subprotocols `tm, <secret>` (the answer
/// selects `tm`).
pub fn signin_secret(method: &str, query: Option<&str>, header: impl Fn(&str) -> Option<String>) -> Option<String> {
    if let Some(s) = header("x-tm-session").filter(|s| !s.is_empty()) {
        return Some(s);
    }
    if let Some(protocols) = header("sec-websocket-protocol") {
        let mut it = protocols.split(',').map(str::trim);
        if it.next() == Some("tm")
            && let Some(s) = it.next().filter(|s| !s.is_empty())
        {
            return Some(s.to_string());
        }
    }
    if method == "GET" {
        return query?.split('&').find_map(|kv| kv.strip_prefix("t=")).filter(|s| !s.is_empty()).map(str::to_string);
    }
    None
}

#[cfg(test)]
#[path = "checks_tests.rs"]
mod tests;
