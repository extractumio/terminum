// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! `terminum tool`, `terminum tools`, `terminum tokens` (terminum-agents.md AC-181): the tools `terminum mcp`
//! offers, from a shell. The caller is the pane the command runs in, as for `terminum mcp`, or outside a pane the
//! token in TERMINUM_TOKEN; terminumd decides what that caller may run. A token is never an argument: a command
//! line is read by every process of the machine.

use std::io::Read;
use std::time::Duration;

use serde_json::{json, Value};

use super::mcp::{self, Env};
use super::sockhttp::Down;
use super::Ctx;

/// A tool's call may open a window or type into one: longer than a plan's update.
pub const BUDGET: Duration = Duration::from_secs(30);
const TOKEN: &str = "TERMINUM_TOKEN";
const NOT_A_CALLER: &str = "terminum tool runs in a session of terminum, or with a token in TERMINUM_TOKEN (terminum tokens add).";
pub const TOOL_USAGE: &str = "usage: terminum tool <name> ['<json arguments>' | -]    (- reads them from stdin; terminum tools lists them)";
pub const TOKENS_USAGE: &str = "usage: terminum tokens [add <name> --role manager|worker --project <project> [--days N] | remove <name>]";

/// Who calls: the pane of the environment, else outside one a token's holder.
enum Caller {
    Token(String),
    Pane(Env),
}

/// In a pane the pane's session calls, whatever token its environment holds: a token is for outside.
fn caller(ctx: &Ctx) -> Result<Caller, String> {
    let env = Env { tmux: ctx.var("TMUX").map(str::to_string), pane: ctx.var("TMUX_PANE").map(str::to_string), ..Default::default() };
    if mcp::pane_of(&env).is_some() {
        return Ok(Caller::Pane(env));
    }
    ctx.var(TOKEN).map(str::trim).filter(|t| !t.is_empty()).map(|token| Caller::Token(token.to_string())).ok_or_else(|| NOT_A_CALLER.to_string())
}

/// One call to terminumd as `who`: (the answer's text, whether it is a refusal).
fn call(ctx: &Ctx, who: &Caller, tool: &str, args: &Value) -> Result<(String, bool), String> {
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    match who {
        Caller::Token(token) => {
            let body = json!({"token": token, "tool": tool, "args": args});
            match super::sockhttp::call(&ctx.sock(), "POST", "/agent", Some(&body), BUDGET) {
                Err(Down) => Err(mcp::DOWN.into()),
                Ok((200, Some(a))) => Ok((text(&a["text"]), a["error"] == true)),
                Ok((status, a)) => Err(format!("terminum refused the call: {}", super::sockhttp::message(a.as_ref(), status))),
            }
        }
        Caller::Pane(env) => {
            let said = mcp::call(tool, args, None, env, &|body| forward(body));
            Ok((text(&said["content"][0]["text"]), said["isError"] == true))
        }
    }
}

/// A pane's call to the first sink that knows it.
fn forward(body: &Value) -> Result<(u16, Option<Value>), Down> {
    super::sinks::first_knowing(&super::sinks::all(), |p| super::sockhttp::call(p, "POST", "/agent", Some(body), BUDGET))
}

/// Whether the caller is a manager: what `terminum tools` shows follows it.
fn manager(ctx: &Ctx, who: &Caller) -> Result<bool, String> {
    match who {
        Caller::Pane(env) => Ok(mcp::manager(env, &|body| forward(body))),
        Caller::Token(_) => match call(ctx, who, "probe", &json!({}))? {
            (why, true) => Err(why),
            (role, false) => Ok(role == crate::roles::name(true)),
        },
    }
}

/// `terminum tool <name> [json | -]`: the tool's answer on stdout; a refusal on stderr, status 1.
pub fn tool(ctx: &Ctx, name: &str, given: Option<&str>) -> Result<i32, String> {
    let raw = match given {
        None => "{}".to_string(),
        Some("-") => {
            let mut all = String::new();
            std::io::stdin().lock().take(mcp::MAX_LINE as u64).read_to_string(&mut all).map_err(|e| format!("stdin: {e}"))?;
            all
        }
        Some(json) => json.to_string(),
    };
    let args: Value = serde_json::from_str(&raw).map_err(|e| format!("The arguments are not JSON: {e}\n{TOOL_USAGE}"))?;
    let who = caller(ctx)?;
    // whether this caller may run it is terminumd's to say
    if super::mcp_catalog::card(name).is_none() {
        return Err(format!("No tool is named {name:?}: terminum tools lists them."));
    }
    let args = mcp::checked(name, &args)?;
    let (text, refused) = call(ctx, &who, name, &args)?;
    if refused {
        return Err(text);
    }
    ctx.say(text);
    Ok(0)
}

/// `terminum tools [search <words> | info <name>…]`: what the caller's role finds, as search_tools and
/// get_tool_info answer it; with nothing, every tool by area.
pub fn tools(ctx: &Ctx, rest: &[String]) -> Result<i32, String> {
    let manager = manager(ctx, &caller(ctx)?)?;
    let said = match rest {
        [] => {
            let areas: Vec<String> = super::mcp_catalog::areas(manager).map(|(area, what)| {
                let names: Vec<&str> = super::mcp_catalog::cards(manager).filter(|c| c.area == *area).map(|c| c.name).collect();
                format!("{area} ({what}):\n  {}", names.join(", "))
            }).collect();
            Ok(format!("You call as a {}.\n{}\nterminum tools search <words> finds one; terminum tools info <name> describes it; terminum tool <name> '<json>' runs it.", crate::roles::name(manager), areas.join("\n")))
        }
        [s, words @ ..] if s == "search" && !words.is_empty() => super::mcp_meta::search(&json!({"query": words.join(" "), "limit": 10}), manager),
        [i, names @ ..] if i == "info" && !names.is_empty() => super::mcp_meta::describe(&json!({"names": names}), manager),
        _ => Err("usage: terminum tools [search <words> | info <name>…]".into()),
    }?;
    ctx.say(said);
    Ok(0)
}

/// A unix time as the Mac's local day and minute.
fn when(at: &Value) -> String {
    at.as_i64().map_or_else(|| "never".into(), crate::tasks::cron::when_words)
}

/// `terminum tokens [add … | remove <name>]`: terminumd refuses add and remove from inside a pane.
pub fn tokens(ctx: &Ctx, rest: &[String]) -> Result<i32, String> {
    let post = |body: Value| super::commands::checked(ctx, "POST", "/tokens", Some(body));
    match rest {
        [] => {
            let listed = post(json!({"action": "list"}))?;
            let all = listed["tokens"].as_array().cloned().unwrap_or_default();
            if all.is_empty() {
                ctx.say("No tokens. A program outside terminum calls its tools with one: terminum tokens add <name> --role manager|worker --project <project>");
            }
            for t in all {
                ctx.say(format!("{}  {}  project {}  made {}  ends {}  last used {}", t["name"].as_str().unwrap_or("?"), t["role"].as_str().unwrap_or("?"), t["project"].as_str().unwrap_or("?"),
                                when(&t["created_at"]), when(&t["expires_at"]), when(&t["last_used_at"])));
            }
        }
        [a, name, flags @ ..] if a == "add" => {
            let mut body = json!({"action": "add", "name": name});
            let mut it = flags.iter();
            while let Some(flag) = it.next() {
                let value = it.next().ok_or(TOKENS_USAGE)?;
                match flag.as_str() {
                    "--role" => body["role"] = json!(value),
                    "--project" => body["project"] = json!(value),
                    "--days" => body["days"] = json!(value.parse::<i64>().map_err(|_| TOKENS_USAGE)?),
                    _ => return Err(TOKENS_USAGE.into()),
                }
            }
            if body["role"].is_null() || body["project"].is_null() {
                return Err(TOKENS_USAGE.into());
            }
            let made = post(body)?;
            ctx.say(made["token"].as_str().unwrap_or_default());
            eprintln!("This is shown once: terminum keeps only its hash. Give it to the program in TERMINUM_TOKEN, never on a command line.");
        }
        [r, name] if r == "remove" => {
            post(json!({"action": "remove", "name": name}))?;
            ctx.say(format!("Token {name} is removed: calls with it are refused."));
        }
        _ => return Err(TOKENS_USAGE.into()),
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::cli::{parse, Cmd};

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn ac181_the_commands_parse() {
        assert_eq!(parse(&args(&["tool", "list_tasks"])), Ok(Cmd::Tool("list_tasks".into(), None)));
        assert_eq!(parse(&args(&["tool", "get_task", r#"{"id":"T-1"}"#])), Ok(Cmd::Tool("get_task".into(), Some(r#"{"id":"T-1"}"#.into()))));
        assert!(parse(&args(&["tool"])).unwrap_err().starts_with(TOOL_USAGE));
        assert!(parse(&args(&["tool", "a", "b", "c"])).is_err());
        assert_eq!(parse(&args(&["tools", "search", "run", "a", "task"])), Ok(Cmd::Tools(args(&["search", "run", "a", "task"]))));
        assert_eq!(parse(&args(&["tokens"])), Ok(Cmd::Tokens(vec![])));
        assert!(crate::cli::wanted(&args(&["terminumd", "tool"])) && crate::cli::wanted(&args(&["terminumd", "tokens"])));
    }

    #[test]
    fn ac181_outside_a_pane_and_with_no_token_nothing_is_called() {
        let home = crate::testutil::fresh_dir("tool-nobody");
        let ctx = Ctx::new(HashMap::from([("HOME".to_string(), home.display().to_string())]), home.clone()).unwrap();
        assert_eq!(tool(&ctx, "list_tasks", None).unwrap_err(), NOT_A_CALLER);
        assert_eq!(tools(&ctx, &[]).unwrap_err(), NOT_A_CALLER);
        assert!(tool(&ctx, "list_tasks", Some("{not json")).unwrap_err().starts_with("The arguments are not JSON"));
        // a token is never an argument: add wants its flags, and nothing else
        for bad in [&["add", "ci"][..], &["add", "ci", "--role", "manager"], &["add", "ci", "--token", "x"], &["remove"], &["rotate", "ci"]] {
            assert_eq!(tokens(&ctx, &args(bad)).unwrap_err(), TOKENS_USAGE, "{bad:?}");
        }
    }
}
