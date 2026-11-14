//! Finding and, when needed, starting the Muster daemon.
//!
//! The shell starts `musterd run` only when no live daemon owns `$MUSTER_HOME`, detaches
//! it into its own process group so it outlives the app, and never stops it. The
//! daemon's own lock on `daemon.lock` is the final guard against two daemons.

use serde::Deserialize;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub fn muster_home() -> Result<PathBuf, String> {
    if let Some(h) = std::env::var_os("MUSTER_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(h));
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/Application Support/Muster"))
}

#[derive(Deserialize, Clone, Copy)]
pub struct DaemonJson {
    pub pid: Option<i32>,
    pub port: u16,
}

pub fn read_daemon_json(home: &Path) -> Result<DaemonJson, String> {
    let path = home.join("daemon.json");
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("invalid {}: {e}", path.display()))
}

pub fn read_token(home: &Path) -> Result<String, String> {
    let path = home.join("token");
    Ok(std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .trim()
        .to_string())
}

fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // Signal 0 checks existence; EPERM still means the process exists.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// A daemon is live when daemon.json names a running `musterd` process. (No TCP probe:
/// the daemon would log every half-open connection.)
pub fn is_live(home: &Path) -> bool {
    let Ok(dj) = read_daemon_json(home) else { return false };
    match dj.pid {
        Some(pid) => pid_alive(pid) && is_musterd(pid),
        None => {
            let addr = SocketAddr::from(([127, 0, 0, 1], dj.port));
            TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
        }
    }
}

/// Guards against a stale daemon.json whose pid now belongs to another program.
fn is_musterd(pid: i32) -> bool {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        // Not allowed to look (other user): trust the pid.
        return true;
    }
    buf.truncate(n as usize);
    Path::new(std::str::from_utf8(&buf).unwrap_or("")).file_name().is_some_and(|f| f == "musterd")
}

/// Locates a Muster binary: `$MUSTER_BIN_DIR`, then beside the app executable (the
/// bundled copies in Contents/MacOS), then, in debug builds, the repository's `target/`.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("MUSTER_BIN_DIR").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(dir).join(name);
        return p.is_file().then_some(p);
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    #[cfg(debug_assertions)]
    {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for profile in ["debug", "release"] {
            let p = repo.join("target").join(profile).join(name);
            if p.is_file() {
                return p.canonicalize().ok();
            }
        }
    }
    None
}

static LAST_SPAWN: Mutex<Option<Instant>> = Mutex::new(None);

#[derive(Debug, PartialEq)]
pub enum Ensure {
    AlreadyRunning,
    Started(PathBuf),
    /// Started recently; give it time to write daemon.json.
    Starting,
}

/// Starts the daemon unless one is live. Rate-limited so a slow start is not doubled.
/// `explicit` is a user request ("Start daemon"); it ignores MUSTER_NO_DAEMON_START.
pub fn ensure_running(explicit: bool) -> Result<Ensure, String> {
    let home = muster_home()?;
    if is_live(&home) {
        return Ok(Ensure::AlreadyRunning);
    }
    // MUSTER_NO_DAEMON_START=1: never start one at launch (e.g. while using the mock).
    if !explicit && std::env::var_os("MUSTER_NO_DAEMON_START").is_some_and(|v| v == "1") {
        return Ok(Ensure::AlreadyRunning);
    }
    let mut last = LAST_SPAWN.lock().unwrap();
    if last.is_some_and(|t| t.elapsed() < Duration::from_secs(10)) {
        return Ok(Ensure::Starting);
    }
    let bin = find_binary("musterd")
        .ok_or("musterd not found (set MUSTER_BIN_DIR, or build it with `cargo build -p musterd`)")?;
    std::fs::create_dir_all(&home).map_err(|e| format!("cannot create {}: {e}", home.display()))?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("musterd.log"))
        .map_err(|e| format!("cannot open musterd.log: {e}"))?;

    let mut cmd = Command::new(&bin);
    cmd.arg("run")
        .env("MUSTER_HOME", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    if let Some(dir) = bin.parent() {
        // Lets the daemon find muster-hook beside itself however it looks it up.
        let hook = dir.join("muster-hook");
        if hook.is_file() {
            cmd.env("MUSTER_HOOK_BIN", hook);
        }
    }
    {
        use std::os::unix::process::CommandExt;
        // Own session (and so own process group, no controlling terminal): signals to the
        // app's group, a closing terminal, or the app exiting never reach the daemon.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = cmd.spawn().map_err(|e| format!("cannot start {}: {e}", bin.display()))?;
    *last = Some(Instant::now());
    // Reap it if it exits while we are alive, so it never lingers as a zombie.
    std::thread::Builder::new()
        .name("musterd-reaper".into())
        .spawn(move || {
            let _ = child.wait();
        })
        .ok();
    Ok(Ensure::Started(bin))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Starts a real daemon in `$MUSTER_HOME` and returns; the caller checks that it
    /// outlives this process. Needs MUSTER_HOME and MUSTER_TMUX_SOCKET set.
    /// `cargo test spawn_detached -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn spawn_detached() {
        assert!(std::env::var("MUSTER_HOME").is_ok(), "set MUSTER_HOME to a temp dir");
        assert!(std::env::var("MUSTER_TMUX_SOCKET").is_ok_and(|s| s.starts_with("muster-test")));
        let r = ensure_running(true).expect("start");
        assert!(matches!(r, Ensure::Started(_)), "{r:?}");
        let home = muster_home().unwrap();
        for _ in 0..50 {
            if is_live(&home) {
                println!("daemon live: {:?}", read_daemon_json(&home).map(|d| d.pid));
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("daemon did not come up");
    }
}
