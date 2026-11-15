//! Shared daemon state: store, sessions, connected clients, reconciliation.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use muster_core::attach::Hub;
use muster_core::store::Store;
use muster_core::tmux::{SessionBackend, SessionWatch, TmuxBackend, WatchEvent};
use muster_proto::{
    Card, CardChanged, CardStatus, Event, SessionExit, SessionSize, Settings, SettingsResult, SetupInfo, SetupState,
};
use tokio::sync::{mpsc, Notify};
use tokio_tungstenite::tungstenite::Message;

use crate::agents::Agents;
use crate::log;

/// Outbound side of one client connection.
#[derive(Clone)]
pub struct Conn {
    pub tx: mpsc::Sender<Message>,
    /// Signalled when the connection must be closed (its queue overflowed).
    pub kill: Arc<Notify>,
}

impl Conn {
    /// Queues a message without waiting. A full queue means the client is
    /// not keeping up; the connection is then closed rather than buffering
    /// without bound or dropping terminal bytes silently.
    pub fn send(&self, m: Message) -> bool {
        match self.tx.try_send(m) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.kill.notify_one();
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }
}

/// Clients that have completed `hello` and receive events.
#[derive(Default)]
pub struct Clients {
    conns: Mutex<HashMap<u64, Conn>>,
}

impl Clients {
    pub fn add(&self, id: u64, conn: Conn) {
        self.conns.lock().unwrap().insert(id, conn);
    }

    pub fn remove(&self, id: u64) {
        self.conns.lock().unwrap().remove(&id);
    }

    /// Sends an event to the given connections only.
    pub fn send_to(&self, ids: &HashSet<u64>, ev: &Event) {
        let text = serde_json::to_string(&ev.to_frame()).expect("event serialises");
        let conns = self.conns.lock().unwrap();
        for id in ids {
            if let Some(conn) = conns.get(id) {
                conn.send(Message::text(text.clone()));
            }
        }
    }

    pub fn broadcast(&self, ev: &Event) {
        let text = serde_json::to_string(&ev.to_frame()).expect("event serialises");
        for conn in self.conns.lock().unwrap().values() {
            conn.send(Message::text(text.clone()));
        }
    }
}

pub struct State {
    pub home: PathBuf,
    pub token: String,
    pub store: Mutex<Store>,
    pub backend: Arc<TmuxBackend>,
    pub hub: Arc<Hub<TmuxBackend>>,
    pub clients: Arc<Clients>,
    /// Card ids with a live session, kept in step with tmux by `reconcile`.
    live: Mutex<HashSet<String>>,
    reconcile_lock: tokio::sync::Mutex<()>,
    watch_kick: Notify,
    next_conn: AtomicU64,
    pub installed_agents: tokio::sync::OnceCell<HashMap<String, std::path::PathBuf>>,
    pub accounts: crate::accounts::Cache,
    pub agents: Arc<Agents>,
    me: std::sync::OnceLock<std::sync::Weak<State>>,
    settings: Mutex<Settings>,
    killed: Mutex<HashSet<String>>,
    awake: Mutex<Awake>,
    pub stats: crate::stats::Stats,
    notices: Mutex<Vec<muster_proto::Notice>>,
    hello_failures: Mutex<std::collections::VecDeque<std::time::Instant>>,
    pub scrollback: crate::scrollback::Scrollback,
    pub touched: crate::idle::Touched,
    pub remote: crate::remote::Remote,
}

#[derive(Default)]
struct Awake {
    held: Option<muster_core::power::Assertion>,
    checking: bool,
    gen: u64,
}

impl State {
    /// Returns the state and the receiver of throttled terminal-output notices, to be
    /// passed to [`State::output_activity`].
    pub fn new(
        home: PathBuf,
        token: String,
        store: Store,
        backend: TmuxBackend,
        agents: Agents,
    ) -> (Arc<State>, mpsc::UnboundedReceiver<String>) {
        let backend = Arc::new(backend);
        let clients = Arc::new(Clients::default());
        let c2 = clients.clone();
        let (out_tx, out_rx) = mpsc::unbounded_channel::<String>();
        // At most one notice per card per second, whatever the output rate.
        let last_notice: Mutex<HashMap<String, i64>> = Mutex::new(HashMap::new());
        let hub = Hub::new(
            backend.clone(),
            Box::new(move |card_id, cols, rows| {
                c2.broadcast(&Event::SessionSize(SessionSize { card_id: card_id.to_string(), cols, rows }));
            }),
            Box::new(move |card_id| {
                let now = muster_core::util::now_ms();
                let mut last = last_notice.lock().unwrap();
                match last.get_mut(card_id) {
                    Some(t) if now - *t < 1000 => {}
                    Some(t) => {
                        *t = now;
                        let _ = out_tx.send(card_id.to_string());
                    }
                    None => {
                        last.insert(card_id.to_string(), now);
                        let _ = out_tx.send(card_id.to_string());
                    }
                }
            }),
        );
        let st = Arc::new(State {
            home,
            token,
            store: Mutex::new(store),
            backend,
            hub,
            clients,
            live: Mutex::new(HashSet::new()),
            reconcile_lock: tokio::sync::Mutex::new(()),
            watch_kick: Notify::new(),
            next_conn: AtomicU64::new(1),
            installed_agents: tokio::sync::OnceCell::new(),
            accounts: Default::default(),
            agents: Arc::new(agents),
            me: std::sync::OnceLock::new(),
            settings: Mutex::new(Settings::default()),
            killed: Mutex::new(HashSet::new()),
            awake: Mutex::new(Awake::default()),
            stats: crate::stats::Stats::default(),
            notices: Mutex::new(Vec::new()),
            hello_failures: Mutex::new(std::collections::VecDeque::new()),
            scrollback: crate::scrollback::Scrollback::default(),
            touched: crate::idle::Touched::default(),
            remote: crate::remote::Remote::default(),
        });
        let _ = st.me.set(Arc::downgrade(&st));
        let saved = st.store.lock().unwrap().meta_get("settings").ok().flatten();
        if let Some(s) = saved.and_then(|j| serde_json::from_str::<Settings>(&j).ok()) {
            *st.settings.lock().unwrap() = s;
        }
        (st, out_rx)
    }

    /// Handles terminal-output notices from the hub for the life of the daemon.
    pub async fn output_activity(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<String>) {
        while let Some(card_id) = rx.recv().await {
            self.agents.on_output(&self, &card_id);
        }
    }

    pub fn next_conn_id(&self) -> u64 {
        self.next_conn.fetch_add(1, Ordering::Relaxed)
    }

    pub fn has_session(&self, card_id: &str) -> bool {
        self.live.lock().unwrap().contains(card_id)
    }

    pub fn live_count(&self) -> usize {
        self.live.lock().unwrap().len()
    }

    /// Fills in the derived `hasSession` field.
    pub fn decorate(&self, mut card: Card) -> Card {
        card.has_session = self.has_session(&card.id);
        card
    }

    pub fn emit_card(&self, card: Card) {
        let card = self.decorate(card);
        let (id, status, queued) = (card.id.clone(), card.status, !card.queued_messages.is_empty());
        self.clients.broadcast(&Event::CardChanged(CardChanged::Updated { card }));
        if let Some(me) = self.me.get().and_then(|w| w.upgrade()) {
            crate::context::on_card_status(&me, &id, status, queued);
            me.update_keep_awake();
        }
    }

    /// Failed `hello` attempts allowed per window before all new hellos are refused
    /// until the window has passed.
    const HELLO_MAX_FAILURES: usize = 10;

    fn hello_window() -> Duration {
        Duration::from_millis(
            std::env::var("MUSTER_HELLO_WINDOW_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(60_000),
        )
    }

    pub fn hello_locked(&self) -> bool {
        let mut f = self.hello_failures.lock().unwrap();
        let window = Self::hello_window();
        while f.front().is_some_and(|t| t.elapsed() > window) {
            f.pop_front();
        }
        f.len() >= Self::HELLO_MAX_FAILURES
    }

    pub fn hello_failed(&self) {
        self.hello_failures.lock().unwrap().push_back(std::time::Instant::now());
        log!("rejected a hello with a bad token");
    }

    pub fn status(&self) -> muster_proto::DaemonStatus {
        let mut notices = Vec::new();
        if let Some(p) = &self.agents.hook_problem {
            notices.push(muster_proto::Notice {
                id: "hook_socket".into(),
                level: "warning".into(),
                message: p.clone(),
            });
        }
        if !self.backend.available() {
            notices.push(muster_proto::Notice {
                id: "tmux_missing".into(),
                level: "warning".into(),
                message: "tmux was not found, so no card can run a session. Install tmux (for example with Homebrew) and restart Muster.".into(),
            });
        }
        let dropped = self.agents.dropped_events();
        if dropped > 0 {
            notices.push(muster_proto::Notice {
                id: "hook_events_dropped".into(),
                level: "warning".into(),
                message: format!(
                    "{dropped} hook event(s) were rejected because they did not carry their card's secret."
                ),
            });
        }
        notices.extend(self.notices.lock().unwrap().iter().cloned());
        muster_proto::DaemonStatus {
            version: env!("CARGO_PKG_VERSION").into(),
            commit: Some(env!("MUSTER_BUILD_COMMIT").into()),
            commit_date: Some(env!("MUSTER_BUILD_COMMIT_DATE").to_string()).filter(|d| !d.is_empty()),
            pid: std::process::id(),
            home: self.home.to_string_lossy().into_owned(),
            tmux_socket: self.backend.socket().to_string(),
            hooks_available: self.agents.hooks_available(),
            notices,
        }
    }

    /// Adds or removes a notice; broadcasts `daemon.status` when that changes anything.
    pub fn set_notice(&self, id: &str, notice: Option<(&str, String)>) {
        let changed = {
            let mut n = self.notices.lock().unwrap();
            let before = n.clone();
            n.retain(|x| x.id != id);
            if let Some((level, message)) = notice {
                n.push(muster_proto::Notice { id: id.into(), level: level.into(), message });
            }
            *n != before
        };
        if changed {
            self.clients.broadcast(&Event::StatusChanged(self.status()));
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    pub fn set_settings(self: &Arc<Self>, s: Settings) {
        *self.settings.lock().unwrap() = s.clone();
        if let Ok(j) = serde_json::to_string(&s) {
            let _ = self.store.lock().unwrap().meta_set("settings", &j);
        }
        self.clients.broadcast(&Event::SettingsChanged(SettingsResult { settings: s }));
        self.update_keep_awake();
    }

    /// Records that the daemon itself is about to end a session, so it is not
    /// offered for restore.
    pub fn mark_killed(&self, card_id: &str) {
        self.killed.lock().unwrap().insert(card_id.to_string());
    }

    /// Holds an idle-sleep assertion exactly while some card is `running` (and the
    /// setting allows it, and the battery is not below 10%).
    pub fn update_keep_awake(self: &Arc<Self>) {
        let running = self
            .store
            .lock()
            .unwrap()
            .all_cards()
            .map(|cs| cs.iter().any(|c| c.status == CardStatus::Running && self.has_session(&c.id)))
            .unwrap_or(false);
        let want = running && self.settings().keep_awake;
        let mut awake = self.awake.lock().unwrap();
        if !want {
            if awake.held.take().is_some() {
                log!("released the keep-awake assertion");
            }
            awake.gen += 1;
            return;
        }
        if awake.held.is_some() || awake.checking {
            return;
        }
        awake.checking = true;
        awake.gen += 1;
        let gen = awake.gen;
        drop(awake);
        let st = self.clone();
        tokio::spawn(async move { st.keep_awake_loop(gen).await });
    }

    /// Acquires the assertion if the battery allows, then re-checks the battery every
    /// few minutes while it is held.
    async fn keep_awake_loop(self: Arc<Self>, gen: u64) {
        loop {
            let low = match muster_core::power::battery().await {
                Some((true, Some(p))) => p < 10,
                _ => false,
            };
            {
                let mut awake = self.awake.lock().unwrap();
                awake.checking = false;
                if awake.gen != gen {
                    return;
                }
                if low {
                    if awake.held.take().is_some() {
                        log!("battery below 10%: released the keep-awake assertion");
                    }
                } else if awake.held.is_none() {
                    awake.held = muster_core::power::Assertion::prevent_idle_sleep();
                    if awake.held.is_some() {
                        log!("holding a keep-awake assertion while cards run");
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(300)).await;
            if self.awake.lock().unwrap().gen != gen {
                return;
            }
        }
    }

    /// Held while a session is created and recorded. A reconcile that listed the
    /// sessions just before would otherwise find the new card missing from its list
    /// and mark it ended.
    pub async fn hold_reconcile(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.reconcile_lock.lock().await
    }

    /// Records a session that was just created, before tmux reports it.
    pub fn mark_live(&self, card_id: &str) {
        self.live.lock().unwrap().insert(card_id.to_string());
        self.watch_kick.notify_one();
        self.scrollback.kick();
    }

    /// Brings card state in line with the sessions tmux actually has: emits
    /// `session.exit` for sessions that ended, marks their cards `exited`,
    /// and revives cards whose session is alive. Safe to call at any time.
    pub async fn reconcile(&self) {
        let _guard = self.reconcile_lock.lock().await;
        let alive = match self.backend.list().await {
            Ok(a) => a,
            Err(e) => {
                log!("reconcile: {e}");
                return;
            }
        };
        let (gone, new): (Vec<String>, Vec<String>) = {
            let mut live = self.live.lock().unwrap();
            let gone = live.difference(&alive).cloned().collect();
            let new = alive.difference(&live).cloned().collect();
            *live = alive.clone();
            (gone, new)
        };
        let mut changed: HashMap<String, Card> = HashMap::new();
        let mut exits = Vec::new();
        let mut codes: HashMap<String, Option<i32>> = HashMap::new();
        {
            let mut store = self.store.lock().unwrap();
            let cards = match store.all_cards() {
                Ok(c) => c,
                Err(e) => {
                    log!("reconcile: {e}");
                    return;
                }
            };
            for card in cards {
                let has = alive.contains(&card.id);
                let next = match (has, card.status) {
                    (true, CardStatus::Draft | CardStatus::Exited) => Some(CardStatus::Running),
                    (false, CardStatus::Draft | CardStatus::Exited) => None,
                    (false, _) => Some(CardStatus::Exited),
                    (true, _) => None,
                };
                let flipped = gone.contains(&card.id) || new.contains(&card.id);
                if next == Some(CardStatus::Exited) {
                    // Gone without an exit status and not killed on purpose: the tmux
                    // server died or the machine rebooted. The card can be restored.
                    let _ = store.set_card_token_hash(&card.id, None);
                    let code = self.backend.take_exit_code(&card.id);
                    let killed = self.killed.lock().unwrap().remove(&card.id);
                    if code.is_none() && !killed {
                        let _ = store.set_restorable(&card.id, true);
                    }
                    codes.insert(card.id.clone(), code);
                }
                let card = match next {
                    Some(s) => match store.set_status(&card.id, s) {
                        Ok(c) => c,
                        Err(e) => {
                            log!("reconcile: {e}");
                            continue;
                        }
                    },
                    None if flipped => card,
                    None => continue,
                };
                changed.insert(card.id.clone(), card);
            }
        }
        for id in &gone {
            self.hub.drop_session(id);
            // The session's card token dies with it.
            let _ = self.store.lock().unwrap().set_card_token_hash(id, None);
            // A session that ended mid-setup counts as a failed setup.
            let setup_running = self
                .store
                .lock()
                .unwrap()
                .get_card(id)
                .is_ok_and(|c| c.setup.as_ref().is_some_and(|s| s.state == SetupState::Running));
            if setup_running {
                let info = SetupInfo { state: SetupState::Failed, exit_code: None };
                let _ = self.store.lock().unwrap().set_setup(id, Some(&info));
            }
            self.agents.on_exit(self, id);
            let exit_code = codes.remove(id).unwrap_or_else(|| self.backend.take_exit_code(id));
            exits.push(SessionExit { card_id: id.clone(), exit_code });
        }
        for card in changed.into_values() {
            let fresh = self.store.lock().unwrap().get_card(&card.id);
            self.emit_card(fresh.unwrap_or(card));
        }
        for e in exits {
            log!("session for card {} ended (exit code {:?})", e.card_id, e.exit_code);
            self.clients.broadcast(&Event::SessionExit(e));
        }
    }

    /// Setups that finished before the watch was attached left their mark in the
    /// window name.
    async fn catch_up_setups(self: &Arc<Self>) {
        let running: Vec<String> = self
            .store
            .lock()
            .unwrap()
            .all_cards()
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.setup.as_ref().is_some_and(|s| s.state == SetupState::Running))
            .map(|c| c.id)
            .collect();
        if running.is_empty() {
            return;
        }
        for (id, code) in self.backend.setup_marks().await {
            if running.contains(&id) {
                self.setup_done(&id, code);
            }
        }
    }

    /// The worktree setup of a card finished with `code`.
    pub fn setup_done(self: &Arc<Self>, card_id: &str, code: i32) {
        let card = self.store.lock().unwrap().get_card(card_id);
        let Ok(card) = card else { return };
        if !card.setup.as_ref().is_some_and(|s| s.state == SetupState::Running) {
            return;
        }
        log!("card {card_id}: worktree setup finished (exit {code})");
        let updated = if code == 0 {
            let r = self.store.lock().unwrap().set_setup(card_id, None);
            if let Ok(c) = &r {
                self.agents.on_started(self, c);
            }
            self.store.lock().unwrap().get_card(card_id)
        } else {
            let info = SetupInfo { state: SetupState::Failed, exit_code: Some(code) };
            let r = self.store.lock().unwrap().set_setup(card_id, Some(&info));
            r.and_then(|_| self.store.lock().unwrap().set_needs_you(card_id, "setup_failed"))
        };
        if let Ok(c) = updated {
            self.emit_card(c);
        }
    }

    /// Keeps a tmux control client running while any session exists and
    /// reconciles whenever tmux reports a change. Idle (no process, no
    /// timers) when there are no sessions.
    pub async fn watch_sessions(self: Arc<Self>) {
        loop {
            if self.live_count() == 0 {
                self.watch_kick.notified().await;
                continue;
            }
            let mut watch = match self.backend.watch().await {
                Ok(w) => w,
                Err(e) => {
                    log!("session watch: {e}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            // Catch anything that happened before the watch was in place.
            self.reconcile().await;
            self.catch_up_setups().await;
            while self.live_count() > 0 {
                match watch.next().await {
                    None => break,
                    Some(WatchEvent::Sessions) => self.reconcile().await,
                    Some(WatchEvent::SetupDone { card_id, code }) => self.setup_done(&card_id, code),
                    Some(WatchEvent::Silence { card_id }) => self.agents.on_silence(&self, &card_id).await,
                    Some(WatchEvent::Activity { card_id }) => self.agents.on_activity(&self, &card_id).await,
                }
            }
            drop(watch);
            self.reconcile().await;
            if self.live_count() == 0 {
                self.backend.unwatch().await;
            } else {
                // The control client ended unexpectedly; avoid a hot loop.
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}
