//! The menu-bar item's own small connection to the daemon (no Tauri types here, so it
//! can be tested against the mock). It keeps needs-you cards and project names, reports
//! status changes worth a notification, and fetches RAM/CPU totals only on request:
//! one subscription, two samples (the first has no CPU figure), then unsubscribe. With
//! no request pending it holds no stats subscription, so the daemon samples nothing.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::{stream::MaybeTlsStream, Message, WebSocket};

#[derive(Clone, Debug)]
pub struct CardLite {
    pub title: String,
    pub project_id: String,
    pub status: String,
    pub status_since: i64,
    /// Finished or waiting, and not opened by the user since.
    pub unread: bool,
}

#[derive(Default)]
pub struct State {
    pub connected: bool,
    /// Cards are known (first board load done); before that, changes are not news.
    pub loaded: bool,
    pub projects: HashMap<String, String>,
    pub cards: HashMap<String, CardLite>,
    /// Memory and CPU of all card sessions, from the last on-demand sample.
    pub total: Option<(u64, f64)>,
}

pub type Shared = Arc<Mutex<State>>;

pub trait Hooks: Send + Sync {
    /// Something shown in the menu changed (or labels need a refresh).
    fn changed(&self, st: &State);
    /// A card started needing the user, or finished.
    fn news(&self, card_id: &str, card: &CardLite, project: &str);
}

pub struct Endpoint {
    pub url: String,
    pub token: String,
}

pub struct Watcher {
    pub state: Shared,
    /// Set (e.g. when the tray menu is about to open) to fetch fresh totals once.
    pub want_stats: Arc<AtomicBool>,
    hooks: Arc<dyn Hooks>,
}

impl Watcher {
    pub fn new(hooks: Arc<dyn Hooks>) -> Self {
        Watcher { state: Arc::default(), want_stats: Arc::new(AtomicBool::new(false)), hooks }
    }

    /// Runs forever, reconnecting with backoff.
    pub fn run(&self, resolve: impl Fn() -> Result<Endpoint, String>) {
        let mut backoff = Duration::from_millis(500);
        loop {
            let started = Instant::now();
            if let Ok(ep) = resolve() {
                let _ = self.session(&ep, None);
            }
            {
                let mut st = self.state.lock().unwrap();
                st.connected = false;
                st.loaded = false;
                st.cards.clear();
            }
            self.hooks.changed(&self.state.lock().unwrap());
            if started.elapsed() > Duration::from_secs(10) {
                backoff = Duration::from_millis(500);
            }
            std::thread::sleep(backoff);
            backoff = (backoff * 2).min(Duration::from_secs(10));
        }
    }

    /// One connection. `max` bounds it (tests); `None` runs until the socket drops.
    /// Returns the methods it sent, in order.
    pub fn session(&self, ep: &Endpoint, max: Option<Duration>) -> Result<Vec<String>, String> {
        let begun = Instant::now();
        let (mut ws, _) = tungstenite::connect(ep.url.as_str()).map_err(|e| e.to_string())?;
        if let MaybeTlsStream::Plain(s) = ws.get_ref() {
            // Short reads so a stats request is noticed quickly; an idle wake-up is cheap.
            let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
        }
        let mut sent = Vec::new();
        let mut next_id = 1u64;
        let mut send = |ws: &mut WebSocket<MaybeTlsStream<TcpStream>>, m: &str, p: Value| -> Result<u64, String> {
            let id = next_id;
            next_id += 1;
            sent.push(m.to_string());
            ws.send(Message::text(json!({"t":"req","id":id,"m":m,"p":p}).to_string())).map_err(|e| e.to_string())?;
            Ok(id)
        };
        let hello = send(&mut ws, "hello", json!({"token": ep.token, "client": "desktop", "protocol": 1}))?;
        let mut board_ids: Vec<u64> = Vec::new();
        // On-demand stats: when the subscription started and how many samples arrived.
        let mut stats: Option<(Instant, u8)> = None;
        let mut last_label_refresh = Instant::now();
        loop {
            if max.is_some_and(|m| begun.elapsed() >= m) {
                drop(send);
                return Ok(sent);
            }
            let connected = self.state.lock().unwrap().connected;
            if connected && stats.is_none() && self.want_stats.swap(false, Ordering::SeqCst) {
                send(&mut ws, "stats.subscribe", json!({"intervalMs": 2000}))?;
                stats = Some((Instant::now(), 0));
            }
            if let Some((t, _)) = stats {
                if t.elapsed() > Duration::from_secs(6) {
                    send(&mut ws, "stats.unsubscribe", json!({}))?;
                    stats = None;
                }
            }
            if last_label_refresh.elapsed() > Duration::from_secs(30) {
                last_label_refresh = Instant::now();
                self.hooks.changed(&self.state.lock().unwrap());
            }
            let msg = match ws.read() {
                Ok(m) => m,
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
                {
                    continue
                }
                Err(e) => return Err(e.to_string()),
            };
            let Message::Text(text) = msg else { continue };
            let Ok(f) = serde_json::from_str::<Value>(text.as_str()) else { continue };
            let p = f.get("p").cloned().unwrap_or(Value::Null);
            match f.get("t").and_then(Value::as_str) {
                Some("res") => {
                    let id = f.get("id").and_then(Value::as_u64).unwrap_or(0);
                    if f.get("ok") != Some(&Value::Bool(true)) {
                        if id == hello {
                            return Err("hello refused".into());
                        }
                        continue;
                    }
                    let r = f.get("r").cloned().unwrap_or(Value::Null);
                    if id == hello {
                        self.state.lock().unwrap().connected = true;
                        send(&mut ws, "project.list", json!({}))?;
                    } else if let Some(projects) = r.get("projects").and_then(Value::as_array) {
                        let ids: Vec<String> = {
                            let mut st = self.state.lock().unwrap();
                            st.projects.clear();
                            st.cards.clear();
                            for p in projects {
                                if let (Some(id), Some(name)) = (p["id"].as_str(), p["name"].as_str()) {
                                    st.projects.insert(id.into(), name.into());
                                }
                            }
                            st.projects.keys().cloned().collect()
                        };
                        for pid in ids {
                            board_ids.push(send(&mut ws, "board.get", json!({"projectId": pid}))?);
                        }
                        if board_ids.is_empty() {
                            self.state.lock().unwrap().loaded = true;
                        }
                        self.hooks.changed(&self.state.lock().unwrap());
                    } else if board_ids.contains(&id) {
                        board_ids.retain(|x| *x != id);
                        let mut st = self.state.lock().unwrap();
                        for c in r.get("cards").and_then(Value::as_array).into_iter().flatten() {
                            if let Some((id, c)) = card_lite(c) {
                                st.cards.insert(id, c);
                            }
                        }
                        if board_ids.is_empty() {
                            st.loaded = true;
                        }
                        self.hooks.changed(&st);
                    }
                }
                Some("ev") => match f.get("m").and_then(Value::as_str) {
                    Some("card.changed") => {
                        let mut st = self.state.lock().unwrap();
                        let matters = if let Some(id) = p.get("removedId").and_then(Value::as_str) {
                            // Any removed card may have been counted by the notch strip.
                            st.cards.remove(id).is_some()
                        } else if let Some((id, c)) = p.get("card").and_then(card_lite) {
                            let before = st.cards.get(&id).cloned();
                            // The menu lists cards that need you; the notch strip counts
                            // working and finished ones, so every status or read change counts.
                            let matters = before
                                .as_ref()
                                .is_none_or(|b| b.status != c.status || b.unread != c.unread || b.title != c.title);
                            let news = st.loaded
                                && (c.status == "needs_you" || c.status == "done")
                                && before.as_ref().is_none_or(|b| b.status != c.status);
                            if news {
                                let project = st.projects.get(&c.project_id).cloned().unwrap_or_default();
                                self.hooks.news(&id, &c, &project);
                            }
                            st.cards.insert(id, c);
                            matters
                        } else {
                            false
                        };
                        if matters {
                            self.hooks.changed(&st);
                        }
                    }
                    Some("stats.changed") => {
                        let t = &p["total"];
                        if let (Some(m), Some(c)) = (t["memBytes"].as_u64(), t["cpuPercent"].as_f64()) {
                            let mut st = self.state.lock().unwrap();
                            let n = stats.map(|(_, n)| n + 1).unwrap_or(2);
                            // First sample: memory only (its CPU is always 0).
                            let cpu = if n >= 2 { c } else { st.total.map(|(_, c)| c).unwrap_or(0.0) };
                            st.total = Some((m, cpu));
                            self.hooks.changed(&st);
                            drop(st);
                            if n >= 2 && stats.is_some() {
                                send(&mut ws, "stats.unsubscribe", json!({}))?;
                                stats = None;
                            } else if let Some((t0, _)) = stats {
                                stats = Some((t0, n));
                            }
                        }
                    }
                    Some("project.changed") => {
                        let mut st = self.state.lock().unwrap();
                        if let Some(id) = p.get("removedId").and_then(Value::as_str) {
                            st.projects.remove(id);
                            st.cards.retain(|_, c| c.project_id != id);
                        } else if let (Some(id), Some(name)) =
                            (p["project"]["id"].as_str(), p["project"]["name"].as_str())
                        {
                            st.projects.insert(id.into(), name.into());
                        }
                        self.hooks.changed(&st);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn card_lite(v: &Value) -> Option<(String, CardLite)> {
    Some((
        v.get("id")?.as_str()?.to_string(),
        CardLite {
            title: v.get("title").and_then(Value::as_str).unwrap_or("Untitled").to_string(),
            project_id: v.get("projectId")?.as_str()?.to_string(),
            status: v.get("status")?.as_str()?.to_string(),
            status_since: v.get("statusSince").and_then(Value::as_i64).unwrap_or(0),
            unread: v.get("unread").and_then(Value::as_bool).unwrap_or(false),
        },
    ))
}

#[cfg(test)]
mod tests {
    //! Run against the mock: `node mock/daemon.mjs --port 7397` in another shell, then
    //! `MUSTER_TEST_WS=ws://127.0.0.1:7397/ws cargo test watcher -- --ignored --nocapture`.
    use super::*;

    struct Count(Mutex<usize>);
    impl Hooks for Count {
        fn changed(&self, _: &State) {
            *self.0.lock().unwrap() += 1;
        }
        fn news(&self, _: &str, _: &CardLite, _: &str) {}
    }

    fn ep() -> Endpoint {
        Endpoint {
            url: std::env::var("MUSTER_TEST_WS").expect("MUSTER_TEST_WS"),
            token: std::env::var("MUSTER_TEST_TOKEN").unwrap_or_else(|_| "dev".into()),
        }
    }

    #[test]
    #[ignore]
    fn watcher_idle_holds_no_stats_subscription() {
        let w = Watcher::new(Arc::new(Count(Mutex::new(0))));
        let sent = w.session(&ep(), Some(Duration::from_secs(4))).unwrap();
        println!("idle sent: {sent:?}");
        assert!(w.state.lock().unwrap().loaded);
        assert!(!sent.iter().any(|m| m.starts_with("stats.")), "{sent:?}");
    }

    #[test]
    #[ignore]
    fn watcher_fetches_totals_once_on_request() {
        let w = Arc::new(Watcher::new(Arc::new(Count(Mutex::new(0)))));
        let w2 = w.clone();
        // Ask for totals after the boards have loaded.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(800));
            w2.want_stats.store(true, Ordering::SeqCst);
        });
        let sent = w.session(&ep(), Some(Duration::from_secs(8))).unwrap();
        println!("on-demand sent: {sent:?}");
        let subs = sent.iter().filter(|m| *m == "stats.subscribe").count();
        let unsubs = sent.iter().filter(|m| *m == "stats.unsubscribe").count();
        assert_eq!((subs, unsubs), (1, 1), "{sent:?}");
        assert_eq!(sent.last().map(String::as_str), Some("stats.unsubscribe"));
        assert!(w.state.lock().unwrap().total.is_some());
    }
}
