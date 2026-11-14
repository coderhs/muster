//! Saved scrollback for restoring sessions after a reboot or a tmux crash.
//!
//! Written when the last viewer detaches, on a slow timer for sessions that had new
//! output since the last save, and at shutdown. Bounded and written atomically.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use muster_core::util::now_ms;
use tokio::sync::Notify;

use crate::log;
use crate::state::State;

/// At most this much of the end of the history is kept per card.
pub const MAX_BYTES: usize = 256 * 1024;
const HISTORY_LINES: u32 = 5000;

fn period() -> Duration {
    Duration::from_millis(std::env::var("MUSTER_SCROLLBACK_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(60_000))
}

#[derive(Default)]
pub struct Scrollback {
    /// When each card was last saved (ms).
    saved: Mutex<HashMap<String, i64>>,
    kick: Notify,
}

pub fn path(st: &State, card_id: &str) -> PathBuf {
    st.home.join("scrollback").join(format!("{card_id}.ans"))
}

/// Keeps the last `max` bytes, starting at a line boundary.
pub fn tail(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    match text[start..].find('\n') {
        Some(i) => &text[start + i + 1..],
        None => &text[start..],
    }
}

impl Scrollback {
    pub fn kick(&self) {
        self.kick.notify_one();
    }

    pub fn forget(&self, card_id: &str) {
        self.saved.lock().unwrap().remove(card_id);
    }

    /// Captures the session's history (with colours) and writes it atomically.
    pub async fn save(&self, st: &State, card_id: &str) {
        let Some(text) = st.backend.capture(card_id, HISTORY_LINES, true).await else { return };
        let text = text.trim_end_matches('\n');
        let body = tail(text, MAX_BYTES);
        let dest = path(st, card_id);
        let _ = std::fs::create_dir_all(dest.parent().unwrap());
        let tmp = dest.with_extension("tmp");
        // Terminals need CR LF when the bytes are replayed raw.
        let body = body.replace('\n', "\r\n");
        if std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &dest)).is_err() {
            log!("could not save scrollback for {card_id}");
            return;
        }
        self.saved.lock().unwrap().insert(card_id.to_string(), now_ms());
    }

    /// Saves every live session with output newer than its last save.
    pub async fn save_changed(&self, st: &State) {
        let times = st.backend.activity_times().await;
        for (id, t) in times {
            let last = self.saved.lock().unwrap().get(&id).copied().unwrap_or(0);
            if t >= last {
                self.save(st, &id).await;
            }
        }
    }
}

/// The slow save timer; idle (waiting on a notification) while no session exists.
pub async fn run(st: Arc<State>) {
    loop {
        if st.live_count() == 0 {
            st.scrollback.kick.notified().await;
            continue;
        }
        tokio::time::sleep(period()).await;
        st.scrollback.save_changed(&st).await;
    }
}

#[cfg(test)]
mod tests {
    use super::tail;

    #[test]
    fn tail_keeps_whole_lines() {
        assert_eq!(tail("abc", 10), "abc");
        assert_eq!(tail("line1\nline2\nline3", 8), "line3");
        assert_eq!(tail("é\nab", 3), "ab");
    }
}
