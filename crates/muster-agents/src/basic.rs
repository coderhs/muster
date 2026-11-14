//! Launch-and-resume adapters for CLIs without structured status (Gemini CLI,
//! Antigravity). The daemon falls back to terminal activity for their status.
//! Flags come from each CLI's `--help`; neither could run a turn on this machine (see
//! docs/AGENTS.md), so resume is unverified.

use std::io;

use crate::adapter::*;
use crate::types::*;

/// Gemini CLI (0.46): `--session-id <uuid>` starts a session with a chosen id,
/// `--resume <id>` resumes, `-i <prompt>` runs a prompt and stays interactive.
#[derive(Debug, Clone)]
pub struct Gemini {
    pub program: String,
}

impl Default for Gemini {
    fn default() -> Self {
        Gemini { program: "gemini".into() }
    }
}

/// Antigravity CLI `agy` (1.2): `--conversation <id>` resumes, `-i <prompt>` runs a
/// prompt interactively, `--add-dir` adds directories. No way to choose an id up front.
#[derive(Debug, Clone)]
pub struct Antigravity {
    pub program: String,
}

impl Default for Antigravity {
    fn default() -> Self {
        Antigravity { program: "agy".into() }
    }
}

fn cmd(program: &str, spec: &LaunchSpec, mut args: Vec<String>) -> CommandSpec {
    args.extend(spec.extra_args.iter().cloned());
    if let Some(p) = spec.prompt.as_ref().filter(|p| !p.trim().is_empty()) {
        args.extend(["-i".into(), p.clone()]);
    }
    CommandSpec {
        program: spec.program.clone().unwrap_or_else(|| program.to_string()),
        args,
        env: Vec::new(),
        cwd: spec.cwd.clone(),
    }
}

fn caps(preset: bool) -> Capabilities {
    Capabilities {
        preset_session_id: preset,
        resume: true,
        status_source: StatusSource::Terminal,
        ..Capabilities::default()
    }
}

impl AgentAdapter for Gemini {
    fn id(&self) -> &'static str {
        "gemini"
    }
    fn label(&self) -> &'static str {
        "Gemini CLI"
    }
    fn capabilities(&self) -> Capabilities {
        caps(true)
    }
    fn installed(&self) -> bool {
        find_on_path(&self.program).is_some()
    }
    fn new_session_id(&self) -> Option<String> {
        crate::claude::uuid_v4().ok()
    }
    fn launch(&self, spec: &LaunchSpec) -> io::Result<CommandSpec> {
        let mut a = Vec::new();
        if let Some(s) = &spec.session_id {
            a.extend(["--session-id".into(), s.clone()]);
        }
        Ok(cmd(&self.program, spec, a))
    }
    fn resume(&self, spec: &LaunchSpec, session_id: &str) -> io::Result<CommandSpec> {
        Ok(cmd(&self.program, spec, vec!["--resume".into(), session_id.into()]))
    }
}

impl AgentAdapter for Antigravity {
    fn id(&self) -> &'static str {
        "antigravity"
    }
    fn label(&self) -> &'static str {
        "Antigravity"
    }
    fn capabilities(&self) -> Capabilities {
        caps(false)
    }
    fn installed(&self) -> bool {
        find_on_path(&self.program).is_some()
    }
    fn launch(&self, spec: &LaunchSpec) -> io::Result<CommandSpec> {
        Ok(cmd(&self.program, spec, Vec::new()))
    }
    fn resume(&self, spec: &LaunchSpec, session_id: &str) -> io::Result<CommandSpec> {
        Ok(cmd(&self.program, spec, vec!["--conversation".into(), session_id.into()]))
    }
}
