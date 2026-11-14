//! Agent CLI integration for Muster.
//!
//! This crate knows how to start, resume and observe coding-agent CLIs. It does not run
//! tmux, store anything or talk WebSocket; the daemon does that and calls in here.
//! Everything is synchronous (std threads and channels) so it can be used from any
//! runtime. Only Claude Code is implemented so far; see `docs/AGENTS.md` for what was
//! verified and the checklist for adding another CLI.
//!
//! # Pieces
//!
//! * [`AgentAdapter`]: one implementation per CLI ([`ClaudeCode`]). Builds launch and
//!   resume commands (with per-launch hook wiring, never editing the CLI's own config),
//!   normalises hook payloads into [`AgentEvent`]s, formats permission answers, gives
//!   terminal key fallbacks, and reads transcripts (paged from the end) and context usage.
//! * `muster-hook` (separate binary crate): the CLI runs it on every hook event; it
//!   forwards `{cardId, agentId, payload}` as one JSON line ([`HookEnvelope`]) to the
//!   daemon's Unix socket and, for permission requests, waits for one line back.
//! * [`HookReceiver`]: listens on that socket, yields [`NormalizedEvent`]s on a channel,
//!   and lets the daemon [`answer`](HookReceiver::answer) pending permission requests.
//! * [`StatusTracker`]: per-card reducer from events to `running` / `needs_you` / `done`
//!   / `idle` / `exited` with the timestamps the tracker UI needs, and session id changes.
//!
//! # Daemon integration
//!
//! ```no_run
//! use std::{path::Path, sync::Arc};
//! use muster_agents::*;
//!
//! # fn main() -> std::io::Result<()> {
//! let home = Path::new("/Users/me/Library/Application Support/Muster");
//! let registry = AdapterRegistry::default();
//! // 1. At startup: listen for hooks.
//! let (receiver, events) = HookReceiver::bind(&home.join("hook.sock"), registry.clone())?;
//! let hooks = HookConfig::new("/Applications/Muster.app/Contents/MacOS/muster-hook",
//!                             home.join("hook.sock"), home.join("agents"));
//!
//! // 2. card.start: build the command, run it in the card's tmux session.
//! let claude = registry.get("claude").unwrap();
//! let mut spec = LaunchSpec::new("card-42", "/Users/me/src/project");
//! spec.session_id = claude.new_session_id();      // store as card.agentSessionId
//! spec.prompt = Some("fix the failing test".into());
//! spec.hooks = Some(hooks.clone());
//! let cmd = claude.launch(&spec)?;                // or claude.resume(&spec, &session_id)?
//! // tmux -L muster new-session -d -s m-card-42 -c <cmd.cwd> -e K=V... '<cmd.shell_line()>'
//! let _line = cmd.shell_line_with_env();
//!
//! // 3. Event loop: feed the per-card tracker, persist and broadcast changes.
//! let mut tracker = StatusTracker::new(now_ms());
//! for ev in events.iter() {
//!     for change in tracker.apply(&ev) {
//!         match change {
//!             TrackerChange::Status { to, .. } => { /* card.status = to; card.changed */ }
//!             TrackerChange::SessionId { new, .. } => { /* card.agentSessionId = new */ }
//!         }
//!     }
//!     match &ev.kind {
//!         AgentEvent::PermissionRequested { request } if request.id.is_some() => {
//!             // show on the card / phone; later:
//!             let _ = receiver.answer(request.id.as_deref().unwrap(),
//!                                     &PermissionDecision::Allow { remember: false });
//!         }
//!         AgentEvent::Usage { usage } => { /* context meter, usage-limit strip */ }
//!         _ => {}
//!     }
//! }
//! # Ok(()) }
//! ```
//!
//! Chat view: `adapter.read_chat(path, None, 50)` for the newest page, `older` from the
//! result for the previous page, and `read_chat_since(path, page.end, ..)` when the
//! transcript grows. The path comes from `NormalizedEvent::transcript_path` (keep the
//! latest per card) or [`AgentAdapter::transcript_path`].
//!
//! Gaps the daemon must cover (details in `docs/AGENTS.md`): Claude Code sends no `Stop`
//! after the user interrupts (Escape), so after a [`PermissionOutcome::Elsewhere`] or when
//! the terminal goes quiet while `running`, check [`AgentAdapter::turn_state`] and call
//! [`StatusTracker::apply_turn_state`]. In a folder the user has not trusted yet, Claude
//! Code shows its trust dialog and no hook runs until it is accepted.

pub mod adapter;
pub mod basic;
mod chat;
pub mod claude;
pub mod codex;
pub mod cursor;
pub mod model;
pub mod opencode;
pub mod receiver;
pub mod status;
pub mod tail;
pub mod types;

pub use adapter::{
    find_on_path, now_ms, AdapterRegistry, AgentAdapter, Capabilities, HookConfig, LaunchSpec, SessionRef, StatusSource,
};
pub use basic::{Antigravity, Gemini};
pub use claude::ClaudeCode;
pub use codex::Codex;
pub use cursor::Cursor;
pub use model::model_label;
pub use opencode::Opencode;
pub use receiver::{AnswerError, HookReceiver, PendingPermission};
pub use status::{StatusTracker, TrackerChange};
pub use types::*;
