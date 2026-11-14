//! Cursor Agent CLI adapter (checked against cursor-agent 2026.11.05; see
//! `docs/AGENTS.md`).
//!
//! Hooks are supplied per launch through `--plugin-dir <dir>`: Muster generates a small
//! plugin (`.cursor-plugin/plugin.json` + `hooks/hooks.json`) in its own state directory,
//! so `~/.cursor/hooks.json` is never touched. Standing instructions travel the same
//! way, as `additional_context` printed by the plugin's `sessionStart` hook (plugin
//! `rules/` were not picked up by the CLI in testing).
//!
//! The session id comes from the first hook event (`conversation_id`). A chat can be
//! created up front with [`Cursor::create_chat`] (`cursor-agent create-chat`) and
//! started with `--resume <id>`, but then `sessionStart` does not fire, so the standing
//! instructions are not delivered; the default launch therefore lets Cursor pick the id.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use crate::adapter::*;
use crate::chat::{self, Line};
use crate::claude::one_line;
use crate::tail::contains;
use crate::types::*;

/// Events the generated plugin subscribes to. None of them waits for an answer.
pub const HOOK_EVENTS: &[&str] = &[
    "sessionStart",
    "sessionEnd",
    "beforeSubmitPrompt",
    "preToolUse",
    "postToolUse",
    "postToolUseFailure",
    "subagentStart",
    "subagentStop",
    "preCompact",
    "stop",
    "afterAgentResponse",
];

#[derive(Debug, Clone)]
pub struct Cursor {
    pub program: String,
    /// `~/.cursor`. Only read (transcripts).
    pub home: PathBuf,
}

impl Default for Cursor {
    fn default() -> Self {
        let home = std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".cursor"))
            .unwrap_or_else(|| PathBuf::from(".cursor"));
        Cursor { program: "cursor-agent".into(), home }
    }
}

fn fnv1a(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf29ce484222325u64, |h, &c| (h ^ c as u64).wrapping_mul(0x100000001b3))
}

fn write_if_changed(path: &Path, content: &str) -> io::Result<()> {
    if std::fs::read_to_string(path).ok().as_deref() != Some(content) {
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&tmp, content)?;
        std::fs::rename(tmp, path)?;
    }
    Ok(())
}

impl Cursor {
    /// Runs `cursor-agent create-chat` in `cwd` (local, no model request) and returns the
    /// new chat id. See the module docs for why the default launch does not use it.
    pub fn create_chat(&self, cwd: &Path) -> Option<String> {
        let out = Command::new(&self.program).arg("create-chat").current_dir(cwd).output().ok()?;
        let id = String::from_utf8(out.stdout).ok()?.trim().to_string();
        (out.status.success() && !id.is_empty() && !id.contains(char::is_whitespace)).then_some(id)
    }

    /// Generate (if changed) the per-launch plugin and return its directory.
    pub fn write_plugin(&self, hooks: &HookConfig, instructions: Option<&Path>) -> io::Result<PathBuf> {
        let bin = crate::types::shell_quote(&hooks.hook_bin.to_string_lossy());
        let context = match instructions {
            Some(f) => Some(json!({ "additional_context": std::fs::read_to_string(f)? }).to_string()),
            None => None,
        };
        let mut hook_map = serde_json::Map::new();
        for &ev in HOOK_EVENTS {
            hook_map.insert(ev.into(), json!([{ "command": format!("{bin} hook {ev}"), "timeout": 10 }]));
        }
        let key = format!("{bin}|{}", context.as_deref().unwrap_or(""));
        let dir = hooks.state_dir.join("cursor").join(format!("plugin-{:016x}", fnv1a(key.as_bytes())));
        std::fs::create_dir_all(dir.join(".cursor-plugin"))?;
        std::fs::create_dir_all(dir.join("hooks"))?;
        if let Some(ctx) = &context {
            let ctx_file = dir.join("session-context.json");
            write_if_changed(&ctx_file, ctx)?;
            let cmd = format!("cat >/dev/null; cat {}", crate::types::shell_quote(&ctx_file.to_string_lossy()));
            hook_map["sessionStart"].as_array_mut().unwrap().push(json!({ "command": cmd, "timeout": 10 }));
        }
        write_if_changed(
            &dir.join(".cursor-plugin").join("plugin.json"),
            &json!({ "name": "muster", "description": "Muster board integration", "version": "1" }).to_string(),
        )?;
        write_if_changed(
            &dir.join("hooks").join("hooks.json"),
            &serde_json::to_string_pretty(&json!({ "version": 1, "hooks": Value::Object(hook_map) })).unwrap(),
        )?;
        Ok(dir)
    }

    fn command(&self, spec: &LaunchSpec, session: Option<&str>) -> io::Result<CommandSpec> {
        let mut args = Vec::new();
        let mut env = Vec::new();
        if let Some(h) = &spec.hooks {
            let dir = self.write_plugin(h, spec.instructions.as_deref())?;
            args.extend(["--plugin-dir".into(), dir.to_string_lossy().into_owned()]);
            env.push(("MUSTER_CARD_ID".into(), spec.card_id.clone()));
            env.push(("MUSTER_AGENT_ID".into(), "cursor".into()));
            env.push(("MUSTER_HOOK_SOCK".into(), h.sock.to_string_lossy().into_owned()));
            if let Some(secret) = &spec.hook_secret {
                env.push(("MUSTER_HOOK_SECRET".into(), secret.clone()));
            }
        }
        if let Some(sid) = session {
            args.extend(["--resume".into(), sid.to_string()]);
        }
        args.extend(spec.extra_args.iter().cloned());
        if let Some(p) = spec.prompt.as_ref().filter(|p| !p.trim().is_empty()) {
            args.push("--".into());
            args.push(p.clone());
        }
        Ok(CommandSpec {
            program: spec.program.clone().unwrap_or_else(|| self.program.clone()),
            args,
            env,
            cwd: spec.cwd.clone(),
        })
    }
}

impl AgentAdapter for Cursor {
    fn id(&self) -> &'static str {
        "cursor"
    }

    fn label(&self) -> &'static str {
        "Cursor"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            hooks: true,
            preset_session_id: false,
            resume: true,
            answer_permissions: false,
            transcript: true,
            context_usage: true,
            rate_limits: false,
            status_source: StatusSource::Hooks,
        }
    }

    fn installed(&self) -> bool {
        find_on_path(&self.program).is_some()
    }

    fn launch(&self, spec: &LaunchSpec) -> io::Result<CommandSpec> {
        self.command(spec, spec.session_id.as_deref())
    }

    fn resume(&self, spec: &LaunchSpec, session_id: &str) -> io::Result<CommandSpec> {
        self.command(spec, Some(session_id))
    }

    fn normalize(&self, env: &HookEnvelope, at_ms: u64) -> Vec<NormalizedEvent> {
        normalize(env, at_ms)
    }

    fn transcript_root(&self) -> Option<PathBuf> {
        Some(self.home.join("projects"))
    }

    fn transcript_path(&self, session_id: &str, _cwd: &Path) -> Option<PathBuf> {
        // `projects/<workspace path with '/' as '-'>/agent-transcripts/<id>/<id>.jsonl`;
        // the workspace is the git root in interactive runs, so scan.
        let rel = PathBuf::from("agent-transcripts").join(session_id).join(format!("{session_id}.jsonl"));
        std::fs::read_dir(self.home.join("projects")).ok()?.flatten().map(|e| e.path().join(&rel)).find(|p| p.is_file())
    }

    fn read_chat(&self, transcript: &Path, before: Option<u64>, max_messages: usize) -> io::Result<ChatPage> {
        chat::read_page(transcript, before, max_messages, &parse_chat_line)
    }

    fn read_chat_since(&self, transcript: &Path, after: u64, max_messages: usize) -> io::Result<ChatPage> {
        chat::read_since(transcript, after, max_messages, &parse_chat_line)
    }

    fn turn_state(&self, transcript: &Path) -> io::Result<TurnState> {
        let mut r = crate::tail::ReverseLines::open(transcript, None)?;
        while let Some((_, line)) = r.next_line()? {
            if contains(&line, b"\"turn_ended\"") {
                return Ok(if contains(&line, b"\"success\"") { TurnState::Ended } else { TurnState::Interrupted });
            }
            if contains(&line, b"\"role\"") {
                return Ok(TurnState::InProgress);
            }
        }
        Ok(TurnState::Unknown)
    }
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

fn tool_summary(tool: &str, input: &Value) -> String {
    let d = ["command", "file_path", "path", "pattern", "query", "url", "task"]
        .iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str));
    match d {
        Some(d) => format!("{tool}: {}", one_line(d, 160)),
        None => tool.to_string(),
    }
}

pub(crate) fn normalize(env: &HookEnvelope, at_ms: u64) -> Vec<NormalizedEvent> {
    let p = &env.payload;
    let name = s(p, "hook_event_name").unwrap_or_else(|| env.event.clone());
    let tool = || s(p, "tool_name").unwrap_or_default();
    let input = || p.get("tool_input").cloned().unwrap_or(Value::Null);
    let kind = match name.as_str() {
        "sessionStart" => AgentEvent::SessionStarted { source: SessionSource::Startup, model: s(p, "model") },
        "sessionEnd" => {
            AgentEvent::SessionEnded { reason: s(p, "reason").unwrap_or_else(|| "other".into()), exited: true }
        }
        "beforeSubmitPrompt" => AgentEvent::TurnStarted { prompt: s(p, "prompt") },
        "preToolUse" => AgentEvent::ToolStarted {
            summary: tool_summary(&tool(), &input()),
            tool: tool(),
            tool_use_id: s(p, "tool_use_id"),
        },
        "postToolUse" => {
            AgentEvent::ToolFinished { tool: tool(), tool_use_id: s(p, "tool_use_id"), ok: true, interrupted: false }
        }
        "postToolUseFailure" => AgentEvent::ToolFinished {
            tool: tool(),
            tool_use_id: s(p, "tool_use_id"),
            ok: false,
            interrupted: p.get("is_interrupt").and_then(Value::as_bool).unwrap_or(false),
        },
        "subagentStart" => AgentEvent::SubagentStarted {
            agent_id: s(p, "subagent_id").unwrap_or_default(),
            agent_type: s(p, "subagent_type"),
        },
        "subagentStop" => AgentEvent::SubagentStopped {
            agent_id: s(p, "subagent_id").unwrap_or_default(),
            agent_type: s(p, "subagent_type"),
            transcript_path: s(p, "agent_transcript_path"),
        },
        "preCompact" => AgentEvent::CompactionStarted { trigger: s(p, "trigger").unwrap_or_default() },
        "stop" => {
            let status = s(p, "status").unwrap_or_else(|| "completed".into());
            AgentEvent::TurnFinished {
                last_message: None,
                error: (status != "completed").then_some(status),
                background_tasks: 0,
            }
        }
        other => AgentEvent::Other { event: other.to_string() },
    };
    let base = |kind: AgentEvent| NormalizedEvent {
        card_id: env.card_id.clone(),
        agent_id: "cursor".into(),
        session_id: s(p, "conversation_id").or_else(|| s(p, "session_id")),
        transcript_path: s(p, "transcript_path"),
        cwd: s(p, "cwd"),
        subagent_id: None,
        at_ms,
        kind,
    };
    let mut out = vec![base(kind)];
    // `stop` and `preCompact` carry token counts.
    let context = match name.as_str() {
        "stop" => p.get("input_tokens").and_then(Value::as_u64).map(|t| ContextUsage {
            used_tokens: t,
            window_tokens: None,
            used_percent: None,
            source: UsageSource::Transcript,
        }),
        "preCompact" => p.get("context_tokens").and_then(Value::as_u64).map(|t| ContextUsage {
            used_tokens: t,
            window_tokens: p.get("context_window_size").and_then(Value::as_u64),
            used_percent: p.get("context_usage_percent").and_then(Value::as_f64),
            source: UsageSource::Transcript,
        }),
        _ => None,
    };
    if let Some(c) = context {
        out.push(base(AgentEvent::Usage {
            usage: UsageSnapshot {
                context: Some(c),
                rate_limits: None,
                model_id: s(p, "model"),
                model_name: None,
                session_name: None,
                cost_usd: None,
                agent_version: s(p, "cursor_version"),
            },
        }));
    }
    out
}

fn strip_user_wrapper(t: &str) -> String {
    let mut t = t.trim().to_string();
    if t.starts_with("<timestamp>") {
        if let Some(i) = t.find("</timestamp>") {
            t = t[i + "</timestamp>".len()..].trim().to_string();
        }
    }
    if let (Some(a), Some(b)) = (t.find("<user_query>"), t.rfind("</user_query>")) {
        if a < b {
            t = t[a + "<user_query>".len()..b].trim().to_string();
        }
    }
    t
}

fn parse_chat_line(line: &[u8], off: u64) -> Option<Line> {
    if !contains(line, b"\"role\"") {
        return None;
    }
    let v: Value = serde_json::from_slice(line).ok()?;
    let role = match v.get("role")?.as_str()? {
        "user" => ChatRole::User,
        "assistant" => ChatRole::Assistant,
        _ => return None,
    };
    let mut parts = Vec::new();
    for c in v.pointer("/message/content").and_then(Value::as_array).into_iter().flatten() {
        if c.get("type").and_then(Value::as_str) == Some("text") {
            let raw = c.get("text").and_then(Value::as_str).unwrap_or_default();
            let text = if role == ChatRole::User { strip_user_wrapper(raw) } else { raw.trim().to_string() };
            if !text.is_empty() {
                parts.push(ChatPart::Text { text });
            }
        }
    }
    Some(Line { key: format!("o{off}"), role, timestamp: None, model: None, parts })
}
