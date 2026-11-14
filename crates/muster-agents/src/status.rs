//! A small reducer from normalised events to the card status model in `docs/PLAN.md`.
//!
//! The daemon keeps one [`StatusTracker`] per card (rebuilt from the card row after a
//! restart with [`StatusTracker::restore`]) and persists what [`StatusTracker::apply`]
//! reports.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::types::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusTracker {
    pub status: AgentStatus,
    pub status_since_ms: u64,
    pub turn_started_at_ms: Option<u64>,
    pub last_activity_at_ms: Option<u64>,
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    /// Pending permission request ids (answerable or not).
    pub pending_permissions: BTreeSet<String>,
    /// A permission prompt or question is open that has no request id (seen only through
    /// a notification or `AskUserQuestion`).
    pub waiting_without_id: bool,
    pub active_subagents: BTreeSet<String>,
}

/// What changed as a result of one event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TrackerChange {
    Status {
        from: AgentStatus,
        to: AgentStatus,
    },
    /// The agent now runs under a different session id (`/clear`, or a resume that forked).
    /// Store `new` as the card's `agentSessionId`.
    SessionId {
        old: Option<String>,
        new: String,
    },
}

impl StatusTracker {
    pub fn new(now_ms: u64) -> Self {
        StatusTracker {
            status: AgentStatus::Idle,
            status_since_ms: now_ms,
            turn_started_at_ms: None,
            last_activity_at_ms: None,
            session_id: None,
            transcript_path: None,
            pending_permissions: BTreeSet::new(),
            waiting_without_id: false,
            active_subagents: BTreeSet::new(),
        }
    }

    /// Rebuild from persisted card fields.
    pub fn restore(status: AgentStatus, status_since_ms: u64, session_id: Option<String>) -> Self {
        StatusTracker {
            status,
            status_since_ms,
            session_id,
            ..StatusTracker::new(status_since_ms)
        }
    }

    fn set(&mut self, to: AgentStatus, now: u64, out: &mut Vec<TrackerChange>) {
        if self.status != to {
            out.push(TrackerChange::Status {
                from: self.status,
                to,
            });
            self.status = to;
            self.status_since_ms = now;
        }
    }

    fn blocked(&self) -> bool {
        !self.pending_permissions.is_empty() || self.waiting_without_id
    }

    /// Back to running once nothing blocks any more (the user answered somewhere).
    fn unblock(&mut self, now: u64, out: &mut Vec<TrackerChange>) {
        if self.status == AgentStatus::NeedsYou && !self.blocked() {
            self.set(AgentStatus::Running, now, out);
        }
    }

    /// Apply one event. Returns the changes to persist and broadcast.
    pub fn apply(&mut self, ev: &NormalizedEvent) -> Vec<TrackerChange> {
        let now = ev.at_ms;
        let mut out = Vec::new();

        if let Some(sid) = &ev.session_id {
            if ev.subagent_id.is_none() && self.session_id.as_deref() != Some(sid) {
                out.push(TrackerChange::SessionId {
                    old: self.session_id.clone(),
                    new: sid.clone(),
                });
                self.session_id = Some(sid.clone());
            }
        }
        if ev.transcript_path.is_some() && ev.subagent_id.is_none() {
            self.transcript_path = ev.transcript_path.clone();
        }
        let housekeeping = match &ev.kind {
            AgentEvent::Usage { .. } => true,
            // helper agents that stop after the turn without ever starting visibly
            AgentEvent::SubagentStopped { agent_id, .. } => {
                !self.active_subagents.contains(agent_id)
            }
            _ => false,
        };
        if !housekeeping {
            self.last_activity_at_ms = Some(now);
        }

        match &ev.kind {
            AgentEvent::SessionStarted { source, .. } => {
                self.pending_permissions.clear();
                self.waiting_without_id = false;
                // A fresh, resumed or cleared session waits for input (an initial prompt
                // follows as UserPromptSubmit). Compaction happens mid-turn.
                if *source != SessionSource::Compact {
                    self.set(AgentStatus::Idle, now, &mut out);
                }
            }
            AgentEvent::SessionEnded { exited, .. } => {
                self.pending_permissions.clear();
                self.waiting_without_id = false;
                if *exited {
                    self.active_subagents.clear();
                    self.set(AgentStatus::Exited, now, &mut out);
                }
            }
            AgentEvent::TurnStarted { .. } => {
                self.pending_permissions.clear();
                self.waiting_without_id = false;
                self.turn_started_at_ms = Some(now);
                self.set(AgentStatus::Running, now, &mut out);
            }
            // Compaction runs mid-turn (auto) or on /compact between turns; either way it
            // ends without a Stop, so it never changes the status by itself.
            AgentEvent::CompactionStarted { .. } | AgentEvent::CompactionFinished { .. } => {}
            AgentEvent::ToolStarted { .. } => {
                if ev.subagent_id.is_none() {
                    self.waiting_without_id = false;
                }
                self.unblock(now, &mut out);
                if matches!(self.status, AgentStatus::Done | AgentStatus::Idle)
                    && ev.subagent_id.is_none()
                {
                    // Work outside a user turn (e.g. a background task woke the agent).
                    self.set(AgentStatus::Running, now, &mut out);
                }
            }
            AgentEvent::ToolFinished { .. } => {
                // The tool ran, so whatever prompt guarded it was answered.
                if ev.subagent_id.is_none() {
                    self.waiting_without_id = false;
                }
                self.unblock(now, &mut out);
            }
            AgentEvent::PermissionRequested { request } => {
                match &request.id {
                    Some(id) => {
                        self.pending_permissions.insert(id.clone());
                    }
                    None => self.waiting_without_id = true,
                }
                self.set(AgentStatus::NeedsYou, now, &mut out);
            }
            AgentEvent::PermissionResolved {
                request_id,
                outcome,
            } => {
                self.pending_permissions.remove(request_id);
                match outcome {
                    PermissionOutcome::Answered { .. } => self.unblock(now, &mut out),
                    // The hook went away without our answer: the user answered in the
                    // terminal (allowed, or rejected with Escape) or the wait ran out and
                    // the terminal prompt is still open. Stay in needs_you until the next
                    // event shows which (a tool event, a new turn, or a turn-state check).
                    PermissionOutcome::Elsewhere => {
                        if self.pending_permissions.is_empty()
                            && self.status == AgentStatus::NeedsYou
                        {
                            self.waiting_without_id = true;
                        }
                    }
                }
            }
            AgentEvent::QuestionAsked { .. } | AgentEvent::InputRequested { .. } => {
                self.waiting_without_id = true;
                self.set(AgentStatus::NeedsYou, now, &mut out);
            }
            AgentEvent::Notification {
                needs_attention: true,
                ..
            } => {
                if self.status != AgentStatus::NeedsYou {
                    self.waiting_without_id = true;
                    self.set(AgentStatus::NeedsYou, now, &mut out);
                }
            }
            AgentEvent::TurnFinished { .. } => {
                self.pending_permissions.clear();
                self.waiting_without_id = false;
                self.set(AgentStatus::Done, now, &mut out);
            }
            AgentEvent::SubagentStarted { agent_id, .. } => {
                self.active_subagents.insert(agent_id.clone());
            }
            AgentEvent::SubagentStopped { agent_id, .. } => {
                self.active_subagents.remove(agent_id);
            }
            AgentEvent::PermissionAutoDenied { .. }
            | AgentEvent::Notification { .. }
            | AgentEvent::Usage { .. }
            | AgentEvent::Other { .. } => {}
        }
        out
    }

    /// The user looked at a finished card: `done` → `idle`.
    pub fn mark_read(&mut self, now_ms: u64) -> Vec<TrackerChange> {
        let mut out = Vec::new();
        if self.status == AgentStatus::Done {
            self.set(AgentStatus::Idle, now_ms, &mut out);
        }
        out
    }

    /// Settle the status from the transcript when no hook will (Claude Code sends no
    /// `Stop` after the user interrupts with Escape). Call it a moment after a
    /// [`PermissionOutcome::Elsewhere`] resolution, or when the terminal goes quiet while
    /// the status is still `running`.
    pub fn apply_turn_state(&mut self, state: TurnState, now_ms: u64) -> Vec<TrackerChange> {
        let mut out = Vec::new();
        if matches!(self.status, AgentStatus::Running | AgentStatus::NeedsYou)
            && matches!(state, TurnState::Interrupted | TurnState::Ended)
        {
            self.pending_permissions.clear();
            self.waiting_without_id = false;
            let to = if state == TurnState::Interrupted {
                AgentStatus::Idle
            } else {
                AgentStatus::Done
            };
            self.set(to, now_ms, &mut out);
        }
        out
    }

    /// The tmux session is gone.
    pub fn exited(&mut self, now_ms: u64) -> Vec<TrackerChange> {
        let mut out = Vec::new();
        self.pending_permissions.clear();
        self.waiting_without_id = false;
        self.set(AgentStatus::Exited, now_ms, &mut out);
        out
    }
}
