//! opencode adapter (checked against opencode v2.0.32; see `docs/AGENTS.md`).
//!
//! opencode 2 runs sessions in a shared background service. `--session <id>` creates a
//! session with an id Muster chooses (verified), and the service's API answers status and
//! message queries through the CLI itself (`opencode api GET /api/session/<id>`), which
//! handles the service's local authentication; Muster never reads the service password.
//! Status is therefore polled with [`Opencode::session_query`] /
//! [`Opencode::messages_query`] and parsed with [`parse_session`] / [`parse_messages`].

use std::io;
use std::path::Path;

use serde_json::Value;

use crate::adapter::*;
use crate::types::*;

#[derive(Debug, Clone)]
pub struct Opencode {
    pub program: String,
}

impl Default for Opencode {
    fn default() -> Self {
        Opencode { program: "opencode".into() }
    }
}

/// State of an opencode session from `GET /api/session/<id>`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpencodeSession {
    pub id: String,
    /// Not idle since its last update (a turn is running).
    pub busy: bool,
    /// `outcome` of the last run (`failed`, ...), when reported.
    pub outcome: Option<String>,
    pub context: Option<ContextUsage>,
    pub cost_usd: Option<f64>,
    pub updated_ms: Option<u64>,
}

impl Opencode {
    fn api(&self, path: String) -> CommandSpec {
        CommandSpec {
            program: self.program.clone(),
            args: vec!["api".into(), "GET".into(), path],
            env: Vec::new(),
            cwd: std::env::temp_dir(),
        }
    }

    /// Command whose stdout is the session JSON (parse with [`parse_session`]).
    pub fn session_query(&self, session_id: &str) -> CommandSpec {
        self.api(format!("/api/session/{session_id}"))
    }

    /// Command whose stdout is the message list JSON (parse with [`parse_messages`]).
    pub fn messages_query(&self, session_id: &str) -> CommandSpec {
        self.api(format!("/api/session/{session_id}/message"))
    }

    /// Command whose stdout lists pending permission requests of the session
    /// (documented endpoint; not exercised, see docs/AGENTS.md).
    pub fn permissions_query(&self, session_id: &str) -> CommandSpec {
        self.api(format!("/api/session/{session_id}/permission"))
    }

    fn command(&self, spec: &LaunchSpec, session: Option<&str>) -> CommandSpec {
        let mut args = Vec::new();
        if let Some(s) = session {
            args.extend(["--session".into(), s.to_string()]);
        }
        args.extend(spec.extra_args.iter().cloned());
        if let Some(p) = spec.prompt.as_ref().filter(|p| !p.trim().is_empty()) {
            args.extend(["--prompt".into(), p.clone()]);
        }
        CommandSpec {
            program: spec.program.clone().unwrap_or_else(|| self.program.clone()),
            args,
            env: Vec::new(),
            cwd: spec.cwd.clone(),
        }
    }
}

impl AgentAdapter for Opencode {
    fn id(&self) -> &'static str {
        "opencode"
    }

    fn label(&self) -> &'static str {
        "opencode"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            hooks: false,
            preset_session_id: true,
            resume: true,
            answer_permissions: false,
            transcript: false,
            context_usage: true,
            rate_limits: false,
            status_source: StatusSource::Terminal,
        }
    }

    fn installed(&self) -> bool {
        find_on_path(&self.program).is_some()
    }

    fn new_session_id(&self) -> Option<String> {
        let u = crate::claude::uuid_v4().ok()?;
        Some(format!("ses_muster{}", u.replace('-', "")))
    }

    fn launch(&self, spec: &LaunchSpec) -> io::Result<CommandSpec> {
        Ok(self.command(spec, spec.session_id.as_deref()))
    }

    fn resume(&self, spec: &LaunchSpec, session_id: &str) -> io::Result<CommandSpec> {
        Ok(self.command(spec, Some(session_id)))
    }

    fn transcript_path(&self, _session_id: &str, _cwd: &Path) -> Option<std::path::PathBuf> {
        None
    }
}

fn data(v: &Value) -> &Value {
    v.get("data").unwrap_or(v)
}

pub fn parse_session(json: &str) -> Option<OpencodeSession> {
    let v: Value = serde_json::from_str(json).ok()?;
    let d = data(&v);
    let t = &d["time"];
    let updated = t.get("updated").and_then(Value::as_u64);
    let idle = t.get("idle").and_then(Value::as_u64);
    let tok = &d["tokens"];
    let used: u64 = ["input"].iter().filter_map(|k| tok.get(*k).and_then(Value::as_u64)).sum::<u64>()
        + tok.pointer("/cache/read").and_then(Value::as_u64).unwrap_or(0)
        + tok.pointer("/cache/write").and_then(Value::as_u64).unwrap_or(0);
    Some(OpencodeSession {
        id: d.get("id")?.as_str()?.to_string(),
        busy: match (updated, idle) {
            (Some(u), Some(i)) => i < u,
            (_, None) => updated.is_some(),
            _ => false,
        },
        outcome: d.get("outcome").and_then(Value::as_str).map(str::to_string),
        context: (used > 0).then_some(ContextUsage {
            used_tokens: used,
            window_tokens: None,
            used_percent: None,
            source: UsageSource::Transcript,
        }),
        cost_usd: d.get("cost").and_then(Value::as_f64),
        updated_ms: updated,
    })
}

/// Message list → chat messages (oldest first). opencode lists newest first.
pub fn parse_messages(json: &str) -> Vec<ChatMessage> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let mut out: Vec<ChatMessage> = data(&v)
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m.get("id")?.as_str()?.to_string();
            let mut parts = Vec::new();
            let role = match m.get("type")?.as_str()? {
                "user" => {
                    if let Some(t) = m.get("text").and_then(Value::as_str) {
                        parts.push(ChatPart::Text { text: t.trim_matches('"').to_string() });
                    }
                    ChatRole::User
                }
                "assistant" => {
                    for c in m.get("content").and_then(Value::as_array).into_iter().flatten() {
                        match c.get("type").and_then(Value::as_str) {
                            Some("text") => {
                                if let Some(t) = c.get("text").and_then(Value::as_str) {
                                    parts.push(ChatPart::Text { text: t.to_string() });
                                }
                            }
                            Some("reasoning") => parts.push(ChatPart::Thinking),
                            Some("tool") => {
                                let name = c.get("tool").and_then(Value::as_str).unwrap_or("tool").to_string();
                                parts.push(ChatPart::ToolCall {
                                    id: c.get("callID").and_then(Value::as_str).unwrap_or_default().to_string(),
                                    summary: name.clone(),
                                    name,
                                });
                            }
                            _ => {}
                        }
                    }
                    if let Some(e) = m.pointer("/error/message").and_then(Value::as_str) {
                        parts.push(ChatPart::Text { text: format!("Error: {e}") });
                    }
                    ChatRole::Assistant
                }
                _ => return None,
            };
            (!parts.is_empty()).then(|| ChatMessage {
                id,
                role,
                timestamp: m.pointer("/time/created").and_then(Value::as_u64).map(|t| t.to_string()),
                model: m.pointer("/model/id").and_then(Value::as_str).map(str::to_string),
                parts,
            })
        })
        .collect();
    out.reverse();
    out
}
