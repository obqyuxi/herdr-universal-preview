//! The phone's chat view: an agent's own session log turned into a short list
//! of messages, tool calls and results (docs/harmony/DESIGN.md §4).
//!
//! - Claude Code: `~/.claude/projects/<cwd with non-alphanumerics as '-'>/<session>.jsonl`
//! - Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, matched by `session_meta.cwd`
//!
//! The newest session of that folder is read from `offset` on; a different
//! newest file (a new session) starts over (`reset`). Formats are undocumented
//! and change between versions, so anything unknown is skipped, not an error.

use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const FIRST_WINDOW: u64 = 1536 * 1024;
const MORE_WINDOW: u64 = 2 * 1024 * 1024;
/// The first page keeps the end of the session; later pages are small anyway.
const MAX_ITEMS: usize = 150;
const TEXT_MAX: usize = 4000;
const DETAIL_MAX: usize = 2000;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Item {
    /// "user", "assistant", "tool" (a call) or "result" (its output)
    pub k: &'static str,
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// Links a result to its call.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub err: bool,
    /// Byte offset of the log line it came from (for loading earlier parts).
    #[serde(rename = "o")]
    pub off: u64,
}

#[derive(Serialize, Debug)]
pub struct Page {
    pub file: String,
    pub offset: u64,
    /// Start over: the file changed, or this is the first read.
    pub reset: bool,
    /// Earlier parts of the session were left out (only the end was read).
    pub truncated: bool,
    /// Where the lines read begin; ask with `before` = this for the part before it.
    pub start: u64,
    /// These items come before what the phone has (a `before` request).
    pub older: bool,
    pub items: Vec<Item>,
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str(" …");
    out
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn mtime(p: &Path) -> SystemTime {
    fs::metadata(p).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
}

pub fn claude_dir(home: &Path, cwd: &str) -> PathBuf {
    let enc: String = cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    home.join(".claude/projects").join(enc)
}

fn newest_claude(home: &Path, cwd: &str) -> Option<PathBuf> {
    let dir = claude_dir(home, cwd);
    fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl") && p.is_file())
        .max_by_key(|p| mtime(p))
}

fn first_line(p: &Path) -> Option<Value> {
    let mut f = fs::File::open(p).ok()?;
    let mut buf = vec![0u8; 64 * 1024];
    let n = f.read(&mut buf).ok()?;
    let line = buf[..n].split(|b| *b == b'\n').next()?;
    serde_json::from_slice(line).ok()
}

fn newest_codex(home: &Path, cwd: &str) -> Option<PathBuf> {
    let root = home.join(".codex/sessions");
    let mut files: Vec<PathBuf> = vec![];
    // The last few dated folders are enough for a live terminal.
    let mut days: Vec<PathBuf> = vec![];
    for y in fs::read_dir(&root).ok()?.flatten() {
        for m in fs::read_dir(y.path()).into_iter().flatten().flatten() {
            for d in fs::read_dir(m.path()).into_iter().flatten().flatten() {
                days.push(d.path());
            }
        }
    }
    days.sort();
    for d in days.iter().rev().take(7) {
        for f in fs::read_dir(d).into_iter().flatten().flatten() {
            let p = f.path();
            if p.extension().is_some_and(|x| x == "jsonl") {
                files.push(p);
            }
        }
    }
    files.sort_by_key(|p| std::cmp::Reverse(mtime(p)));
    let mut fallback = None;
    for p in files {
        let Some(meta) = first_line(&p) else { continue };
        if meta["payload"]["cwd"].as_str() != Some(cwd) {
            continue;
        }
        // The main thread, not a sub-agent's, when there is one.
        if meta["payload"]["source"]["subagent"].is_null() {
            return Some(p);
        }
        fallback.get_or_insert(p);
    }
    fallback
}

/// Claude Code keeps `~/.claude/sessions/<pid>.json` with the session it's on.
fn claude_by_pid(home: &Path, pid: u32) -> Option<PathBuf> {
    let v: Value = serde_json::from_slice(&fs::read(home.join(format!(".claude/sessions/{pid}.json"))).ok()?).ok()?;
    let p = claude_dir(home, v["cwd"].as_str()?).join(format!("{}.jsonl", v["sessionId"].as_str()?));
    p.is_file().then_some(p)
}

/// Codex keeps its rollout file open while it runs.
fn codex_by_pid(pid: u32) -> Option<PathBuf> {
    let out = std::process::Command::new("/usr/sbin/lsof").args(["-n", "-p", &pid.to_string(), "-Fn"]).output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix('n'))
        .find(|n| n.contains("/.codex/sessions/") && n.ends_with(".jsonl"))
        .map(PathBuf::from)
}

/// Which log a terminal's agent writes: by the agent's process when the core
/// knows it, else the newest session of the folder. `app` is its OSC 7501 name.
pub fn locate(home: &Path, app: &str, cwd: &str, pid: Option<u32>) -> Option<PathBuf> {
    let codex = app.contains("codex");
    let exact = pid.and_then(|p| if codex { codex_by_pid(p) } else { claude_by_pid(home, p) });
    exact.or_else(|| if codex { newest_codex(home, cwd) } else { newest_claude(home, cwd) })
}

pub fn read(app: &str, cwd: &str, pid: Option<u32>, file: Option<&str>, offset: Option<u64>, before: Option<u64>) -> Result<Page> {
    read_in(&home(), app, cwd, pid, file, offset, before)
}

/// Parse whole lines of `buf` (which begins at byte `base` of the file).
fn parse(buf: &[u8], base: u64, codex: bool) -> Vec<Item> {
    let mut items = vec![];
    let mut at = 0usize;
    for line in buf.split(|b| *b == b'\n') {
        let off = base + at as u64;
        at += line.len() + 1;
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else { continue };
        let n = items.len();
        if codex {
            codex_items(&v, &mut items)
        } else {
            claude_items(&v, &mut items)
        }
        for it in &mut items[n..] {
            it.off = off;
        }
    }
    items
}

/// Keep the last MAX_ITEMS, cut at a line boundary; returns where the kept lines begin.
fn keep_last(items: &mut Vec<Item>, start: u64) -> u64 {
    if items.len() <= MAX_ITEMS {
        return start;
    }
    let mut cut = items.len() - MAX_ITEMS;
    while cut < items.len() && items[cut].off == items[cut - 1].off {
        cut += 1;
    }
    items.drain(..cut);
    items.first().map_or(start, |i| i.off)
}

fn read_range(path: &Path, from: u64, to: u64) -> Result<Vec<u8>> {
    let mut f = fs::File::open(path)?;
    f.seek(SeekFrom::Start(from))?;
    let mut buf = vec![0u8; (to - from) as usize];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

pub fn read_in(home: &Path, app: &str, cwd: &str, pid: Option<u32>, file: Option<&str>, offset: Option<u64>, before: Option<u64>) -> Result<Page> {
    let Some(path) = locate(home, app, cwd, pid) else { bail!("没找到这个目录的会话记录（{app}）") };
    let name = path.to_string_lossy().into_owned();
    let len = fs::metadata(&path)?.len();
    let codex = app.contains("codex");
    let same_file = file == Some(name.as_str());

    // The part before what the phone already shows.
    if let Some(b) = before.filter(|b| same_file && *b <= len) {
        let s = b.saturating_sub(FIRST_WINDOW);
        let buf = read_range(&path, s, b)?;
        // Begin at a whole line (the window's end already is one).
        let skip = if s > 0 { buf.iter().position(|c| *c == b'\n').map_or(buf.len(), |i| i + 1) } else { 0 };
        let mut items = parse(&buf[skip..], s + skip as u64, codex);
        let start = keep_last(&mut items, s + skip as u64);
        return Ok(Page { file: name, offset: offset.unwrap_or(len), reset: false, truncated: start > 0, start, older: true, items });
    }

    let same = same_file && offset.is_some_and(|o| o <= len);
    let (s, reset) = if same { (offset.unwrap_or(0), false) } else { (len.saturating_sub(FIRST_WINDOW), true) };
    let buf = read_range(&path, s, len.min(s + MORE_WINDOW))?;
    // From the middle of the file: begin at the next whole line.
    let from = if reset && s > 0 { buf.iter().position(|b| *b == b'\n').map_or(buf.len(), |i| i + 1) } else { 0 };
    // Stop after the last whole line (the agent may be writing the next).
    let upto = buf.iter().rposition(|b| *b == b'\n').map_or(from, |i| i + 1).max(from);
    let mut items = parse(&buf[from..upto], s + from as u64, codex);
    let start = keep_last(&mut items, s + from as u64);
    Ok(Page { file: name, offset: s + upto as u64, reset, truncated: reset && start > 0, start, older: false, items })
}

/// A one-line summary of a tool call and the longer detail behind it.
fn tool_summary(name: &str, input: &Value) -> (String, String) {
    let s = |k: &str| input[k].as_str().unwrap_or_default().to_owned();
    match name {
        // The description reads better on a phone; the command is one tap away.
        "Bash" if !s("description").is_empty() => (s("description"), s("command")),
        "Bash" => (s("command"), String::new()),
        "Edit" | "MultiEdit" => {
            let mut d = String::new();
            let edits: Vec<Value> = if name == "MultiEdit" { input["edits"].as_array().cloned().unwrap_or_default() } else { vec![input.clone()] };
            for e in edits {
                for l in e["old_string"].as_str().unwrap_or_default().lines() {
                    d.push_str(&format!("- {l}\n"));
                }
                for l in e["new_string"].as_str().unwrap_or_default().lines() {
                    d.push_str(&format!("+ {l}\n"));
                }
            }
            (s("file_path"), d)
        }
        "Write" => (s("file_path"), s("content")),
        "Read" | "NotebookEdit" => (s("file_path"), String::new()),
        "Grep" | "Glob" => (format!("{} {}", s("pattern"), s("path")).trim().to_owned(), String::new()),
        "WebFetch" => (s("url"), s("prompt")),
        "WebSearch" => (s("query"), String::new()),
        "Task" | "Agent" => (s("description"), s("prompt")),
        "TodoWrite" => {
            let todos = input["todos"].as_array().cloned().unwrap_or_default();
            let list: Vec<String> = todos.iter().map(|t| format!("[{}] {}", t["status"].as_str().unwrap_or(""), t["content"].as_str().unwrap_or(""))).collect();
            (format!("{} 项待办", todos.len()), list.join("\n"))
        }
        _ => {
            let first = input.as_object().and_then(|o| o.values().find_map(|v| v.as_str().map(str::to_owned))).unwrap_or_default();
            (first, serde_json::to_string_pretty(input).unwrap_or_default())
        }
    }
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(|b| match b["type"].as_str() {
                Some("text") => b["text"].as_str().map(str::to_owned),
                Some("image") => Some("[图片]".into()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Text a person typed (not tool plumbing, reminders or command echoes).
fn is_person_text(t: &str) -> bool {
    let t = t.trim_start();
    !(t.is_empty() || t.starts_with('<') || t.starts_with("Caveat:") || t.starts_with("[Request interrupted"))
}

pub fn claude_items(v: &Value, out: &mut Vec<Item>) {
    let kind = v["type"].as_str().unwrap_or_default();
    if (kind != "user" && kind != "assistant") || v["isMeta"].as_bool() == Some(true) || v["isSidechain"].as_bool() == Some(true) {
        return;
    }
    let content = &v["message"]["content"];
    let item = |k: &'static str, text: String| Item { off: 0, k, text, name: String::new(), detail: String::new(), id: String::new(), err: false };
    if let Some(text) = content.as_str() {
        if kind == "user" && is_person_text(text) {
            out.push(item("user", clip(text, TEXT_MAX)));
        } else if kind == "assistant" && !text.trim().is_empty() {
            out.push(item("assistant", clip(text, TEXT_MAX)));
        }
        return;
    }
    for b in content.as_array().into_iter().flatten() {
        match (kind, b["type"].as_str().unwrap_or_default()) {
            ("user", "text") => {
                let t = b["text"].as_str().unwrap_or_default();
                if is_person_text(t) {
                    out.push(item("user", clip(t, TEXT_MAX)));
                }
            }
            ("user", "image") => out.push(item("user", "[图片]".into())),
            ("user", "tool_result") => out.push(Item {
                off: 0,
                k: "result",
                text: clip(&result_text(&b["content"]), DETAIL_MAX),
                name: String::new(),
                detail: String::new(),
                id: b["tool_use_id"].as_str().unwrap_or_default().to_owned(),
                err: b["is_error"].as_bool().unwrap_or(false),
            }),
            ("assistant", "text") => {
                let t = b["text"].as_str().unwrap_or_default();
                if !t.trim().is_empty() {
                    out.push(item("assistant", clip(t, TEXT_MAX)));
                }
            }
            ("assistant", "tool_use") => {
                let name = b["name"].as_str().unwrap_or("tool").to_owned();
                let (text, detail) = tool_summary(&name, &b["input"]);
                out.push(Item {
                    off: 0,
                    k: "tool",
                    text: clip(&text, 300),
                    name,
                    detail: clip(&detail, DETAIL_MAX),
                    id: b["id"].as_str().unwrap_or_default().to_owned(),
                    err: false,
                });
            }
            _ => {}
        }
    }
}

fn codex_text(content: &Value) -> String {
    content.as_array().into_iter().flatten().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n")
}

fn join_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => Some(a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" ")),
        _ => None,
    }
}

/// Every `cmd:"…"` string literal in a JavaScript snippet (Codex's `exec` tool).
fn js_cmds(src: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = src;
    while let Some(i) = rest.find("cmd:") {
        let after = rest[i + 4..].trim_start();
        let mut chars = after.char_indices();
        let Some((_, q)) = chars.next().filter(|(_, c)| matches!(c, '"' | '\'' | '`')) else {
            rest = &rest[i + 4..];
            continue;
        };
        let mut s = String::new();
        let mut end = after.len();
        let mut escaped = false;
        for (j, c) in chars {
            if escaped {
                s.push(match c {
                    'n' => '\n',
                    't' => '\t',
                    c => c,
                });
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                end = j + 1;
                break;
            } else {
                s.push(c);
            }
        }
        out.push(s);
        rest = &after[end..];
    }
    out
}

/// A tool call's one-line summary: the command it runs, else its main argument.
fn codex_call_summary(raw: &str) -> String {
    let args: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    if args.is_object() {
        for k in ["cmd", "command", "path", "file_path", "query", "target", "url"] {
            if let Some(s) = join_str(&args[k]) {
                return s;
            }
        }
        return raw.chars().take(120).collect();
    }
    if raw.contains("*** Begin Patch") {
        let files: Vec<&str> = raw
            .lines()
            .filter_map(|l| l.strip_prefix("*** Update File: ").or(l.strip_prefix("*** Add File: ")).or(l.strip_prefix("*** Delete File: ")))
            .collect();
        return files.join(", ");
    }
    let cmds = js_cmds(raw);
    if !cmds.is_empty() {
        return cmds.join(" ; ");
    }
    raw.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_owned()
}

/// Tool output as text: a string, `{"output": …}`, or a list of text blocks.
fn codex_output(o: &Value) -> String {
    let parsed;
    let o = match o.as_str() {
        Some(s) => match serde_json::from_str::<Value>(s) {
            Ok(v) if v.is_object() || v.is_array() => {
                parsed = v;
                &parsed
            }
            _ => return s.to_owned(),
        },
        None => o,
    };
    let text = match o {
        Value::Array(a) => a.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join(""),
        Value::Object(_) => o["output"].as_str().map(str::to_owned).unwrap_or_else(|| o.to_string()),
        _ => String::new(),
    };
    // The exec tool's preamble says nothing the phone needs.
    let text = match text.strip_prefix("Script completed\n") {
        Some(rest) => rest.split_once("Output:\n").map_or(rest, |(_, r)| r).to_owned(),
        None => text,
    };
    // exec_command's own result: {"exit_code": …, "output": …}.
    match serde_json::from_str::<Value>(text.trim()) {
        Ok(v) if v["output"].is_string() => {
            let out = v["output"].as_str().unwrap_or_default();
            match v["exit_code"].as_i64() {
                Some(c) if c != 0 => format!("{out}\n[exit {c}]"),
                _ => out.to_owned(),
            }
        }
        _ => text,
    }
}

pub fn codex_items(v: &Value, out: &mut Vec<Item>) {
    if v["type"].as_str() != Some("response_item") {
        return;
    }
    let p = &v["payload"];
    let item = |k: &'static str, text: String| Item { off: 0, k, text, name: String::new(), detail: String::new(), id: String::new(), err: false };
    match p["type"].as_str().unwrap_or_default() {
        "message" => {
            let text = codex_text(&p["content"]);
            match p["role"].as_str() {
                // Instructions (AGENTS.md, environment) arrive as user messages too.
                Some("user") if is_person_text(&text) && !text.trim_start().starts_with("# AGENTS.md") => out.push(item("user", clip(&text, TEXT_MAX))),
                Some("assistant") if !text.trim().is_empty() => out.push(item("assistant", clip(&text, TEXT_MAX))),
                _ => {}
            }
        }
        "function_call" | "custom_tool_call" => {
            let name = p["name"].as_str().unwrap_or("tool").to_owned();
            let raw = p["arguments"].as_str().or(p["input"].as_str()).unwrap_or_default();
            out.push(Item {
                off: 0,
                k: "tool",
                text: clip(&codex_call_summary(raw), 300),
                name,
                detail: clip(raw, DETAIL_MAX),
                id: p["call_id"].as_str().unwrap_or_default().to_owned(),
                err: false,
            });
        }
        "function_call_output" | "custom_tool_call_output" => {
            let text = codex_output(&p["output"]);
            if text.trim().is_empty() {
                return;
            }
            out.push(Item {
                off: 0,
                k: "result",
                err: text.ends_with(']') && text.rsplit_once("\n[exit ").is_some(),
                text: clip(&text, DETAIL_MAX),
                name: String::new(),
                detail: String::new(),
                id: p["call_id"].as_str().unwrap_or_default().to_owned(),
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claude(lines: &[Value]) -> Vec<Item> {
        let mut out = vec![];
        for l in lines {
            claude_items(l, &mut out);
        }
        out
    }

    #[test]
    fn claude_messages_tools_and_results() {
        let items = claude(&[
            json!({"type": "user", "message": {"role": "user", "content": "把首页改成可筛选"}}),
            json!({"type": "user", "message": {"content": "<command-name>/clear</command-name>"}}),
            json!({"type": "assistant", "message": {"content": [{"type": "thinking", "thinking": "..."}, {"type": "text", "text": "好的，先看代码。"}]}}),
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Edit", "input": {"file_path": "/p/a.ts", "old_string": "a", "new_string": "b"}}]}}),
            json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok", "is_error": false}]}}),
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "npm test", "description": "Run tests"}}]}}),
            json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t2", "content": [{"type": "text", "text": "1 failed"}], "is_error": true}]}}),
            json!({"type": "api-request-blob", "message": {"content": [{"type": "text", "text": "x"}]}}),
            json!({"type": "user", "isMeta": true, "message": {"content": "meta"}}),
        ]);
        let kinds: Vec<&str> = items.iter().map(|i| i.k).collect();
        assert_eq!(kinds, ["user", "assistant", "tool", "result", "tool", "result"]);
        assert_eq!(items[2].name, "Edit");
        assert_eq!(items[2].text, "/p/a.ts");
        assert_eq!(items[2].detail, "- a\n+ b\n");
        assert_eq!(items[4].text, "Run tests");
        assert_eq!(items[4].detail, "npm test");
        assert!(items[5].err && items[5].id == "t2" && items[5].text == "1 failed");
    }

    #[test]
    fn codex_messages_and_calls() {
        let mut out = vec![];
        for l in [
            json!({"type": "session_meta", "payload": {"cwd": "/p"}}),
            json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "# AGENTS.md instructions for /p"}]}}),
            json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "修一下登录页"}]}}),
            json!({"type": "response_item", "payload": {"type": "reasoning"}}),
            json!({"type": "response_item", "payload": {"type": "function_call", "name": "exec_command", "arguments": "{\"cmd\":\"rg login\"}", "call_id": "c1"}}),
            json!({"type": "response_item", "payload": {"type": "function_call_output", "call_id": "c1", "output": "src/login.ts"}}),
            json!({"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "改好了"}]}}),
            json!({"type": "event_msg", "payload": {"type": "agent_message", "message": "改好了"}}),
        ] {
            codex_items(&l, &mut out);
        }
        let kinds: Vec<&str> = out.iter().map(|i| i.k).collect();
        assert_eq!(kinds, ["user", "tool", "result", "assistant"]);
        assert_eq!(out[1].text, "rg login");
        assert_eq!(out[2].id, "c1");
        let mut out = vec![];
        codex_items(
            &json!({"type": "response_item", "payload": {"type": "custom_tool_call", "name": "exec", "call_id": "c2",
                "input": "const r = await tools.exec_command({cmd:\"rg -n \\\"a b\\\" src\", yield_time_ms: 1000});\ntext(r)"}}),
            &mut out,
        );
        codex_items(
            &json!({"type": "response_item", "payload": {"type": "custom_tool_call_output", "call_id": "c2",
                "output": [{"type": "input_text", "text": "Script completed\nWall time 0.2 seconds\nOutput:\n"}, {"type": "input_text", "text": "{\"exit_code\":0,\"output\":\"src/x.rs:1\"}"}]}}),
            &mut out,
        );
        codex_items(&json!({"type": "response_item", "payload": {"type": "function_call_output", "call_id": "c3", "output": ""}}), &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "rg -n \"a b\" src");
        assert_eq!(out[1].text, "src/x.rs:1");
        assert_eq!(codex_call_summary("*** Begin Patch\n*** Update File: a.rs\n@@\n*** Add File: b.rs\n*** End Patch"), "a.rs, b.rs");
    }

    #[test]
    fn reads_incrementally_and_switches_to_a_new_session() {
        let home = std::env::temp_dir().join(format!("ddup-tr-{}", std::process::id()));
        let cwd = "/Users/x/my.proj";
        let dir = claude_dir(&home, cwd);
        fs::create_dir_all(&dir).unwrap();
        assert!(dir.ends_with("-Users-x-my-proj"));
        let a = dir.join("a.jsonl");
        fs::write(&a, "{\"type\":\"user\",\"message\":{\"content\":\"一\"}}\n{\"type\":\"user\",\"mess").unwrap();
        let p1 = read_in(&home, "claude-code", cwd, None, None, None, None).unwrap();
        assert!(p1.reset && p1.items.len() == 1, "{p1:?}");
        fs::write(&a, "{\"type\":\"user\",\"message\":{\"content\":\"一\"}}\n{\"type\":\"user\",\"message\":{\"content\":\"二\"}}\n").unwrap();
        let p2 = read_in(&home, "claude-code", cwd, None, Some(&p1.file), Some(p1.offset), None).unwrap();
        assert!(!p2.reset && p2.items.len() == 1 && p2.items[0].text == "二", "{p2:?}");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(dir.join("b.jsonl"), "{\"type\":\"user\",\"message\":{\"content\":\"新会话\"}}\n").unwrap();
        let p3 = read_in(&home, "claude-code", cwd, None, Some(&p2.file), Some(p2.offset), None).unwrap();
        assert!(p3.reset && p3.items[0].text == "新会话", "{p3:?}");
        // The process's own session wins over the newest file.
        fs::create_dir_all(home.join(".claude/sessions")).unwrap();
        fs::write(home.join(".claude/sessions/4242.json"), format!("{{\"pid\":4242,\"sessionId\":\"a\",\"cwd\":\"{cwd}\"}}")).unwrap();
        let p4 = read_in(&home, "claude-code", "/elsewhere", Some(4242), Some(&p3.file), Some(p3.offset), None).unwrap();
        assert!(p4.reset && p4.file.ends_with("/a.jsonl") && p4.items[0].text == "一", "{p4:?}");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn earlier_parts_by_line() {
        let home = std::env::temp_dir().join(format!("ddup-tr-old-{}", std::process::id()));
        let cwd = "/p/q";
        let dir = claude_dir(&home, cwd);
        fs::create_dir_all(&dir).unwrap();
        // Lines big enough that the first read leaves the beginning out.
        let pad = "x".repeat(4000);
        let mut log = String::new();
        for i in 0..1000 {
            log.push_str(&format!("{{\"type\":\"user\",\"message\":{{\"content\":\"m{i} {pad}\"}}}}\n"));
        }
        fs::write(dir.join("s.jsonl"), &log).unwrap();
        let p1 = read_in(&home, "claude-code", cwd, None, None, None, None).unwrap();
        assert!(p1.truncated && p1.start > 0 && p1.items.len() <= MAX_ITEMS);
        assert!(p1.items.last().unwrap().text.starts_with("m999 "));
        assert_eq!(p1.start, p1.items[0].off);
        let first = p1.items[0].text.split(' ').next().unwrap().trim_start_matches('m').parse::<usize>().unwrap();
        let p2 = read_in(&home, "claude-code", cwd, None, Some(&p1.file), Some(p1.offset), Some(p1.start)).unwrap();
        assert!(p2.older && !p2.reset);
        // Ends right before what was shown, no overlap, no gap.
        let last = p2.items.last().unwrap().text.split(' ').next().unwrap().trim_start_matches('m').parse::<usize>().unwrap();
        assert_eq!(last + 1, first);
        assert!(p2.start < p1.start);
        let _ = fs::remove_dir_all(&home);
    }
}
