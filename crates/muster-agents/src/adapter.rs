//! The [`AgentAdapter`] trait every CLI integration implements, and the inputs it takes.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::types::*;

/// Where the hook machinery lives. One value per daemon; the per-card part travels in
/// the environment (`MUSTER_CARD_ID`).
#[derive(Debug, Clone, PartialEq)]
pub struct HookConfig {
    /// Absolute path of the `muster-hook` binary.
    pub hook_bin: PathBuf,
    /// Unix socket the receiver listens on (keep the path under ~100 bytes: macOS limits
    /// `sun_path` to 104).
    pub sock: PathBuf,
    /// A directory the adapter may write generated files into (e.g.
    /// `$MUSTER_HOME/agents`). Never an agent's own config directory.
    pub state_dir: PathBuf,
    /// How long a permission request is held open waiting for an answer from the board.
    /// The agent's own terminal prompt stays usable the whole time.
    pub approval_wait: Duration,
    /// Also capture the status line feed (Claude Code: context window and rate limits).
    /// The user's own status line keeps working through chaining.
    pub capture_statusline: bool,
}

impl HookConfig {
    pub fn new(hook_bin: impl Into<PathBuf>, sock: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Self {
        HookConfig {
            hook_bin: hook_bin.into(),
            sock: sock.into(),
            state_dir: state_dir.into(),
            approval_wait: Duration::from_secs(1800),
            capture_statusline: true,
        }
    }
}

/// Everything needed to start an agent for a card.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchSpec {
    pub card_id: String,
    pub cwd: PathBuf,
    /// The session id Muster chose (see [`AgentAdapter::new_session_id`]); `None` lets the
    /// agent pick and Muster learns it from the first event.
    pub session_id: Option<String>,
    /// First prompt, submitted as soon as the agent starts.
    pub prompt: Option<String>,
    /// Hooks; `None` launches without status integration.
    pub hooks: Option<HookConfig>,
    /// Extra arguments the user configured for this agent, placed before the prompt.
    pub extra_args: Vec<String>,
    /// Override for the agent executable (default: found on `PATH` by name).
    pub program: Option<String>,
    /// Markdown with standing instructions for the agent (e.g. how to use the `muster`
    /// CLI). Passed per launch, never written into the agent's or the repo's config.
    /// Claude Code: `--append-system-prompt-file`.
    pub instructions: Option<PathBuf>,
    /// Per-launch hook secret (see [`new_hook_secret`]). Passed to the agent only in the
    /// session environment (`MUSTER_HOOK_SECRET`); register the same value with
    /// [`crate::HookReceiver::set_card_secret`] before starting the session.
    pub hook_secret: Option<String>,
    /// A directory of skills laid out as `<dir>/.claude/skills/<name>/SKILL.md`.
    /// Claude Code: `--add-dir <dir>` (which also gives the agent file access to it).
    pub skills_dir: Option<PathBuf>,
}

impl LaunchSpec {
    pub fn new(card_id: impl Into<String>, cwd: impl Into<PathBuf>) -> Self {
        LaunchSpec {
            card_id: card_id.into(),
            cwd: cwd.into(),
            session_id: None,
            prompt: None,
            hooks: None,
            extra_args: Vec::new(),
            program: None,
            instructions: None,
            skills_dir: None,
            hook_secret: None,
        }
    }
}

/// What an adapter can do; lets the daemon and UI hide features per agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// Hook events give live status.
    pub hooks: bool,
    /// The launch command accepts a session id chosen by Muster.
    pub preset_session_id: bool,
    pub resume: bool,
    /// Permission requests can be answered through the hook.
    pub answer_permissions: bool,
    pub transcript: bool,
    pub context_usage: bool,
    pub rate_limits: bool,
    /// Where live status comes from.
    pub status_source: StatusSource,
}

/// How the daemon learns an agent's status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusSource {
    /// Hook events through `muster-hook` and the [`crate::HookReceiver`].
    Hooks,
    /// The agent's own session log: call [`AgentAdapter::log_events`] when the file grows.
    SessionLog,
    /// Hooks for some events plus the session log for the rest.
    HooksAndLog,
    /// Nothing structured: terminal activity only (the daemon's fallback).
    #[default]
    Terminal,
}

/// A session found on disk.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRef {
    pub session_id: String,
    pub transcript: Option<PathBuf>,
}

fn unsupported<T>() -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "not supported by this agent"))
}

/// One agent CLI integration. Implementations are cheap, stateless values; all state
/// (status, session ids) belongs to the daemon or to [`crate::StatusTracker`].
pub trait AgentAdapter: Send + Sync {
    /// Agent id as used in `docs/PROTOCOL.md` (`claude`, `codex`, ...).
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;

    /// Is the CLI installed (found on PATH or at the configured program path)?
    fn installed(&self) -> bool;

    /// A fresh session id in the format the agent accepts, if it accepts one.
    fn new_session_id(&self) -> Option<String> {
        None
    }

    /// Like [`new_session_id`](Self::new_session_id) for agents that must create the
    /// session first (Cursor runs `create-chat` in `cwd`). Call this one when launching.
    fn prepare_session_id(&self, _cwd: &Path) -> Option<String> {
        self.new_session_id()
    }

    /// Command that starts a new session. May write generated hook files into
    /// `spec.hooks.state_dir` (never into the agent's own config).
    fn launch(&self, spec: &LaunchSpec) -> io::Result<CommandSpec>;

    /// Command that resumes `session_id`. `spec.prompt`, if any, is sent as the first new
    /// message.
    fn resume(&self, spec: &LaunchSpec, session_id: &str) -> io::Result<CommandSpec>;

    /// Translate one hook envelope into zero or more events.
    fn normalize(&self, _envelope: &HookEnvelope, _at_ms: u64) -> Vec<NormalizedEvent> {
        Vec::new()
    }

    /// Find the session a launch created when Muster could not choose its id: the newest
    /// session in `cwd` started at or after `started_after_ms`.
    fn find_session(&self, _cwd: &Path, _started_after_ms: u64) -> Option<SessionRef> {
        None
    }

    /// Events from the agent's own session log after byte offset `after` (start with 0,
    /// or with the file length to skip history). Returns the events and the next offset.
    /// Used when [`Capabilities::status_source`] includes the log.
    fn log_events(&self, _card_id: &str, _log: &Path, _after: u64) -> io::Result<(Vec<NormalizedEvent>, u64)> {
        Ok((Vec::new(), _after))
    }

    /// Does this event show that `request` (same card) was settled without our answer?
    /// The receiver then releases the waiting hook. Needed because an agent may keep a
    /// permission hook running after the user answered in the terminal (Claude Code does
    /// when the call is allowed there).
    fn settles_permission(&self, _envelope: &HookEnvelope, _request: &PermissionRequest) -> bool {
        false
    }

    /// The exact line the hook must print to stdout to apply `decision` to `request`.
    fn permission_reply(&self, _request: &PermissionRequest, _decision: &PermissionDecision) -> String {
        String::new()
    }

    /// Keys that answer the agent's own permission prompt in the terminal (fallback when
    /// the hook cannot answer). `None` if there is no reliable key sequence.
    fn permission_keys(&self, _decision: &PermissionDecision) -> Option<Vec<KeyInput>> {
        None
    }

    /// Directory every transcript of this agent lives under (Claude Code:
    /// `<config>/projects`). Used by [`AgentAdapter::validate_transcript_path`].
    fn transcript_root(&self) -> Option<PathBuf> {
        None
    }

    /// `Some(resolved path)` when `path` resolves (after following symlinks, also of a
    /// not-yet-existing file's parent directories) to a `.jsonl` file under
    /// [`transcript_root`](Self::transcript_root); `None` otherwise. Transcript paths
    /// arrive in hook payloads, which any local process could forge, so never read one
    /// without this check. The receiver already applies it to every event.
    fn validate_transcript_path(&self, path: &Path) -> Option<PathBuf> {
        resolve_under(&self.transcript_root()?, path).filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
    }

    /// Where the transcript for `session_id` lives, if it exists. Prefer the
    /// `transcript_path` from events; this is the fallback (e.g. after a reboot).
    fn transcript_path(&self, _session_id: &str, _cwd: &Path) -> Option<PathBuf> {
        None
    }

    /// The command that, typed at the agent's prompt between turns, makes it name its
    /// session from the conversation (Claude Code: `/rename`).
    fn rename_command(&self) -> Option<&'static str> {
        None
    }

    /// The newest session name the agent recorded in its transcript after byte offset
    /// `after`, if any.
    fn session_name_since(&self, _transcript: &Path, _after: u64) -> io::Result<Option<String>> {
        Ok(None)
    }

    /// Read a page of chat messages, newest last. `before` is a byte offset from a previous
    /// [`ChatPage::older`]; `None` starts at the end of the file. Reads at most about
    /// `max_messages` messages and never the whole file unless it is small.
    fn read_chat(&self, _transcript: &Path, _before: Option<u64>, _max_messages: usize) -> io::Result<ChatPage> {
        unsupported()
    }

    /// Messages appended after byte offset `after` (from [`ChatPage::end`]), for live
    /// updates of an open chat view.
    fn read_chat_since(&self, _transcript: &Path, _after: u64, _max_messages: usize) -> io::Result<ChatPage> {
        unsupported()
    }

    /// Context usage derived from the transcript tail (fallback when no status line feed).
    fn context_usage(&self, _transcript: &Path) -> io::Result<Option<ContextUsage>> {
        Ok(None)
    }

    /// State of the latest turn from the transcript tail.
    fn turn_state(&self, _transcript: &Path) -> io::Result<TurnState> {
        Ok(TurnState::Unknown)
    }
}

/// The set of adapters the receiver dispatches to, by agent id.
#[derive(Clone)]
pub struct AdapterRegistry {
    adapters: Vec<Arc<dyn AgentAdapter>>,
}

impl AdapterRegistry {
    pub fn empty() -> Self {
        AdapterRegistry { adapters: Vec::new() }
    }

    pub fn with(mut self, adapter: Arc<dyn AgentAdapter>) -> Self {
        self.adapters.retain(|a| a.id() != adapter.id());
        self.adapters.push(adapter);
        self
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn AgentAdapter>> {
        self.adapters.iter().find(|a| a.id() == id)
    }

    pub fn all(&self) -> &[Arc<dyn AgentAdapter>] {
        &self.adapters
    }
}

impl Default for AdapterRegistry {
    /// All built-in adapters.
    fn default() -> Self {
        AdapterRegistry::empty()
            .with(Arc::new(crate::claude::ClaudeCode::default()))
            .with(Arc::new(crate::codex::Codex::default()))
            .with(Arc::new(crate::cursor::Cursor::default()))
            .with(Arc::new(crate::opencode::Opencode::default()))
            .with(Arc::new(crate::basic::Gemini::default()))
            .with(Arc::new(crate::basic::Antigravity::default()))
    }
}

/// Look `name` up on `PATH`.
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return is_executable(&p).then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

/// Current time in unix milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A new per-launch hook secret: 32 random bytes, hex.
pub fn new_hook_secret() -> String {
    use std::io::Read;
    let mut b = [0u8; 32];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).expect("read /dev/urandom");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Constant-time equality for secrets.
pub fn secret_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Resolves `path` with symlinks followed and returns it if it lies under `root`
/// (also resolved). For a path that does not exist yet, the deepest existing ancestor is
/// resolved and the rest must be plain names (no `..`, no `.`).
pub fn resolve_under(root: &Path, path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    if !path.is_absolute() {
        return None;
    }
    let root = std::fs::canonicalize(root).ok()?;
    let mut existing = path.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    let resolved = loop {
        match std::fs::canonicalize(&existing) {
            Ok(p) => break p,
            Err(_) => {
                let name = match existing.components().next_back()? {
                    Component::Normal(n) => n.to_os_string(),
                    _ => return None,
                };
                rest.push(name);
                if !existing.pop() {
                    return None;
                }
            }
        }
    };
    let mut full = resolved;
    for n in rest.into_iter().rev() {
        full.push(n);
    }
    (full.starts_with(&root) && full != root).then_some(full)
}
