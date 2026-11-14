//! `muster-hook`: the forwarder an agent CLI runs on each hook event.
//!
//! ```text
//! muster-hook hook <EventName> [--wait <secs>]
//! muster-hook statusline
//! ```
//!
//! Behaviour:
//!
//! * Unless both `MUSTER_CARD_ID` and `MUSTER_HOOK_SOCK` are set, it drains stdin and exits 0
//!   without output (for `statusline` it still runs the chained status line, see below).
//! * Otherwise it reads the hook payload from stdin and sends one line of JSON to the Unix
//!   socket at `MUSTER_HOOK_SOCK`:
//!   `{"v":1,"kind":"hook","event":"<EventName>","cardId":"..","agentId":"..","wantsReply":false,"payload":{..}}`
//!   plus `"secret":".."` when `MUSTER_HOOK_SECRET` is set (the per-launch secret the
//!   receiver checks), (`payload` is the CLI's JSON verbatim with line breaks removed, or `null` with
//!   `"truncated":true` when larger than [`MAX_PAYLOAD`]).
//! * With `--wait <secs>` it sets `"wantsReply":true` and waits up to that long for one line
//!   back; a non-empty line is printed to stdout verbatim (it is the decision JSON the CLI
//!   expects, built by the daemon's adapter). On timeout, EOF or any error it prints nothing,
//!   so the CLI falls back to its own prompt.
//! * `statusline` forwards the status line JSON with `"kind":"statusline"` and then, if
//!   `MUSTER_STATUSLINE_CHAIN` is set, runs that command through `/bin/sh -c` with the same
//!   JSON on stdin and passes its stdout through, so the user's own status line still shows.
//!
//! Every failure path exits 0 quickly: the agent must never break because the daemon is
//! down. The connect is to a local socket (immediate refusal when nobody listens) and writes
//! are bounded by a short timeout.

use std::env;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Largest payload forwarded verbatim. Bigger payloads (huge tool outputs) are replaced by
/// `null` so a hook event is never dropped or stalled on size.
const MAX_PAYLOAD: usize = 4 * 1024 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_millis(1500);
/// Upper bound for `--wait`, as a guard against a typo holding the agent for hours.
const MAX_WAIT_SECS: u64 = 3600;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("hook");
    match mode {
        "statusline" => statusline(),
        "--version" | "-V" => println!("muster-hook {}", env!("CARGO_PKG_VERSION")),
        _ => hook(&args),
    }
    // Always exit 0: a non-zero exit would surface as a hook error in the agent's UI.
    std::process::exit(0);
}

struct Target {
    card_id: String,
    agent_id: String,
    sock: String,
    secret: String,
}

fn target() -> Option<Target> {
    let card_id = env::var("MUSTER_CARD_ID").ok().filter(|s| !s.is_empty())?;
    let sock = env::var("MUSTER_HOOK_SOCK")
        .ok()
        .filter(|s| !s.is_empty())?;
    let agent_id = env::var("MUSTER_AGENT_ID").unwrap_or_default();
    let secret = env::var("MUSTER_HOOK_SECRET").unwrap_or_default();
    Some(Target {
        card_id,
        agent_id,
        sock,
        secret,
    })
}

fn read_stdin() -> (Vec<u8>, bool) {
    let mut buf = Vec::with_capacity(8192);
    let mut stdin = io::stdin().lock();
    let mut chunk = [0u8; 65536];
    let mut truncated = false;
    loop {
        match stdin.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() + n > MAX_PAYLOAD {
                    truncated = true;
                    // keep draining so the writer never sees EPIPE
                    continue;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    if truncated {
        buf.clear();
    }
    (buf, truncated)
}

fn hook(args: &[String]) {
    let event = args.get(1).map(String::as_str).unwrap_or("");
    let mut wait: Option<u64> = None;
    let mut i = 2;
    while i < args.len() {
        if args[i] == "--wait" {
            wait = args
                .get(i + 1)
                .and_then(|s| s.parse().ok())
                .map(|w: u64| w.min(MAX_WAIT_SECS));
            i += 1;
        }
        i += 1;
    }

    let (payload, truncated) = read_stdin();
    let Some(t) = target() else { return };
    let line = envelope("hook", event, &t, wait.is_some(), &payload, truncated);

    let Some(mut stream) = connect_and_send(&t.sock, &line) else {
        return;
    };
    let Some(secs) = wait else { return };
    if let Some(reply) = read_line(&mut stream, Duration::from_secs(secs)) {
        let reply = reply.trim();
        if !reply.is_empty() {
            let mut out = io::stdout().lock();
            let _ = out.write_all(reply.as_bytes());
            let _ = out.write_all(b"\n");
            let _ = out.flush();
        }
    }
}

fn statusline() {
    let (payload, truncated) = read_stdin();
    if let Some(t) = target() {
        let line = envelope("statusline", "StatusLine", &t, false, &payload, truncated);
        let _ = connect_and_send(&t.sock, &line);
    }
    let Ok(chain) = env::var("MUSTER_STATUSLINE_CHAIN") else {
        return;
    };
    if chain.trim().is_empty() {
        return;
    }
    let child = Command::new("/bin/sh")
        .arg("-c")
        .arg(&chain)
        .env_remove("MUSTER_STATUSLINE_CHAIN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn();
    let Ok(mut child) = child else { return };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&payload);
    }
    if let Ok(output) = child.wait_with_output() {
        let _ = io::stdout().lock().write_all(&output.stdout);
    }
}

fn envelope(
    kind: &str,
    event: &str,
    t: &Target,
    wants_reply: bool,
    payload: &[u8],
    truncated: bool,
) -> Vec<u8> {
    let mut line = Vec::with_capacity(payload.len() + 256);
    line.extend_from_slice(b"{\"v\":1,\"kind\":");
    push_json_str(&mut line, kind);
    line.extend_from_slice(b",\"event\":");
    push_json_str(&mut line, event);
    line.extend_from_slice(b",\"cardId\":");
    push_json_str(&mut line, &t.card_id);
    line.extend_from_slice(b",\"agentId\":");
    push_json_str(&mut line, &t.agent_id);
    if !t.secret.is_empty() {
        line.extend_from_slice(b",\"secret\":");
        push_json_str(&mut line, &t.secret);
    }
    line.extend_from_slice(b",\"pid\":");
    line.extend_from_slice(parent_pid().to_string().as_bytes());
    line.extend_from_slice(if wants_reply {
        b",\"wantsReply\":true"
    } else {
        b",\"wantsReply\":false"
    });
    if truncated {
        line.extend_from_slice(b",\"truncated\":true");
    }
    line.extend_from_slice(b",\"payload\":");
    let trimmed = trim_ascii(payload);
    if trimmed.first() == Some(&b'{') && trimmed.last() == Some(&b'}') {
        // Raw CR/LF can only occur as insignificant whitespace in valid JSON (inside
        // strings they must be escaped), so dropping them keeps the document intact
        // and makes the envelope a single line.
        line.extend(
            trimmed
                .iter()
                .copied()
                .filter(|&b| b != b'\n' && b != b'\r'),
        );
    } else {
        line.extend_from_slice(b"null");
    }
    line.extend_from_slice(b"}\n");
    line
}

fn trim_ascii(b: &[u8]) -> &[u8] {
    let start = b
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .unwrap_or(b.len());
    let end = b
        .iter()
        .rposition(|c| !c.is_ascii_whitespace())
        .map_or(start, |e| e + 1);
    &b[start..end]
}

fn push_json_str(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    for c in s.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes())
            }
            c => {
                let mut b = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
        }
    }
    out.push(b'"');
}

fn parent_pid() -> u32 {
    std::os::unix::process::parent_id()
}

fn connect_and_send(sock: &str, line: &[u8]) -> Option<UnixStream> {
    let mut stream = UnixStream::connect(sock).ok()?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT)).ok()?;
    stream.write_all(line).ok()?;
    stream.flush().ok()?;
    Some(stream)
}

/// Read until the first newline or `limit` elapses. Returns `None` on timeout or EOF without
/// a complete line.
fn read_line(stream: &mut UnixStream, limit: Duration) -> Option<String> {
    let deadline = Instant::now() + limit;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        if left.is_zero() {
            return None;
        }
        stream.set_read_timeout(Some(left)).ok()?;
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    buf.truncate(pos);
                    return String::from_utf8(buf).ok();
                }
                if buf.len() > MAX_PAYLOAD {
                    return None;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
}
