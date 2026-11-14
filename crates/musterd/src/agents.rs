//! Agent integration: launch/resume through the adapters in `muster-agents`, the
//! embedded hook receiver, per-card status trackers, approvals, chat and usage.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use muster_agents::{
    AdapterRegistry, AgentAdapter, AgentEvent, AgentStatus, AnswerError, HookConfig, HookReceiver, KeyInput,
    LaunchSpec, NormalizedEvent, PermissionDecision, PermissionOutcome, PromptKind, StatusTracker,
};
use muster_core::store::{title_source, AgentState};
use muster_core::util::{err, now_ms};
use muster_core::Result;
use muster_proto::{
    AgentCapabilities, ApprovalAnswerParams, ApprovalDecision, ApprovalKind, ApprovalOption, ApprovalQuestion, Card,
    CardStatus, ChatAppended, ChatMessage, ChatPageParams, ChatPageResult, ContextInfo, ErrorCode, Event, LimitWindow,
    NamedWindow, PendingApproval, StatusSource, UsageChanged, UsageInfo,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::log;
use crate::state::State;

/// How long a launched agent may take to report `SessionStart` before the card is shown
/// as waiting on a startup prompt (Claude Code's folder-trust dialog runs no hooks).
fn session_start_timeout() -> Duration {
    env_ms("MUSTER_SESSION_START_TIMEOUT_MS", 10_000)
}

/// Terminal silence after which a card counts as quiet: the `silenceSeconds` setting
/// (`MUSTER_QUIET_MS` overrides it, for tests).
fn quiet_after(st: &State) -> Duration {
    match std::env::var("MUSTER_QUIET_MS").ok().and_then(|v| v.parse().ok()) {
        Some(ms) => Duration::from_millis(ms),
        None => Duration::from_secs(st.settings().silence_seconds.clamp(1, 600)),
    }
}

fn quiet_secs(st: &State) -> u64 {
    quiet_after(st).as_millis().div_ceil(1000).max(1) as u64
}

/// How long to look for a Codex session file after launch (Codex picks its own id).
const FIND_SESSION_FOR: Duration = Duration::from_secs(20);

/// Where a card's status comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Hook events (Claude Code, Cursor).
    Hooks,
    /// The agent's session log, read at terminal activity/silence edges (Codex).
    Log,
    /// Coarse running/idle from terminal output (shell, opencode, Gemini, ...).
    Activity,
}

/// Delay before checking the transcript after a permission prompt was settled in the
/// terminal (the agent writes the transcript a moment later).
fn settle_delay() -> Duration {
    env_ms("MUSTER_SETTLE_MS", 1_000)
}

/// Least time between `lastActivityAt` writes from terminal output.
const ACTIVITY_WRITE_EVERY_MS: i64 = 10_000;

fn env_ms(key: &str, default: u64) -> Duration {
    Duration::from_millis(std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default))
}

/// What is persisted per card (in `cards.tracker`) so a restart restores it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Persisted {
    tracker: StatusTracker,
    session_started: bool,
    #[serde(default)]
    log_offset: u64,
    /// The per-launch hook secret the running agent holds (re-registered on restart).
    #[serde(default)]
    hook_secret: Option<String>,
}

struct CardAgent {
    tracker: StatusTracker,
    session_started: bool,
    /// Bumped on every launch so stale timers do nothing.
    launch_gen: u64,
    /// How far the session log has been read (log-driven agents).
    log_offset: u64,
    hook_secret: Option<String>,
}

#[derive(Default)]
struct ChatSub {
    conns: HashSet<u64>,
    path: Option<PathBuf>,
    end: u64,
    /// Message ids already sent (opencode, which has no byte offsets).
    seen: HashSet<String>,
    /// A refresh task runs while the chat is open (opencode).
    ticking: bool,
}

#[derive(Default)]
struct Activity {
    /// Last time terminal output was seen (ms).
    last_output: i64,
    /// Last time `lastActivityAt` was written for a card without hooks.
    last_written: i64,
    quiet_timer: bool,
    /// Output seen since the last quiet period (attached sessions).
    active: bool,
}

/// `(argv, env, session id to store)` for a launch through an adapter.
pub type Launch = (Vec<String>, Vec<(String, String)>, Option<String>);

pub struct Agents {
    pub registry: AdapterRegistry,
    receiver: Option<HookReceiver>,
    hooks: Option<HookConfig>,
    cards: Mutex<HashMap<String, CardAgent>>,
    usage: Mutex<Option<UsageInfo>>,
    chat: Mutex<HashMap<String, ChatSub>>,
    activity: Mutex<HashMap<String, Activity>>,
    gen: std::sync::atomic::AtomicU64,
    guidance: std::sync::OnceLock<crate::context::Guidance>,
    /// Why hooks are unavailable, for `daemon.status`.
    pub hook_problem: Option<String>,
    /// Secrets handed to launches that have not reached `on_started` yet.
    launch_secrets: Mutex<HashMap<String, String>>,
}

fn agent_status(s: AgentStatus) -> CardStatus {
    match s {
        AgentStatus::Running => CardStatus::Running,
        AgentStatus::NeedsYou => CardStatus::NeedsYou,
        AgentStatus::Done => CardStatus::Done,
        AgentStatus::Idle => CardStatus::Idle,
        AgentStatus::Exited => CardStatus::Exited,
    }
}

fn card_status_to_agent(s: CardStatus) -> AgentStatus {
    match s {
        CardStatus::Running => AgentStatus::Running,
        CardStatus::NeedsYou => AgentStatus::NeedsYou,
        CardStatus::Done => AgentStatus::Done,
        CardStatus::Idle | CardStatus::Draft => AgentStatus::Idle,
        CardStatus::Exited => AgentStatus::Exited,
    }
}

/// The `muster-hook` binary: `MUSTER_HOOK_BIN`, else next to this executable.
fn find_hook_bin() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MUSTER_HOOK_BIN").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let exe = std::env::current_exe().ok()?;
    let p = exe.parent()?.join("muster-hook");
    p.is_file().then_some(p)
}

/// Bytes for a key in tmux key-name syntax, as a terminal would send them.
fn key_bytes(k: &KeyInput) -> Vec<u8> {
    match k {
        KeyInput::Text(t) => t.as_bytes().to_vec(),
        KeyInput::Key(name) => match name.as_str() {
            "Escape" => b"\x1b".to_vec(),
            "Enter" => b"\r".to_vec(),
            "Tab" => b"\t".to_vec(),
            "Up" => b"\x1b[A".to_vec(),
            "Down" => b"\x1b[B".to_vec(),
            "Right" => b"\x1b[C".to_vec(),
            "Left" => b"\x1b[D".to_vec(),
            "BSpace" => b"\x7f".to_vec(),
            n if n.starts_with("C-") && n.len() == 3 => vec![n.as_bytes()[2].to_ascii_lowercase() & 0x1f],
            n => n.as_bytes().to_vec(),
        },
    }
}

impl Agents {
    /// Sets up the registry and, when the hook binary is available, the receiver on
    /// `home/hook.sock`. Returns the event channel to pass to [`run`].
    pub fn new(home: &Path) -> (Agents, Option<mpsc::UnboundedReceiver<NormalizedEvent>>) {
        let registry = AdapterRegistry::default();
        let mut agents = Agents {
            registry: registry.clone(),
            receiver: None,
            hooks: None,
            cards: Mutex::new(HashMap::new()),
            usage: Mutex::new(None),
            chat: Mutex::new(HashMap::new()),
            activity: Mutex::new(HashMap::new()),
            gen: std::sync::atomic::AtomicU64::new(1),
            guidance: std::sync::OnceLock::new(),
            hook_problem: None,
            launch_secrets: Mutex::new(HashMap::new()),
        };
        let Some(hook_bin) = find_hook_bin() else {
            log!("muster-hook not found next to musterd (set MUSTER_HOOK_BIN); agents run without status hooks");
            agents.hook_problem = Some(
                "The muster-hook program was not found next to musterd, so agents run without live status.".into(),
            );
            return (agents, None);
        };
        let sock = home.join("hook.sock");
        let (receiver, events) = match HookReceiver::bind(&sock, registry) {
            Ok(r) => r,
            Err(e) => {
                log!("hook receiver on {}: {e}; agents run without status hooks", sock.display());
                let len = sock.as_os_str().len();
                agents.hook_problem = Some(if len >= 104 {
                    format!(
                        "The hook socket path is {len} bytes, over macOS's 104-byte limit, so agents run without live \
                         status. Use a shorter MUSTER_HOME."
                    )
                } else {
                    format!("The hook socket could not be opened ({e}), so agents run without live status.")
                });
                return (agents, None);
            }
        };
        // The receiver is thread-based; bridge its channel into the runtime with one
        // thread that sleeps in `recv`.
        let (tx, rx) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("muster-hook-bridge".into())
            .spawn(move || {
                for ev in events.iter() {
                    if tx.send(ev).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn hook bridge thread");
        agents.hooks = Some(HookConfig::new(hook_bin, sock, home.join("agents")));
        agents.receiver = Some(receiver);
        (agents, Some(rx))
    }

    /// Hook events dropped for a missing or wrong secret.
    pub fn dropped_events(&self) -> u64 {
        self.receiver.as_ref().map_or(0, |r| r.dropped_events())
    }

    pub fn hooks_available(&self) -> bool {
        self.receiver.is_some()
    }

    pub fn close(&self) {
        if let Some(r) = &self.receiver {
            r.close();
        }
    }

    /// Instructions and skills passed to every agent launch and resume.
    pub fn set_guidance(&self, g: crate::context::Guidance) {
        let _ = self.guidance.set(g);
    }

    pub fn adapter(&self, agent_id: &str) -> Option<&Arc<dyn AgentAdapter>> {
        self.registry.get(agent_id)
    }

    /// Whether this card's status comes from hooks.
    pub fn hooked(&self, card: &Card) -> bool {
        self.source(card) == Source::Hooks
    }

    pub fn source_of(&self, agent_id: &str) -> Source {
        use muster_agents::adapter::StatusSource as S;
        match self.adapter(agent_id).map(|a| a.capabilities()) {
            Some(c) if c.hooks && self.hooks.is_some() && matches!(c.status_source, S::Hooks | S::HooksAndLog) => {
                Source::Hooks
            }
            Some(c) if matches!(c.status_source, S::SessionLog | S::HooksAndLog) => Source::Log,
            _ => Source::Activity,
        }
    }

    pub fn source(&self, card: &Card) -> Source {
        self.source_of(&card.agent_id)
    }

    /// Capabilities for `agent.list`.
    pub fn capabilities(&self, agent_id: &str) -> AgentCapabilities {
        let status_source = match self.source_of(agent_id) {
            Source::Hooks => StatusSource::Hooks,
            Source::Log => StatusSource::Log,
            Source::Activity => StatusSource::Activity,
        };
        match self.adapter(agent_id).map(|a| a.capabilities()) {
            Some(c) => {
                let answers = c.answer_permissions && status_source == StatusSource::Hooks;
                AgentCapabilities {
                    status_source,
                    approvals: answers,
                    questions: answers,
                    chat: c.transcript || agent_id == "opencode",
                    context: c.context_usage,
                    usage: c.rate_limits,
                    // Antigravity's resume flag is documented but unverified, and there is no
                    // way to learn a conversation id to resume.
                    resume: c.resume && agent_id != "antigravity",
                }
            }
            None => AgentCapabilities {
                status_source,
                approvals: false,
                questions: false,
                chat: false,
                context: false,
                usage: false,
                resume: false,
            },
        }
    }

    pub fn load_usage(&self, json: Option<String>) {
        *self.usage.lock().unwrap() = json.and_then(|j| serde_json::from_str(&j).ok());
    }

    pub fn usage(&self) -> Option<UsageInfo> {
        self.usage.lock().unwrap().clone()
    }

    /// Builds the launch (or resume, when the card has an `agentSessionId`) command for
    /// agents the registry knows. Returns `(argv, env, session id to store)`, or `None` for
    /// agents handled by the simple command table.
    pub fn launch_command(&self, card: &Card) -> Result<Option<Launch>> {
        let Some(adapter) = self.adapter(&card.agent_id) else { return Ok(None) };
        let mut spec = LaunchSpec::new(card.id.clone(), card.cwd.clone());
        spec.hooks = self.hooks.clone();
        // Hooks are accepted only with this launch's secret, so nothing else on the
        // machine can post status for the card.
        if let (Some(receiver), true) = (&self.receiver, self.hooked(card)) {
            let secret = muster_agents::adapter::new_hook_secret();
            receiver.set_card_secret(&card.id, &secret);
            self.launch_secrets.lock().unwrap().insert(card.id.clone(), secret.clone());
            spec.hook_secret = Some(secret);
        }
        if let Some(g) = self.guidance.get() {
            spec.instructions = Some(g.instructions.clone());
            spec.skills_dir = Some(g.skills_dir.clone());
        }
        let (cmd, sid) = match &card.agent_session_id {
            Some(sid) => (adapter.resume(&spec, sid), Some(sid.clone())),
            None => {
                spec.session_id = adapter.prepare_session_id(Path::new(&card.cwd));
                spec.prompt = card.prompt.clone();
                (adapter.launch(&spec), spec.session_id.clone())
            }
        };
        let cmd = cmd.map_err(|e| err(ErrorCode::Internal, format!("building {} command: {e}", card.agent_id)))?;
        let mut argv = vec![cmd.program];
        argv.extend(cmd.args);
        Ok(Some((argv, cmd.env, sid)))
    }

    fn load(&self, st: &State, card: &Card) -> CardAgent {
        let persisted = st
            .store
            .lock()
            .unwrap()
            .tracker(&card.id)
            .ok()
            .flatten()
            .and_then(|j| serde_json::from_str::<Persisted>(&j).ok());
        match persisted {
            Some(mut p) => {
                // Hook connections do not survive a daemon restart: requests that were
                // answerable are now only answerable in the terminal.
                if !p.tracker.pending_permissions.is_empty() {
                    p.tracker.pending_permissions.clear();
                    p.tracker.waiting_without_id = true;
                }
                CardAgent {
                    tracker: p.tracker,
                    session_started: p.session_started,
                    launch_gen: 0,
                    log_offset: p.log_offset,
                    hook_secret: p.hook_secret,
                }
            }
            None => CardAgent {
                tracker: StatusTracker::restore(
                    card_status_to_agent(card.status),
                    card.status_since.max(0) as u64,
                    card.agent_session_id.clone(),
                ),
                session_started: true,
                launch_gen: 0,
                log_offset: 0,
                hook_secret: None,
            },
        }
    }

    /// Writes the tracker-derived fields; returns the updated card.
    fn save(
        &self,
        st: &State,
        card: &Card,
        ca: &CardAgent,
        reason: Option<String>,
        pending: Option<PendingApproval>,
        context: Option<ContextInfo>,
    ) -> Option<Card> {
        let t = &ca.tracker;
        let status = agent_status(t.status);
        let (reason, pending) = if status == CardStatus::NeedsYou { (reason, pending) } else { (None, None) };
        let state = AgentState {
            card_id: card.id.clone(),
            status,
            status_since: t.status_since_ms as i64,
            turn_started_at: t.turn_started_at_ms.map(|v| v as i64),
            last_activity_at: t.last_activity_at_ms.map(|v| v as i64).or(card.last_activity_at),
            agent_session_id: t.session_id.clone().or(card.agent_session_id.clone()),
            needs_you_reason: reason,
            pending_approval: pending,
            context,
            tracker: serde_json::to_string(&Persisted {
                tracker: t.clone(),
                session_started: ca.session_started,
                log_offset: ca.log_offset,
                hook_secret: ca.hook_secret.clone(),
            })
            .ok(),
        };
        match st.store.lock().unwrap().save_agent_state(&state) {
            Ok(c) => Some(c),
            Err(e) => {
                log!("saving agent state for {}: {e}", card.id);
                None
            }
        }
    }

    /// Restores trackers of cards with a live session after a restart.
    pub fn restore_all(&self, st: &Arc<State>) {
        let cards = st.store.lock().unwrap().all_cards().unwrap_or_default();
        for card in cards.iter().filter(|c| st.has_session(&c.id) && !self.hooked(c)) {
            if self.source(card) == Source::Log {
                let ca = self.load(st, card);
                self.cards.lock().unwrap().insert(card.id.clone(), ca);
            }
            arm(st, &card.id, Watch::Silence);
        }
        for card in cards.into_iter().filter(|c| st.has_session(&c.id) && self.hooked(c)) {
            let ca = self.load(st, &card);
            if let (Some(r), Some(secret)) = (&self.receiver, &ca.hook_secret) {
                r.set_card_secret(&card.id, secret);
            }
            let pending = card.pending_approval.clone().map(|p| PendingApproval { request_id: None, ..p });
            self.save(st, &card, &ca, card.needs_you_reason.clone(), pending, card.context.clone());
            let status = ca.tracker.status;
            self.cards.lock().unwrap().insert(card.id.clone(), ca);
            self.sync_quiet(st, &card.id, AgentStatus::Idle, status);
        }
    }

    /// A session was just started for `card` (with `session_id` stored on the card).
    pub fn on_started(self: &Arc<Self>, st: &Arc<State>, card: &Card) {
        match self.source(card) {
            Source::Hooks => {}
            Source::Activity => {
                arm(st, &card.id, Watch::Silence);
                return;
            }
            Source::Log => {
                self.start_log_card(st, card);
                return;
            }
        }
        let gen = self.gen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let now = now_ms() as u64;
        let mut tracker = StatusTracker::new(now);
        tracker.status = AgentStatus::Running;
        tracker.session_id = card.agent_session_id.clone();
        let hook_secret = self.launch_secrets.lock().unwrap().remove(&card.id);
        let ca = CardAgent { tracker, session_started: false, launch_gen: gen, log_offset: 0, hook_secret };
        self.save(st, card, &ca, None, None, card.context.clone());
        self.cards.lock().unwrap().insert(card.id.clone(), ca);
        self.sync_quiet(st, &card.id, AgentStatus::Idle, AgentStatus::Running);
        let (agents, st, id) = (self.clone(), st.clone(), card.id.clone());
        tokio::spawn(async move {
            tokio::time::sleep(session_start_timeout()).await;
            agents.startup_timeout(&st, &id, gen);
        });
    }

    fn startup_timeout(&self, st: &Arc<State>, card_id: &str, gen: u64) {
        if !st.has_session(card_id) {
            return;
        }
        let Ok(card) = st.store.lock().unwrap().get_card(card_id) else { return };
        let mut cards = self.cards.lock().unwrap();
        let Some(ca) = cards.get_mut(card_id).filter(|c| c.launch_gen == gen && !c.session_started) else {
            return;
        };
        log!("card {card_id}: no SessionStart from {} yet; it is waiting at a startup prompt", card.agent_id);
        let before = ca.tracker.status;
        ca.tracker.status = AgentStatus::NeedsYou;
        ca.tracker.status_since_ms = now_ms() as u64;
        self.sync_quiet(st, card_id, before, AgentStatus::NeedsYou);
        ca.tracker.waiting_without_id = true;
        if let Some(c) = self.save(st, &card, ca, Some("startup_prompt".into()), None, card.context.clone()) {
            drop(cards);
            st.emit_card(c);
        }
    }

    /// The card's session is gone (the store already says `exited`).
    pub fn on_exit(&self, st: &State, card_id: &str) {
        if let Some(r) = &self.receiver {
            r.remove_card_secret(card_id);
        }
        let removed = self.cards.lock().unwrap().remove(card_id);
        if let Some(mut ca) = removed {
            ca.tracker.exited(now_ms() as u64);
            let card = st.store.lock().unwrap().get_card(card_id);
            if let Ok(card) = card {
                self.save(st, &card, &ca, None, None, card.context.clone());
            }
        }
        self.activity.lock().unwrap().remove(card_id);
    }

    /// `card.markRead`: keeps the tracker in step (`done` → `idle`).
    pub fn mark_read(&self, st: &State, card: &Card) -> Option<Card> {
        let mut cards = self.cards.lock().unwrap();
        let ca = cards.get_mut(&card.id)?;
        if ca.tracker.mark_read(now_ms() as u64).is_empty() {
            return None;
        }
        self.save(st, card, ca, card.needs_you_reason.clone(), card.pending_approval.clone(), card.context.clone())
    }

    pub fn chat_unsubscribe(&self, card_id: &str, conn: u64) {
        let mut chat = self.chat.lock().unwrap();
        if let Some(sub) = chat.get_mut(card_id) {
            sub.conns.remove(&conn);
            if sub.conns.is_empty() {
                chat.remove(card_id);
            }
        }
    }

    pub fn chat_unsubscribe_all(&self, conn: u64) {
        let mut chat = self.chat.lock().unwrap();
        chat.retain(|_, s| {
            s.conns.remove(&conn);
            !s.conns.is_empty()
        });
    }

    fn transcript(&self, st: &State, card: &Card) -> Option<(Arc<dyn AgentAdapter>, PathBuf)> {
        let adapter = self.adapter(&card.agent_id)?.clone();
        let known = self.cards.lock().unwrap().get(&card.id).map(|c| c.tracker.transcript_path.clone());
        let known = known.unwrap_or_else(|| self.load(st, card).tracker.transcript_path);
        let path = known.map(PathBuf::from).or_else(|| {
            card.agent_session_id.as_deref().and_then(|s| adapter.transcript_path(s, Path::new(&card.cwd)))
        })?;
        // Only files under the agent's own transcript directory are ever read.
        let path = adapter.validate_transcript_path(&path)?;
        Some((adapter, path))
    }

    /// The newest `limit` transcript messages of a card, if it has a transcript.
    pub fn transcript_messages(
        &self,
        st: &State,
        card: &Card,
        limit: usize,
    ) -> Option<Vec<muster_agents::ChatMessage>> {
        let (adapter, path) = self.transcript(st, card).filter(|(_, p)| p.is_file())?;
        blocking(|| adapter.read_chat(&path, None, limit)).ok().map(|p| p.messages)
    }

    pub async fn chat_page(
        self: &Arc<Self>,
        st: &Arc<State>,
        conn: u64,
        card: &Card,
        p: &ChatPageParams,
    ) -> Result<ChatPageResult> {
        if card.agent_id == "opencode" {
            return self.chat_page_opencode(st, conn, card, p).await;
        }
        let Some((adapter, path)) = self.transcript(st, card).filter(|(_, p)| p.is_file()) else {
            if p.before.is_none() {
                self.chat.lock().unwrap().entry(card.id.clone()).or_default().conns.insert(conn);
            }
            return Ok(ChatPageResult { messages: vec![], older: None });
        };
        let limit = (p.limit as usize).clamp(1, 500);
        let page = blocking(|| adapter.read_chat(&path, p.before, limit))
            .map_err(|e| err(ErrorCode::Unavailable, format!("reading transcript: {e}")))?;
        if p.before.is_none() {
            let mut chat = self.chat.lock().unwrap();
            let sub = chat.entry(card.id.clone()).or_default();
            if sub.path.as_deref() != Some(path.as_path()) {
                sub.path = Some(path.clone());
                sub.end = page.end;
            }
            sub.conns.insert(conn);
        }
        Ok(ChatPageResult { messages: convert_messages(page.messages), older: page.older })
    }

    /// opencode keeps its conversation behind `opencode api`; the whole list is fetched
    /// (one CLI call) and, while the chat is open, refreshed every few seconds.
    async fn chat_page_opencode(
        self: &Arc<Self>,
        st: &Arc<State>,
        conn: u64,
        card: &Card,
        p: &ChatPageParams,
    ) -> Result<ChatPageResult> {
        let msgs = self.opencode_messages(card).await.unwrap_or_default();
        let limit = (p.limit as usize).clamp(1, 500);
        let end = p.before.map_or(msgs.len(), |b| (b as usize).min(msgs.len()));
        let start = end.saturating_sub(limit);
        let page: Vec<_> = msgs[start..end].to_vec();
        if p.before.is_none() {
            let start_ticker = {
                let mut chat = self.chat.lock().unwrap();
                let sub = chat.entry(card.id.clone()).or_default();
                sub.seen.extend(msgs.iter().map(|m| m.id.clone()));
                sub.conns.insert(conn);
                !std::mem::replace(&mut sub.ticking, true)
            };
            if start_ticker {
                let (agents, st, id) = (self.clone(), st.clone(), card.id.clone());
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        let open = agents.chat.lock().unwrap().get(&id).is_some_and(|s| !s.conns.is_empty());
                        if !open {
                            if let Some(s) = agents.chat.lock().unwrap().get_mut(&id) {
                                s.ticking = false;
                            }
                            return;
                        }
                        let card = st.store.lock().unwrap().get_card(&id);
                        let Ok(card) = card else { return };
                        agents.push_chat_opencode(&st, &card).await;
                    }
                });
            }
        }
        Ok(ChatPageResult { messages: convert_messages(page), older: (start > 0).then_some(start as u64) })
    }

    /// Pushes new transcript messages to clients that have the chat open.
    fn push_chat(&self, st: &State, card: &Card) {
        let (conns, path, end) = {
            let chat = self.chat.lock().unwrap();
            let Some(sub) = chat.get(&card.id) else { return };
            (sub.conns.clone(), sub.path.clone(), sub.end)
        };
        let Some((adapter, current)) = self.transcript(st, card) else { return };
        // A new transcript (e.g. after /clear) starts from its beginning.
        let after = if path.as_deref() == Some(current.as_path()) { end } else { 0 };
        let Ok(page) = blocking(|| adapter.read_chat_since(&current, after, 200)) else { return };
        if let Some(sub) = self.chat.lock().unwrap().get_mut(&card.id) {
            sub.path = Some(current);
            sub.end = page.end;
        }
        if !page.messages.is_empty() {
            let ev = Event::ChatAppended(ChatAppended {
                card_id: card.id.clone(),
                messages: convert_messages(page.messages),
            });
            st.clients.send_to(&conns, &ev);
        }
    }

    /// `approval.answer`: through the waiting hook when possible, else with the
    /// agent's terminal keys.
    pub async fn answer(&self, st: &Arc<State>, card: &Card, p: &ApprovalAnswerParams) -> Result<&'static str> {
        let decision = match p.decision {
            ApprovalDecision::Allow => PermissionDecision::Allow { remember: p.remember },
            ApprovalDecision::Deny => PermissionDecision::Deny { message: p.message.clone(), interrupt: false },
            ApprovalDecision::Answer => PermissionDecision::Answer {
                answers: p
                    .answers
                    .clone()
                    .filter(|a| !a.is_empty())
                    .ok_or_else(|| err(ErrorCode::Invalid, "decision \"answer\" needs answers"))?,
            },
            ApprovalDecision::ApprovePlan => PermissionDecision::ApprovePlan { accept_edits: p.accept_edits },
        };
        if let (Some(rid), Some(receiver)) = (&p.request_id, &self.receiver) {
            if receiver.pending().iter().any(|q| q.request.id.as_deref() == Some(rid) && q.card_id == card.id) {
                match receiver.answer(rid, &decision) {
                    Ok(()) => return Ok("hook"),
                    Err(AnswerError::NotFound) => {}
                    Err(AnswerError::Io(e)) => log!("answering {rid} through the hook failed: {e}; using keys"),
                }
            }
        }
        if card.status != CardStatus::NeedsYou {
            return Err(err(ErrorCode::Conflict, "the card is not waiting for an answer"));
        }
        if !st.has_session(&card.id) {
            return Err(err(ErrorCode::NotFound, "no live session"));
        }
        let adapter = self
            .adapter(&card.agent_id)
            .ok_or_else(|| err(ErrorCode::Unavailable, "this agent has no terminal answer keys"))?;
        let keys = adapter
            .permission_keys(&decision)
            .ok_or_else(|| err(ErrorCode::Unavailable, "no reliable terminal keys for that decision"))?;
        let bytes: Vec<u8> = keys.iter().flat_map(key_bytes).collect();
        st.hub.write(&card.id, &bytes).await?;
        Ok("keys")
    }

    /// Feeds one hook event through the card's tracker, persists and broadcasts.
    fn handle(self: &Arc<Self>, st: &Arc<State>, ev: NormalizedEvent) {
        if std::env::var_os("MUSTER_DEBUG_EVENTS").is_some() {
            log!("event for {}: {:?}", ev.card_id, ev.kind);
        }
        let Ok(card) = st.store.lock().unwrap().get_card(&ev.card_id) else { return };
        let mut cards = self.cards.lock().unwrap();
        if !cards.contains_key(&card.id) {
            let ca = self.load(st, &card);
            cards.insert(card.id.clone(), ca);
        }
        let ca = cards.get_mut(&card.id).unwrap();
        let before = (ca.tracker.clone(), card.clone());
        let mut reason = card.needs_you_reason.clone();
        let mut pending = card.pending_approval.clone();
        let mut context = card.context.clone();
        let mut check_turn = false;
        let mut auto_title: Option<(String, &str)> = None;
        let mut usage_changed = None;
        let mut session_changed = false;

        let changes = ca.tracker.apply(&ev);
        for c in &changes {
            if let muster_agents::TrackerChange::SessionId { new, .. } = c {
                log!("card {}: agent session id is now {new}", card.id);
                session_changed = true;
            }
        }
        match &ev.kind {
            AgentEvent::SessionStarted { .. } => {
                ca.session_started = true;
                reason = None;
                pending = None;
            }
            AgentEvent::PermissionRequested { request } => {
                let (kind, questions, plan, plan_file_path) = match &request.prompt {
                    PromptKind::Tool => (ApprovalKind::Tool, vec![], None, None),
                    PromptKind::Question { questions } => (
                        ApprovalKind::Question,
                        questions
                            .iter()
                            .map(|q| ApprovalQuestion {
                                question: q.question.clone(),
                                header: q.header.clone(),
                                options: q
                                    .options
                                    .iter()
                                    .map(|o| ApprovalOption {
                                        label: o.label.clone(),
                                        description: o.description.clone(),
                                    })
                                    .collect(),
                                multi_select: q.multi_select,
                            })
                            .collect(),
                        None,
                        None,
                    ),
                    PromptKind::Plan { plan, plan_file_path } => {
                        (ApprovalKind::Plan, vec![], plan.clone(), plan_file_path.clone())
                    }
                };
                reason = Some(
                    match kind {
                        ApprovalKind::Tool => "permission",
                        ApprovalKind::Question => "question",
                        ApprovalKind::Plan => "plan",
                    }
                    .into(),
                );
                pending = Some(PendingApproval {
                    request_id: request.id.clone(),
                    tool: request.tool.clone(),
                    summary: request.summary.clone(),
                    since: ev.at_ms as i64,
                    kind,
                    questions,
                    plan,
                    plan_file_path,
                });
            }
            AgentEvent::PermissionResolved { request_id, outcome } => {
                if pending.as_ref().and_then(|p| p.request_id.as_deref()) == Some(request_id.as_str()) {
                    match outcome {
                        PermissionOutcome::Answered { .. } => pending = None,
                        // Settled in the terminal, or the hook's wait ran out with the
                        // prompt still open: only keys can answer it now.
                        PermissionOutcome::Elsewhere => {
                            if let Some(p) = pending.as_mut() {
                                p.request_id = None;
                            }
                        }
                    }
                }
                if matches!(outcome, PermissionOutcome::Elsewhere) {
                    check_turn = true;
                }
            }
            AgentEvent::QuestionAsked { .. } => reason = Some("question".into()),
            AgentEvent::InputRequested { .. } => reason = Some("input".into()),
            AgentEvent::Notification { needs_attention: true, .. } if reason.is_none() => {
                reason = Some("notification".into())
            }
            AgentEvent::TurnStarted { prompt } => {
                pending = None;
                if let Some(t) = prompt.as_deref().and_then(title_from_prompt) {
                    auto_title = Some((t, title_source::PROMPT));
                }
            }
            AgentEvent::TurnFinished { .. } | AgentEvent::SessionEnded { .. } => pending = None,
            AgentEvent::Usage { usage } => {
                if let Some(name) = usage.session_name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                    if card.title_auto && card.title != name {
                        auto_title = Some((name.chars().take(80).collect(), title_source::AGENT));
                    }
                }
                if let Some(c) = &usage.context {
                    context = Some(ContextInfo {
                        used_tokens: c.used_tokens,
                        window_tokens: c.window_tokens,
                        percent: c.used_percent,
                    });
                }
                if let Some(rl) = &usage.rate_limits {
                    let win = |w: &Option<muster_agents::LimitWindow>| {
                        w.as_ref().map(|w| LimitWindow {
                            used_percent: w.used_percent,
                            resets_at: w.resets_at_ms.map(|v| v as i64),
                        })
                    };
                    let windows = rl
                        .windows
                        .iter()
                        .map(|w| NamedWindow {
                            name: w.name.clone(),
                            window_minutes: w.window_minutes,
                            used_percent: w.used_percent,
                            resets_at: w.resets_at_ms.map(|v| v as i64),
                        })
                        .collect();
                    let info = UsageInfo {
                        five_hour: win(&rl.five_hour),
                        seven_day: win(&rl.seven_day),
                        windows,
                        agent_id: Some(card.agent_id.clone()),
                        updated_at: now_ms(),
                    };
                    let mut cur = self.usage.lock().unwrap();
                    let differs = cur.as_ref().is_none_or(|c| {
                        c.five_hour != info.five_hour || c.seven_day != info.seven_day || c.windows != info.windows
                    });
                    *cur = Some(info.clone());
                    if differs {
                        usage_changed = Some(info);
                    }
                }
            }
            _ => {}
        }
        if ca.tracker.status == AgentStatus::NeedsYou && reason.is_none() {
            reason = Some(if pending.is_some() { "permission" } else { "terminal_prompt" }.into());
        }
        if matches!(ev.kind, AgentEvent::TurnFinished { .. }) && context.is_none() {
            if let Some(path) = ca.tracker.transcript_path.as_deref() {
                if let Some(a) = self.adapter(&card.agent_id) {
                    let valid = a.validate_transcript_path(Path::new(path));
                    if let Some(Ok(Some(c))) = valid.map(|p| blocking(|| a.context_usage(&p))) {
                        context = Some(ContextInfo {
                            used_tokens: c.used_tokens,
                            window_tokens: c.window_tokens,
                            percent: c.used_percent,
                        });
                    }
                }
            }
        }

        // Status-line refreshes arrive often; skip the write when nothing moved.
        let unchanged = changes.is_empty()
            && !session_changed
            && reason == before.1.needs_you_reason
            && pending == before.1.pending_approval
            && context == before.1.context
            && matches!(ev.kind, AgentEvent::Usage { .. });
        let saved = if unchanged { None } else { self.save(st, &card, ca, reason, pending, context) };
        let after_status = ca.tracker.status;
        drop(cards);
        self.sync_quiet(st, &card.id, before.0.status, after_status);

        if let Some(info) = usage_changed {
            if let Ok(j) = serde_json::to_string(&info) {
                let _ = st.store.lock().unwrap().meta_set("usage", &j);
            }
            st.clients.broadcast(&Event::UsageChanged(UsageChanged { usage: info }));
        }
        if let Some(c) = saved {
            let visible = |c: &Card| {
                (
                    c.status,
                    c.status_since,
                    c.turn_started_at,
                    c.unread,
                    c.agent_session_id.clone(),
                    c.needs_you_reason.clone(),
                    c.pending_approval.clone(),
                    c.context.clone(),
                )
            };
            // lastActivityAt alone changes on every event; it is sent with the next
            // visible change rather than as a stream of card.changed.
            if visible(&c) != visible(&before.1) {
                st.emit_card(c.clone());
            }
            self.push_chat(st, &c);
        }
        if let Some((title, source)) = auto_title.filter(|_| card.title_auto) {
            let titled = st.store.lock().unwrap().set_auto_title(&card.id, &title, source);
            if let Ok(Some(c)) = titled {
                st.emit_card(c);
            }
        }
        if check_turn {
            let (agents, st, id) = (self.clone(), st.clone(), card.id.clone());
            tokio::spawn(async move {
                tokio::time::sleep(settle_delay()).await;
                agents.check_turn_state(&st, &id);
            });
        }
    }

    /// Settles the status from the transcript when no hook will report it (an
    /// interrupted turn sends no Stop).
    fn check_turn_state(&self, st: &Arc<State>, card_id: &str) {
        let Ok(card) = st.store.lock().unwrap().get_card(card_id) else { return };
        let Some(adapter) = self.adapter(&card.agent_id).cloned() else { return };
        let mut cards = self.cards.lock().unwrap();
        let Some(ca) = cards.get_mut(card_id) else { return };
        if !matches!(ca.tracker.status, AgentStatus::Running | AgentStatus::NeedsYou) {
            return;
        }
        let Some(path) = ca.tracker.transcript_path.clone() else { return };
        let Some(path) = adapter.validate_transcript_path(Path::new(&path)) else { return };
        let Ok(state) = blocking(|| adapter.turn_state(&path)) else { return };
        let before = ca.tracker.status;
        if ca.tracker.apply_turn_state(state, now_ms() as u64).is_empty() {
            return;
        }
        let after = ca.tracker.status;
        log!("card {card_id}: turn state {state:?} from the transcript");
        let saved = self.save(st, &card, ca, None, None, card.context.clone());
        drop(cards);
        self.sync_quiet(st, card_id, before, after);
        if let Some(c) = saved {
            st.emit_card(c.clone());
            self.push_chat(st, &c);
        }
    }

    /// Keeps tmux's silence alert armed exactly while a hooked card is `running`, so a
    /// turn that ends without a hook (interrupt) is noticed even with nobody attached.
    fn sync_quiet(&self, st: &Arc<State>, card_id: &str, before: AgentStatus, after: AgentStatus) {
        if (before == AgentStatus::Running) == (after == AgentStatus::Running) {
            return;
        }
        // Each task applies the card's state at the time it runs, so tasks that run out
        // of order on a busy machine still leave tmux matching the latest status.
        let (st, id) = (st.clone(), card_id.to_string());
        tokio::spawn(async move {
            if st.agents.status_of(&id) == Some(AgentStatus::Running) {
                st.backend.watch_silence(&id, quiet_secs(&st)).await;
            } else {
                st.backend.unwatch_quiet(&id).await;
            }
        });
    }

    /// tmux reports the session has been silent.
    pub async fn on_silence(self: &Arc<Self>, st: &Arc<State>, card_id: &str) {
        let Ok(card) = st.store.lock().unwrap().get_card(card_id) else { return };
        match self.source(&card) {
            Source::Hooks => {
                // Check the turn, then wait for output if the card is still running.
                self.check_turn_state(st, card_id);
                if self.status_of(card_id) == Some(AgentStatus::Running) {
                    st.backend.watch_activity(card_id).await;
                }
            }
            Source::Log => {
                self.read_log(st, card_id);
                self.check_turn_state(st, card_id);
                st.backend.watch_activity(card_id).await;
            }
            Source::Activity => {
                self.set_coarse(st, &card, CardStatus::Idle).await;
                st.backend.watch_activity(card_id).await;
            }
        }
    }

    /// Output after a silence.
    pub async fn on_activity(self: &Arc<Self>, st: &Arc<State>, card_id: &str) {
        let Ok(card) = st.store.lock().unwrap().get_card(card_id) else { return };
        match self.source(&card) {
            Source::Hooks => {
                if self.status_of(card_id) == Some(AgentStatus::Running) {
                    st.backend.watch_silence(card_id, quiet_secs(st)).await;
                }
            }
            Source::Log => {
                self.read_log(st, card_id);
                // The agent writes the turn start a moment after the terminal reacts.
                let (agents, st2, id) = (self.clone(), st.clone(), card_id.to_string());
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    agents.read_log(&st2, &id);
                });
                st.backend.watch_silence(card_id, quiet_secs(st)).await;
            }
            Source::Activity => {
                self.set_coarse(st, &card, CardStatus::Running).await;
                st.backend.watch_silence(card_id, quiet_secs(st)).await;
            }
        }
    }

    /// Coarse status for cards without hooks or a log: running while the terminal
    /// produces output, idle once it has been quiet for the silence window. opencode
    /// is asked (one CLI call, only at these edges) whether its session is still busy.
    async fn set_coarse(&self, st: &Arc<State>, card: &Card, to: CardStatus) {
        let mut to = to;
        let mut context = None;
        if card.agent_id == "opencode" {
            if let Some(s) = self.opencode_session(card).await {
                if to == CardStatus::Idle && s.busy {
                    to = CardStatus::Running;
                }
                context = s.context.map(|c| ContextInfo {
                    used_tokens: c.used_tokens,
                    window_tokens: c.window_tokens,
                    percent: c.used_percent,
                });
            }
        }
        if card.status == to && (context.is_none() || context == card.context) {
            return;
        }
        if card.status == CardStatus::Draft || card.status == CardStatus::Exited || card.setup.is_some() {
            return;
        }
        let r = {
            let mut store = st.store.lock().unwrap();
            let r = store.set_status(&card.id, to);
            if let Some(c) = &context {
                let _ = store.set_context(&card.id, Some(c));
            }
            let _ = store.touch_activity(&card.id, now_ms());
            r.and_then(|_| store.get_card(&card.id))
        };
        if let Ok(c) = r {
            st.emit_card(c.clone());
            if card.agent_id == "opencode" {
                self.push_chat_opencode(st, &c).await;
            }
        }
    }

    /// Codex: find the session file the launch created (Codex picks its own id), with a
    /// short bounded retry, then follow its log.
    fn start_log_card(self: &Arc<Self>, st: &Arc<State>, card: &Card) {
        let now = now_ms() as u64;
        let mut tracker = StatusTracker::new(now);
        tracker.status = AgentStatus::Running;
        tracker.session_id = card.agent_session_id.clone();
        let resumed = card.agent_session_id.is_some();
        let ca = CardAgent { tracker, session_started: true, launch_gen: 0, log_offset: 0, hook_secret: None };
        self.save(st, card, &ca, None, None, card.context.clone());
        self.cards.lock().unwrap().insert(card.id.clone(), ca);
        arm(st, &card.id, Watch::Silence);
        let Some(adapter) = self.adapter(&card.agent_id).cloned() else { return };
        let (agents, st, card) = (self.clone(), st.clone(), card.clone());
        tokio::spawn(async move {
            let deadline = std::time::Instant::now() + FIND_SESSION_FOR;
            let found = loop {
                let r = if resumed {
                    card.agent_session_id.as_deref().and_then(|s| adapter.transcript_path(s, Path::new(&card.cwd))).map(
                        |p| muster_agents::adapter::SessionRef {
                            session_id: card.agent_session_id.clone().unwrap(),
                            transcript: Some(p),
                        },
                    )
                } else {
                    adapter.find_session(Path::new(&card.cwd), now)
                };
                if r.is_some() || std::time::Instant::now() > deadline {
                    break r;
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            };
            let Some(found) = found else {
                log!("card {}: no {} session file appeared", card.id, card.agent_id);
                return;
            };
            {
                let mut cards = agents.cards.lock().unwrap();
                let Some(ca) = cards.get_mut(&card.id) else { return };
                ca.tracker.session_id = Some(found.session_id.clone());
                ca.tracker.transcript_path = found.transcript.as_ref().map(|p| p.to_string_lossy().into_owned());
                if resumed {
                    // Skip the history of a resumed session.
                    ca.log_offset =
                        found.transcript.as_ref().and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len());
                }
            }
            if !resumed {
                let _ = st.store.lock().unwrap().set_agent_session_id(&card.id, Some(&found.session_id));
                log!("card {}: {} session {}", card.id, card.agent_id, found.session_id);
            }
            agents.read_log(&st, &card.id);
            let c = st.store.lock().unwrap().get_card(&card.id);
            if let Ok(c) = c {
                st.emit_card(c);
            }
        });
    }

    /// Feeds new session-log lines through the card's tracker.
    fn read_log(self: &Arc<Self>, st: &Arc<State>, card_id: &str) {
        let Ok(card) = st.store.lock().unwrap().get_card(card_id) else { return };
        let Some(adapter) = self.adapter(&card.agent_id).cloned() else { return };
        let (path, offset) = {
            let cards = self.cards.lock().unwrap();
            let Some(ca) = cards.get(card_id) else { return };
            let Some(p) = ca.tracker.transcript_path.clone() else { return };
            (p, ca.log_offset)
        };
        // Validated when it was found, and again now (it may have been replaced since).
        let Some(path) = adapter.validate_transcript_path(Path::new(&path)) else { return };
        let Ok((events, next)) = blocking(|| adapter.log_events(card_id, &path, offset)) else { return };
        if let Some(ca) = self.cards.lock().unwrap().get_mut(card_id) {
            ca.log_offset = next;
        }
        for ev in events {
            self.handle(st, ev);
        }
    }

    async fn opencode_session(&self, card: &Card) -> Option<muster_agents::opencode::OpencodeSession> {
        let sid = card.agent_session_id.as_deref()?;
        let oc = muster_agents::opencode::Opencode::default();
        let out = run_query(&oc.session_query(sid)).await?;
        muster_agents::opencode::parse_session(&out)
    }

    async fn opencode_messages(&self, card: &Card) -> Option<Vec<muster_agents::ChatMessage>> {
        let sid = card.agent_session_id.as_deref()?;
        let oc = muster_agents::opencode::Opencode::default();
        let out = run_query(&oc.messages_query(sid)).await?;
        Some(muster_agents::opencode::parse_messages(&out))
    }

    /// opencode chat: sends messages not yet sent to subscribers (by id).
    async fn push_chat_opencode(&self, st: &State, card: &Card) {
        let conns = match self.chat.lock().unwrap().get(&card.id) {
            Some(sub) if !sub.conns.is_empty() => sub.conns.clone(),
            _ => return,
        };
        let Some(msgs) = self.opencode_messages(card).await else { return };
        let new: Vec<_> = {
            let mut chat = self.chat.lock().unwrap();
            let Some(sub) = chat.get_mut(&card.id) else { return };
            let fresh: Vec<_> = msgs.into_iter().filter(|m| sub.seen.insert(m.id.clone())).collect();
            fresh
        };
        if !new.is_empty() {
            let ev = Event::ChatAppended(ChatAppended { card_id: card.id.clone(), messages: convert_messages(new) });
            st.clients.send_to(&conns, &ev);
        }
    }

    fn status_of(&self, card_id: &str) -> Option<AgentStatus> {
        self.cards.lock().unwrap().get(card_id).map(|c| c.tracker.status)
    }

    /// Terminal output was seen for `card_id` (throttled upstream to about once a
    /// second per card). Updates `lastActivityAt` for cards without hooks and arms the
    /// quiet check for running hooked cards.
    pub fn on_output(self: &Arc<Self>, st: &Arc<State>, card_id: &str) {
        let now = now_ms();
        let Ok(card) = st.store.lock().unwrap().get_card(card_id) else { return };
        let source = self.source(&card);
        if source != Source::Hooks {
            let write = {
                let mut act = self.activity.lock().unwrap();
                let a = act.entry(card_id.to_string()).or_default();
                if now - a.last_written >= ACTIVITY_WRITE_EVERY_MS {
                    a.last_written = now;
                    true
                } else {
                    false
                }
            };
            if write && st.store.lock().unwrap().touch_activity(card_id, now).is_ok() {
                let card = st.store.lock().unwrap().get_card(card_id);
                if let Ok(c) = card {
                    st.emit_card(c);
                }
            }
        } else {
            let running = self
                .cards
                .lock()
                .unwrap()
                .get(card_id)
                .is_some_and(|c| matches!(c.tracker.status, AgentStatus::Running | AgentStatus::NeedsYou));
            if !running {
                return;
            }
        }
        // tmux raises no activity or silence alerts for a window someone is looking
        // at, so while a client is attached the terminal output seen here drives the
        // same transitions.
        let (start_timer, was_quiet) = {
            let mut act = self.activity.lock().unwrap();
            let a = act.entry(card_id.to_string()).or_default();
            a.last_output = now;
            let was_quiet = !std::mem::replace(&mut a.active, true);
            (!std::mem::replace(&mut a.quiet_timer, true), was_quiet)
        };
        if was_quiet && source != Source::Hooks {
            let (agents, st, id) = (self.clone(), st.clone(), card_id.to_string());
            tokio::spawn(async move { agents.on_activity(&st, &id).await });
        }
        if start_timer {
            let (agents, st, id) = (self.clone(), st.clone(), card_id.to_string());
            tokio::spawn(async move {
                // Notices arrive at most once a second, so allow for that.
                let quiet = quiet_after(&st) + Duration::from_secs(1);
                loop {
                    let last = agents.activity.lock().unwrap().get(&id).map(|a| a.last_output);
                    let Some(last) = last else { return };
                    let due = last + quiet.as_millis() as i64;
                    let now = now_ms();
                    if now >= due {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis((due - now) as u64)).await;
                }
                if let Some(a) = agents.activity.lock().unwrap().get_mut(&id) {
                    a.quiet_timer = false;
                    a.active = false;
                }
                agents.on_silence(&st, &id).await;
            });
        }
    }
}

/// A card title from a prompt: the first non-empty line, whitespace collapsed, cut to
/// 60 characters. Slash commands give no title.
pub fn title_from_prompt(prompt: &str) -> Option<String> {
    let line = prompt.lines().map(str::trim).find(|l| !l.is_empty())?;
    if line.starts_with('/') {
        return None;
    }
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX: usize = 60;
    Some(if line.chars().count() > MAX {
        format!("{}…", line.chars().take(MAX).collect::<String>().trim_end())
    } else {
        line
    })
}

fn convert_messages(m: Vec<muster_agents::ChatMessage>) -> Vec<ChatMessage> {
    // The shapes match; go through JSON so the wire types stay independent.
    m.into_iter().filter_map(|m| serde_json::to_value(m).ok().and_then(|v| serde_json::from_value(v).ok())).collect()
}

/// Consumes hook events for the life of the daemon.
pub async fn run(st: Arc<State>, mut rx: mpsc::UnboundedReceiver<NormalizedEvent>) {
    while let Some(ev) = rx.recv().await {
        st.agents.handle(&st, ev);
    }
}

#[cfg(test)]
mod tests {
    use super::title_from_prompt;

    #[test]
    fn prompt_titles() {
        assert_eq!(title_from_prompt("\n  fix   the\tbuild \nmore"), Some("fix the build".into()));
        assert_eq!(title_from_prompt("/clear"), None);
        assert_eq!(title_from_prompt("   "), None);
        let long = "word ".repeat(30);
        let t = title_from_prompt(&long).unwrap();
        assert!(t.ends_with('…') && t.chars().count() <= 61);
    }
}

#[derive(Clone, Copy)]
enum Watch {
    Silence,
}

/// Arms tmux's silence alert for a session (in the background).
fn arm(st: &Arc<State>, card_id: &str, w: Watch) {
    let (st, id) = (st.clone(), card_id.to_string());
    tokio::spawn(async move {
        match w {
            Watch::Silence => st.backend.watch_silence(&id, quiet_secs(&st)).await,
        }
    });
}

/// Runs file-reading work (transcript scans can read tens of MB) without holding up
/// the other tasks: the runtime moves them to another worker while this one blocks.
pub fn blocking<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()) {
        Ok(tokio::runtime::RuntimeFlavor::MultiThread) => tokio::task::block_in_place(f),
        _ => f(),
    }
}

/// Runs an adapter's query command and returns its stdout (bounded wait).
async fn run_query(cmd: &muster_agents::CommandSpec) -> Option<String> {
    let run = tokio::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .envs(cmd.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .current_dir(&cmd.cwd)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(10), run).await.ok()?.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}
