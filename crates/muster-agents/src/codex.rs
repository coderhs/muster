//! OpenAI Codex CLI adapter (checked against codex-cli 0.139.0; see `docs/AGENTS.md`).
//!
//! Status comes from Codex's own rollout log
//! (`$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<local time>-<session id>.jsonl`), not from
//! hooks: Codex hooks must be trusted per hook (trust is stored in the user's
//! `config.toml`) or run with `--dangerously-bypass-hook-trust`, and in testing they did
//! not fire at all for an unauthenticated `codex exec`. The log carries turn start/end,
//! tool calls, token usage with the context window, and rate-limit windows.
//!
//! Codex cannot be given a session id; [`AgentAdapter::find_session`] finds the rollout a
//! launch created by working directory and start time.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::adapter::*;
use crate::chat::{self, secs_ms, Line};
use crate::claude::one_line;
use crate::tail::{contains, lines_after, ReverseLines};
use crate::types::*;

#[derive(Debug, Clone)]
pub struct Codex {
    pub program: String,
    /// `$CODEX_HOME` or `~/.codex`. Only read.
    pub home: PathBuf,
}

impl Default for Codex {
    fn default() -> Self {
        let home = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex")))
            .unwrap_or_else(|| PathBuf::from(".codex"));
        Codex { program: "codex".into(), home }
    }
}

/// TOML basic string for a `-c key=value` override.
fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Codex {
    fn common_args(&self, spec: &LaunchSpec) -> io::Result<Vec<String>> {
        let mut args = Vec::new();
        if let Some(f) = &spec.instructions {
            // Added as a developer message; verified with `codex debug prompt-input`.
            let text = std::fs::read_to_string(f)?;
            args.push("-c".into());
            args.push(format!("developer_instructions={}", toml_string(text.trim())));
        }
        args.extend(spec.extra_args.iter().cloned());
        Ok(args)
    }

    fn sessions_dir(&self) -> PathBuf {
        self.home.join("sessions")
    }

    /// Rollout files, newest day directories first (at most `days` directories).
    fn recent_rollouts(&self, days: usize) -> Vec<PathBuf> {
        let mut day_dirs = Vec::new();
        let list = |p: &Path| -> Vec<PathBuf> {
            let mut v: Vec<PathBuf> =
                std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            v.sort();
            v.reverse();
            v
        };
        'outer: for y in list(&self.sessions_dir()) {
            for m in list(&y) {
                for d in list(&m) {
                    day_dirs.push(d);
                    if day_dirs.len() >= days {
                        break 'outer;
                    }
                }
            }
        }
        let mut files = Vec::new();
        for d in day_dirs {
            files.extend(
                std::fs::read_dir(d)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "jsonl")),
            );
        }
        files
    }
}

/// Session id from a rollout file name (`rollout-<time>-<uuid>.jsonl`).
pub fn session_id_from_path(p: &Path) -> Option<String> {
    let stem = p.file_stem()?.to_str()?;
    let id = stem.get(stem.len().checked_sub(36)?..)?;
    let b = id.as_bytes();
    let shaped =
        b.iter().enumerate().all(
            |(i, c)| {
                if [8, 13, 18, 23].contains(&i) {
                    *c == b'-'
                } else {
                    c.is_ascii_hexdigit()
                }
            },
        );
    shaped.then(|| id.to_string())
}

/// Session id of a rollout: from its name, else from its `session_meta` line.
pub fn rollout_session_id(p: &Path) -> Option<String> {
    session_id_from_path(p).or_else(|| first_line(p)?.pointer("/payload/id")?.as_str().map(str::to_string))
}

fn first_line(p: &Path) -> Option<Value> {
    use std::io::{BufRead, Read};
    let f = std::fs::File::open(p).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(f.take(1 << 20)).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

impl AgentAdapter for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn label(&self) -> &'static str {
        "Codex"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            hooks: false,
            preset_session_id: false,
            resume: true,
            answer_permissions: false,
            transcript: true,
            context_usage: true,
            rate_limits: true,
            status_source: StatusSource::SessionLog,
        }
    }

    fn installed(&self) -> bool {
        find_on_path(&self.program).is_some()
    }

    fn launch(&self, spec: &LaunchSpec) -> io::Result<CommandSpec> {
        let mut args = self.common_args(spec)?;
        if let Some(p) = spec.prompt.as_ref().filter(|p| !p.trim().is_empty()) {
            args.push("--".into());
            args.push(p.clone());
        }
        Ok(CommandSpec {
            program: spec.program.clone().unwrap_or_else(|| self.program.clone()),
            args,
            env: Vec::new(),
            cwd: spec.cwd.clone(),
        })
    }

    fn resume(&self, spec: &LaunchSpec, session_id: &str) -> io::Result<CommandSpec> {
        // `codex resume [OPTIONS] [SESSION_ID] [PROMPT]`
        let mut args = vec!["resume".to_string()];
        args.extend(self.common_args(spec)?);
        args.push(session_id.to_string());
        if let Some(p) = spec.prompt.as_ref().filter(|p| !p.trim().is_empty()) {
            args.push(p.clone());
        }
        Ok(CommandSpec {
            program: spec.program.clone().unwrap_or_else(|| self.program.clone()),
            args,
            env: Vec::new(),
            cwd: spec.cwd.clone(),
        })
    }

    fn find_session(&self, cwd: &Path, started_after_ms: u64) -> Option<SessionRef> {
        let cwd = cwd.to_string_lossy();
        let mut best: Option<(u64, PathBuf, String)> = None;
        for p in self.recent_rollouts(2) {
            let created = std::fs::metadata(&p)
                .and_then(|m| m.created().or_else(|_| m.modified()))
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            // Small slack for clock granularity.
            if created + 2000 < started_after_ms {
                continue;
            }
            let Some(meta) = first_line(&p) else { continue };
            if meta.get("type").and_then(Value::as_str) != Some("session_meta") {
                continue;
            }
            let pl = &meta["payload"];
            if pl.get("cwd").and_then(Value::as_str) != Some(cwd.as_ref()) {
                continue;
            }
            let id = pl.get("id").and_then(Value::as_str).map(str::to_string).or_else(|| session_id_from_path(&p));
            let Some(id) = id else { continue };
            if best.as_ref().is_none_or(|b| created < b.0) {
                best = Some((created, p, id));
            }
        }
        best.map(|(_, p, id)| SessionRef { session_id: id, transcript: Some(p) })
    }

    fn transcript_root(&self) -> Option<PathBuf> {
        Some(self.sessions_dir())
    }

    fn transcript_path(&self, session_id: &str, _cwd: &Path) -> Option<PathBuf> {
        let suffix = format!("-{session_id}.jsonl");
        // Recent days first, then everything.
        self.recent_rollouts(7)
            .into_iter()
            .find(|p| p.to_string_lossy().ends_with(&suffix))
            .or_else(|| self.recent_rollouts(usize::MAX).into_iter().find(|p| p.to_string_lossy().ends_with(&suffix)))
    }

    fn read_chat(&self, transcript: &Path, before: Option<u64>, max_messages: usize) -> io::Result<ChatPage> {
        chat::read_page(transcript, before, max_messages, &parse_chat_line)
    }

    fn read_chat_since(&self, transcript: &Path, after: u64, max_messages: usize) -> io::Result<ChatPage> {
        chat::read_since(transcript, after, max_messages, &parse_chat_line)
    }

    fn context_usage(&self, transcript: &Path) -> io::Result<Option<ContextUsage>> {
        let mut r = ReverseLines::open(transcript, None)?;
        while let Some((_, line)) = r.next_line()? {
            if r.bytes_read > chat::SCAN_BUDGET {
                break;
            }
            if contains(&line, b"\"token_count\"") {
                let Ok(v) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                if let Some(u) = usage_from_token_count(&v["payload"]).context {
                    return Ok(Some(u));
                }
            }
        }
        Ok(None)
    }

    fn turn_state(&self, transcript: &Path) -> io::Result<TurnState> {
        let mut r = ReverseLines::open(transcript, None)?;
        while let Some((_, line)) = r.next_line()? {
            if r.bytes_read > chat::SCAN_BUDGET {
                break;
            }
            if contains(&line, b"\"task_complete\"") {
                return Ok(TurnState::Ended);
            }
            if contains(&line, b"\"turn_aborted\"") {
                return Ok(TurnState::Interrupted);
            }
            if contains(&line, b"\"task_started\"") {
                return Ok(TurnState::InProgress);
            }
        }
        Ok(TurnState::Unknown)
    }

    fn log_events(&self, card_id: &str, log: &Path, after: u64) -> io::Result<(Vec<NormalizedEvent>, u64)> {
        let (lines, next) = lines_after(log, after, chat::SCAN_BUDGET)?;
        let sid = rollout_session_id(log);
        let mut out = Vec::new();
        let now = now_ms();
        for (_, line) in lines {
            let Ok(v) = serde_json::from_slice::<Value>(&line) else {
                continue;
            };
            let Some(kind) = log_event(&v) else { continue };
            out.push(NormalizedEvent {
                card_id: card_id.to_string(),
                agent_id: "codex".into(),
                session_id: sid.clone(),
                transcript_path: Some(log.to_string_lossy().into_owned()),
                cwd: None,
                subagent_id: None,
                at_ms: now,
                kind,
            });
        }
        Ok((out, next))
    }
}

/// One rollout line → status event, if it is one.
pub fn log_event(v: &Value) -> Option<AgentEvent> {
    let t = v.get("type")?.as_str()?;
    let p = v.get("payload")?;
    let pt = p.get("type").and_then(Value::as_str).unwrap_or("");
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    Some(match (t, pt) {
        ("session_meta", _) => AgentEvent::SessionStarted { source: SessionSource::Startup, model: None },
        ("event_msg", "task_started") => AgentEvent::TurnStarted { prompt: None },
        ("event_msg", "task_complete") => {
            AgentEvent::TurnFinished { last_message: s("last_agent_message"), error: None, background_tasks: 0 }
        }
        ("event_msg", "turn_aborted") => AgentEvent::TurnFinished {
            last_message: None,
            error: Some(format!("aborted: {}", s("reason").unwrap_or_default())),
            background_tasks: 0,
        },
        ("event_msg", "token_count") => AgentEvent::Usage { usage: usage_from_token_count(p) },
        // Written at the start of every turn, so a `/model` switch shows up on the next one.
        ("turn_context", _) => AgentEvent::Usage {
            usage: UsageSnapshot {
                context: None,
                rate_limits: None,
                model_id: Some(s("model")?),
                model_name: None,
                session_name: None,
                cost_usd: None,
                agent_version: None,
            },
        },
        ("response_item", "function_call" | "custom_tool_call" | "web_search_call" | "tool_search_call") => {
            let name = s("name").unwrap_or_else(|| pt.trim_end_matches("_call").to_string());
            AgentEvent::ToolStarted { summary: call_summary(&name, p), tool: name, tool_use_id: s("call_id") }
        }
        ("response_item", "function_call_output" | "custom_tool_call_output" | "tool_search_output") => {
            AgentEvent::ToolFinished { tool: String::new(), tool_use_id: s("call_id"), ok: true, interrupted: false }
        }
        ("event_msg", "user_message" | "agent_message") => AgentEvent::Other { event: pt.to_string() },
        _ => return None,
    })
}

fn window(name: &str, w: &Value) -> Option<RateWindow> {
    Some(RateWindow {
        name: name.to_string(),
        window_minutes: w.get("window_minutes").and_then(Value::as_u64),
        used_percent: w.get("used_percent")?.as_f64()?,
        resets_at_ms: secs_ms(w.get("resets_at")),
    })
}

/// `token_count` payload → usage snapshot (context and rate limits).
pub fn usage_from_token_count(p: &Value) -> UsageSnapshot {
    let info = &p["info"];
    let last = &info["last_token_usage"];
    let context = last.get("input_tokens").and_then(Value::as_u64).map(|input| {
        // Codex reports `cached_input_tokens` as part of `input_tokens`.
        let used = input + last.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
        let window = info.get("model_context_window").and_then(Value::as_u64);
        ContextUsage {
            used_tokens: used,
            window_tokens: window,
            used_percent: window.filter(|w| *w > 0).map(|w| used as f64 * 100.0 / w as f64),
            source: UsageSource::Transcript,
        }
    });
    let rate_limits = p.get("rate_limits").filter(|r| r.is_object()).map(|r| {
        let windows: Vec<RateWindow> = ["primary", "secondary"]
            .iter()
            .filter_map(|n| r.get(*n).filter(|w| w.is_object()).and_then(|w| window(n, w)))
            .collect();
        let pick = |mins: u64| {
            windows
                .iter()
                .find(|w| w.window_minutes == Some(mins))
                .map(|w| LimitWindow { used_percent: w.used_percent, resets_at_ms: w.resets_at_ms })
        };
        RateLimits { five_hour: pick(300), seven_day: pick(10080), spend: None, windows: windows.clone() }
    });
    UsageSnapshot {
        context,
        rate_limits,
        model_id: None,
        model_name: None,
        session_name: None,
        cost_usd: None,
        agent_version: None,
    }
}

fn call_summary(name: &str, p: &Value) -> String {
    let args: Value =
        p.get("arguments").and_then(Value::as_str).and_then(|a| serde_json::from_str(a).ok()).unwrap_or(Value::Null);
    let detail = args
        .get("cmd")
        .or_else(|| args.get("command"))
        .map(|c| match c {
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .or_else(|| p.get("input").and_then(Value::as_str).map(str::to_string))
        .or_else(|| p.pointer("/action/query").and_then(Value::as_str).map(str::to_string));
    match detail {
        Some(d) if !d.is_empty() => format!("{name}: {}", one_line(&d, 160)),
        _ => name.to_string(),
    }
}

fn is_context_text(t: &str) -> bool {
    let t = t.trim_start();
    [
        "<environment_context>",
        "<permissions instructions>",
        "<user_instructions>",
        "# AGENTS.md",
        "<INSTRUCTIONS>",
        "<turn_aborted>",
    ]
    .iter()
    .any(|p| t.starts_with(p))
}

fn parse_chat_line(line: &[u8], off: u64) -> Option<Line> {
    if !contains(line, b"\"response_item\"") {
        return None;
    }
    let v: Value = serde_json::from_slice(line).ok()?;
    let p = v.get("payload")?;
    let ts = v.get("timestamp").and_then(Value::as_str).map(str::to_string);
    let pt = p.get("type")?.as_str()?;
    let key = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("o{off}"));
    let mut parts = Vec::new();
    let role = match pt {
        "message" => {
            let role = match p.get("role").and_then(Value::as_str) {
                Some("user") => ChatRole::User,
                Some("assistant") => ChatRole::Assistant,
                _ => return None, // developer / system context
            };
            for c in p.get("content").and_then(Value::as_array).into_iter().flatten() {
                match c.get("type").and_then(Value::as_str) {
                    Some("input_text" | "output_text") => {
                        let t = c.get("text").and_then(Value::as_str).unwrap_or_default();
                        if !t.trim().is_empty() && !is_context_text(t) {
                            parts.push(ChatPart::Text { text: t.to_string() });
                        }
                    }
                    Some("input_image") => parts.push(ChatPart::Image),
                    _ => {}
                }
            }
            role
        }
        "reasoning" => {
            parts.push(ChatPart::Thinking);
            ChatRole::Assistant
        }
        "function_call" | "custom_tool_call" | "web_search_call" | "tool_search_call" => {
            let name = p
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| pt.trim_end_matches("_call").to_string());
            parts.push(ChatPart::ToolCall { id: key("call_id"), summary: call_summary(&name, p), name });
            ChatRole::Assistant
        }
        "function_call_output" | "custom_tool_call_output" | "tool_search_output" => {
            let out = match p.get("output") {
                Some(Value::String(s)) => s.clone(),
                Some(o) => {
                    o.get("content").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| o.to_string())
                }
                None => String::new(),
            };
            parts.push(ChatPart::ToolResult {
                tool_use_id: key("call_id"),
                is_error: false,
                summary: one_line(&out, 200),
            });
            ChatRole::Tool
        }
        _ => return None,
    };
    // Consecutive assistant items (reasoning, calls, text) of one turn merge by role.
    let key = match role {
        ChatRole::Assistant => "assistant-run".to_string(),
        _ => format!("o{off}"),
    };
    Some(Line { key, role, timestamp: ts, model: None, parts })
}

/// Arguments for an opt-in per-launch hook set (`-c hooks.<Event>=[...]`). Not used by
/// default: Codex skips hooks the user has not trusted in `/hooks` unless
/// `--dangerously-bypass-hook-trust` is passed, which also stops it re-checking every
/// other hook. Kept for when the user decides to allow that (see docs/AGENTS.md).
pub fn inline_hook_args(hook_bin: &Path, events: &[&str]) -> Vec<String> {
    let bin = hook_bin.to_string_lossy();
    events
        .iter()
        .flat_map(|e| {
            let cmd = format!("{} hook {e}", crate::types::shell_quote(&bin));
            let v = json!([{ "hooks": [{ "type": "command", "command": cmd, "timeout": 10 }] }]);
            ["-c".to_string(), format!("hooks.{e}={}", toml_inline(&v))]
        })
        .collect()
}

fn toml_inline(v: &Value) -> String {
    match v {
        Value::Array(a) => format!("[{}]", a.iter().map(toml_inline).collect::<Vec<_>>().join(",")),
        Value::Object(o) => {
            format!("{{{}}}", o.iter().map(|(k, v)| format!("{k}={}", toml_inline(v))).collect::<Vec<_>>().join(","))
        }
        Value::String(s) => toml_string(s),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_strings_and_inline_tables() {
        assert_eq!(toml_string("a\"b\\c\nd"), r#""a\"b\\c\nd""#);
        let a = inline_hook_args(Path::new("/opt/muster-hook"), &["Stop"]);
        assert_eq!(a[0], "-c");
        assert_eq!(a[1], r#"hooks.Stop=[{hooks=[{command="/opt/muster-hook hook Stop",timeout=10,type="command"}]}]"#);
    }

    #[test]
    fn launch_and_resume() {
        let c = Codex { program: "codex".into(), home: "/nonexistent".into() };
        let dir = std::env::temp_dir().join(format!("muster-codex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("i.md");
        std::fs::write(&f, "Use \"muster\".\n").unwrap();
        let mut spec = LaunchSpec::new("c1", "/w");
        spec.prompt = Some("-x".into());
        spec.instructions = Some(f);
        let l = c.launch(&spec).unwrap();
        assert_eq!(l.args, ["-c", r#"developer_instructions="Use \"muster\".""#, "--", "-x"]);
        let r = c.resume(&spec, "abc").unwrap();
        assert_eq!(r.args[0], "resume");
        assert_eq!(&r.args[3..], ["abc", "-x"]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
