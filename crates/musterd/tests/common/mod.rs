//! Shared helpers for the daemon integration tests: a temporary MUSTER_HOME with a
//! unique `muster-test-*` tmux socket, a daemon process, and a protocol client.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use muster_proto::*;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Generous so that a busy machine (parallel builds, other test runs) does not fail
/// the tests; waits end as soon as their condition holds.
pub const TIMEOUT: Duration = Duration::from_secs(60);

pub fn unique() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{n}-{:x}", std::process::id(), t & 0xffff_ffff)
}

pub struct Env {
    pub root: PathBuf,
    pub home: PathBuf,
    pub socket: String,
    pub daemon: Option<Child>,
    /// Where the daemon's stderr goes (inherited when `None`).
    pub log_file: Option<PathBuf>,
    /// Extra environment for the daemon.
    pub env: Vec<(String, String)>,
}

impl Env {
    pub fn new() -> Env {
        let id = unique();
        let root = std::env::temp_dir().join(format!("muster-it-{id}"));
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(root.join("project")).unwrap();
        Env { root: root.clone(), home, socket: format!("muster-test-{id}"), daemon: None,
            log_file: None,
            // Tests that exercise idle detection set their own (short) window.
            env: vec![
                ("MUSTER_QUIET_MS".into(), "600000".into()),
                // Never look at the real agent data of the machine running the tests.
                ("CODEX_HOME".into(), root.join("codex-home").to_string_lossy().into_owned()),
                ("CLAUDE_CONFIG_DIR".into(), root.join("claude-config").to_string_lossy().into_owned()),
            ],
        }
    }

    /// A stand-in login shell for sessions: `/bin/sh` without its login profile. A real
    /// login shell runs macOS's path_helper, which puts /opt/homebrew/bin (where real
    /// agent CLIs may live) ahead of the tests' fake agents.
    pub fn test_shell(&self) -> PathBuf {
        let bin = self.root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let sh = bin.join("testsh");
        if !sh.exists() {
            std::fs::write(
                &sh,
                "#!/bin/sh\nwhile [ $# -gt 0 ]; do case \"$1\" in -l|-i) shift ;; *) break ;; esac; done\nexec /bin/sh \"$@\"\n",
            )
            .unwrap();
            assert!(Command::new("chmod").arg("755").arg(&sh).status().unwrap().success());
        }
        sh
    }

    pub fn path_var(&self) -> String {
        format!("{}:{}", self.root.join("bin").display(), std::env::var("PATH").unwrap_or_default())
    }

    /// Panics unless `name` resolves to the fake in this test's bin directory, exactly
    /// as a session would resolve it, so a real agent CLI is never started.
    pub fn assert_fake(&self, name: &str) {
        let out = Command::new(self.test_shell())
            .args(["-l", "-i", "-c", &format!("command -v {name}")])
            .env("PATH", self.path_var())
            .output()
            .unwrap();
        let found = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(Path::new(&found), self.root.join("bin").join(name), "{name} would not be the fake");
    }

    pub fn start_daemon(&mut self) -> u16 {
        let _ = std::fs::remove_file(self.home.join("daemon.json"));
        for agent in ["claude", "codex", "cursor-agent", "opencode", "gemini", "antigravity", "copilot", "grok"] {
            if self.root.join("bin").join(agent).exists() {
                self.assert_fake(agent);
            }
        }
        let child = Command::new(env!("CARGO_BIN_EXE_musterd"))
            .env("MUSTER_HOME", &self.home)
            .env("MUSTER_TMUX_SOCKET", &self.socket)
            .env("SHELL", self.test_shell())
            .env("PATH", self.path_var())
            .envs(self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .env_remove("MUSTER_PORT")
            .env_remove("TMUX")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(match &self.log_file {
                Some(p) => Stdio::from(std::fs::OpenOptions::new().create(true).append(true).open(p).unwrap()),
                None => Stdio::inherit(),
            })
            .spawn()
            .unwrap();
        let pid = child.id();
        self.daemon = Some(child);
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Ok(s) = std::fs::read_to_string(self.home.join("daemon.json")) {
                let v: Value = serde_json::from_str(&s).unwrap();
                if v["pid"] == pid {
                    return v["port"].as_u64().unwrap() as u16;
                }
            }
            assert!(Instant::now() < deadline, "daemon did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn crash_daemon(&mut self) {
        if let Some(mut d) = self.daemon.take() {
            // SIGKILL: no chance to clean up. Never panics (this also runs in Drop).
            let _ = d.kill();
            let _ = d.wait();
        }
    }

    pub fn tmux_sessions(&self) -> String {
        let out = Command::new("tmux").args(["-L", &self.socket, "list-sessions", "-F", "#{session_name}"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

/// The `muster` CLI binary from this workspace, built on first use.
pub fn cli_bin() -> PathBuf {
    static BUILT: std::sync::Once = std::sync::Once::new();
    let dir = Path::new(env!("CARGO_BIN_EXE_musterd")).parent().unwrap().to_path_buf();
    BUILT.call_once(|| {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
        let status = Command::new(env!("CARGO"))
            .args(["build", "-q", "-p", "muster-cli", "--manifest-path"])
            .arg(manifest)
            .status()
            .unwrap();
        assert!(status.success(), "building muster-cli failed");
    });
    dir.join("muster")
}

/// The `muster-hook` binary from this workspace, built on first use.
pub fn hook_bin() -> PathBuf {
    static BUILT: std::sync::Once = std::sync::Once::new();
    let dir = Path::new(env!("CARGO_BIN_EXE_musterd")).parent().unwrap().to_path_buf();
    BUILT.call_once(|| {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
        let status = Command::new(env!("CARGO"))
            .args(["build", "-q", "-p", "muster-hook", "--manifest-path"])
            .arg(manifest)
            .status()
            .unwrap();
        assert!(status.success(), "building muster-hook failed");
    });
    let p = dir.join("muster-hook");
    assert!(p.is_file(), "{} missing", p.display());
    p
}

/// Kills the test's tmux server and every tmux process for that socket. `kill-server`
/// alone is not enough: when a daemon was SIGKILLed, its control client stays attached
/// and the server waits for it forever.
pub fn kill_tmux(socket: &str) {
    assert!(socket.starts_with("muster-test-"), "only test sockets are killed");
    let _ = Command::new("tmux").args(["-L", socket, "kill-server"]).output();
    // The trailing space keeps `muster-test-1` from matching `muster-test-10`.
    let pattern = format!("-L {socket} ");
    for _ in 0..20 {
        let out = Command::new("/usr/bin/pgrep").args(["-f", "--", &pattern]).output();
        let pids: Vec<String> = out
            .map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();
        if pids.is_empty() {
            return;
        }
        let _ = Command::new("/bin/kill").arg("-9").args(&pids).output();
        std::thread::sleep(Duration::from_millis(50));
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        self.crash_daemon();
        assert!(self.socket.starts_with("muster-test-"));
        kill_tmux(&self.socket);
        let tmpdir = std::env::var("TMUX_TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let uid = unsafe { libc::getuid() };
        let _ = std::fs::remove_file(Path::new(&tmpdir).join(format!("tmux-{uid}")).join(&self.socket));
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub struct Client {
    pub ws: Ws,
    pub next: u64,
    pub events: Vec<EventFrame>,
    pub output: Vec<u8>,
    /// Only output of this card is collected (the one attached last).
    pub focus: Option<String>,
}

impl Client {
    /// Connects and says hello with `token`; `Err` if the daemon refuses it.
    pub async fn connect_token(port: u16, token: &str) -> Result<Client, ErrorBody> {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await.unwrap();
        let mut c = Client { ws, next: 1, events: vec![], output: vec![], focus: None };
        c.call("hello", json!({"token": token, "client": "cli", "protocol": 1})).await?;
        Ok(c)
    }

    pub async fn connect(port: u16, home: &Path) -> Client {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await.unwrap();
        let mut c = Client { ws, next: 1, events: vec![], output: vec![], focus: None };
        let token = std::fs::read_to_string(home.join("token")).unwrap();
        let r = c.call("hello", json!({"token": token.trim(), "client": "cli", "protocol": 1})).await.unwrap();
        assert_eq!(r["protocol"], 1);
        c
    }

    pub async fn send(&mut self, m: &str, p: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        let f = json!({"t": "req", "id": id, "m": m, "p": p});
        self.ws.send(Message::text(f.to_string())).await.unwrap();
        id
    }

    /// Reads one frame, recording events and terminal output.
    pub async fn pump(&mut self) -> Option<ResponseFrame> {
        let Ok(msg) = tokio::time::timeout(TIMEOUT, self.ws.next()).await else {
            let out = String::from_utf8_lossy(&self.output).into_owned();
            let tail: String = out.chars().rev().take(800).collect::<Vec<_>>().into_iter().rev().collect();
            panic!("timed out waiting for daemon; terminal tail: {tail:?}");
        };
        match msg.expect("socket closed").unwrap() {
            Message::Text(t) => match serde_json::from_str::<ServerFrame>(t.as_str()).unwrap() {
                ServerFrame::Response(r) => Some(r),
                ServerFrame::Event(e) => {
                    self.events.push(e);
                    None
                }
            },
            Message::Binary(b) => {
                let (sid, data) = decode_term_frame(&b).expect("valid terminal frame");
                if self.focus.as_deref().is_none_or(|f| f == sid) {
                    self.output.extend_from_slice(data);
                }
                None
            }
            _ => None,
        }
    }

    pub async fn call(&mut self, m: &str, p: Value) -> Result<Value, ErrorBody> {
        let id = self.send(m, p).await;
        loop {
            if let Some(r) = self.pump().await {
                assert_eq!(r.id, id, "responses arrive in order here");
                return if r.ok { Ok(r.r.unwrap()) } else { Err(r.e.unwrap()) };
            }
        }
    }

    /// Attaches and checks that the response precedes any output.
    pub async fn attach(&mut self, card: &str, cols: u16, rows: u16) -> Value {
        self.output.clear();
        self.focus = Some(card.to_string());
        let id = self.send("session.attach", json!({"cardId": card, "cols": cols, "rows": rows})).await;
        loop {
            let before = self.output.len();
            if let Some(r) = self.pump().await {
                assert_eq!(r.id, id);
                assert!(r.ok, "{:?}", r.e);
                return r.r.unwrap();
            }
            assert_eq!(self.output.len(), before, "output arrived before the attach response");
        }
    }

    pub async fn wait_output(&mut self, needle: &str) {
        let deadline = Instant::now() + TIMEOUT;
        while !String::from_utf8_lossy(&self.output).contains(needle) {
            if Instant::now() >= deadline {
                let out = String::from_utf8_lossy(&self.output).into_owned();
                let tail: String = out.chars().rev().take(1200).collect::<Vec<_>>().into_iter().rev().collect();
                panic!("never saw {needle:?}; output ends with {tail:?}");
            }
            self.pump().await;
        }
    }

    pub async fn wait_event(&mut self, name: &str, card: &str) -> Value {
        loop {
            if let Some(i) = self.events.iter().position(|e| e.m == name && e.p["cardId"] == card) {
                return self.events.remove(i).p;
            }
            self.pump().await;
        }
    }
}


impl Client {
    pub async fn wait_event_named(&mut self, name: &str) -> Value {
        loop {
            if let Some(i) = self.events.iter().position(|e| e.m == name) {
                return self.events.remove(i).p;
            }
            self.pump().await;
        }
    }
}

