//! Claude Code hook settings and event normalisation.

use std::path::Path;

use serde_json::{json, Map, Value};

use super::{tool_summary, usage::parse_statusline};
use crate::adapter::HookConfig;
use crate::types::*;

/// Events Muster subscribes to. All are forwarded without waiting except
/// `PermissionRequest`, which waits for the board's answer.
pub const HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "PermissionDenied",
    "Notification",
    "Stop",
    "StopFailure",
    "SubagentStart",
    "SubagentStop",
    "PreCompact",
    "PostCompact",
    "Elicitation",
    "ElicitationResult",
];

/// Seconds Claude Code waits for a non-blocking forward before cancelling it. The hook
/// itself finishes in milliseconds; this only bounds a pathological case.
const FORWARD_TIMEOUT_SECS: u64 = 10;
/// Extra seconds on top of the approval wait before Claude Code cancels the hook.
const APPROVAL_TIMEOUT_SLACK_SECS: u64 = 30;

/// The settings object passed with `--settings`. Uses the exec form (`command` + `args`)
/// so no shell is spawned per event.
pub fn settings_json(h: &HookConfig, user_statusline: Option<&Value>) -> Value {
    let bin = h.hook_bin.to_string_lossy().into_owned();
    let mut hooks = Map::new();
    for &ev in HOOK_EVENTS {
        let (args, timeout) = if ev == "PermissionRequest" {
            let wait = h.approval_wait.as_secs().max(1);
            (json!(["hook", ev, "--wait", wait.to_string()]), wait + APPROVAL_TIMEOUT_SLACK_SECS)
        } else {
            (json!(["hook", ev]), FORWARD_TIMEOUT_SECS)
        };
        hooks.insert(
            ev.to_string(),
            json!([{ "hooks": [{ "type": "command", "command": bin, "args": args, "timeout": timeout }] }]),
        );
    }
    let mut settings = json!({ "hooks": Value::Object(hooks) });
    if h.capture_statusline {
        let mut sl = json!({
            "type": "command",
            "command": format!("{} statusline", crate::types::shell_quote(&bin)),
        });
        // Keep the look of the user's own status line.
        for key in ["padding", "refreshInterval", "hideVimModeIndicator"] {
            if let Some(v) = user_statusline.and_then(|u| u.get(key)) {
                sl[key] = v.clone();
            }
        }
        settings["statusLine"] = sl;
    }
    settings
}

/// The user's effective `statusLine` setting (command type only), read from the same
/// files Claude Code reads, highest precedence first: project local, project, user.
/// Managed settings are not consulted: a managed status line overrides ours anyway.
pub fn user_statusline_command(config_dir: &Path, cwd: &Path) -> Option<Value> {
    let candidates = [
        cwd.join(".claude").join("settings.local.json"),
        cwd.join(".claude").join("settings.json"),
        config_dir.join("settings.json"),
    ];
    candidates.iter().find_map(|p| {
        let text = std::fs::read_to_string(p).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        let sl = v.get("statusLine")?;
        (sl.get("type").and_then(Value::as_str) == Some("command")
            && sl.get("command").and_then(Value::as_str).is_some())
        .then(|| sl.clone())
    })
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

pub(super) fn normalize(env: &HookEnvelope, at_ms: u64) -> Vec<NormalizedEvent> {
    let p = &env.payload;
    let base = |kind: AgentEvent| NormalizedEvent {
        card_id: env.card_id.clone(),
        agent_id: if env.agent_id.is_empty() { "claude".into() } else { env.agent_id.clone() },
        session_id: s(p, "session_id"),
        transcript_path: s(p, "transcript_path"),
        cwd: s(p, "cwd"),
        subagent_id: s(p, "agent_id"),
        at_ms,
        kind,
    };

    if env.kind == EnvelopeKind::Statusline {
        if p.is_null() {
            return Vec::new();
        }
        let mut ev = base(AgentEvent::Usage { usage: parse_statusline(p) });
        ev.subagent_id = None;
        return vec![ev];
    }

    let name = s(p, "hook_event_name").unwrap_or_else(|| env.event.clone());
    let tool = || s(p, "tool_name").unwrap_or_default();
    let input = || p.get("tool_input").cloned().unwrap_or(Value::Null);

    let kind = match name.as_str() {
        "SessionStart" => AgentEvent::SessionStarted {
            source: match p.get("source").and_then(Value::as_str) {
                Some("startup") => SessionSource::Startup,
                Some("resume") => SessionSource::Resume,
                Some("clear") => SessionSource::Clear,
                Some("compact") => SessionSource::Compact,
                Some("fork") => SessionSource::Fork,
                _ => SessionSource::Other,
            },
            model: match p.get("model") {
                Some(Value::String(m)) => Some(m.clone()),
                Some(Value::Object(o)) => o.get("id").and_then(Value::as_str).map(str::to_string),
                _ => None,
            },
        },
        "SessionEnd" => {
            let reason = s(p, "reason").unwrap_or_else(|| "other".into());
            // `clear` and `resume` (switching sessions with /resume) keep the process.
            let exited = !matches!(reason.as_str(), "clear" | "resume");
            AgentEvent::SessionEnded { reason, exited }
        }
        "UserPromptSubmit" => AgentEvent::TurnStarted { prompt: s(p, "prompt") },
        "PreToolUse" if tool() == "AskUserQuestion" => {
            AgentEvent::QuestionAsked { tool_use_id: s(p, "tool_use_id"), questions: parse_questions(&input()) }
        }
        "PreToolUse" => AgentEvent::ToolStarted {
            tool: tool(),
            tool_use_id: s(p, "tool_use_id"),
            summary: tool_summary(&tool(), &input()),
        },
        "PostToolUse" => {
            AgentEvent::ToolFinished { tool: tool(), tool_use_id: s(p, "tool_use_id"), ok: true, interrupted: false }
        }
        "PostToolUseFailure" => AgentEvent::ToolFinished {
            tool: tool(),
            tool_use_id: s(p, "tool_use_id"),
            ok: false,
            interrupted: p.get("is_interrupt").and_then(Value::as_bool).unwrap_or(false),
        },
        "PermissionRequest" => AgentEvent::PermissionRequested {
            request: PermissionRequest {
                id: None,
                tool: tool(),
                summary: tool_summary(&tool(), &input()),
                input: input(),
                suggestions: p.get("permission_suggestions").cloned().unwrap_or(Value::Null),
                prompt: prompt_kind(&tool(), &input()),
            },
        },
        "PermissionDenied" => AgentEvent::PermissionAutoDenied { tool: tool(), reason: s(p, "reason") },
        "Notification" => {
            let t = s(p, "notification_type").unwrap_or_default();
            let needs_attention =
                matches!(t.as_str(), "permission_prompt" | "elicitation_dialog" | "elicitation_url_dialog");
            AgentEvent::Notification { notification_type: t, message: s(p, "message"), needs_attention }
        }
        "Stop" => AgentEvent::TurnFinished {
            last_message: s(p, "last_assistant_message"),
            error: None,
            background_tasks: running_background_tasks(p),
        },
        "StopFailure" => AgentEvent::TurnFinished {
            last_message: s(p, "last_assistant_message"),
            error: Some(s(p, "error").unwrap_or_else(|| "unknown".into())),
            background_tasks: running_background_tasks(p),
        },
        // Claude Code also reports its own helper agents (e.g. the next-prompt suggestion
        // after each turn) as SubagentStop with an empty agent_type and no SubagentStart.
        "SubagentStart" => AgentEvent::SubagentStarted {
            agent_id: s(p, "agent_id").unwrap_or_default(),
            agent_type: s(p, "agent_type").filter(|t| !t.is_empty()),
        },
        "SubagentStop" => AgentEvent::SubagentStopped {
            agent_id: s(p, "agent_id").unwrap_or_default(),
            agent_type: s(p, "agent_type").filter(|t| !t.is_empty()),
            transcript_path: s(p, "agent_transcript_path"),
        },
        "PreCompact" => AgentEvent::CompactionStarted { trigger: s(p, "trigger").unwrap_or_default() },
        "PostCompact" => AgentEvent::CompactionFinished { trigger: s(p, "trigger").unwrap_or_default() },
        "Elicitation" => {
            AgentEvent::InputRequested { source: s(p, "mcp_server_name").unwrap_or_default(), message: s(p, "message") }
        }
        other => AgentEvent::Other { event: other.to_string() },
    };
    let mut ev = base(kind);
    // For subagent lifecycle events the agent_id names the subagent itself, not a source.
    if matches!(ev.kind, AgentEvent::SubagentStarted { .. } | AgentEvent::SubagentStopped { .. }) {
        ev.subagent_id = None;
    }
    vec![ev]
}

/// AskUserQuestion input → typed questions (unknown shapes give an empty list).
pub fn parse_questions(input: &Value) -> Vec<Question> {
    input
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|q| {
            Some(Question {
                question: q.get("question")?.as_str()?.to_string(),
                header: s(q, "header"),
                options: q
                    .get("options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|o| {
                        Some(QuestionOption {
                            label: o.get("label")?.as_str()?.to_string(),
                            description: s(o, "description"),
                        })
                    })
                    .collect(),
                multi_select: q.get("multiSelect").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

pub(super) fn prompt_kind(tool: &str, input: &Value) -> PromptKind {
    match tool {
        "AskUserQuestion" => PromptKind::Question { questions: parse_questions(input) },
        "ExitPlanMode" => PromptKind::Plan { plan: s(input, "plan"), plan_file_path: s(input, "planFilePath") },
        _ => PromptKind::Tool,
    }
}

fn running_background_tasks(p: &Value) -> u32 {
    p.get("background_tasks")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter().filter(|t| t.get("status").and_then(Value::as_str).unwrap_or("running") == "running").count()
                as u32
        })
        .unwrap_or(0)
}
