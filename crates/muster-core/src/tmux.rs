//! Session backend trait and its tmux implementation.
//!
//! Every tmux invocation goes through [`TmuxBackend::cmd`], which always passes
//! `-L <socket>` so that only Muster's own tmux server is ever touched.

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use muster_proto::ErrorCode;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::util::{err, hex, internal, shell_quote, Result};

pub const SESSION_PREFIX: &str = "m-";

/// Bytes per `send-keys -H` call.
const SEND_CHUNK: usize = 2048;
pub const CONTROL_SESSION: &str = "muster-ctl";

pub fn session_name(card_id: &str) -> String {
    format!("{SESSION_PREFIX}{card_id}")
}

/// Everything needed to start a card's session.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub card_id: String,
    pub cwd: String,
    /// Argv run inside the login shell, or `None` for an interactive shell.
    pub argv: Option<Vec<String>>,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

/// What a [`SessionWatch`] reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// Sessions may have been created or destroyed.
    Sessions,
    /// A card session finished its worktree setup with this exit code.
    SetupDone { card_id: String, code: i32 },
    /// A session armed with [`TmuxBackend::watch_silence`] produced no output for the
    /// configured time.
    Silence { card_id: String },
    /// A session armed with [`TmuxBackend::watch_activity`] produced output.
    Activity { card_id: String },
}

/// Paste-buffer names the alert hooks set (and delete at once) so the control client
/// hears about silence and activity in a session.
pub const SILENCE_MARK: &str = "muster-silence:";
pub const ACTIVITY_MARK: &str = "muster-activity:";

/// Window name a setup wrapper sets when it finishes: `muster-setup:<exit code>`.
pub const SETUP_MARK: &str = "muster-setup:";

/// Notifications that the set of live sessions may have changed.
pub trait SessionWatch: Send {
    /// Resolves with the next event, or `None` when the watch has ended, after which
    /// it must be recreated.
    fn next(&mut self) -> impl Future<Output = Option<WatchEvent>> + Send;
}

pub trait SessionBackend: Send + Sync + 'static {
    type Watch: SessionWatch;

    fn create(&self, spec: &SessionSpec) -> impl Future<Output = Result<()>> + Send;
    fn kill(&self, card_id: &str) -> impl Future<Output = Result<()>> + Send;
    fn exists(&self, card_id: &str) -> impl Future<Output = bool> + Send;
    /// Card ids that currently have a session.
    fn list(&self) -> impl Future<Output = Result<HashSet<String>>> + Send;
    fn window_size(&self, card_id: &str) -> impl Future<Output = Option<(u16, u16)>> + Send;
    /// Sends input to a session nobody is attached to.
    fn send_input(&self, card_id: &str, data: &[u8]) -> impl Future<Output = Result<()>> + Send;
    /// The command for a terminal client of this session, to run in a PTY.
    fn client_command(&self, card_id: &str) -> Command;
    /// The exit code recorded for an ended session, consumed on read.
    fn take_exit_code(&self, card_id: &str) -> Option<i32>;
    fn watch(&self) -> impl Future<Output = Result<Self::Watch>> + Send;
    /// Stops whatever `watch` started (called when no sessions remain).
    fn unwatch(&self) -> impl Future<Output = ()> + Send;
}

pub struct TmuxBackend {
    tmux: PathBuf,
    socket: String,
    config: PathBuf,
    exit_dir: PathBuf,
    shell: String,
}

/// Finds the tmux binary: `MUSTER_TMUX`, then PATH, then common locations
/// (launchd gives daemons a minimal PATH).
pub fn find_tmux() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MUSTER_TMUX") {
        return Some(PathBuf::from(p));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin", "/usr/bin"].iter().map(PathBuf::from))
        .map(|d| d.join("tmux"))
        .find(|p| p.is_file())
}

/// The user's login shell: `$SHELL`, else the password database, else zsh.
pub fn login_shell() -> String {
    if let Ok(s) = std::env::var("SHELL") {
        if !s.is_empty() {
            return s;
        }
    }
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if !pw.is_null() && !(*pw).pw_shell.is_null() {
            if let Ok(s) = std::ffi::CStr::from_ptr((*pw).pw_shell).to_str() {
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    "/bin/zsh".to_string()
}

const FALLBACK_LOCALE: &str = "en_US.UTF-8";

/// Whether the daemon's own environment names a UTF-8 locale.
fn has_utf8_locale() -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty())).is_some_and(|v| {
        let v = v.to_ascii_lowercase();
        v.contains("utf-8") || v.contains("utf8")
    })
}

fn config_text(tmux: &Path, exit_dir: &Path) -> String {
    let exit_dir = exit_dir.display();
    let tmux = tmux.display();
    let (silence, activity) = (SILENCE_MARK, ACTIVITY_MARK);
    format!(
        r#"# Generated by musterd. Changes are overwritten.
# Hook commands must print nothing and exit 0: tmux shows run-shell output (and a
# non-zero exit status) over the pane of an attached client, and that view mode
# swallows the client's keys.
set -g status off
set -g history-limit 50000
set -g default-terminal "xterm-256color"
set -as terminal-features ",xterm-256color:RGB"
set -as terminal-overrides ",xterm-256color:Tc"
set -g mouse on
set -g escape-time 10
set -g focus-events on
set -g set-clipboard off
set -g prefix None
set -g prefix2 None
set -g exit-empty on
set -g destroy-unattached off
set -g window-size latest
set -g remain-on-exit on
set -g silence-action any
set -g activity-action any
set -g visual-silence off
set -g visual-activity off
set -g visual-bell off
bind -T copy-mode WheelUpPane select-pane \; send-keys -X -N 1 scroll-up
bind -T copy-mode WheelDownPane select-pane \; send-keys -X -N 1 scroll-down
bind -T copy-mode-vi WheelUpPane select-pane \; send-keys -X -N 1 scroll-up
bind -T copy-mode-vi WheelDownPane select-pane \; send-keys -X -N 1 scroll-down
set-hook -g alert-silence "run-shell -b \"'{tmux}' -S '#{{socket_path}}' set-buffer -b '{silence}#{{session_name}}' 1 >/dev/null 2>&1; '{tmux}' -S '#{{socket_path}}' delete-buffer -b '{silence}#{{session_name}}' >/dev/null 2>&1; true\""
set-hook -g alert-activity "run-shell -b \"'{tmux}' -S '#{{socket_path}}' set-buffer -b '{activity}#{{session_name}}' 1 >/dev/null 2>&1; '{tmux}' -S '#{{socket_path}}' delete-buffer -b '{activity}#{{session_name}}' >/dev/null 2>&1; true\""
set-hook -g pane-died "run-shell -b \"echo '#{{pane_dead_status}}:#{{pane_dead_signal}}' > '{exit_dir}/#{{session_name}}' 2>/dev/null; '{tmux}' -S '#{{socket_path}}' kill-session -t '=#{{session_name}}' >/dev/null 2>&1; true\""
"#
    )
}

impl TmuxBackend {
    /// Prepares the backend: writes the tmux config into `home` and makes
    /// the exit-code directory. Does not start tmux.
    pub fn new(home: &Path, socket: &str) -> Result<TmuxBackend> {
        if !socket.starts_with("muster") {
            return Err(err(ErrorCode::Invalid, "the tmux socket name must start with \"muster\""));
        }
        // Without tmux the daemon still starts (and says so in daemon.status); every
        // session operation then fails with `unavailable`.
        let tmux = find_tmux().filter(|p| p.is_file()).unwrap_or_else(|| PathBuf::from("/nonexistent/tmux"));
        let exit_dir = home.join("exit");
        std::fs::create_dir_all(&exit_dir).map_err(internal)?;
        let config = home.join("tmux.conf");
        std::fs::write(&config, config_text(&tmux, &exit_dir)).map_err(internal)?;
        Ok(TmuxBackend { tmux, socket: socket.to_string(), config, exit_dir, shell: login_shell() })
    }

    /// Whether a tmux binary was found.
    pub fn available(&self) -> bool {
        self.tmux.is_file()
    }

    /// Lets programs in cards set the viewer's clipboard (OSC 52) or not.
    pub async fn set_clipboard(&self, on: bool) {
        let _ = self.run(["set-option", "-g", "set-clipboard", if on { "external" } else { "off" }]).await;
    }

    pub fn socket(&self) -> &str {
        &self.socket
    }

    pub fn shell(&self) -> &str {
        &self.shell
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new(&self.tmux);
        // `-u`: always treat clients as UTF-8. Started from the Finder the daemon has no
        // locale, and tmux would then replace every non-ASCII character with `_`.
        c.arg("-u").arg("-L").arg(&self.socket).arg("-f").arg(&self.config);
        c.env_remove("TMUX").env_remove("TMUX_PANE");
        if !has_utf8_locale() {
            c.env("LANG", FALLBACK_LOCALE);
        }
        c.stdin(Stdio::null());
        c.kill_on_drop(true);
        c
    }

    async fn run<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        if !self.available() {
            return Err(err(ErrorCode::Unavailable, "tmux is not installed"));
        }
        let out = self.cmd().args(args).output().await.map_err(|e| internal(format!("tmux: {e}")))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(err(ErrorCode::Unavailable, format!("tmux: {}", String::from_utf8_lossy(&out.stderr).trim())))
        }
    }

    /// Reloads the generated config into a server that is already running.
    pub async fn reload_config(&self) {
        let _ = self.run([std::ffi::OsStr::new("source-file"), self.config.as_os_str()]).await;
    }

    /// Last output time (unix ms) per card session, from tmux's `window_activity`.
    pub async fn activity_times(&self) -> std::collections::HashMap<String, i64> {
        let Ok(out) = self.run(["list-windows", "-a", "-F", "#{session_name} #{window_activity}"]).await else {
            return Default::default();
        };
        let mut map = std::collections::HashMap::new();
        for line in out.lines() {
            let mut it = line.split_whitespace();
            let (Some(name), Some(t)) = (it.next(), it.next().and_then(|t| t.parse::<i64>().ok())) else { continue };
            if let Some(id) = name.strip_prefix(SESSION_PREFIX) {
                let e = map.entry(id.to_string()).or_insert(0);
                *e = (*e).max(t * 1000);
            }
        }
        map
    }

    /// Setup exit codes recorded in window names (for setups that finished while
    /// nobody was watching).
    pub async fn setup_marks(&self) -> std::collections::HashMap<String, i32> {
        let Ok(out) = self.run(["list-windows", "-a", "-F", "#{session_name} #{window_name}"]).await else {
            return Default::default();
        };
        out.lines()
            .filter_map(|l| {
                let (s, name) = l.split_once(' ')?;
                let id = s.strip_prefix(SESSION_PREFIX)?;
                let code = name.strip_prefix(SETUP_MARK)?.trim().parse().ok()?;
                Some((id.to_string(), code))
            })
            .collect()
    }

    /// Arms a one-shot silence alert: the watch reports `Silence` once the session has
    /// been quiet for `secs`. tmux's own timers do the waiting.
    pub async fn watch_silence(&self, card_id: &str, secs: u64) {
        self.set_monitors(card_id, secs.max(1), false).await;
    }

    /// Arms a one-shot activity alert: the watch reports `Activity` on the next output.
    pub async fn watch_activity(&self, card_id: &str) {
        self.set_monitors(card_id, 0, true).await;
    }

    pub async fn unwatch_quiet(&self, card_id: &str) {
        self.set_monitors(card_id, 0, false).await;
    }

    async fn set_monitors(&self, card_id: &str, silence: u64, activity: bool) {
        let win = format!("={}:", session_name(card_id));
        let sess = format!("={}", session_name(card_id));
        let silence = silence.to_string();
        let activity = if activity { "on" } else { "off" };
        let _ = self
            .run([
                "set-option", "-w", "-t", &win, "monitor-silence", &silence, ";",
                "set-option", "-w", "-t", &win, "monitor-activity", activity, ";",
                // Clear earlier alerts so the next one fires.
                "kill-session", "-C", "-t", &sess,
            ])
            .await;
    }

    /// The pid of each card session's pane process.
    pub async fn pane_pids(&self) -> std::collections::HashMap<String, i32> {
        let Ok(out) = self.run(["list-panes", "-a", "-F", "#{session_name} #{pane_pid}"]).await else {
            return Default::default();
        };
        out.lines()
            .filter_map(|l| {
                let (s, pid) = l.split_once(' ')?;
                Some((s.strip_prefix(SESSION_PREFIX)?.to_string(), pid.trim().parse().ok()?))
            })
            .collect()
    }

    /// The session's history and screen, with escape sequences (`-e`), joined lines.
    pub async fn capture(&self, card_id: &str, lines: u32, escapes: bool) -> Option<String> {
        let target = format!("={}:", session_name(card_id));
        let start = format!("-{lines}");
        let mut args = vec!["capture-pane", "-p", "-J", "-t", &target];
        if lines > 0 {
            args.extend(["-S", &start]);
        }
        if escapes {
            args.push("-e");
        }
        self.run(&args).await.ok()
    }

    /// Ends every control client attached to the control session. A control client
    /// does not exit when the daemon that started it dies (tmux holds its pipes), so a
    /// restarted daemon clears the old one, and a stopping daemon its own.
    pub async fn stop_control_clients(&self) {
        let target = format!("={CONTROL_SESSION}");
        let Ok(out) = self.run(["list-clients", "-t", &target, "-F", "#{client_pid}"]).await else { return };
        for pid in out.lines().filter_map(|l| l.trim().parse::<i32>().ok()) {
            if pid > 1 {
                unsafe {
                    libc::kill(pid, libc::SIGTERM);
                }
            }
        }
    }

    pub fn tmux_path(&self) -> &Path {
        &self.tmux
    }

    /// Kills this backend's whole tmux server. For tests and explicit reset.
    pub async fn kill_server(&self) {
        let _ = self.run(["kill-server"]).await;
    }

    /// The session command. `TMUX`, `TMUX_PANE` and `MUSTER_HOME` are removed so that tmux commands
    /// run inside a card (`tmux kill-server`, `send-keys`, ...) cannot reach Muster's
    /// own server.
    fn shell_argv(&self, argv: &Option<Vec<String>>) -> Vec<String> {
        let mut out: Vec<String> =
            // MUSTER_HOME would also point the card at the master token (the daemon's own
            // environment reaches sessions through the tmux server).
            ["/usr/bin/env", "-u", "TMUX", "-u", "TMUX_PANE", "-u", "MUSTER_HOME"].iter().map(|s| s.to_string()).collect();
        match argv {
            None => out.extend([self.shell.clone(), "-l".into()]),
            Some(argv) => {
                let line = argv.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
                out.extend([self.shell.clone(), "-l".into(), "-i".into(), "-c".into(), format!("exec {line}")]);
            }
        }
        out
    }
}

impl SessionBackend for TmuxBackend {
    type Watch = ControlWatch;

    async fn create(&self, spec: &SessionSpec) -> Result<()> {
        if !Path::new(&spec.cwd).is_dir() {
            return Err(err(ErrorCode::Invalid, format!("cwd {} is not a directory", spec.cwd)));
        }
        let name = session_name(&spec.card_id);
        let _ = std::fs::remove_file(self.exit_dir.join(&name));
        let mut args: Vec<String> = vec![
            "new-session".into(),
            "-d".into(),
            "-s".into(),
            name,
            "-x".into(),
            spec.cols.to_string(),
            "-y".into(),
            spec.rows.to_string(),
            "-c".into(),
            spec.cwd.clone(),
        ];
        // A server started earlier may have no locale in its environment; programs in
        // the card need one to print anything but ASCII.
        if !has_utf8_locale() && !spec.env.iter().any(|(k, _)| k == "LANG" || k == "LC_ALL") {
            args.push("-e".into());
            args.push(format!("LANG={FALLBACK_LOCALE}"));
        }
        for (k, v) in &spec.env {
            args.push("-e".into());
            args.push(format!("{k}={v}"));
        }
        args.push("--".into());
        args.extend(self.shell_argv(&spec.argv));
        self.run(&args).await.map(|_| ())
    }

    async fn kill(&self, card_id: &str) -> Result<()> {
        self.run(["kill-session", "-t", &format!("={}", session_name(card_id))]).await.map(|_| ())
    }

    async fn exists(&self, card_id: &str) -> bool {
        self.run(["has-session", "-t", &format!("={}", session_name(card_id))]).await.is_ok()
    }

    async fn list(&self) -> Result<HashSet<String>> {
        if !self.available() {
            return Ok(HashSet::new());
        }
        let out = self.cmd().args(["list-sessions", "-F", "#{session_name}"]).output().await.map_err(internal)?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stderr);
            if msg.contains("no server running") || msg.contains("error connecting") || msg.contains("No such file") {
                return Ok(HashSet::new());
            }
            return Err(err(ErrorCode::Unavailable, format!("tmux: {}", msg.trim())));
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.strip_prefix(SESSION_PREFIX))
            .map(str::to_string)
            .collect())
    }

    async fn window_size(&self, card_id: &str) -> Option<(u16, u16)> {
        let out = self
            .run(["display-message", "-p", "-t", &format!("={}:", session_name(card_id)), "#{window_width} #{window_height}"])
            .await
            .ok()?;
        let mut it = out.split_whitespace().map(|s| s.parse::<u16>().ok());
        Some((it.next()??, it.next()??))
    }

    async fn send_input(&self, card_id: &str, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        // One argument per byte: send in chunks so large input stays far below the
        // argument-size limit. Chunks are sent in order, one tmux call each.
        let target = format!("={}:", session_name(card_id));
        for chunk in data.chunks(SEND_CHUNK) {
            let mut args: Vec<String> = vec!["send-keys".into(), "-t".into(), target.clone(), "-H".into()];
            args.extend(chunk.iter().map(|b| hex(&[*b])));
            self.run(&args).await?;
        }
        Ok(())
    }

    fn client_command(&self, card_id: &str) -> Command {
        let mut c = self.cmd();
        c.args(["attach-session", "-t", &format!("={}", session_name(card_id))]);
        c.env("TERM", "xterm-256color").env("COLORTERM", "truecolor");
        c
    }

    fn take_exit_code(&self, card_id: &str) -> Option<i32> {
        let path = self.exit_dir.join(session_name(card_id));
        let text = std::fs::read_to_string(&path).ok()?;
        let _ = std::fs::remove_file(&path);
        text.trim().split(':').next().and_then(|s| s.parse().ok())
    }

    async fn watch(&self) -> Result<ControlWatch> {
        // The control client needs a session to attach to; keep a tiny one.
        let _ = self.run(["new-session", "-d", "-s", CONTROL_SESSION, "-x", "20", "-y", "5", "cat"]).await;
        let mut child = self
            .cmd()
            .args(["-C", "attach-session", "-t", &format!("={CONTROL_SESSION}")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| internal(format!("tmux control client: {e}")))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or_else(|| internal("no control stdout"))?;
        Ok(ControlWatch { tmux: self.tmux.clone(), socket: self.socket.clone(), _child: child, _stdin: stdin, lines: BufReader::new(stdout).lines() })
    }

    async fn unwatch(&self) {
        let _ = self.run(["kill-session", "-t", &format!("={CONTROL_SESSION}")]).await;
    }
}

/// A tmux control-mode client; tmux pushes `%sessions-changed` whenever a
/// session is created or destroyed, so no polling is needed.
pub struct ControlWatch {
    tmux: PathBuf,
    socket: String,
    _child: Child,
    _stdin: Option<ChildStdin>,
    lines: tokio::io::Lines<BufReader<ChildStdout>>,
}

impl SessionWatch for ControlWatch {
    async fn next(&mut self) -> Option<WatchEvent> {
        loop {
            let line = self.lines.next_line().await.ok()??;
            if line.starts_with("%sessions-changed") {
                return Some(WatchEvent::Sessions);
            }
            if line.starts_with("%exit") {
                return None;
            }
            if let Some(name) = line.strip_prefix("%paste-buffer-changed ") {
                let name = name.trim();
                let card = |mark: &str| name.strip_prefix(mark).and_then(|s| s.strip_prefix(SESSION_PREFIX)).map(str::to_string);
                if let Some(card_id) = card(SILENCE_MARK) {
                    return Some(WatchEvent::Silence { card_id });
                }
                if let Some(card_id) = card(ACTIVITY_MARK) {
                    return Some(WatchEvent::Activity { card_id });
                }
                continue;
            }
            // `%unlinked-window-renamed @7 muster-setup:0`
            let rest = line.strip_prefix("%unlinked-window-renamed ").or_else(|| line.strip_prefix("%window-renamed "));
            if let Some((window, name)) = rest.and_then(|r| r.split_once(' ')) {
                if let Some(code) = name.strip_prefix(SETUP_MARK).and_then(|c| c.trim().parse().ok()) {
                    if let Some(card_id) = self.session_of(window).await {
                        return Some(WatchEvent::SetupDone { card_id, code });
                    }
                }
            }
        }
    }
}

impl ControlWatch {
    async fn session_of(&self, window: &str) -> Option<String> {
        let out = Command::new(&self.tmux)
            .arg("-L")
            .arg(&self.socket)
            .args(["display-message", "-p", "-t", window, "#{session_name}"])
            .env_remove("TMUX")
            .stdin(Stdio::null())
            .output()
            .await
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().strip_prefix(SESSION_PREFIX).map(str::to_string)
    }
}
