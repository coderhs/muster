//! Terminal attachment hub.
//!
//! For each session with at least one subscriber, the hub runs exactly one
//! terminal client (a tmux client in a PTY) and fans its output out to the
//! subscribers. When a subscriber joins, the client is replaced by a fresh
//! one: a new tmux client always starts by initialising the terminal and
//! drawing the whole screen, so the joiner's first bytes reproduce the screen
//! exactly (existing subscribers simply see a repaint). The client is dropped
//! when the last subscriber leaves.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use muster_proto::ErrorCode;

use crate::pty::Pty;
use crate::tmux::SessionBackend;
use crate::util::{err, internal, Result};
use tokio::process::Child;

/// Receives output bytes; returns `false` if the subscriber is gone.
pub type Sink = Arc<dyn Fn(&[u8]) -> bool + Send + Sync>;

/// Called with `(card_id, cols, rows)` whenever a session's size changes.
pub type SizeListener = Box<dyn Fn(&str, u16, u16) + Send + Sync>;

/// Called with the card id after each chunk of output is delivered. Runs with the
/// hub's lock held: it must be cheap and must not call back into the hub.
pub type OutputListener = Box<dyn Fn(&str) + Send + Sync>;

pub const DEFAULT_SIZE: (u16, u16) = (120, 36);

struct Sub {
    sink: Sink,
    cols: u16,
    rows: u16,
    watch: bool,
}

struct Attachment {
    gen: u64,
    pty: Arc<Pty>,
    /// The terminal client. It is killed through this handle (never by a stored pid,
    /// which could belong to another process once the client has been reaped).
    child: Child,
    size: (u16, u16),
    subs: HashMap<u64, Sub>,
    restarts: u32,
    started: Instant,
}

impl Attachment {
    fn wanted_size(&self) -> Option<(u16, u16)> {
        wanted_size(&self.subs)
    }
}

fn wanted_size(subs: &HashMap<u64, Sub>) -> Option<(u16, u16)> {
    subs.values().filter(|s| !s.watch).map(|s| (s.cols, s.rows)).reduce(|a, b| (a.0.min(b.0), a.1.min(b.1)))
}

/// tmux draws pane alternate screens itself, so its client only sends
/// "leave alternate screen" to the outer terminal when it exits. That exit
/// sequence also clears the screen and prints a message like `[exited]`;
/// forwarding it would wipe the session's last screen in every viewer.
/// Returns the bytes to forward and whether the client is exiting.
fn strip_teardown(data: &[u8]) -> (&[u8], bool) {
    const RMCUP: &[u8] = b"\x1b[?1049l";
    const CLEAR: &[u8] = b"\x1b[H\x1b[2J";
    let Some(end) = find(data, RMCUP) else { return (data, false) };
    let cut = rfind(&data[..end], CLEAR).unwrap_or(end);
    (&data[..cut], true)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

pub struct Hub<B: SessionBackend> {
    backend: Arc<B>,
    sessions: Mutex<HashMap<String, Attachment>>,
    gen: AtomicU64,
    on_size: SizeListener,
    on_output: OutputListener,
}

fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(2, 1000), rows.clamp(2, 1000))
}

/// Ends a terminal client. tokio does nothing if the process already exited, and
/// reaps it either way.
fn end_client(att: &mut Attachment) {
    let _ = att.child.start_kill();
}

impl<B: SessionBackend> Hub<B> {
    pub fn new(backend: Arc<B>, on_size: SizeListener, on_output: OutputListener) -> Arc<Self> {
        Arc::new(Hub { backend, sessions: Mutex::new(HashMap::new()), gen: AtomicU64::new(1), on_size, on_output })
    }

    /// Subscribes `conn` to the session's output. `respond` is called with
    /// the effective size before any output is delivered to `sink`, so a
    /// caller can queue its reply ahead of the first output bytes.
    #[allow(clippy::too_many_arguments)]
    pub async fn attach(
        self: &Arc<Self>,
        card_id: &str,
        conn: u64,
        cols: u16,
        rows: u16,
        watch: bool,
        sink: Sink,
        respond: impl FnOnce(u16, u16),
    ) -> Result<(u16, u16)> {
        if !self.backend.exists(card_id).await {
            return Err(err(ErrorCode::NotFound, format!("card {card_id} has no live session")));
        }
        let (cols, rows) = clamp_size(cols, rows);
        let fallback = if watch && !self.has_attachment(card_id) {
            self.backend.window_size(card_id).await.unwrap_or(DEFAULT_SIZE)
        } else {
            DEFAULT_SIZE
        };

        let mut map = self.sessions.lock().unwrap();
        let (old_size, mut subs) = match map.remove(card_id) {
            Some(mut old) => {
                end_client(&mut old);
                (Some(old.size), std::mem::take(&mut old.subs))
            }
            None => (None, HashMap::new()),
        };
        subs.insert(conn, Sub { sink, cols, rows, watch });
        let size = wanted_size(&subs).or(old_size).unwrap_or(fallback);
        if let Err(e) = self.start_client(&mut map, card_id, subs, size, 0) {
            return Err(internal(format!("starting terminal client: {e}")));
        }
        respond(size.0, size.1);
        drop(map);
        if old_size != Some(size) {
            (self.on_size)(card_id, size.0, size.1);
        }
        Ok(size)
    }

    /// Starts a terminal client for `card_id` in a PTY and the task that pumps its
    /// output to `subs`. Must be called with the session map locked; the pump cannot
    /// deliver anything before the caller releases the lock.
    fn start_client(
        self: &Arc<Self>,
        map: &mut HashMap<String, Attachment>,
        card_id: &str,
        subs: HashMap<u64, Sub>,
        size: (u16, u16),
        restarts: u32,
    ) -> std::io::Result<()> {
        let gen = self.gen.fetch_add(1, Ordering::Relaxed);
        let cmd = self.backend.client_command(card_id);
        let (pty, child) = Pty::spawn(cmd, size.0, size.1)?;
        let pty = Arc::new(pty);
        map.insert(
            card_id.to_string(),
            Attachment { gen, pty: pty.clone(), child, size, subs, restarts, started: Instant::now() },
        );
        let hub = self.clone();
        let id = card_id.to_string();
        tokio::spawn(async move { hub.pump(id, gen, pty).await });
        Ok(())
    }

    fn has_attachment(&self, card_id: &str) -> bool {
        self.sessions.lock().unwrap().contains_key(card_id)
    }

    pub fn is_attached(&self, card_id: &str, conn: u64) -> bool {
        self.sessions.lock().unwrap().get(card_id).is_some_and(|a| a.subs.contains_key(&conn))
    }

    pub fn subscriber_count(&self, card_id: &str) -> usize {
        self.sessions.lock().unwrap().get(card_id).map_or(0, |a| a.subs.len())
    }

    /// Number of sessions with a live terminal client.
    pub fn attached_sessions(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    async fn pump(self: Arc<Self>, card_id: String, gen: u64, pty: Arc<Pty>) {
        let mut buf = vec![0u8; 32 * 1024];
        let leaving = loop {
            let n = pty.read(&mut buf).await.unwrap_or(0);
            let mut map = self.sessions.lock().unwrap();
            let Some(att) = map.get_mut(&card_id).filter(|a| a.gen == gen) else { return };
            let (data, ending) = if n == 0 { (&buf[..0], true) } else { strip_teardown(&buf[..n]) };
            if !data.is_empty() {
                att.subs.retain(|_, s| (s.sink)(data));
                (self.on_output)(&card_id);
            }
            if att.subs.is_empty() {
                let mut att = map.remove(&card_id).unwrap();
                end_client(&mut att);
                return;
            }
            if ending {
                // The client is leaving: the session ended, or the client died or was
                // detached while the session lives on. Ignore the rest of its output.
                let mut att = map.remove(&card_id).unwrap();
                end_client(&mut att);
                break att;
            }
        };
        // Bring a new client up if the session still exists.
        self.revive(card_id, leaving).await;
    }

    /// Replaces a terminal client that went away while its session may still be alive,
    /// so subscribers keep receiving output (a fresh client repaints the screen).
    async fn revive(self: Arc<Self>, card_id: String, old: Attachment) {
        // Quick successive failures mean the session is going away; give up then.
        let restarts = if old.started.elapsed() < Duration::from_secs(2) { old.restarts + 1 } else { 0 };
        if restarts > 3 {
            return;
        }
        // Let tmux finish tearing down a session that just ended.
        tokio::time::sleep(Duration::from_millis(150)).await;
        if !self.backend.exists(&card_id).await {
            return;
        }
        let mut map = self.sessions.lock().unwrap();
        if map.contains_key(&card_id) {
            return; // someone attached meanwhile and started a client
        }
        let size = wanted_size(&old.subs).unwrap_or(old.size);
        if self.start_client(&mut map, &card_id, old.subs, size, restarts).is_err() {
            map.remove(&card_id);
        }
    }

    /// Removes a subscriber. Returns `true` if it was attached.
    pub fn detach(&self, card_id: &str, conn: u64) -> bool {
        let mut map = self.sessions.lock().unwrap();
        let Some(att) = map.get_mut(card_id) else { return false };
        if att.subs.remove(&conn).is_none() {
            return false;
        }
        if att.subs.is_empty() {
            let mut att = map.remove(card_id).unwrap();
            end_client(&mut att);
            return true;
        }
        let new = att.wanted_size().unwrap_or(att.size);
        if new != att.size {
            att.size = new;
            let _ = att.pty.resize(new.0, new.1);
            drop(map);
            (self.on_size)(card_id, new.0, new.1);
        }
        true
    }

    /// Sessions `conn` is attached to.
    pub fn cards_of(&self, conn: u64) -> Vec<String> {
        self.sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, a)| a.subs.contains_key(&conn))
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Removes `conn` from every session (connection closed).
    pub fn detach_all(&self, conn: u64) {
        let ids: Vec<String> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, a)| a.subs.contains_key(&conn))
            .map(|(k, _)| k.clone())
            .collect();
        for id in ids {
            self.detach(&id, conn);
        }
    }

    /// Pid of the terminal client for a session, if one is running (for tests).
    pub fn client_pid(&self, card_id: &str) -> Option<u32> {
        self.sessions.lock().unwrap().get(card_id).and_then(|a| a.child.id())
    }

    /// Drops the terminal client for a session that has ended.
    pub fn drop_session(&self, card_id: &str) {
        if let Some(mut att) = self.sessions.lock().unwrap().remove(card_id) {
            end_client(&mut att);
        }
    }

    /// Updates a subscriber's size and returns the effective session size.
    pub fn resize(&self, card_id: &str, conn: u64, cols: u16, rows: u16) -> Result<(u16, u16)> {
        let (cols, rows) = clamp_size(cols, rows);
        let mut map = self.sessions.lock().unwrap();
        let att = map
            .get_mut(card_id)
            .filter(|a| a.subs.contains_key(&conn))
            .ok_or_else(|| err(ErrorCode::Invalid, "not attached to this session"))?;
        let sub = att.subs.get_mut(&conn).unwrap();
        sub.cols = cols;
        sub.rows = rows;
        let new = att.wanted_size().unwrap_or(att.size);
        if new != att.size {
            att.size = new;
            att.pty.resize(new.0, new.1).map_err(internal)?;
            drop(map);
            (self.on_size)(card_id, new.0, new.1);
        }
        Ok(new)
    }

    /// Writes input to the session, through the attached client if there is
    /// one, otherwise directly through the backend.
    pub async fn write(&self, card_id: &str, data: &[u8]) -> Result<()> {
        let pty = self.sessions.lock().unwrap().get(card_id).map(|a| a.pty.clone());
        match pty {
            Some(pty) => pty.write_all(data).await.map_err(|e| err(ErrorCode::Unavailable, format!("write: {e}"))),
            None => self.backend.send_input(card_id, data).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teardown_is_stripped() {
        assert_eq!(strip_teardown(b"hello"), (&b"hello"[..], false));
        let t = b"last line\x1b[1;24r\x1b(B\x1b[m\x1b[H\x1b[2J\x1b[?12l\x1b[?1049l[exited]\r\n";
        assert_eq!(strip_teardown(t), (&b"last line\x1b[1;24r\x1b(B\x1b[m"[..], true));
        assert_eq!(strip_teardown(b"x\x1b[?1049l"), (&b"x"[..], true));
    }
}
