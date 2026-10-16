// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The refused-request table of AC-09 and the web listener's rules of AC-10.

use super::*;

const PORT: u16 = 47900;
const LOOP: Listener<'static> = Listener::Loopback { port: PORT };
const PAGE: &str = "http://127.0.0.1:47900";

fn web<'a>(allow: &'a [String], own: &'a [String]) -> Listener<'a> {
    Listener::Web { allow, own_names: own }
}

/// Headers as (lowercase name, value) pairs.
fn ask(l: &Listener, method: &str, path: &str, headers: &[(&str, &str)], signed_in: bool) -> Result<(), (u16, &'static str)> {
    let h = |name: &str| headers.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string());
    decide(l, method, path, h, signed_in).map_err(|d| (d.status, d.code))
}

fn local(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    let mut h = vec![("host", "127.0.0.1:47900")];
    h.extend_from_slice(extra);
    h
}

#[test]
fn ac09_outline_requests_without_a_sign_in_or_from_elsewhere_are_refused() {
    // GET /ws (no sign-in) → 401 bad_session, with or without the page's Origin
    assert_eq!(ask(&LOOP, "GET", "/ws", &local(&[("origin", PAGE), ("upgrade", "websocket")]), false), Err((401, "bad_session")));
    assert_eq!(ask(&LOOP, "GET", "/ws", &local(&[]), false), Err((401, "bad_session")));
    // GET /api/ls?path=/Users/alex, Host: evil.example:47900 → 403 bad_host, signed in or not
    for path in ["/api/ls", "/fb/api/ls"] {
        for signed in [true, false] {
            assert_eq!(ask(&LOOP, "GET", path, &[("host", "evil.example:47900")], signed), Err((403, "bad_host")), "{path}");
        }
    }
    // GET /ws with a sign-in, Origin: http://evil.example → 403 bad_origin
    assert_eq!(ask(&LOOP, "GET", "/ws", &local(&[("origin", "http://evil.example"), ("upgrade", "websocket")]), true),
               Err((403, "bad_origin")));
    // POST /api/fs/mkdir with a sign-in, Content-Type: text/plain → 415 bad_content_type
    for path in ["/api/fs/mkdir", "/fb/api/fs/mkdir"] {
        assert_eq!(ask(&LOOP, "POST", path, &local(&[("origin", PAGE), ("content-type", "text/plain")]), true),
                   Err((415, "bad_content_type")), "{path}");
    }
    // POST /api/os/reveal on the web listener → 403 not_here, signed in or not
    let own = vec!["mac.local".to_string()];
    let w = web(&[], &own);
    for path in ["/api/os/reveal", "/fb/api/os/reveal", "/fb/api/terminal/insert", "/fb/api/os", "//fb/api/os/open"] {
        for signed in [true, false] {
            let h = [("host", "mac.local:8765"), ("origin", "http://mac.local:8765"), ("content-type", "application/json")];
            assert_eq!(ask(&w, "POST", path, &h, signed), Err((403, "not_here")), "{path}");
        }
    }
    // (GET /open/<spent code> → 410 code_spent is signin.rs's Codes::redeem)
}

#[test]
fn the_mac_may_do_what_the_network_may_not() {
    let json = local(&[("origin", PAGE), ("content-type", "application/json")]);
    assert_eq!(ask(&LOOP, "POST", "/fb/api/os/reveal", &json, true), Ok(()));
    assert_eq!(ask(&LOOP, "POST", "/fb/api/terminal/insert", &json, true), Ok(()));
    assert_eq!(ask(&LOOP, "POST", "/fb/api/osx", &json, true), Ok(()), "a prefix is a whole segment");
}

#[test]
fn internal_paths_are_not_on_tcp_at_all() {
    let own = vec!["mac.local".to_string()];
    for path in ["/internal/web", "/internal", "//internal/x"] {
        assert_eq!(ask(&LOOP, "GET", path, &local(&[]), true), Err((404, "not_found")), "{path}");
        assert_eq!(ask(&web(&[], &own), "POST", path, &[("host", "evil.example")], false), Err((404, "not_found")), "{path}");
    }
}

#[test]
fn the_page_hello_and_the_open_link_need_no_sign_in() {
    for path in ["/", "/index.html", "/app.js", "/fb/", "/fb/renderers.js", "/api/hello", "/open/0123abcd", "/custom.css"] {
        assert_eq!(ask(&LOOP, "GET", path, &local(&[]), false), Ok(()), "{path}");
    }
    for path in ["/ws", "/paste", "/upload", "/api/settings", "/api/hello/x", "/fb/api/state", "/fb/api/ws", "/fb/api", "//ws"] {
        assert_eq!(ask(&LOOP, "GET", path, &local(&[("origin", PAGE)]), false), Err((401, "bad_session")), "{path}");
    }
}

#[test]
fn loopback_writes_need_json_and_an_absent_or_exact_origin() {
    let ok = |h: &[(&'static str, &'static str)]| ask(&LOOP, "POST", "/fb/api/fs/mkdir", &local(h), true);
    assert_eq!(ok(&[("content-type", "application/json")]), Ok(()), "no Origin: not from a page");
    assert_eq!(ok(&[("origin", PAGE), ("content-type", "application/json; charset=utf-8")]), Ok(()));
    assert_eq!(ok(&[("origin", PAGE), ("content-type", "Application/JSON")]), Ok(()));
    assert_eq!(ok(&[("origin", PAGE)]), Err((415, "bad_content_type")), "none");
    assert_eq!(ok(&[("origin", PAGE), ("content-type", "application/jsonp")]), Err((415, "bad_content_type")));
    assert_eq!(ok(&[("origin", PAGE), ("content-type", "multipart/form-data; boundary=x")]), Err((415, "bad_content_type")));
    for bad in ["http://evil.example", "null", "http://127.0.0.1:47901", "http://localhost:47900", "https://127.0.0.1:47900"] {
        assert_eq!(ok(&[("origin", bad), ("content-type", "application/json")]), Err((403, "bad_origin")), "{bad}");
    }
    assert_eq!(ask(&LOOP, "PUT", "/fb/api/file", &local(&[("origin", PAGE), ("content-type", "text/plain")]), true),
               Err((415, "bad_content_type")), "every method but GET and HEAD writes");
}

#[test]
fn uploads_take_any_type_but_only_from_the_page() {
    for path in ["/paste", "/upload"] {
        assert_eq!(ask(&LOOP, "POST", path, &local(&[("origin", PAGE), ("content-type", "image/png")]), true), Ok(()));
        assert_eq!(ask(&LOOP, "POST", path, &local(&[("content-type", "image/png")]), true), Err((403, "bad_origin")), "{path}");
        assert_eq!(ask(&LOOP, "POST", path, &local(&[("origin", "http://evil.example")]), true), Err((403, "bad_origin")));
        assert_eq!(ask(&LOOP, "POST", path, &local(&[("origin", PAGE)]), false), Err((401, "bad_session")));
    }
}

#[test]
fn a_websocket_needs_the_exact_origin() {
    let ws = |h: &[(&'static str, &'static str)], path| ask(&LOOP, "GET", path, &local(h), true);
    assert_eq!(ws(&[("origin", PAGE), ("upgrade", "websocket")], "/ws"), Ok(()));
    assert_eq!(ws(&[("upgrade", "websocket")], "/ws"), Err((403, "bad_origin")));
    assert_eq!(ws(&[], "/fb/api/ws"), Err((403, "bad_origin")));
    assert_eq!(ws(&[("upgrade", "WebSocket")], "/api/hello"), Err((403, "bad_origin")), "any path that upgrades");
}

#[test]
fn an_origin_present_on_any_request_must_be_exact() {
    assert_eq!(ask(&LOOP, "GET", "/", &local(&[("origin", "http://evil.example")]), false), Err((403, "bad_origin")));
    assert_eq!(ask(&LOOP, "GET", "/fb/api/ls", &local(&[("origin", PAGE)]), true), Ok(()));
}

#[test]
fn the_loopback_host_is_exact() {
    for host in ["localhost:47900", "127.0.0.1", "127.0.0.1:47901", "[::1]:47900", "127.0.0.1:47900.evil.example", ""] {
        assert_eq!(ask(&LOOP, "GET", "/", &[("host", host)], false), Err((403, "bad_host")), "{host}");
    }
    assert_eq!(ask(&LOOP, "GET", "/", &[], false), Err((403, "bad_host")), "no Host");
}

#[test]
fn ac10_the_web_listener_answers_to_this_macs_names_only() {
    let own = vec!["mac".to_string(), "mac.local".to_string(), "192.168.1.20".to_string()];
    let allow = vec!["Devbox.Example".to_string()];
    let w = web(&allow, &own);
    for host in ["mac.local:8765", "MAC.local:8765", "192.168.1.20:8765", "mac", "localhost:8765", "127.0.0.1:8765",
                 "[::1]:8765", "devbox.example:8765", "mac.tail1234.ts.net", "mac.tail1234.ts.net:443"] {
        assert_eq!(ask(&w, "GET", "/", &[("host", host)], false), Ok(()), "{host}");
    }
    for host in ["evil.example", "evil.example:8765", "mac.local.evil.example", "ts.net", ".ts.net", "x..ts.net", "evilts.net", ""] {
        assert_eq!(ask(&w, "GET", "/", &[("host", host)], false), Err((403, "bad_host")), "{host}");
    }
}

#[test]
fn ac10_web_sign_in_sign_out_and_writes_need_the_pages_own_origin() {
    let own = vec!["mac.local".to_string()];
    let w = web(&[], &own);
    let h = |origin: Option<&'static str>| {
        let mut h = vec![("host", "mac.local:8765"), ("content-type", "application/json")];
        h.extend(origin.map(|o| ("origin", o)));
        h
    };
    for path in ["/auth/login", "/login", "/logout", "/auth/logout"] {
        assert_eq!(ask(&w, "POST", path, &h(Some("http://mac.local:8765")), false), Ok(()), "{path} needs no sign-in");
        assert_eq!(ask(&w, "POST", path, &h(None), false), Err((403, "bad_origin")), "{path}: a browser always names the page");
        assert_eq!(ask(&w, "POST", path, &h(Some("http://evil.example")), false), Err((403, "bad_origin")));
    }
    assert_eq!(ask(&w, "GET", "/auth/check", &[("host", "mac.local:8765")], false), Ok(()));
    // behind `tailscale serve` the page is HTTPS on the proxy's name
    let ts = [("host", "mac.tail1234.ts.net"), ("origin", "https://mac.tail1234.ts.net"), ("content-type", "application/json")];
    assert_eq!(ask(&w, "POST", "/fb/api/fs/mkdir", &ts, true), Ok(()));
    let ws = [("host", "mac.local:8765"), ("origin", "http://mac.local:8765"), ("upgrade", "websocket")];
    assert_eq!(ask(&w, "GET", "/ws", &ws, true), Ok(()));
    assert_eq!(ask(&w, "GET", "/ws", &ws, false), Err((401, "bad_session")));
    assert_eq!(ask(&w, "GET", "/ws", &ws[..1], true), Err((403, "bad_origin")));
    assert_eq!(ask(&w, "GET", "/api/hello", &ws[..1], false), Ok(()), "the hello needs no sign-in here either");
}

#[test]
fn paths_that_route_differently_than_they_read_are_refused() {
    let own = vec!["mac.local".to_string()];
    let w = web(&[], &own);
    let h = [("host", "mac.local:8765"), ("origin", "http://mac.local:8765"), ("content-type", "application/json")];
    for path in ["/fb/api/%6fs/reveal", "/fb/api%2Fos/reveal", "/api/%6fs/reveal", "/fb/api/fs/../os/reveal", "/fb/./api/os/x", "/a/.."] {
        assert_eq!(ask(&w, "POST", path, &h, true), Err((400, "bad_path")), "{path}");
        assert_eq!(ask(&LOOP, "GET", path, &local(&[]), true), Err((400, "bad_path")), "{path}");
    }
    assert_eq!(ask(&w, "GET", "/fb/renderers.js", &h, false), Ok(()));
    assert_eq!(ask(&w, "GET", "/a.b/..c", &h, false), Ok(()), "dots inside a name are a name");
}

#[test]
fn the_web_listener_signs_in_only_through_a_network_name() {
    let own = vec!["mac.local".to_string()];
    let w = web(&[], &own);
    for host in ["127.0.0.1:8765", "localhost:8765", "[::1]:8765"] {
        let h = [("host", host), ("origin", "x"), ("content-type", "application/json")];
        assert_eq!(ask(&w, "POST", "/login", &h, false), Err((403, "bad_host")), "{host}");
        assert_eq!(ask(&w, "POST", "/auth/login", &h, false), Err((403, "bad_host")), "{host}");
        assert_eq!(ask(&w, "GET", "/", &h[..1], false), Ok(()), "{host}: the page itself is served");
    }
}

#[test]
fn host_names_lose_their_port_and_brackets() {
    assert_eq!(host_name("10.0.0.5:8765"), "10.0.0.5");
    assert_eq!(host_name("[::1]:8765"), "::1");
    assert_eq!(host_name("[::1]"), "::1");
    assert_eq!(host_name("fe80::1"), "fe80::1");
    assert_eq!(host_name("Mac.Local"), "mac.local");
}

#[test]
fn the_sign_in_comes_from_the_header_the_subprotocol_or_an_images_query() {
    let none = |_: &str| None;
    let header = |v: &'static str| move |n: &str| (n == "x-tm-session").then(|| v.to_string());
    assert_eq!(signin_secret("POST", None, header("abc")), Some("abc".into()));
    assert_eq!(signin_secret("GET", Some("x=1&t=abc"), none), Some("abc".into()));
    assert_eq!(signin_secret("POST", Some("t=abc"), none), None, "only a GET carries it in the query");
    assert_eq!(signin_secret("GET", Some("t="), none), None);
    let proto = |v: &'static str| move |n: &str| (n == "sec-websocket-protocol").then(|| v.to_string());
    assert_eq!(signin_secret("GET", None, proto("tm, abc")), Some("abc".into()));
    assert_eq!(signin_secret("GET", None, proto("chat, abc")), None);
    assert_eq!(signin_secret("GET", None, proto("tm")), None);
}

#[test]
fn a_denial_is_the_error_body_and_quotes_at_most_200_characters() {
    let long = format!("http://{}", "a".repeat(500));
    let d = decide(&LOOP, "GET", "/", |n| match n {
        "host" => Some("127.0.0.1:47900".into()),
        "origin" => Some(long.clone()),
        _ => None,
    }, false).unwrap_err();
    assert_eq!(d.body()["error"], "bad_origin");
    assert!(d.message.len() < 260);
}
