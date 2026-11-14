//! Reference hook receiver: listens on the Unix socket `muster-hook` connects to, yields
//! normalised events on a channel, and answers pending permission requests.
//!
//! Threads, not async: one accept thread plus one short-lived thread per connection
//! (a connection lives for the duration of one hook call; only permission requests stay
//! open, until answered or until the hook goes away).

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::adapter::{now_ms, secret_eq, AdapterRegistry, AgentAdapter};
use crate::types::*;

/// Largest envelope line accepted (the hook caps payloads at 4 MiB).
const MAX_LINE: u64 = 6 * 1024 * 1024;
/// A hook that connects but sends nothing within this time is dropped.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Summary of a permission request that can still be answered.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingPermission {
    pub card_id: String,
    pub agent_id: String,
    pub session_id: Option<String>,
    pub request: PermissionRequest,
    pub since_ms: u64,
}

struct Pending {
    info: PendingPermission,
    stream: UnixStream,
    adapter: Arc<dyn AgentAdapter>,
}

struct Inner {
    path: PathBuf,
    registry: AdapterRegistry,
    pending: Mutex<HashMap<String, Pending>>,
    next_id: AtomicU64,
    closed: AtomicBool,
    tx: Mutex<Sender<NormalizedEvent>>,
    tap: Mutex<Option<Sender<HookEnvelope>>>,
    id_prefix: String,
    /// card id -> per-launch hook secret.
    secrets: Mutex<HashMap<String, String>>,
    require_secret: AtomicBool,
    dropped: AtomicU64,
}

/// Handle to a running receiver. Cheap to clone; the listener stops on [`close`]
/// (or when the process exits).
///
/// [`close`]: HookReceiver::close
#[derive(Clone)]
pub struct HookReceiver {
    inner: Arc<Inner>,
}

#[derive(Debug)]
pub enum AnswerError {
    /// No such pending request (never existed, already answered, or the hook went away).
    NotFound,
    /// The hook went away while answering.
    Io(io::Error),
}

impl std::fmt::Display for AnswerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnswerError::NotFound => write!(f, "no such pending permission request"),
            AnswerError::Io(e) => write!(f, "hook connection failed: {e}"),
        }
    }
}

impl std::error::Error for AnswerError {}

impl HookReceiver {
    /// Bind `path` (removing a stale socket file left by a previous run, refusing to
    /// replace anything that is not a socket or that still accepts connections), make it
    /// owner-only, and start accepting. Events arrive on the returned channel.
    pub fn bind(path: &Path, registry: AdapterRegistry) -> io::Result<(HookReceiver, Receiver<NormalizedEvent>)> {
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            if !meta.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a socket", path.display()),
                ));
            }
            if UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("{} is in use by another receiver", path.display()),
                ));
            }
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let (tx, rx) = mpsc::channel();
        let inner = Arc::new(Inner {
            path: path.to_path_buf(),
            registry,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
            tx: Mutex::new(tx),
            tap: Mutex::new(None),
            secrets: Mutex::new(HashMap::new()),
            require_secret: AtomicBool::new(true),
            dropped: AtomicU64::new(0),
            id_prefix: format!("p{:x}-", now_ms()),
        });
        let accept_inner = inner.clone();
        thread::Builder::new().name("muster-hook-accept".into()).spawn(move || {
            for conn in listener.incoming() {
                if accept_inner.closed.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = conn else { continue };
                let inner = accept_inner.clone();
                let _ = thread::Builder::new().name("muster-hook-conn".into()).spawn(move || handle(inner, stream));
            }
        })?;
        Ok((HookReceiver { inner }, rx))
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Answer a pending permission request. The decision is written to the waiting hook,
    /// which prints it for the agent, and a [`AgentEvent::PermissionResolved`] event
    /// follows on the channel.
    pub fn answer(&self, request_id: &str, decision: &PermissionDecision) -> Result<(), AnswerError> {
        let p = self.inner.pending.lock().unwrap().remove(request_id).ok_or(AnswerError::NotFound)?;
        let mut line = p.adapter.permission_reply(&p.info.request, decision);
        line.push('\n');
        let mut s = &p.stream;
        s.set_write_timeout(Some(Duration::from_secs(2))).ok();
        let res = s.write_all(line.as_bytes()).and_then(|_| s.flush());
        let _ = p.stream.shutdown(std::net::Shutdown::Write);
        match res {
            Ok(()) => {
                emit_resolved(
                    &self.inner,
                    &p.info,
                    request_id,
                    PermissionOutcome::Answered { decision: decision.clone() },
                );
                Ok(())
            }
            Err(e) => {
                emit_resolved(&self.inner, &p.info, request_id, PermissionOutcome::Elsewhere);
                Err(AnswerError::Io(e))
            }
        }
    }

    /// Register the hook secret of a card's current launch (the value passed as
    /// `LaunchSpec::hook_secret`). Replaces any earlier secret for the card. Call it
    /// before starting the session, and again for every live card after a daemon
    /// restart (persist the secret with the card: the running agent keeps its value).
    pub fn set_card_secret(&self, card_id: &str, secret: &str) {
        self.inner.secrets.lock().unwrap().insert(card_id.to_string(), secret.to_string());
    }

    /// Forget a card's secret (session ended or card deleted).
    pub fn remove_card_secret(&self, card_id: &str) {
        self.inner.secrets.lock().unwrap().remove(card_id);
    }

    /// With `true` (the default) an envelope is accepted only when its card has a
    /// registered secret and the envelope carries the same value. With `false`, cards
    /// without a registered secret are accepted as before (migration only).
    pub fn set_require_secret(&self, require: bool) {
        self.inner.require_secret.store(require, Ordering::SeqCst);
    }

    /// Envelopes dropped so far for a missing or wrong secret.
    pub fn dropped_events(&self) -> u64 {
        self.inner.dropped.load(Ordering::SeqCst)
    }

    /// Also send every raw envelope to `tap` (for debugging and fixture capture).
    pub fn set_raw_tap(&self, tap: Option<Sender<HookEnvelope>>) {
        *self.inner.tap.lock().unwrap() = tap;
    }

    /// Requests that can still be answered.
    pub fn pending(&self) -> Vec<PendingPermission> {
        self.inner.pending.lock().unwrap().values().map(|p| p.info.clone()).collect()
    }

    /// Stop accepting and release all waiting hooks (they print nothing, so the agents
    /// fall back to their own prompts). Removes the socket file.
    pub fn close(&self) {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = UnixStream::connect(&self.inner.path); // wake the accept loop
        let _ = std::fs::remove_file(&self.inner.path);
        for (_, p) in self.inner.pending.lock().unwrap().drain() {
            let _ = p.stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

fn send(inner: &Inner, ev: NormalizedEvent) {
    let _ = inner.tx.lock().unwrap().send(ev);
}

fn emit_resolved(inner: &Inner, info: &PendingPermission, request_id: &str, outcome: PermissionOutcome) {
    send(
        inner,
        NormalizedEvent {
            card_id: info.card_id.clone(),
            agent_id: info.agent_id.clone(),
            session_id: info.session_id.clone(),
            transcript_path: None,
            cwd: None,
            subagent_id: None,
            at_ms: now_ms(),
            kind: AgentEvent::PermissionResolved { request_id: request_id.to_string(), outcome },
        },
    );
}

fn handle(inner: Arc<Inner>, stream: UnixStream) {
    if inner.closed.load(Ordering::SeqCst) {
        return;
    }
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok();
    let mut line = Vec::new();
    {
        let Ok(clone) = stream.try_clone() else {
            return;
        };
        let mut reader = BufReader::new(clone.take(MAX_LINE));
        if reader.read_until(b'\n', &mut line).is_err() || line.is_empty() {
            return;
        }
    }
    let Ok(mut env) = serde_json::from_slice::<HookEnvelope>(&line) else {
        return;
    };
    let given = env.secret.take();
    let accepted = {
        let secrets = inner.secrets.lock().unwrap();
        match secrets.get(&env.card_id) {
            Some(expected) => given.as_deref().is_some_and(|g| secret_eq(g.as_bytes(), expected.as_bytes())),
            None => !inner.require_secret.load(Ordering::SeqCst),
        }
    };
    if !accepted {
        inner.dropped.fetch_add(1, Ordering::SeqCst);
        reply_nothing(&stream, env.wants_reply);
        return;
    }
    if let Some(tap) = inner.tap.lock().unwrap().as_ref() {
        let _ = tap.send(env.clone());
    }
    let agent_id = if env.agent_id.is_empty() { "claude" } else { env.agent_id.as_str() };
    let Some(adapter) = inner.registry.get(agent_id).cloned() else {
        reply_nothing(&stream, env.wants_reply);
        return;
    };
    let at = now_ms();
    // Release hooks still waiting on prompts this event shows were settled elsewhere,
    // and report that before the event itself.
    let settled: Vec<(String, Pending)> = {
        let mut pending = inner.pending.lock().unwrap();
        let ids: Vec<String> = pending
            .iter()
            .filter(|(_, p)| p.info.card_id == env.card_id && p.adapter.settles_permission(&env, &p.info.request))
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter().filter_map(|id| pending.remove(&id).map(|p| (id, p))).collect()
    };
    for (id, p) in settled {
        reply_nothing(&p.stream, true);
        let _ = p.stream.shutdown(std::net::Shutdown::Both);
        emit_resolved(&inner, &p.info, &id, PermissionOutcome::Elsewhere);
    }
    let mut events = adapter.normalize(&env, at);
    // Paths in payloads are only trusted under the agent's own transcript directory.
    for ev in &mut events {
        if let Some(p) = ev.transcript_path.take() {
            ev.transcript_path =
                adapter.validate_transcript_path(Path::new(&p)).map(|p| p.to_string_lossy().into_owned());
        }
        if let AgentEvent::SubagentStopped { transcript_path, .. } = &mut ev.kind {
            if let Some(p) = transcript_path.take() {
                *transcript_path =
                    adapter.validate_transcript_path(Path::new(&p)).map(|p| p.to_string_lossy().into_owned());
            }
        }
    }

    let mut waiting: Option<String> = None;
    if env.wants_reply {
        if let Some(ev) = events.iter_mut().find(|e| matches!(e.kind, AgentEvent::PermissionRequested { .. })) {
            if let AgentEvent::PermissionRequested { request } = &mut ev.kind {
                let id = format!("{}{}", inner.id_prefix, inner.next_id.fetch_add(1, Ordering::SeqCst));
                request.id = Some(id.clone());
                let Ok(write_half) = stream.try_clone() else {
                    return;
                };
                inner.pending.lock().unwrap().insert(
                    id.clone(),
                    Pending {
                        info: PendingPermission {
                            card_id: ev.card_id.clone(),
                            agent_id: ev.agent_id.clone(),
                            session_id: ev.session_id.clone(),
                            request: request.clone(),
                            since_ms: at,
                        },
                        stream: write_half,
                        adapter: adapter.clone(),
                    },
                );
                waiting = Some(id);
            }
        }
        if waiting.is_none() {
            reply_nothing(&stream, true);
        }
    }
    for ev in events {
        send(&inner, ev);
    }

    // Hold the connection until the hook goes away (answered, killed by the agent because
    // the terminal answered, or its wait ran out).
    if let Some(id) = waiting {
        stream.set_read_timeout(None).ok();
        let mut buf = [0u8; 256];
        let mut s = &stream;
        loop {
            match s.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let gone = inner.pending.lock().unwrap().remove(&id);
        if let Some(p) = gone {
            emit_resolved(&inner, &p.info, &id, PermissionOutcome::Elsewhere);
        }
    }
}

fn reply_nothing(stream: &UnixStream, wants_reply: bool) {
    if wants_reply {
        let mut s = stream;
        let _ = s.write_all(b"\n");
    }
}
