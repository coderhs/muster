//! Claude Code adapter (verified against Claude Code 2.1.323; see `docs/AGENTS.md`).
//!
//! * Hooks and the status line are supplied per launch with `--settings <file>`; the
//!   file lives in Muster's own state directory. Claude Code merges hooks from
//!   `--settings` with the user's own hooks, so nothing in `~/.claude` is touched.
//! * The status line from `--settings` replaces the user's for that session, so the
//!   user's command (looked up read-only in their settings files) is run by
//!   `muster-hook statusline` as a chain and its output is shown unchanged.
//! * Permission requests use the `PermissionRequest` hook, which may print an
//!   allow/deny decision. Claude Code shows its own prompt at the same time; whichever
//!   answers first wins (the hook process is killed if the terminal answers first).

mod hooks;
mod transcript;
mod usage;

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::adapter::*;
use crate::types::*;

pub use hooks::{settings_json, user_statusline_command, HOOK_EVENTS};
pub use usage::parse_statusline;

/// The Claude Code adapter.
#[derive(Debug, Clone)]
pub struct ClaudeCode {
    /// Executable name or path; default `claude`.
    pub program: String,
    /// Claude Code's config directory (`$CLAUDE_CONFIG_DIR` or `~/.claude`). Only ever
    /// read: transcripts and the user's status line setting.
    pub config_dir: PathBuf,
}

impl Default for ClaudeCode {
    fn default() -> Self {
        let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude")))
            .unwrap_or_else(|| PathBuf::from(".claude"));
        ClaudeCode {
            program: "claude".into(),
            config_dir,
        }
    }
}

impl ClaudeCode {
    /// Write (if changed) the per-launch settings file and return its path.
    pub fn write_settings(
        &self,
        hooks: &HookConfig,
        user_statusline: Option<&Value>,
    ) -> io::Result<PathBuf> {
        let dir = hooks.state_dir.join("claude");
        std::fs::create_dir_all(&dir)?;
        let content = serde_json::to_string_pretty(&settings_json(hooks, user_statusline))
            .map_err(io::Error::other)?;
        // One file per distinct content (the status line padding may differ per project).
        let name = format!("settings-{:016x}.json", fnv1a(content.as_bytes()));
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
            let tmp = dir.join(format!(
                ".tmp-{}-{}",
                std::process::id(),
                fnv1a(content.as_bytes())
            ));
            std::fs::write(&tmp, &content)?;
            std::fs::rename(&tmp, &path)?;
        }
        Ok(path)
    }

    fn command(&self, spec: &LaunchSpec, resume: Option<&str>) -> io::Result<CommandSpec> {
        let mut args: Vec<String> = Vec::new();
        let mut env: Vec<(String, String)> = Vec::new();
        match resume {
            Some(sid) => args.extend(["--resume".into(), sid.to_string()]),
            None => {
                if let Some(sid) = &spec.session_id {
                    args.extend(["--session-id".into(), sid.clone()]);
                }
            }
        }
        if let Some(h) = &spec.hooks {
            let user_sl = if h.capture_statusline {
                user_statusline_command(&self.config_dir, &spec.cwd)
            } else {
                None
            };
            let settings = self.write_settings(h, user_sl.as_ref())?;
            args.extend(["--settings".into(), settings.to_string_lossy().into_owned()]);
            env.push(("MUSTER_CARD_ID".into(), spec.card_id.clone()));
            env.push(("MUSTER_AGENT_ID".into(), self.id().into()));
            env.push((
                "MUSTER_HOOK_SOCK".into(),
                h.sock.to_string_lossy().into_owned(),
            ));
            if let Some(secret) = &spec.hook_secret {
                env.push(("MUSTER_HOOK_SECRET".into(), secret.clone()));
            }
            if let Some(cmd) = user_sl
                .as_ref()
                .and_then(|v| v.get("command"))
                .and_then(Value::as_str)
            {
                env.push(("MUSTER_STATUSLINE_CHAIN".into(), cmd.to_string()));
            }
        }
        if let Some(f) = &spec.instructions {
            args.extend([
                "--append-system-prompt-file".into(),
                f.to_string_lossy().into_owned(),
            ]);
        }
        if let Some(d) = &spec.skills_dir {
            // Variadic option: the prompt is separated by `--` below.
            args.extend(["--add-dir".into(), d.to_string_lossy().into_owned()]);
        }
        args.extend(spec.extra_args.iter().cloned());
        if let Some(p) = spec.prompt.as_ref().filter(|p| !p.trim().is_empty()) {
            // `--` so a prompt starting with "-" is never taken for an option.
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

    /// `~/.claude/projects/<cwd with every non-alphanumeric char replaced by '-'>`.
    pub fn project_dir(&self, cwd: &Path) -> PathBuf {
        let s: String = cwd
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        self.config_dir.join("projects").join(s)
    }
}

impl AgentAdapter for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn label(&self) -> &'static str {
        "Claude Code"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            hooks: true,
            preset_session_id: true,
            resume: true,
            answer_permissions: true,
            transcript: true,
            context_usage: true,
            rate_limits: true,
            status_source: StatusSource::Hooks,
        }
    }

    fn installed(&self) -> bool {
        find_on_path(&self.program).is_some()
    }

    fn new_session_id(&self) -> Option<String> {
        uuid_v4().ok()
    }

    fn launch(&self, spec: &LaunchSpec) -> io::Result<CommandSpec> {
        self.command(spec, None)
    }

    fn resume(&self, spec: &LaunchSpec, session_id: &str) -> io::Result<CommandSpec> {
        self.command(spec, Some(session_id))
    }

    fn normalize(&self, envelope: &HookEnvelope, at_ms: u64) -> Vec<NormalizedEvent> {
        hooks::normalize(envelope, at_ms)
    }

    fn settles_permission(&self, envelope: &HookEnvelope, request: &PermissionRequest) -> bool {
        if envelope.kind != EnvelopeKind::Hook {
            return false;
        }
        let p = &envelope.payload;
        let name = p
            .get("hook_event_name")
            .and_then(Value::as_str)
            .unwrap_or(&envelope.event);
        match name {
            // The guarded call ran (or failed): its prompt was answered in the terminal.
            "PostToolUse" | "PostToolUseFailure" => {
                p.get("tool_name").and_then(Value::as_str) == Some(request.tool.as_str())
                    && p.get("tool_input") == Some(&request.input)
            }
            // The turn or session moved on: nothing can still be waiting on that prompt.
            "Stop" | "StopFailure" | "UserPromptSubmit" | "SessionEnd" | "SessionStart" => true,
            _ => false,
        }
    }

    fn permission_reply(
        &self,
        request: &PermissionRequest,
        decision: &PermissionDecision,
    ) -> String {
        let decision = match decision {
            PermissionDecision::Allow { remember } => {
                let mut d = json!({"behavior": "allow"});
                if *remember {
                    // Only rule and directory grants; never a mode switch the user did not
                    // explicitly pick (Claude Code also suggests e.g. `setMode`).
                    let grants: Vec<Value> = request
                        .suggestions
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|s| {
                            matches!(
                                s.get("type").and_then(Value::as_str),
                                Some("addRules" | "addDirectories")
                            )
                        })
                        .cloned()
                        .collect();
                    if !grants.is_empty() {
                        d["updatedPermissions"] = Value::Array(grants);
                    }
                }
                d
            }
            PermissionDecision::Answer { answers } => {
                // AskUserQuestion needs the original input back plus `answers`
                // (question text -> label; several labels joined with ", ").
                let mut input = request.input.clone();
                if !input.is_object() {
                    input = json!({});
                }
                let map: serde_json::Map<String, Value> = answers
                    .iter()
                    .map(|(q, labels)| (q.clone(), Value::String(labels.join(", "))))
                    .collect();
                input["answers"] = Value::Object(map);
                json!({"behavior": "allow", "updatedInput": input})
            }
            PermissionDecision::ApprovePlan { accept_edits } => {
                let mut d = json!({"behavior": "allow", "updatedInput": request.input.clone()});
                if *accept_edits {
                    d["updatedPermissions"] = json!([{"type": "setMode", "mode": "acceptEdits", "destination": "session"}]);
                }
                d
            }
            PermissionDecision::Deny { message, interrupt } => {
                let mut d = json!({"behavior": "deny"});
                d["message"] = json!(message
                    .clone()
                    .unwrap_or_else(|| "Denied from Muster".into()));
                if *interrupt {
                    d["interrupt"] = json!(true);
                }
                d
            }
        };
        json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": decision}})
            .to_string()
    }

    fn permission_keys(&self, decision: &PermissionDecision) -> Option<Vec<KeyInput>> {
        // The prompt lists "1. Yes" first with the cursor on it; "No" is the last entry
        // but its number depends on how many "Yes, and ..." variants are offered, so a
        // refusal uses Escape, which rejects the call and interrupts the turn.
        match decision {
            PermissionDecision::Allow { remember: false } => Some(vec![KeyInput::Text("1".into())]),
            // Entry 2 is a "Yes, and ..." variant whose scope differs per tool (directory,
            // command prefix, mode switch), so there is no key that reliably means "remember".
            PermissionDecision::Allow { remember: true } => None,
            PermissionDecision::Deny { .. } => Some(vec![KeyInput::Key("Escape".into())]),
            PermissionDecision::Answer { .. } | PermissionDecision::ApprovePlan { .. } => None,
        }
    }

    fn transcript_root(&self) -> Option<PathBuf> {
        Some(self.config_dir.join("projects"))
    }

    fn transcript_path(&self, session_id: &str, cwd: &Path) -> Option<PathBuf> {
        let file = format!("{session_id}.jsonl");
        let direct = self.project_dir(cwd).join(&file);
        if direct.is_file() {
            return Some(direct);
        }
        // The directory naming is Claude Code's business; fall back to a scan.
        let projects = self.config_dir.join("projects");
        std::fs::read_dir(projects)
            .ok()?
            .flatten()
            .map(|e| e.path().join(&file))
            .find(|p| p.is_file())
    }

    fn read_chat(
        &self,
        transcript: &Path,
        before: Option<u64>,
        max_messages: usize,
    ) -> io::Result<ChatPage> {
        transcript::read_page(transcript, before, max_messages)
    }

    fn read_chat_since(
        &self,
        transcript: &Path,
        after: u64,
        max_messages: usize,
    ) -> io::Result<ChatPage> {
        transcript::read_since(transcript, after, max_messages)
    }

    fn context_usage(&self, transcript: &Path) -> io::Result<Option<ContextUsage>> {
        transcript::context_usage(transcript)
    }

    fn turn_state(&self, transcript: &Path) -> io::Result<TurnState> {
        transcript::turn_state(transcript)
    }
}

fn fnv1a(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf29ce484222325u64, |h, &c| {
        (h ^ c as u64).wrapping_mul(0x100000001b3)
    })
}

/// Random RFC 4122 version 4 UUID.
pub fn uuid_v4() -> io::Result<String> {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    ))
}

/// One-line description of a tool call for badges, notifications and the chat view.
pub fn tool_summary(tool: &str, input: &Value) -> String {
    let field = |k: &str| input.get(k).and_then(Value::as_str);
    match tool {
        "AskUserQuestion" => {
            let q = input
                .pointer("/questions/0/question")
                .and_then(Value::as_str)
                .unwrap_or("question");
            return format!("Question: {}", one_line(q, 160));
        }
        "ExitPlanMode" => return "Plan ready for approval".to_string(),
        _ => {}
    }
    let detail = match tool {
        "Bash" => field("command"),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            field("file_path").or(field("notebook_path"))
        }
        "Glob" | "Grep" => field("pattern"),
        "WebFetch" => field("url"),
        "WebSearch" => field("query"),
        "Task" | "Agent" => field("description"),
        _ => None,
    }
    .map(str::to_string)
    .or_else(|| {
        input
            .as_object()
            .and_then(|o| o.values().find_map(|v| v.as_str().map(str::to_string)))
    });
    match detail {
        Some(d) => format!("{tool}: {}", one_line(&d, 160)),
        None => tool.to_string(),
    }
}

pub(crate) fn one_line(s: &str, max: usize) -> String {
    let first = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let multi = s.trim().lines().count() > 1;
    let mut out: String = first.chars().take(max).collect();
    if first.chars().count() > max || multi {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_format() {
        let u = uuid_v4().unwrap();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn project_dir_mapping_matches_observed_names() {
        let c = ClaudeCode {
            program: "claude".into(),
            config_dir: "/h/.claude".into(),
        };
        assert_eq!(
            c.project_dir(Path::new("/Users/user/sandbox/aiterm")),
            PathBuf::from("/h/.claude/projects/-Users-user-sandbox-aiterm")
        );
        assert_eq!(
            c.project_dir(Path::new("/private/tmp/claude-501/-Users-x/scratchpad")),
            PathBuf::from("/h/.claude/projects/-private-tmp-claude-501--Users-x-scratchpad")
        );
    }

    #[test]
    fn summaries() {
        assert_eq!(
            tool_summary("Bash", &json!({"command": "touch a\necho b"})),
            "Bash: touch a…"
        );
        assert_eq!(
            tool_summary("Edit", &json!({"file_path": "/x/y.rs", "old_string": "a"})),
            "Edit: /x/y.rs"
        );
        assert_eq!(tool_summary("mcp__x__y", &json!({"n": 1})), "mcp__x__y");
    }

    #[test]
    fn permission_replies() {
        let c = ClaudeCode::default();
        let req = PermissionRequest {
            id: Some("1".into()),
            tool: "Bash".into(),
            summary: String::new(),
            input: json!({}),
            prompt: PromptKind::Tool,
            suggestions: json!([
                {"type": "addDirectories", "directories": ["/p"], "destination": "session"},
                {"type": "setMode", "mode": "acceptEdits", "destination": "session"}
            ]),
        };
        let allow: Value = serde_json::from_str(
            &c.permission_reply(&req, &PermissionDecision::Allow { remember: false }),
        )
        .unwrap();
        assert_eq!(
            allow,
            json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": {"behavior": "allow"}}})
        );
        let remember: Value = serde_json::from_str(
            &c.permission_reply(&req, &PermissionDecision::Allow { remember: true }),
        )
        .unwrap();
        assert_eq!(
            remember["hookSpecificOutput"]["decision"]["updatedPermissions"],
            json!([{"type": "addDirectories", "directories": ["/p"], "destination": "session"}])
        );
        let deny: Value = serde_json::from_str(&c.permission_reply(
            &req,
            &PermissionDecision::Deny {
                message: Some("no".into()),
                interrupt: true,
            },
        ))
        .unwrap();
        assert_eq!(
            deny["hookSpecificOutput"]["decision"],
            json!({"behavior": "deny", "message": "no", "interrupt": true})
        );
        // the hook prints the reply as one line
        assert!(!c
            .permission_reply(&req, &PermissionDecision::Allow { remember: false })
            .contains('\n'));
    }

    #[test]
    fn launch_and_resume_commands() {
        let dir = std::env::temp_dir().join(format!("muster-agents-launch-{}", std::process::id()));
        let c = ClaudeCode {
            program: "claude".into(),
            config_dir: dir.join("no-config"),
        };
        let mut spec = LaunchSpec::new("card1", "/work/p");
        spec.session_id = Some("2f0c3f0e-8a5e-4c7c-9b1e-2d6f9a0b1c2d".into());
        spec.prompt = Some("-starts with a dash".into());
        let mut h = HookConfig::new("/opt/muster-hook", "/tmp/m.sock", dir.join("state"));
        h.approval_wait = std::time::Duration::from_secs(120);
        spec.hooks = Some(h);
        let cmd = c.launch(&spec).unwrap();
        assert_eq!(
            cmd.args[0..2],
            ["--session-id", "2f0c3f0e-8a5e-4c7c-9b1e-2d6f9a0b1c2d"]
        );
        assert_eq!(cmd.args[2], "--settings");
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(&cmd.args[3]).unwrap()).unwrap();
        let pr = &settings["hooks"]["PermissionRequest"][0]["hooks"][0];
        assert_eq!(
            pr["args"],
            json!(["hook", "PermissionRequest", "--wait", "120"])
        );
        assert_eq!(pr["timeout"], json!(150));
        assert_eq!(
            settings["statusLine"]["command"],
            json!("/opt/muster-hook statusline")
        );
        assert_eq!(cmd.args[4..], ["--", "-starts with a dash"]);
        assert!(cmd.env.contains(&("MUSTER_CARD_ID".into(), "card1".into())));
        assert!(cmd
            .env
            .contains(&("MUSTER_HOOK_SOCK".into(), "/tmp/m.sock".into())));
        assert!(cmd.shell_line().ends_with("-- '-starts with a dash'"));

        let r = c
            .resume(
                &LaunchSpec {
                    prompt: None,
                    ..spec.clone()
                },
                "abc",
            )
            .unwrap();
        assert_eq!(r.args[0..2], ["--resume", "abc"]);
        assert!(!r.args.contains(&"--session-id".to_string()));
        assert!(!r.args.contains(&"--".to_string()));
        let _ = std::fs::remove_dir_all(dir);
    }
}
