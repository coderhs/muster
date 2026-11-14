//! Launch commands for agent CLIs.
//!
//! The daemon uses the adapters in `crates/muster-agents` for agents they cover
//! (Claude Code); [`launch_command`] is the plain fallback for the rest.

use muster_proto::Agent;

pub struct AgentInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub color: &'static str,
    /// Executable looked up on the login shell's PATH; `None` for the shell.
    pub command: Option<&'static str>,
}

pub const AGENTS: &[AgentInfo] = &[
    AgentInfo { id: "claude", label: "Claude Code", color: "#d97757", command: Some("claude") },
    AgentInfo { id: "codex", label: "Codex", color: "#10a37f", command: Some("codex") },
    AgentInfo { id: "cursor", label: "Cursor", color: "#7c7c7c", command: Some("cursor-agent") },
    AgentInfo { id: "opencode", label: "opencode", color: "#f5a623", command: Some("opencode") },
    AgentInfo { id: "antigravity", label: "Antigravity", color: "#4285f4", command: Some("antigravity") },
    AgentInfo { id: "gemini", label: "Gemini", color: "#8e75b2", command: Some("gemini") },
    AgentInfo { id: "copilot", label: "GitHub Copilot", color: "#6e40c9", command: Some("copilot") },
    AgentInfo { id: "grok", label: "Grok", color: "#222222", command: Some("grok") },
    AgentInfo { id: "shell", label: "Shell", color: "#888888", command: None },
];

pub fn find(id: &str) -> Option<&'static AgentInfo> {
    AGENTS.iter().find(|a| a.id == id)
}

/// The argv to run inside the user's login shell for this agent, or `None`
/// for a plain interactive shell. Errors for unknown agents.
pub fn launch_command(
    agent_id: &str,
    prompt: Option<&str>,
    agent_session_id: Option<&str>,
) -> Result<Option<Vec<String>>, String> {
    let info = find(agent_id).ok_or_else(|| format!("unknown agent {agent_id}"))?;
    let Some(cmd) = info.command else { return Ok(None) };
    let mut argv = vec![cmd.to_string()];
    let prompt = prompt.map(str::trim).filter(|p| !p.is_empty());
    match agent_id {
        "claude" => {
            if let Some(sid) = agent_session_id {
                argv.extend(["--resume".to_string(), sid.to_string()]);
            } else if let Some(p) = prompt {
                argv.push(p.to_string());
            }
        }
        "codex" => {
            if let Some(sid) = agent_session_id {
                argv.extend(["resume".to_string(), sid.to_string()]);
            } else if let Some(p) = prompt {
                argv.push(p.to_string());
            }
        }
        // cursor-agent takes an initial prompt as a positional argument.
        "cursor" => {
            if let Some(p) = prompt {
                argv.push(p.to_string());
            }
        }
        _ => {}
    }
    Ok(Some(argv))
}

pub fn list(installed: impl Fn(&str) -> bool) -> Vec<Agent> {
    AGENTS
        .iter()
        .map(|a| Agent {
            id: a.id.to_string(),
            label: a.label.to_string(),
            color: a.color.to_string(),
            installed: a.command.map(&installed).unwrap_or(true),
            capabilities: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands() {
        assert_eq!(launch_command("shell", Some("x"), None).unwrap(), None);
        assert_eq!(launch_command("claude", Some(" fix it "), None).unwrap().unwrap(), ["claude", "fix it"]);
        assert_eq!(launch_command("claude", Some("x"), Some("s1")).unwrap().unwrap(), ["claude", "--resume", "s1"]);
        assert_eq!(launch_command("opencode", Some("x"), None).unwrap().unwrap(), ["opencode"]);
        assert!(launch_command("custom:foo", None, None).is_err());
    }
}
