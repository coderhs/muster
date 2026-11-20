//! musterd: owns Muster's state and serves the WebSocket API.

mod accounts;
mod agents;
mod context;
mod docs;
mod files;
mod idle;
mod launchd;
mod remote;
mod rename;
mod scrollback;
mod server;
mod state;
mod stats;
mod summarize;
mod usage;

use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use muster_core::store::Store;
use muster_core::tmux::{SessionBackend, TmuxBackend};
use muster_core::util::{hex, random_bytes};
use tokio::net::TcpListener;

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log_line(&format!($($arg)*))
    };
}

/// Lines written since the last size check of the log file.
static LOG_LINES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn log_line(msg: &str) {
    eprintln!("[musterd {}] {msg}", timestamp());
    if LOG_LINES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 200 == 199 {
        rotate_log();
    }
}

fn log_max_bytes() -> u64 {
    std::env::var("MUSTER_LOG_MAX_BYTES").ok().and_then(|v| v.parse().ok()).unwrap_or(5 * 1024 * 1024)
}

/// When stderr is a regular file (launchd or the app redirect it) larger than the
/// limit, it is renamed to `<name>.1` (replacing an older one) and a fresh file takes
/// its place, so the log never grows without bound.
pub fn rotate_log() {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(2, &mut st) } != 0 || (st.st_mode & libc::S_IFMT) != libc::S_IFREG {
        return;
    }
    if (st.st_size as u64) <= log_max_bytes() {
        return;
    }
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    if unsafe { libc::fcntl(2, libc::F_GETPATH, buf.as_mut_ptr()) } != 0 {
        return;
    }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(0);
    let path = PathBuf::from(String::from_utf8_lossy(&buf[..len]).into_owned());
    let mut old = path.clone().into_os_string();
    old.push(".1");
    if std::fs::rename(&path, &old).is_err() {
        return;
    }
    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(&path) {
        unsafe {
            libc::dup2(f.as_raw_fd(), 2);
        }
    }
}

pub fn timestamp() -> String {
    let ms = muster_core::util::now_ms();
    let secs = ms / 1000;
    format!("{:02}:{:02}:{:02}.{:03}Z", (secs / 3600) % 24, (secs / 60) % 60, secs % 60, ms % 1000)
}

const USAGE: &str = "usage: musterd [run | install-agent | uninstall-agent | print-plist | --version]

Environment:
  MUSTER_HOME          data directory (default ~/Library/Application Support/Muster)
  MUSTER_PORT          fixed port to listen on (default: ephemeral)
  MUSTER_TMUX_SOCKET   tmux socket name, must start with \"muster\" (default muster)
  MUSTER_TMUX          path to the tmux binary";

/// The `muster` CLI: `MUSTER_CLI_BIN`, else next to this executable.
pub fn cli_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MUSTER_CLI_BIN").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let p = std::env::current_exe().ok()?.parent()?.join("muster");
    p.is_file().then_some(p)
}

pub fn muster_home() -> PathBuf {
    if let Some(h) = std::env::var_os("MUSTER_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(h);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join("Library/Application Support/Muster")
}

fn main() {
    let arg = std::env::args().nth(1);
    let result = match arg.as_deref() {
        None | Some("run") => run(),
        Some("install-agent") => launchd::install(),
        Some("uninstall-agent") => launchd::uninstall(),
        Some("print-plist") => {
            print!("{}", launchd::plist());
            Ok(())
        }
        Some("--version" | "-V") => {
            println!("musterd {} ({})", env!("CARGO_PKG_VERSION"), env!("MUSTER_BUILD_COMMIT"));
            Ok(())
        }
        Some("--help" | "-h" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command {other:?}\n{USAGE}")),
    };
    if let Err(e) = result {
        eprintln!("musterd: {e}");
        std::process::exit(1);
    }
}

/// Takes an exclusive lock on `daemon.lock`; held for the life of the process.
fn lock_home(home: &Path) -> Result<std::fs::File, String> {
    let path = home.join("daemon.lock");
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let owner = std::fs::read_to_string(home.join("daemon.json")).unwrap_or_default();
        return Err(format!("another musterd is already running for {} {}", home.display(), owner.trim()));
    }
    Ok(f)
}

fn ensure_token(home: &Path) -> Result<String, String> {
    let path = home.join("token");
    match std::fs::read_to_string(&path) {
        Ok(t) if !t.trim().is_empty() => {
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            return Ok(t.trim().to_string());
        }
        _ => {}
    }
    let token = hex(&random_bytes(32));
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(token.as_bytes()).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    Ok(token)
}

fn write_daemon_json(home: &Path, port: u16) -> Result<(), String> {
    let body = serde_json::json!({ "pid": std::process::id(), "port": port }).to_string();
    let tmp = home.join("daemon.json.tmp");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, home.join("daemon.json")).map_err(|e| e.to_string())
}

fn run() -> Result<(), String> {
    let home = muster_home();
    std::fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
    let _ = std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700));
    rotate_log();
    let _lock = lock_home(&home)?;
    let token = ensure_token(&home)?;
    let mut store = Store::open(&home.join("muster.db")).map_err(|e| e.to_string())?;
    let _ = store.prune_activity();
    let socket = std::env::var("MUSTER_TMUX_SOCKET").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "muster".into());
    let backend = TmuxBackend::new(&home, &socket).map_err(|e| e.to_string())?;
    let port: u16 = match std::env::var("MUSTER_PORT") {
        Ok(p) if !p.is_empty() => p.parse().map_err(|_| format!("bad MUSTER_PORT {p:?}"))?,
        _ => 0,
    };

    // Two workers: file reads (transcripts) run in `block_in_place` on one while the
    // other keeps terminals and pings flowing.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(512 * 1024)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async move {
        let listener =
            TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| format!("bind 127.0.0.1:{port}: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let (agents, hook_events) = agents::Agents::new(&home);
        agents.load_usage(
            store.meta_get(usage::META_KEY).ok().flatten(),
            store.meta_get(usage::LEGACY_META_KEY).ok().flatten(),
        );
        let (state, output_rx) = state::State::new(home.clone(), token, store, backend, agents);
        state.backend.reload_config().await;
        state.backend.stop_control_clients().await;
        state.backend.set_clipboard(state.settings().clipboard_passthrough).await;
        // Sessions outlive the daemon: pick up whatever is still running.
        state.reconcile().await;
        state.agents.restore_all(&state);
        if let Some(rx) = hook_events {
            tokio::spawn(agents::run(state.clone(), rx));
        }
        tokio::spawn(state.clone().output_activity(output_rx));
        tokio::spawn(stats::run(state.clone()));
        tokio::spawn(scrollback::run(state.clone()));
        tokio::spawn(idle::run(state.clone()));
        match cli_path().map(|cli| context::write_guidance(&home, &cli)) {
            Some(Ok(g)) => state.agents.set_guidance(g),
            Some(Err(e)) => log!("writing agent instructions: {e}"),
            None => log!("muster CLI not found next to musterd (set MUSTER_CLI_BIN); agents get no CLI instructions"),
        }
        state.update_keep_awake();
        if state.settings().auto_restore {
            let cards = state.store.lock().unwrap().all_cards().unwrap_or_default();
            for card in cards.into_iter().filter(|c| c.restorable) {
                log!("restoring card {}", card.id);
                match server::start_card(&state, card).await {
                    Ok(c) => state.emit_card(c),
                    Err(e) => log!("restore failed: {e}"),
                }
            }
        }
        log!(
            "listening on 127.0.0.1:{port}, home {}, tmux socket {}, {} live session(s)",
            home.display(),
            socket,
            state.live_count()
        );
        write_daemon_json(&home, port)?;
        tokio::spawn(state.clone().watch_sessions());
        tokio::spawn(server::serve(state.clone(), listener));
        remote::init(&state).await;

        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).map_err(|e| e.to_string())?;
        let mut int = signal(SignalKind::interrupt()).map_err(|e| e.to_string())?;
        let mut hup = signal(SignalKind::hangup()).map_err(|e| e.to_string())?;
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
            _ = hup.recv() => {}
        }
        log!("shutting down; sessions keep running in tmux");
        state.scrollback.save_changed(&state).await;
        // Leave nothing of ours behind in tmux but the card sessions themselves.
        state.backend.stop_control_clients().await;
        if state.live_count() == 0 {
            state.backend.unwatch().await;
        }
        state.agents.close();
        let _ = std::fs::remove_file(home.join("daemon.json"));
        Ok::<(), String>(())
    })?;
    // Leave without running destructors for the runtime: terminal clients in
    // PTYs exit on their own when their masters close with the process.
    std::process::exit(0);
}
