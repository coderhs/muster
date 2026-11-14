//! Agent-independent types shared by every adapter, the receiver and the status tracker.
//!
//! All types serialise with camelCase field names so the daemon can put them on the wire
//! unchanged if it wants to.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Card status from `docs/PLAN.md`. `draft` is owned by the daemon (no session yet) and is
/// therefore not represented here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// The agent is working on a turn.
    Running,
    /// The agent is blocked on the user: a permission prompt, a question, an MCP elicitation.
    NeedsYou,
    /// A turn finished and the user has not looked at it yet.
    Done,
    /// Waiting for input and already seen (or nothing happened yet).
    Idle,
    /// The agent process ended.
    Exited,
}

impl AgentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentStatus::Running => "running",
            AgentStatus::NeedsYou => "needs_you",
            AgentStatus::Done => "done",
            AgentStatus::Idle => "idle",
            AgentStatus::Exited => "exited",
        }
    }
}

/// What `muster-hook` sends: one JSON line per hook invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookEnvelope {
    #[serde(default = "one")]
    pub v: u32,
    /// `hook` or `statusline`.
    pub kind: EnvelopeKind,
    /// Event name as configured on the command line of the hook (for Claude Code this
    /// equals `hook_event_name`; for the status line it is `StatusLine`).
    #[serde(default)]
    pub event: String,
    pub card_id: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub pid: Option<u32>,
    /// The hook is waiting for one line back (only permission requests do).
    #[serde(default)]
    pub wants_reply: bool,
    /// The payload exceeded the hook's size cap and was replaced by `null`.
    #[serde(default)]
    pub truncated: bool,
    /// Per-launch hook secret from `MUSTER_HOOK_SECRET`; checked by the receiver and
    /// removed before the envelope goes anywhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    /// The agent CLI's own JSON, untouched.
    #[serde(default)]
    pub payload: Value,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvelopeKind {
    Hook,
    Statusline,
}

/// A normalised event for one card. `kind` carries the meaning, the other fields are the
/// context every agent event has.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedEvent {
    pub card_id: String,
    pub agent_id: String,
    /// The agent's own session id at the time of the event. It changes after `/clear`
    /// (see [`crate::StatusTracker`], which reports the change).
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub cwd: Option<String>,
    /// Set when the event came from inside a subagent (Claude Code `agent_id`).
    pub subagent_id: Option<String>,
    /// Unix milliseconds when the receiver got the event.
    pub at_ms: u64,
    pub kind: AgentEvent,
}

/// Why a session (re)started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionSource {
    Startup,
    Resume,
    /// `/clear`: a new session id in the same process.
    Clear,
    Compact,
    Fork,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AgentEvent {
    SessionStarted {
        source: SessionSource,
        model: Option<String>,
    },
    /// The agent's session ended. `exited` is false when the process keeps running
    /// (Claude Code ends a session on `/clear` and starts a new one).
    SessionEnded {
        reason: String,
        exited: bool,
    },
    TurnStarted {
        prompt: Option<String>,
    },
    ToolStarted {
        tool: String,
        tool_use_id: Option<String>,
        summary: String,
    },
    ToolFinished {
        tool: String,
        tool_use_id: Option<String>,
        ok: bool,
        interrupted: bool,
    },
    /// The agent wants permission. When `request.id` is set the hook is holding the agent
    /// open and the request can be answered with [`crate::HookReceiver::answer`].
    PermissionRequested {
        request: PermissionRequest,
    },
    /// A pending permission request is no longer pending.
    PermissionResolved {
        request_id: String,
        outcome: PermissionOutcome,
    },
    /// The agent's own policy (Claude Code auto mode) refused a tool call.
    PermissionAutoDenied {
        tool: String,
        reason: Option<String>,
    },
    /// The agent asked the user a multiple-choice question (Claude Code `AskUserQuestion`).
    QuestionAsked {
        tool_use_id: Option<String>,
        questions: Vec<Question>,
    },
    /// MCP server asks the user for input.
    InputRequested {
        source: String,
        message: Option<String>,
    },
    Notification {
        notification_type: String,
        message: Option<String>,
        needs_attention: bool,
    },
    /// The turn ended. `error` is set when it ended on an API error (rate limit, auth...).
    TurnFinished {
        last_message: Option<String>,
        error: Option<String>,
        /// Background tasks still running (Claude Code: subagents started in the
        /// background); their completion starts a new turn by itself.
        #[serde(default)]
        background_tasks: u32,
    },
    SubagentStarted {
        agent_id: String,
        agent_type: Option<String>,
    },
    SubagentStopped {
        agent_id: String,
        agent_type: Option<String>,
        transcript_path: Option<String>,
    },
    CompactionStarted {
        trigger: String,
    },
    CompactionFinished {
        trigger: String,
    },
    /// Context, rate-limit and cost data (Claude Code: from the status line input).
    Usage {
        usage: UsageSnapshot,
    },
    /// Anything else the agent reported; counts as activity only.
    Other {
        event: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    /// Receiver-assigned id; `None` when nobody can answer it through the hook.
    pub id: Option<String>,
    pub tool: String,
    /// One line for a badge or notification ("Bash: touch x.txt").
    pub summary: String,
    pub input: Value,
    /// Agent-provided "always allow" rule suggestions, verbatim.
    #[serde(default)]
    pub suggestions: Value,
    /// What kind of answer the prompt needs.
    #[serde(default)]
    pub prompt: PromptKind,
}

/// The shape of a prompt the user must answer.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PromptKind {
    /// Allow or deny a tool call: answer with [`PermissionDecision::Allow`] / `Deny`.
    #[default]
    Tool,
    /// Multiple-choice questions (Claude Code `AskUserQuestion`): answer with
    /// [`PermissionDecision::Answer`], or `Deny` to decline.
    Question { questions: Vec<Question> },
    /// A plan waiting for approval (Claude Code `ExitPlanMode`): answer with
    /// [`PermissionDecision::ApprovePlan`], or `Deny` with a message to keep planning.
    Plan { plan: Option<String>, plan_file_path: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    /// Full question text; also the key of its answer.
    pub question: String,
    /// Short label (Claude Code: at most 12 characters).
    #[serde(default)]
    pub header: Option<String>,
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "behavior", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PermissionDecision {
    /// Allow this call. With `remember`, also apply the agent's own suggested rules (for
    /// Claude Code: `permission_suggestions` passed back as `updatedPermissions`).
    Allow {
        #[serde(default)]
        remember: bool,
    },
    /// Refuse. `message` is shown to the agent; `interrupt` also stops the turn.
    Deny {
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        interrupt: bool,
    },
    /// Answer a [`PromptKind::Question`]: question text → chosen option labels (several
    /// for multi-select). A value that is not one of the labels is passed as free text.
    Answer { answers: std::collections::BTreeMap<String, Vec<String>> },
    /// Approve a [`PromptKind::Plan`]. `accept_edits` leaves plan mode into accept-edits
    /// mode instead of the default (ask before edits) mode.
    ApprovePlan {
        #[serde(default)]
        accept_edits: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PermissionOutcome {
    /// Answered through [`crate::HookReceiver::answer`].
    Answered { decision: PermissionDecision },
    /// The hook went away before an answer: the user answered in the terminal, the agent
    /// cancelled, or the hook hit its wait limit. Look at the following events (or at
    /// [`crate::TurnState`]) to learn what happened.
    Elsewhere,
}

/// Context-window usage of the current conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    /// Tokens currently in the context (input + cache creation + cache read of the last
    /// request).
    pub used_tokens: u64,
    /// Size of the context window, when known.
    pub window_tokens: Option<u64>,
    pub used_percent: Option<f64>,
    pub source: UsageSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    Statusline,
    Transcript,
}

/// Subscription allowance windows. Account-wide: the newest observation from any session
/// is the current one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimits {
    /// Rolling 5-hour session allowance.
    pub five_hour: Option<LimitWindow>,
    /// Weekly allowance.
    pub seven_day: Option<LimitWindow>,
    /// Gateway spend limit, when one applies.
    pub spend: Option<SpendLimit>,
    /// Every window as reported, for agents whose windows are not 5 h / 7 d (Codex
    /// reports e.g. a 30-day window on some plans). Empty for Claude Code.
    #[serde(default)]
    pub windows: Vec<RateWindow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateWindow {
    /// Agent's own name for it (`primary`, `secondary`, ...).
    pub name: String,
    pub window_minutes: Option<u64>,
    pub used_percent: f64,
    pub resets_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    /// 0..=100.
    pub used_percent: f64,
    /// Unix milliseconds.
    pub resets_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendLimit {
    pub used_percent: f64,
    pub resets_at_ms: Option<u64>,
    pub used_usd: Option<f64>,
    pub limit_usd: Option<f64>,
    pub period: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub context: Option<ContextUsage>,
    pub rate_limits: Option<RateLimits>,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub session_name: Option<String>,
    pub cost_usd: Option<f64>,
    pub agent_version: Option<String>,
}

/// A program to run, with its environment. The daemon turns it into a tmux command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: std::path::PathBuf,
}

impl CommandSpec {
    /// The command as a single POSIX shell line, without the environment.
    pub fn shell_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// `env K=V ... program args...` as one shell line (for `tmux new-session <cmd>`).
    /// tmux's `-e K=V` is the alternative when the daemon prefers it.
    pub fn shell_line_with_env(&self) -> String {
        if self.env.is_empty() {
            return self.shell_line();
        }
        let env: Vec<String> = self.env.iter().map(|(k, v)| shell_quote(&format!("{k}={v}"))).collect();
        format!("env {} {}", env.join(" "), self.shell_line())
    }
}

/// Quote for POSIX sh. Leaves simple words alone.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./:=@%+,".contains(&b)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// One message in the chat view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    /// Stable id (transcript line uuid of the first line of the message).
    pub id: String,
    pub role: ChatRole,
    /// RFC 3339 timestamp as written by the agent.
    pub timestamp: Option<String>,
    pub model: Option<String>,
    pub parts: Vec<ChatPart>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
    /// Tool results, sent back to the model on the user side.
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChatPart {
    Text {
        text: String,
    },
    /// Reasoning is never copied out; this only marks that there was some.
    Thinking,
    ToolCall {
        id: String,
        name: String,
        summary: String,
    },
    ToolResult {
        tool_use_id: String,
        is_error: bool,
        summary: String,
    },
    Image,
    /// The user interrupted the turn.
    Interrupted,
}

/// A page of chat messages, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatPage {
    pub messages: Vec<ChatMessage>,
    /// Pass as `before` to get the next older page; `None` at the start of the file.
    pub older: Option<u64>,
    /// Byte length of the file when read; pass as `after` later to read only newer lines.
    pub end: u64,
}

/// Coarse state of the latest turn, read from the transcript tail. Used to settle the
/// status after something no hook reports (Claude Code sends no `Stop` on interrupt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnState {
    InProgress,
    Ended,
    Interrupted,
    Unknown,
}

/// Raw key input for typing into a terminal (tmux `send-keys` names or literal text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum KeyInput {
    /// Literal text (`tmux send-keys -l`).
    Text(String),
    /// A named key in tmux syntax: `Enter`, `Escape`, `Up`, `Down`, `Tab`, `C-c`.
    Key(String),
}
