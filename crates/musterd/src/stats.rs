//! Resource stats: sampled only while some client has subscribed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use muster_core::procstats::{usage, ProcTable};
use muster_core::util::now_ms;
use muster_proto::{CardStats, Event, ProcStats, StatsChanged};
use tokio::sync::Notify;

use crate::state::State;

pub const MIN_INTERVAL_MS: u64 = 2_000;
pub const DEFAULT_INTERVAL_MS: u64 = 5_000;

#[derive(Default)]
pub struct Stats {
    /// Subscribed connection -> requested interval.
    subs: Mutex<HashMap<u64, u64>>,
    kick: Notify,
    /// Last CPU time seen per pid, for percentages.
    prev: Mutex<HashMap<i32, (u64, Instant)>>,
}

impl Stats {
    pub fn subscribe(&self, conn: u64, interval_ms: Option<u64>) -> u64 {
        let ms = interval_ms.unwrap_or(DEFAULT_INTERVAL_MS).max(MIN_INTERVAL_MS);
        self.subs.lock().unwrap().insert(conn, ms);
        self.kick.notify_one();
        ms
    }

    pub fn unsubscribe(&self, conn: u64) {
        if self.subs.lock().unwrap().remove(&conn).is_some() {
            self.kick.notify_one();
        }
    }

    fn interval(&self) -> Option<u64> {
        self.subs.lock().unwrap().values().copied().min()
    }

    /// CPU percent of `pid` since its previous sample (0 on the first).
    fn cpu_percent(&self, prev: &mut HashMap<i32, (u64, Instant)>, pid: i32, cpu_ns: u64, now: Instant) -> f64 {
        let pct = match prev.get(&pid) {
            Some(&(last, at)) if cpu_ns >= last => {
                let wall = now.duration_since(at).as_nanos() as f64;
                if wall > 0.0 {
                    (cpu_ns - last) as f64 / wall * 100.0
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };
        prev.insert(pid, (cpu_ns, now));
        pct
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

async fn sample(st: &Arc<State>) -> StatsChanged {
    let panes = st.backend.pane_pids().await;
    let table = ProcTable::read();
    let now = Instant::now();
    let mut prev = st.stats.prev.lock().unwrap();
    let mut seen = Vec::new();
    let mut cards = Vec::new();
    let mut total = ProcStats::default();
    for (card_id, root) in panes {
        let mut s = ProcStats::default();
        for pid in table.tree(root) {
            if let Some(u) = usage(pid) {
                s.mem_bytes += u.footprint;
                s.cpu_percent += st.stats.cpu_percent(&mut prev, pid, u.cpu_ns, now);
                s.processes += 1;
                seen.push(pid);
            }
        }
        s.cpu_percent = round1(s.cpu_percent);
        total.mem_bytes += s.mem_bytes;
        total.cpu_percent += s.cpu_percent;
        total.processes += s.processes;
        cards.push(CardStats { card_id, stats: s });
    }
    total.cpu_percent = round1(total.cpu_percent);
    cards.sort_by(|a, b| a.card_id.cmp(&b.card_id));
    let me = std::process::id() as i32;
    let daemon = usage(me)
        .map(|u| ProcStats {
            mem_bytes: u.footprint,
            cpu_percent: round1(st.stats.cpu_percent(&mut prev, me, u.cpu_ns, now)),
            processes: 1,
        })
        .unwrap_or_default();
    seen.push(me);
    prev.retain(|pid, _| seen.contains(pid));
    StatsChanged { cards, total, daemon, at: now_ms() }
}

/// Runs for the life of the daemon; sleeps on a notification while nobody has
/// subscribed, so it costs nothing then.
pub async fn run(st: Arc<State>) {
    let mut sampling = false;
    loop {
        let Some(ms) = st.stats.interval() else {
            if std::mem::replace(&mut sampling, false) {
                crate::log!("stats sampling stopped (no subscribers)");
            }
            st.stats.prev.lock().unwrap().clear();
            st.stats.kick.notified().await;
            continue;
        };
        if !std::mem::replace(&mut sampling, true) {
            crate::log!("stats sampling started (every {ms} ms)");
        }
        let ev = sample(&st).await;
        let conns = st.stats.subs.lock().unwrap().keys().copied().collect();
        st.clients.send_to(&conns, &Event::StatsChanged(ev));
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(ms)) => {}
            _ = st.stats.kick.notified() => {}
        }
    }
}
