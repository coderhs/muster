//! Wire types for the Muster daemon protocol (see `docs/PROTOCOL.md`).
//!
//! Everything here is plain serde data. Field names are camelCase on the wire,
//! timestamps are unix milliseconds and ids are opaque strings.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// A request frame as it arrives on the wire. `p` is kept raw so that the
/// method can be resolved first and parameter errors reported as `invalid`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequestFrame {
    pub t: FrameTag,
    pub id: u64,
    pub m: String,
    #[serde(default)]
    pub p: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FrameTag {
    #[serde(rename = "req")]
    Req,
    #[serde(rename = "res")]
    Res,
    #[serde(rename = "ev")]
    Ev,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResponseFrame {
    pub t: FrameTag,
    pub id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e: Option<ErrorBody>,
}

impl ResponseFrame {
    pub fn ok(id: u64, r: Value) -> Self {
        ResponseFrame { t: FrameTag::Res, id, ok: true, r: Some(r), e: None }
    }
    pub fn err(id: u64, e: ErrorBody) -> Self {
        ResponseFrame { t: FrameTag::Res, id, ok: false, r: None, e: Some(e) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventFrame {
    pub t: FrameTag,
    pub m: String,
    pub p: Value,
}

/// Any frame a client may receive as text.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ServerFrame {
    Response(ResponseFrame),
    Event(EventFrame),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unauthorized,
    NotFound,
    Invalid,
    Conflict,
    Unavailable,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub msg: String,
}

impl ErrorBody {
    pub fn new(code: ErrorCode, msg: impl Into<String>) -> Self {
        ErrorBody { code, msg: msg.into() }
    }
}

impl std::fmt::Display for ErrorBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.msg)
    }
}

impl std::error::Error for ErrorBody {}

// ---------------------------------------------------------------------------
// Binary terminal frames: 0x01 | u16 BE sid length | sid | bytes
// ---------------------------------------------------------------------------

pub const TERM_FRAME_TAG: u8 = 0x01;

pub fn encode_term_frame(sid: &str, data: &[u8]) -> Vec<u8> {
    let sid = sid.as_bytes();
    assert!(sid.len() <= u16::MAX as usize, "sid too long");
    let mut out = Vec::with_capacity(3 + sid.len() + data.len());
    out.push(TERM_FRAME_TAG);
    out.extend_from_slice(&(sid.len() as u16).to_be_bytes());
    out.extend_from_slice(sid);
    out.extend_from_slice(data);
    out
}

/// Returns `(sid, data)` or `None` for a malformed frame.
pub fn decode_term_frame(frame: &[u8]) -> Option<(&str, &[u8])> {
    if frame.len() < 3 || frame[0] != TERM_FRAME_TAG {
        return None;
    }
    let n = u16::from_be_bytes([frame[1], frame[2]]) as usize;
    let rest = &frame[3..];
    if rest.len() < n {
        return None;
    }
    let sid = std::str::from_utf8(&rest[..n]).ok()?;
    Some((sid, &rest[n..]))
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub color: String,
    pub position: i64,
    /// Shell script run in a new worktree before the agent starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_setup: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ColumnCategory {
    Unstarted,
    Started,
    Done,
    Closed,
}

impl ColumnCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            ColumnCategory::Unstarted => "unstarted",
            ColumnCategory::Started => "started",
            ColumnCategory::Done => "done",
            ColumnCategory::Closed => "closed",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "unstarted" => ColumnCategory::Unstarted,
            "started" => ColumnCategory::Started,
            "done" => ColumnCategory::Done,
            "closed" => ColumnCategory::Closed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub color: String,
    pub category: ColumnCategory,
    pub position: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CardStatus {
    Draft,
    Running,
    NeedsYou,
    Done,
    Idle,
    Exited,
}

impl CardStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CardStatus::Draft => "draft",
            CardStatus::Running => "running",
            CardStatus::NeedsYou => "needs_you",
            CardStatus::Done => "done",
            CardStatus::Idle => "idle",
            CardStatus::Exited => "exited",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "draft" => CardStatus::Draft,
            "running" => CardStatus::Running,
            "needs_you" => CardStatus::NeedsYou,
            "done" => CardStatus::Done,
            "idle" => CardStatus::Idle,
            "exited" => CardStatus::Exited,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
    Urgent,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Medium => "medium",
            Priority::High => "high",
            Priority::Urgent => "urgent",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "low" => Priority::Low,
            "medium" => Priority::Medium,
            "high" => Priority::High,
            "urgent" => Priority::Urgent,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: String,
    pub project_id: String,
    pub column_id: String,
    pub rank: String,
    pub title: String,
    pub agent_id: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    pub status: CardStatus,
    pub status_since: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<i64>,
    pub unread: bool,
    pub has_session: bool,
    pub created_at: i64,
    pub updated_at: i64,
    /// Why the card is `needs_you` (phase 3): `permission`, `question`, `input`,
    /// `notification`, `startup_prompt` or `terminal_prompt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_you_reason: Option<String>,
    /// A permission request waiting for the user (phase 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_approval: Option<PendingApproval>,
    /// Context-window usage of the agent's conversation (phase 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextInfo>,
    /// The daemon maintains the title (placeholder, then the first prompt, then the
    /// agent's own session name) until the user sets one.
    #[serde(default)]
    pub title_auto: bool,
    /// Worktree setup state; absent when there is no setup or it succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<SetupInfo>,
    /// The session disappeared without exiting (reboot, tmux server gone); `card.start`
    /// replays the saved scrollback and resumes the agent.
    #[serde(default)]
    pub restorable: bool,
    /// Messages waiting for the card to become idle or done.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queued_messages: Vec<Message>,
    /// Number of comments on the card (`comment.list` returns them).
    #[serde(default)]
    pub comment_count: u32,
    /// Who last changed the notes, and when. Absent until the notes are first edited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_edited_by: Option<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_edited_at: Option<i64>,
    /// Short name of the model the agent last reported using ("Opus 5.5", "gpt-5.5").
    /// Absent until the agent reports one; kept after the session ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The user exempted this card from being stopped when idle.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_running: bool,
    /// When the daemon stopped the session for being idle (ms). Cleared when the
    /// session starts again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_stopped_at: Option<i64>,
    /// Why an idle session was left running at the last check, in words for the user
    /// ("has a scheduled job"). Absent when it is not idle or nothing holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_kept: Option<String>,
    /// The start of the agent's last reply, on one line, when its latest turn ended.
    /// Kept until the next turn ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    /// The person using Muster (the app, or the CLI outside a card).
    User,
    /// The agent running in a card.
    Agent,
}

/// Who wrote a comment or edited notes. The daemon fills this in from the connection
/// (a card's own token means that card's agent); a client cannot choose it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub kind: AuthorKind,
    /// For an agent: its card, that card's agent id and its title when it wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_title: Option<String>,
}

impl Author {
    pub fn user() -> Author {
        Author { kind: AuthorKind::User, card_id: None, agent_id: None, card_title: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    /// The card the comment is on.
    pub card_id: String,
    pub author: Author,
    pub text: String,
    pub created_at: i64,
}

/// An undirected link between two cards; `a` sorts before `b`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub a: String,
    pub b: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    /// The receiving card.
    pub card_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_card_id: Option<String>,
    pub text: String,
    pub created_at: i64,
    /// Set once typed into the receiving session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Prevent idle sleep while any card is running.
    pub keep_awake: bool,
    /// Restore restorable cards automatically when the daemon starts.
    pub auto_restore: bool,
    /// Seconds of terminal silence after which a card without hook-driven status counts
    /// as idle (and a running card's turn is checked). 1..=600.
    #[serde(default = "default_silence")]
    pub silence_seconds: u64,
    /// Let programs in cards set the viewer's clipboard (OSC 52 through tmux). Off by
    /// default: any agent output could otherwise replace the clipboard.
    #[serde(default)]
    pub clipboard_passthrough: bool,
    /// Stop a session that has been idle this many minutes (its card can be resumed).
    /// 0, the default, never stops one.
    #[serde(default)]
    pub idle_stop_minutes: u32,
    /// The file viewer may open any file on the Mac, not only those in the card's folder
    /// and project. Never applies to paired devices.
    #[serde(default)]
    pub any_file_path: bool,
}

fn default_silence() -> u64 {
    5
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            keep_awake: true,
            auto_restore: false,
            silence_seconds: default_silence(),
            clipboard_passthrough: false,
            idle_stop_minutes: 0,
            any_file_path: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// Stable id, e.g. `hook_socket` or `claude_permission_hooks`.
    pub id: String,
    /// `warning` or `info`.
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatus {
    pub version: String,
    /// Short git commit the daemon was built from ("+" appended for a build with
    /// uncommitted changes), and that commit's date (ISO 8601).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_date: Option<String>,
    pub pid: u32,
    pub home: String,
    pub tmux_socket: String,
    /// Agents can report status through hooks (the hook socket is listening).
    pub hooks_available: bool,
    /// Conditions the app should show.
    pub notices: Vec<Notice>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcStats {
    /// Physical footprint in bytes.
    pub mem_bytes: u64,
    /// Percent of one core (can exceed 100).
    pub cpu_percent: f64,
    pub processes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardStats {
    pub card_id: String,
    #[serde(flatten)]
    pub stats: ProcStats,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SetupState {
    Running,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SetupInfo {
    pub state: SetupState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    /// Set when the request can be answered through the agent's hook; absent when only
    /// the terminal-key fallback can answer it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub tool: String,
    /// One line, e.g. "Bash: touch x.txt".
    pub summary: String,
    pub since: i64,
    /// What answer the prompt needs (phase 3b). Older daemons omit it: treat as `tool`.
    #[serde(default)]
    pub kind: ApprovalKind,
    /// For `question`: the questions to answer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<ApprovalQuestion>,
    /// For `plan`: the plan text (markdown), when the agent sent it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_file_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    /// Allow or deny a tool call.
    #[default]
    Tool,
    /// Multiple-choice questions: answer with `decision: "answer"`.
    Question,
    /// A plan to approve (`decision: "approve_plan"`) or send back (`deny` + message).
    Plan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalQuestion {
    /// Full question text; also the key in `answers`.
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    pub options: Vec<ApprovalOption>,
    #[serde(default)]
    pub multi_select: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContextInfo {
    pub used_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_tokens: Option<u64>,
    /// 0..=100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
}

/// Account rate limits of one agent (phase 3). Account-wide: merged from every card of
/// that agent, so a session that has been idle for hours cannot roll the numbers back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<LimitWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<LimitWindow>,
    /// Every window the agent reported, by its own name (Codex: `primary`, `secondary`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<NamedWindow>,
    /// The agent that reported these numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NamedWindow {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<u64>,
    pub used_percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    /// 0..=100.
    pub used_percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    /// `user`, `assistant` or `tool`.
    pub role: String,
    /// RFC 3339, as written by the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub parts: Vec<ChatPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChatPart {
    Text {
        text: String,
    },
    /// Marks that there was reasoning; its text is never copied out.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub label: String,
    pub color: String,
    pub installed: bool,
    /// What Muster can do with this agent (absent from older daemons).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<AgentCapabilities>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatusSource {
    /// The agent's hooks report every state change.
    Hooks,
    /// The agent's own session log, read when the terminal starts or stops output.
    Log,
    /// Coarse running/idle from terminal output only.
    Activity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub status_source: StatusSource,
    /// Tool approvals can be answered from the board.
    pub approvals: bool,
    /// Questions and plans can be answered from the board.
    pub questions: bool,
    /// `chat.page` returns the conversation.
    pub chat: bool,
    /// The card shows context-window usage.
    pub context: bool,
    /// The agent reports account rate limits.
    pub usage: bool,
    /// `card.start` resumes the agent's own session.
    pub resume: bool,
    /// `card.aiRename` can ask the agent to name its session.
    #[serde(default)]
    pub ai_rename: bool,
}

// ---------------------------------------------------------------------------
// Method parameters and results
// ---------------------------------------------------------------------------

/// Deserialises a field that may be absent (outer `None`), explicitly `null`
/// (`Some(None)`, meaning "clear it") or a value.
fn double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Desktop,
    Mobile,
    Cli,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HelloParams {
    pub token: String,
    pub client: ClientKind,
    pub protocol: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HelloResult {
    pub protocol: u32,
    pub daemon_version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Empty {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IdParams {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectListResult {
    pub projects: Vec<Project>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAddParams {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUpdateParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
    /// `null` clears it.
    #[serde(default, deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub worktree_setup: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectResult {
    pub project: Project,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoardGetParams {
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoardResult {
    pub project: Project,
    pub columns: Vec<Column>,
    pub cards: Vec<Card>,
    /// Links that touch a card of this project (phase 5).
    #[serde(default)]
    pub links: Vec<Link>,
}

/// What a file in a project's documentation is.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    /// `CLAUDE.md`: instructions Claude Code reads.
    Claude,
    /// `AGENTS.md`: instructions other agents read.
    Agents,
    /// Any other Markdown file.
    Doc,
}

/// A Markdown file of a project.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DocEntry {
    /// Relative to the project's folder, with `/` separators.
    pub path: String,
    pub kind: DocKind,
    pub size: u64,
    /// Milliseconds since the epoch.
    pub modified_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DocsListParams {
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DocsListResult {
    pub docs: Vec<DocEntry>,
    /// There were more files than the daemon lists.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DocsReadParams {
    pub project_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DocsReadResult {
    pub path: String,
    pub content: String,
    /// When the file was last changed, to hand back with `docs.write`.
    pub modified_at: i64,
    /// The file is longer than the daemon returns.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DocsWriteParams {
    pub project_id: String,
    pub path: String,
    pub content: String,
    /// Make a new file; refused when one exists.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub create: bool,
    /// The `modifiedAt` the text was read with; the save is refused when the file has
    /// changed since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_modified_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DocsWriteResult {
    pub doc: DocEntry,
}

/// How a file differs from the base in `card.changes`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    /// A new file git does not track yet.
    Untracked,
}

/// Which changes `card.changes` lists.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeScope {
    /// Committed and uncommitted: from the base to the files on disk.
    #[default]
    All,
    /// The branch's commits since the base.
    Committed,
    /// What is not committed yet, new files included.
    Uncommitted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardChangesParams {
    pub id: String,
    /// A branch to compare against instead of the automatic one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default)]
    pub scope: ChangeScope,
}

/// One changed file of a card's folder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    /// Relative to the repository's top folder.
    pub path: String,
    /// The path before a rename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub kind: ChangeKind,
    pub additions: u32,
    pub deletions: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub binary: bool,
}

/// `card.changes`: what the card's folder changed against the branch it started from,
/// committed or not.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardChangesResult {
    /// The repository's top folder.
    pub root: String,
    /// The branch checked out in the card's folder; absent when detached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// What the changes are measured against: a branch name, or `HEAD` when the card
    /// works on the base branch itself (then only uncommitted changes show).
    pub base: String,
    /// The base used when none is chosen (`HEAD` when that means uncommitted only).
    pub default_base: String,
    /// Branches that can be chosen as the base, most recently changed first.
    pub branches: Vec<String>,
    pub scope: ChangeScope,
    /// The commit the comparison starts from (short form).
    pub base_commit: String,
    /// Commits on the card's branch since `base_commit`.
    pub commits: u32,
    /// The folder has changes that are not committed.
    pub uncommitted: bool,
    pub files: Vec<ChangedFile>,
    /// More files changed than are listed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardDiffParams {
    pub id: String,
    /// A path from `card.changes` with the same base and scope.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default)]
    pub scope: ChangeScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardDiffResult {
    pub path: String,
    /// The unified diff of the file, as `git diff` prints it.
    pub patch: String,
    /// The patch is longer than the daemon returns.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// One side of a changed image in `card.image`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImageSide {
    /// The file's size.
    pub bytes: u64,
    /// The file, base64; absent when it is larger than the daemon returns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// `card.image`: a changed image file before and after, to show instead of its diff.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardImageResult {
    pub path: String,
    /// The media type, from the file's extension.
    pub mime: String,
    /// The file at the base; absent when it was added.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<ImageSide>,
    /// The file now; absent when it was deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<ImageSide>,
}

/// A file named in a card's terminal or chat, for the file viewer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardFileParams {
    pub id: String,
    /// Relative to the card's folder (then the project's), or absolute. A leading `~/`
    /// is the user's home folder.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardFileResult {
    /// The path to show: relative to the card's folder or the project when inside one,
    /// otherwise absolute.
    pub path: String,
    /// The file's absolute path, links resolved (for opening it in an editor).
    pub abs_path: String,
    /// The file's text (lossy UTF-8); empty for a binary file.
    pub content: String,
    pub size: u64,
    /// The text is cut at the daemon's limit.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// The file is not text.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub binary: bool,
}

/// A time range for `project.summary`, in unix ms: `from` inclusive, `to` exclusive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummaryParams {
    pub project_id: String,
    pub from: i64,
    pub to: i64,
    /// Times strictly inside the range that cut it into parts (the client's local
    /// midnights); each part is also summarised on its own, in `days`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub splits: Vec<i64>,
}

/// How a card's last run inside a range ended.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryOutcome {
    /// Still running at the end of the range.
    Working,
    NeedsYou,
    /// The agent reported the turn as finished.
    Finished,
    /// The run stopped without a finished turn: interrupted, or an agent that only
    /// reports output and silence.
    Idle,
    /// Created in the range and never started.
    NotStarted,
}

/// How a card's session ended inside a range.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummarySessionEnd {
    Exited,
    /// The daemon stopped it for being idle.
    IdleStopped,
}

/// What happened on one card inside a range.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SummaryCard {
    pub id: String,
    pub title: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub column_id: String,
    pub column_title: String,
    /// The card's status now, not at the end of the range.
    pub status: CardStatus,
    pub has_session: bool,
    /// First and last moment of work (or the creation) inside the range.
    pub first_at: i64,
    pub last_at: i64,
    /// Time spent `running` inside the range.
    pub working_ms: i64,
    /// Runs that overlap the range. A run is a stretch of `running`; coming back from
    /// `needs_you` continues the same run.
    pub runs: u32,
    pub outcome: SummaryOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_end: Option<SummarySessionEnd>,
    /// The card was created inside the range.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub created: bool,
    /// A few sentences on what was done, written on request (`project.summarize`).
    /// Only on the cards of the whole range, not on those inside `days`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// When `summary` was written, or when the card was found to have nothing to
    /// summarise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_at: Option<i64>,
    /// The card ran again after its conversation was last read for the summary.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub summary_stale: bool,
    /// The card had no conversation in the range that could be read.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub summary_none: bool,
}

/// How far a `project.summarize` job is: cards done out of those it looks at.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SummaryProgress {
    pub done: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummarizeParams {
    pub project_id: String,
    pub from: i64,
    pub to: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummarizeResult {
    /// `false` when a job for the project was already running.
    pub started: bool,
}

/// Progress of a `project.summarize` job; the texts are read with `project.summary`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SummaryChanged {
    pub project_id: String,
    pub from: i64,
    pub to: i64,
    pub done: u32,
    pub total: u32,
    /// `false` on the job's last event.
    pub running: bool,
    /// Why the job stopped early.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SummaryTotals {
    /// Cards with at least one run in the range.
    pub cards: u32,
    pub working_ms: i64,
    /// Cards whose last run in the range finished.
    pub finished: u32,
    /// Cards created in the range.
    pub created: u32,
}

/// One part of a range cut by `splits`. Parts without cards are left out.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SummaryDay {
    pub from: i64,
    pub to: i64,
    pub cards: Vec<SummaryCard>,
    pub totals: SummaryTotals,
}

/// A card that is running or waiting for the user right now.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SummaryActive {
    pub id: String,
    pub title: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub column_title: String,
    pub status: CardStatus,
    pub status_since: i64,
    pub has_session: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummaryResult {
    pub from: i64,
    pub to: i64,
    /// Cards worked on or created in the range, most recent first.
    pub cards: Vec<SummaryCard>,
    pub totals: SummaryTotals,
    /// Most recent part first; empty without `splits`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub days: Vec<SummaryDay>,
    /// Cards running or needing the user now, whatever the range.
    pub active: Vec<SummaryActive>,
    /// A paragraph on the whole range, written on request (`project.summarize`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_at: Option<i64>,
    /// A card was worked on, or its summary rewritten, after `text` was written.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub text_stale: bool,
    /// Present while a `project.summarize` job for this range is running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarizing: Option<SummaryProgress>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColumnCreateParams {
    pub project_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<ColumnCategory>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColumnUpdateParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<ColumnCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColumnResult {
    pub column: Column,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDeleteParams {
    pub id: String,
    pub move_cards_to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeSpec {
    pub branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardCreateParams {
    pub project_id: String,
    pub column_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default)]
    pub start: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeSpec>,
    /// An existing session of the agent to continue. The card starts by resuming it
    /// instead of opening a new session; `prompt` is then not sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_session_id: Option<String>,
}

/// For the optional fields, an absent key leaves the value unchanged and an
/// explicit `null` clears it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardUpdateParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub prompt: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub notes: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub priority: Option<Option<Priority>>,
    #[serde(default, deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub due_at: Option<Option<i64>>,
    /// `true` hands the title back to the daemon. Setting `title` makes it `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_auto: Option<bool>,
    /// Exempt the card from being stopped when idle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_running: Option<bool>,
    /// The card whose agent makes the change (recorded as the notes' editor). Set by
    /// the daemon for a card token; absent for a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_card_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardMoveParams {
    pub id: String,
    pub column_id: String,
    /// Place the card immediately before (above) this card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    /// Place the card immediately after (below) this card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardDeleteParams {
    pub id: String,
    #[serde(default)]
    pub kill_session: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardResult {
    pub card: Card,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionAttachParams {
    pub card_id: String,
    pub cols: u16,
    pub rows: u16,
    #[serde(default)]
    pub watch: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SizeResult {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardIdParams {
    pub card_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TermWriteParams {
    pub card_id: String,
    pub data: String,
    /// Deliver the bytes to the program as they are, past the session's terminal
    /// client. The client reads input as keys and re-encodes them for the program, so
    /// a key the program's terminal type cannot express (Ctrl+Enter in the "CSI u"
    /// form) would arrive as its plain equivalent.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub raw: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TermResizeParams {
    pub card_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentListResult {
    pub agents: Vec<Agent>,
}

/// Sign-in state and defaults of one agent CLI, read from local files and the CLI's own
/// non-interactive status command. Secrets are never read out.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentAccount {
    pub agent_id: String,
    pub installed: bool,
    /// `None` when Muster cannot tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_in: Option<bool>,
    /// How it is signed in ("Claude account", "API key", "ChatGPT", "Google account").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Extra detail, e.g. opencode's signed-in providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Model new sessions start with, from the CLI's own settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// Why usage limits are not shown for this agent, when they cannot be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_note: Option<String>,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentAccountsResult {
    pub accounts: Vec<AgentAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentAccountsParams {
    /// Check again instead of answering from the cache (kept for a minute).
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Allow,
    Deny,
    /// Answers to a `question` prompt (needs `answers`).
    Answer,
    /// Approve a `plan` prompt.
    ApprovePlan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalAnswerParams {
    pub card_id: String,
    /// The `pendingApproval.requestId` being answered. Absent: answer the prompt open in
    /// the terminal with keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub decision: ApprovalDecision,
    /// Allow and also apply the agent's suggested "always allow" rules (hook path only).
    #[serde(default)]
    pub remember: bool,
    /// Shown to the agent on deny (hook path only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// For `answer`: question text -> chosen option labels (several when multi-select;
    /// a value that is not a label is passed as free text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers: Option<std::collections::BTreeMap<String, Vec<String>>>,
    /// For `approve_plan`: continue in accept-edits mode instead of asking before edits.
    #[serde(default)]
    pub accept_edits: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalAnswerResult {
    /// `hook` or `keys`.
    pub via: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatPageParams {
    pub card_id: String,
    /// Cursor from a previous page's `older`; absent for the newest page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<u64>,
    #[serde(default = "default_chat_limit")]
    pub limit: u32,
}

fn default_chat_limit() -> u32 {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatPageResult {
    /// Oldest first.
    pub messages: Vec<ChatMessage>,
    /// Pass as `before` for the previous page; absent at the start of the conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageResult {
    /// Absent until an agent has reported limits. With several agents reporting, Claude
    /// Code's; kept for older clients, newer ones read `accounts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageInfo>,
    /// One entry per agent that has reported limits.
    #[serde(default)]
    pub accounts: Vec<UsageInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatsSubscribeParams {
    /// Sampling interval; at least 2000, default 5000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatsSubscribeResult {
    pub interval_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsUpdateParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_awake: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_restore: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silence_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard_passthrough: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_stop_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub any_file_path: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsResult {
    pub settings: Settings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LinkParams {
    pub a: String,
    pub b: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContextListParams {
    pub card_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContextListResult {
    pub cards: Vec<Card>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    Transcript,
    Screen,
    Notes,
    Summary,
    Comments,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContextReadParams {
    /// The card to read.
    pub card_id: String,
    /// The reading card (an agent); it must be linked to `cardId`. Absent for a human.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_card_id: Option<String>,
    pub kind: ContextKind,
    /// Transcript messages to include (newest last); default 20, max 200.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextResult {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MessageSendParams {
    pub card_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_card_id: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MessageResult {
    pub message: Message,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardHandoffParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommentAddParams {
    /// The card to comment on.
    pub card_id: String,
    /// The commenting card (an agent). Set by the daemon for a card token; absent for
    /// a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_card_id: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommentResult {
    pub comment: Comment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommentListResult {
    /// Oldest first.
    pub comments: Vec<Comment>,
}

// ---------------------------------------------------------------------------
// Remote access (devices reaching the daemon through `tailscale serve`)
// ---------------------------------------------------------------------------

/// A paired device: a phone, tablet or other computer that reaches the daemon over
/// Tailscale with its own token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TailscaleInfo {
    /// The `tailscale` command was found.
    pub found: bool,
    /// Logged in and connected.
    pub running: bool,
    /// This Mac's MagicDNS name, without the trailing dot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns_name: Option<String>,
    /// The tailnet can issue HTTPS certificates for this Mac.
    pub https: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub enabled: bool,
    /// Loopback port of the remote listener (what `tailscale serve` proxies to).
    pub port: u16,
    /// Port devices use on the Mac's tailnet name.
    pub https_port: u16,
    /// The address devices open, once Tailscale serves it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The built front end was found, so the daemon can serve it.
    pub web_ui: bool,
    pub tailscale: TailscaleInfo,
    /// Why remote access is not working, when it is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSetParams {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemotePairResult {
    /// Opened once on the new device; carries a one-time pairing code.
    pub url: String,
    pub expires_at: i64,
}

/// Every request a client can make, with typed parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum Method {
    Hello(HelloParams),
    ProjectList(Empty),
    ProjectAdd(ProjectAddParams),
    ProjectUpdate(ProjectUpdateParams),
    ProjectRemove(IdParams),
    BoardGet(BoardGetParams),
    ColumnCreate(ColumnCreateParams),
    ColumnUpdate(ColumnUpdateParams),
    ColumnDelete(ColumnDeleteParams),
    CardCreate(CardCreateParams),
    CardUpdate(CardUpdateParams),
    CardMove(CardMoveParams),
    CardDelete(CardDeleteParams),
    CardStart(IdParams),
    CardMarkRead(IdParams),
    SessionAttach(SessionAttachParams),
    SessionDetach(CardIdParams),
    SessionKill(CardIdParams),
    TermWrite(TermWriteParams),
    TermResize(TermResizeParams),
    AgentList(Empty),
    AgentAccounts(AgentAccountsParams),
    ApprovalAnswer(ApprovalAnswerParams),
    ChatPage(ChatPageParams),
    ChatUnsubscribe(CardIdParams),
    UsageGet(Empty),
    CardGet(IdParams),
    StatsSubscribe(StatsSubscribeParams),
    StatsUnsubscribe(Empty),
    SettingsGet(Empty),
    SettingsUpdate(SettingsUpdateParams),
    LinkAdd(LinkParams),
    LinkRemove(LinkParams),
    ContextList(ContextListParams),
    ContextRead(ContextReadParams),
    MessageSend(MessageSendParams),
    MessageCancel(IdParams),
    CardHandoff(CardHandoffParams),
    DaemonStatus(Empty),
    CommentList(CardIdParams),
    CommentAdd(CommentAddParams),
    CommentDelete(IdParams),
    DocsList(DocsListParams),
    DocsRead(DocsReadParams),
    DocsWrite(DocsWriteParams),
    CardAiRename(IdParams),
    CardChanges(CardChangesParams),
    CardDiff(CardDiffParams),
    CardImage(CardDiffParams),
    CardFile(CardFileParams),
    ProjectSummary(ProjectSummaryParams),
    ProjectSummarize(ProjectSummarizeParams),
    RemoteStatus(Empty),
    RemoteSet(RemoteSetParams),
    RemotePair(Empty),
    RemoteRevoke(IdParams),
}

macro_rules! methods {
    ($($variant:ident => $name:literal),* $(,)?) => {
        pub const METHOD_NAMES: &[&str] = &[$($name),*];

        impl Method {
            pub fn name(&self) -> &'static str {
                match self { $(Method::$variant(_) => $name),* }
            }

            pub fn params(&self) -> Value {
                match self {
                    $(Method::$variant(p) => serde_json::to_value(p).expect("params serialise")),*
                }
            }

            /// Resolves a method name and its raw parameters. A `null` or
            /// missing `p` is treated as `{}`.
            pub fn parse(m: &str, p: Value) -> Result<Method, ErrorBody> {
                let p = if p.is_null() { Value::Object(Default::default()) } else { p };
                match m {
                    $($name => serde_json::from_value(p)
                        .map(Method::$variant)
                        .map_err(|e| ErrorBody::new(ErrorCode::Invalid, format!("{}: {e}", $name))),)*
                    _ => Err(ErrorBody::new(ErrorCode::NotFound, format!("unknown method {m}"))),
                }
            }
        }
    };
}

methods! {
    Hello => "hello",
    ProjectList => "project.list",
    ProjectAdd => "project.add",
    ProjectUpdate => "project.update",
    ProjectRemove => "project.remove",
    BoardGet => "board.get",
    ColumnCreate => "column.create",
    ColumnUpdate => "column.update",
    ColumnDelete => "column.delete",
    CardCreate => "card.create",
    CardUpdate => "card.update",
    CardMove => "card.move",
    CardDelete => "card.delete",
    CardStart => "card.start",
    CardMarkRead => "card.markRead",
    SessionAttach => "session.attach",
    SessionDetach => "session.detach",
    SessionKill => "session.kill",
    TermWrite => "term.write",
    TermResize => "term.resize",
    AgentList => "agent.list",
    AgentAccounts => "agent.accounts",
    ApprovalAnswer => "approval.answer",
    ChatPage => "chat.page",
    ChatUnsubscribe => "chat.unsubscribe",
    UsageGet => "usage.get",
    CardGet => "card.get",
    StatsSubscribe => "stats.subscribe",
    StatsUnsubscribe => "stats.unsubscribe",
    SettingsGet => "settings.get",
    SettingsUpdate => "settings.update",
    LinkAdd => "link.add",
    LinkRemove => "link.remove",
    ContextList => "context.list",
    ContextRead => "context.read",
    MessageSend => "message.send",
    MessageCancel => "message.cancel",
    CardHandoff => "card.handoff",
    DaemonStatus => "daemon.status",
    CommentList => "comment.list",
    CommentAdd => "comment.add",
    CommentDelete => "comment.delete",
    DocsList => "docs.list",
    DocsRead => "docs.read",
    DocsWrite => "docs.write",
    CardAiRename => "card.aiRename",
    CardChanges => "card.changes",
    CardDiff => "card.diff",
    CardImage => "card.image",
    CardFile => "card.file",
    ProjectSummary => "project.summary",
    ProjectSummarize => "project.summarize",
    RemoteStatus => "remote.status",
    RemoteSet => "remote.set",
    RemotePair => "remote.pair",
    RemoteRevoke => "remote.revoke",
}

impl Method {
    pub fn to_frame(&self, id: u64) -> RequestFrame {
        RequestFrame { t: FrameTag::Req, id, m: self.name().to_string(), p: self.params() }
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ProjectChanged {
    #[serde(rename_all = "camelCase")]
    Updated { project: Project },
    #[serde(rename_all = "camelCase")]
    Removed { removed_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ColumnChanged {
    #[serde(rename_all = "camelCase")]
    Updated { column: Column },
    #[serde(rename_all = "camelCase")]
    Removed { removed_id: String, project_id: String },
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum CardChanged {
    #[serde(rename_all = "camelCase")]
    Updated { card: Card },
    #[serde(rename_all = "camelCase")]
    Removed { removed_id: String, project_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSize {
    pub card_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionExit {
    pub card_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatAppended {
    pub card_id: String,
    /// New messages, oldest first. A message can arrive again with the same `id` when
    /// the agent extended it; replace the earlier copy.
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageChanged {
    pub usage: UsageInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatsChanged {
    pub cards: Vec<CardStats>,
    /// Sum over all card sessions.
    pub total: ProcStats,
    /// The daemon process itself.
    pub daemon: ProcStats,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LinkChanged {
    pub link: Link,
    pub linked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum CommentChanged {
    #[serde(rename_all = "camelCase")]
    Added { comment: Comment },
    #[serde(rename_all = "camelCase")]
    Removed { removed_id: String, card_id: String },
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    ProjectChanged(ProjectChanged),
    ColumnChanged(ColumnChanged),
    CardChanged(CardChanged),
    SessionSize(SessionSize),
    SessionExit(SessionExit),
    ChatAppended(ChatAppended),
    UsageChanged(UsageChanged),
    StatsChanged(StatsChanged),
    SettingsChanged(SettingsResult),
    LinkChanged(LinkChanged),
    StatusChanged(DaemonStatus),
    CommentChanged(CommentChanged),
    SummaryChanged(SummaryChanged),
    RemoteChanged(RemoteStatus),
}

impl Event {
    pub fn name(&self) -> &'static str {
        match self {
            Event::ProjectChanged(_) => "project.changed",
            Event::ColumnChanged(_) => "column.changed",
            Event::CardChanged(_) => "card.changed",
            Event::SessionSize(_) => "session.size",
            Event::SessionExit(_) => "session.exit",
            Event::ChatAppended(_) => "chat.appended",
            Event::UsageChanged(_) => "usage.changed",
            Event::StatsChanged(_) => "stats.changed",
            Event::SettingsChanged(_) => "settings.changed",
            Event::LinkChanged(_) => "link.changed",
            Event::StatusChanged(_) => "daemon.status",
            Event::CommentChanged(_) => "comment.changed",
            Event::SummaryChanged(_) => "summary.changed",
            Event::RemoteChanged(_) => "remote.changed",
        }
    }

    pub fn to_frame(&self) -> EventFrame {
        let p = match self {
            Event::ProjectChanged(p) => serde_json::to_value(p),
            Event::ColumnChanged(p) => serde_json::to_value(p),
            Event::CardChanged(p) => serde_json::to_value(p),
            Event::SessionSize(p) => serde_json::to_value(p),
            Event::SessionExit(p) => serde_json::to_value(p),
            Event::ChatAppended(p) => serde_json::to_value(p),
            Event::UsageChanged(p) => serde_json::to_value(p),
            Event::StatsChanged(p) => serde_json::to_value(p),
            Event::SettingsChanged(p) => serde_json::to_value(p),
            Event::LinkChanged(p) => serde_json::to_value(p),
            Event::StatusChanged(p) => serde_json::to_value(p),
            Event::CommentChanged(p) => serde_json::to_value(p),
            Event::SummaryChanged(p) => serde_json::to_value(p),
            Event::RemoteChanged(p) => serde_json::to_value(p),
        }
        .expect("event serialises");
        EventFrame { t: FrameTag::Ev, m: self.name().to_string(), p }
    }

    /// Parses an event frame. Unknown events return `None` so that clients
    /// keep working when the daemon adds new ones.
    pub fn from_frame(f: &EventFrame) -> Option<Result<Event, serde_json::Error>> {
        let p = f.p.clone();
        Some(match f.m.as_str() {
            "project.changed" => serde_json::from_value(p).map(Event::ProjectChanged),
            "column.changed" => serde_json::from_value(p).map(Event::ColumnChanged),
            "card.changed" => serde_json::from_value(p).map(Event::CardChanged),
            "session.size" => serde_json::from_value(p).map(Event::SessionSize),
            "session.exit" => serde_json::from_value(p).map(Event::SessionExit),
            "chat.appended" => serde_json::from_value(p).map(Event::ChatAppended),
            "usage.changed" => serde_json::from_value(p).map(Event::UsageChanged),
            "stats.changed" => serde_json::from_value(p).map(Event::StatsChanged),
            "settings.changed" => serde_json::from_value(p).map(Event::SettingsChanged),
            "link.changed" => serde_json::from_value(p).map(Event::LinkChanged),
            "daemon.status" => serde_json::from_value(p).map(Event::StatusChanged),
            "comment.changed" => serde_json::from_value(p).map(Event::CommentChanged),
            "summary.changed" => serde_json::from_value(p).map(Event::SummaryChanged),
            "remote.changed" => serde_json::from_value(p).map(Event::RemoteChanged),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn term_frame_roundtrip() {
        let f = encode_term_frame("abc", b"hello\x1b[0m");
        assert_eq!(&f[..3], &[1, 0, 3]);
        assert_eq!(decode_term_frame(&f), Some(("abc", &b"hello\x1b[0m"[..])));
        assert_eq!(decode_term_frame(&[1, 0, 5, b'a']), None);
        assert_eq!(decode_term_frame(&[2, 0, 0]), None);
        assert_eq!(decode_term_frame(&encode_term_frame("", b"")), Some(("", &b""[..])));
    }

    #[test]
    fn request_parsing() {
        let raw: RequestFrame = serde_json::from_value(json!({
            "t": "req", "id": 7, "m": "card.create",
            "p": {"projectId": "p", "columnId": "c", "agentId": "shell", "worktree": {"branch": "x"}}
        }))
        .unwrap();
        let m = Method::parse(&raw.m, raw.p).unwrap();
        match &m {
            Method::CardCreate(p) => {
                assert_eq!(p.agent_id, "shell");
                assert!(!p.start);
                assert_eq!(p.worktree.as_ref().unwrap().branch, "x");
            }
            _ => panic!(),
        }
        assert_eq!(m.to_frame(7).m, "card.create");
        assert_eq!(Method::parse("project.list", Value::Null).unwrap(), Method::ProjectList(Empty {}));
        assert_eq!(Method::parse("nope", json!({})).unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(Method::parse("card.start", json!({})).unwrap_err().code, ErrorCode::Invalid);
    }

    #[test]
    fn update_null_clears() {
        let p: CardUpdateParams =
            serde_json::from_value(json!({"id": "a", "notes": null, "priority": "high"})).unwrap();
        assert_eq!(p.notes, Some(None));
        assert_eq!(p.priority, Some(Some(Priority::High)));
        assert_eq!(p.prompt, None);
    }

    #[test]
    fn wire_shapes() {
        let r = ResponseFrame::err(3, ErrorBody::new(ErrorCode::NotFound, "x"));
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({"t":"res","id":3,"ok":false,"e":{"code":"not_found","msg":"x"}})
        );
        let ev = Event::CardChanged(CardChanged::Removed { removed_id: "c".into(), project_id: "p".into() });
        assert_eq!(
            serde_json::to_value(ev.to_frame()).unwrap(),
            json!({"t":"ev","m":"card.changed","p":{"removedId":"c","projectId":"p"}})
        );
        let back = Event::from_frame(&ev.to_frame()).unwrap().unwrap();
        assert_eq!(back, ev);
        let card = Card {
            id: "c".into(),
            project_id: "p".into(),
            column_id: "k".into(),
            rank: "V".into(),
            title: "t".into(),
            agent_id: "shell".into(),
            cwd: "/".into(),
            branch: None,
            prompt: None,
            notes: None,
            priority: None,
            due_at: None,
            agent_session_id: None,
            status: CardStatus::NeedsYou,
            status_since: 1,
            turn_started_at: None,
            last_activity_at: None,
            unread: false,
            has_session: true,
            created_at: 1,
            updated_at: 2,
            needs_you_reason: None,
            pending_approval: None,
            context: None,
            title_auto: false,
            setup: None,
            restorable: false,
            queued_messages: vec![],
            comment_count: 0,
            notes_edited_by: None,
            notes_edited_at: None,
            model: None,
            keep_running: false,
            idle_stopped_at: None,
            idle_kept: None,
            last_message: None,
        };
        let v = serde_json::to_value(&card).unwrap();
        assert_eq!(v["status"], "needs_you");
        assert_eq!(v["hasSession"], true);
        assert!(v.get("branch").is_none());
        let s: ServerFrame = serde_json::from_value(json!({"t":"res","id":1,"ok":true,"r":{}})).unwrap();
        assert!(matches!(s, ServerFrame::Response(_)));
        let s: ServerFrame = serde_json::from_value(json!({"t":"ev","m":"session.exit","p":{"cardId":"c"}})).unwrap();
        assert!(matches!(s, ServerFrame::Event(_)));
    }
}
