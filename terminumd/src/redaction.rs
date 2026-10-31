// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Redact stored text without interpreting or executing shell syntax.

const SECRET_WORD: [&str; 6] = ["token", "secret", "password", "passwd", "apikey", "api_key"];
const SECRET_SHAPE: [&str; 9] = ["sk-", "ghp_", "gho_", "github_pat_", "glpat-", "xoxb-", "xoxp-", "AKIA", "eyJ"];

/// `text` with secrets' values left out: `NAME=value`, `NAME: value`, `--name value` for a
/// name like a secret's, `Bearer value`, words of a known secret's shape and private key blocks.
pub fn redact(text: &str) -> String {
    let text = pem_out(text);
    let secretish = |w: &str| { let w = w.trim_start_matches('-').to_ascii_lowercase(); SECRET_WORD.iter().any(|s| w.contains(s)) };
    let mut out = String::with_capacity(text.len());
    let mut hide_next = false;
    let mut at = 0;
    while at < text.len() {
        let piece = text[at..].split_inclusive(char::is_whitespace).next().expect("remaining text");
        let word = piece.trim_end_matches(char::is_whitespace);
        let gap = &piece[word.len()..];
        let bare = word.trim_matches(|c: char| "'\"`(),;".contains(c));
        if word.is_empty() {
            out.push_str(piece);
            at += piece.len();
            continue;
        }
        let hidden = if hide_next && (!bare.starts_with('-') || word.starts_with(['\'', '"'])) {
            hide_next = false;
            at = value_end(&text, at, None);
            out.push_str("[redacted]");
            continue;
        } else if let Some((k, _)) = word.split_once('=').filter(|(k, v)| !v.is_empty() && secretish(k)) {
            let opened = k.chars().next().filter(|q| matches!(q, '\'' | '"') && !k[1..].contains(*q));
            let end = value_end(&text, at + k.len() + 1, opened);
            out.push_str(&format!("{k}=[redacted]"));
            hide_next = false;
            at = end;
            continue;
        } else if SECRET_SHAPE.iter().any(|s| bare.starts_with(s) && bare.len() >= s.len() + 12) {
            at = value_end(&text, at, None);
            hide_next = false;
            out.push_str("[redacted]");
            continue;
        } else {
            let key = bare.trim_end_matches(':').trim_matches('"');
            hide_next = (bare.starts_with("--") && !word.contains('=') && secretish(bare)) || (bare.ends_with(':') && secretish(key))
                || bare.eq_ignore_ascii_case("bearer");
            word.to_string()
        };
        out.push_str(&hidden);
        out.push_str(gap);
        at += piece.len();
    }
    out
}

/// End of one secret value starting at `start`, including quoted/escaped whitespace; `inside`:
/// the quote its word opened before the value (`'API_KEY=a b'`). Only a quote at the value's
/// start or right after a closed one opens: text before the value and a mid-word apostrophe
/// (`it's`, `don't`) are prose, never a quote that hides what follows. An open quote hides to EOF.
fn value_end(text: &str, start: usize, inside: Option<char>) -> usize {
    let (mut quote, mut ansi, mut escaped) = (inside, false, false);
    let (mut may_open, mut dollar) = (inside.is_none(), false);
    for (offset, c) in text[start..].char_indices() {
        if escaped { escaped = false; continue; }
        if let Some(q) = quote {
            if c == '\\' && (q == '"' || ansi) { escaped = true; }
            else if c == q { (quote, ansi, may_open) = (None, false, true); }
            continue;
        }
        if c.is_whitespace() { return start + offset; }
        if c == '\\' { (escaped, may_open, dollar) = (true, false, false); continue; }
        if (may_open || dollar) && matches!(c, '\'' | '"') {
            (quote, ansi, may_open, dollar) = (Some(c), dollar && c == '\'', false, false);
            continue;
        }
        (dollar, may_open) = (may_open && c == '$', false);
    }
    text.len()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_values_preserve_following_nonsecret_arguments() {
        for source in [
            "tool --password 'alpha beta' tail",
            "tool '--password' \"alpha beta\" tail",
            "tool --password \"alpha\\\" beta\" tail",
            "tool --password alpha\\ beta tail",
            "tool --password '-alpha beta' tail",
            "tool --password $'alpha\\' beta' tail",
            "tool --password $'alpha\\\\ beta' tail",
        ] {
            assert_eq!(redact(source).split("[redacted]").last(), Some(" tail"), "{source}");
            assert!(!redact(source).contains("alpha") && !redact(source).contains("beta"));
        }
        assert_eq!(redact("PASSWORD='alpha beta' tool"), "PASSWORD=[redacted] tool");
        assert_eq!(redact("PASSWORD=$'alpha\\' beta' tool"), "PASSWORD=[redacted] tool");
        assert_eq!(redact("tool --password 'alpha beta"), "tool --password [redacted]");
        assert_eq!(redact("curl -H 'Authorization: Bearer alpha' tail"), "curl -H 'Authorization: Bearer [redacted] tail");
        assert_eq!(redact("tool 'ordinary quoted words' tail"), "tool 'ordinary quoted words' tail");
        assert_eq!(redact("tool --password --user alex"), "tool --password --user alex");
    }

    #[test]
    fn apostrophes_in_prose_never_hide_the_rest_of_a_prompt() {
        for (source, kept) in [
            ("Fix the token: it's broken. Then rename the module", "Fix the token: [redacted] broken. Then rename the module"),
            ("We don't send a bearer token yet, so add one", "We don't send a bearer [redacted] yet, so add one"),
            ("don't print password: hunter2 and then run make test", "don't print password: [redacted] and then run make test"),
            ("it's set: API_TOKEN=abc123 and the user's tests pass", "it's set: API_TOKEN=[redacted] and the user's tests pass"),
            ("run \"tool --password alpha\" then report", "run \"tool --password [redacted] then report"),
        ] {
            assert_eq!(redact(source), kept, "{source}");
        }
    }

    #[test]
    fn a_value_quoted_from_before_its_name_or_in_parts_stays_hidden() {
        assert_eq!(redact("env 'API_KEY=alpha beta' tool"), "env 'API_KEY=[redacted] tool");
        assert_eq!(redact("env \"API_KEY=alpha \\\" beta\" tool"), "env \"API_KEY=[redacted] tool");
        assert_eq!(redact("tool --password 'alpha beta'\"gamma delta\" tail"), "tool --password [redacted] tail");
        assert_eq!(redact("tool --password $'alpha beta' tail"), "tool --password [redacted] tail");
    }

    #[test]
    fn non_ascii_text_around_values_is_cut_on_character_boundaries() {
        // a quote opened inside the value's word is not followed (SECURITY.md: Sent history)
        assert_eq!(redact("é=x TOKEN=é'ü ö' tail"), "é=x TOKEN=[redacted] ö' tail");
        assert_eq!(redact("пароль --password 'ü ö' ещё"), "пароль --password [redacted] ещё");
        assert_eq!(redact("TOKEN=\"ü"), "TOKEN=[redacted]");
    }
}
