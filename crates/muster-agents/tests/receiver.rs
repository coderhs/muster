//! Receiver round trips, with a plain socket client standing in for `muster-hook`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use muster_agents::*;
use serde_json::{json, Value};

fn sock_dir(tag: &str) -> PathBuf {
    // Short path: sun_path is limited to 104 bytes on macOS.
    let d = PathBuf::from(format!("/tmp/mar-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

const SECRET: &str = "0123456789abcdef0123456789abcdef";

/// Binds a receiver with card `c1` registered under [`SECRET`].
fn bind(path: &std::path::Path) -> (HookReceiver, Receiver<NormalizedEvent>) {
    let (r, rx) = HookReceiver::bind(path, AdapterRegistry::default()).unwrap();
    r.set_card_secret("c1", SECRET);
    (r, rx)
}

fn envelope(event: &str, wants_reply: bool, payload: Value) -> String {
    let mut s = json!({"v":1,"kind":"hook","event":event,"cardId":"c1","agentId":"claude","pid":1,"wantsReply":wants_reply,"secret":SECRET,"payload":payload}).to_string();
    s.push('\n');
    s
}

fn perm_payload() -> Value {
    json!({"session_id":"s1","transcript_path":"/t/s1.jsonl","cwd":"/w","hook_event_name":"PermissionRequest",
           "tool_name":"Bash","tool_input":{"command":"touch x"},"permission_suggestions":[]})
}

fn next(rx: &Receiver<NormalizedEvent>) -> NormalizedEvent {
    rx.recv_timeout(Duration::from_secs(5)).expect("event")
}

#[test]
fn forwards_plain_events() {
    let d = sock_dir("plain");
    let (r, rx) = bind(&d.join("h.sock"));
    let mut s = UnixStream::connect(r.path()).unwrap();
    s.write_all(
        envelope(
            "Stop",
            false,
            json!({"session_id":"s1","hook_event_name":"Stop","last_assistant_message":"ok"}),
        )
        .as_bytes(),
    )
    .unwrap();
    let ev = next(&rx);
    assert_eq!(ev.card_id, "c1");
    assert_eq!(ev.session_id.as_deref(), Some("s1"));
    assert!(matches!(ev.kind, AgentEvent::TurnFinished { .. }));
    r.close();
    assert!(!d.join("h.sock").exists());
}

#[test]
fn permission_answered_through_receiver() {
    let d = sock_dir("answer");
    let (r, rx) = bind(&d.join("h.sock"));
    let mut s = UnixStream::connect(r.path()).unwrap();
    s.write_all(envelope("PermissionRequest", true, perm_payload()).as_bytes())
        .unwrap();
    let ev = next(&rx);
    let AgentEvent::PermissionRequested { request } = ev.kind else {
        panic!("{ev:?}")
    };
    let id = request.id.clone().expect("answerable");
    assert_eq!(r.pending().len(), 1);
    r.answer(
        &id,
        &PermissionDecision::Deny {
            message: Some("no".into()),
            interrupt: false,
        },
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line).unwrap();
    let v: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "deny");
    let ev = next(&rx);
    assert!(
        matches!(ev.kind, AgentEvent::PermissionResolved { ref request_id, outcome: PermissionOutcome::Answered { .. } } if *request_id == id)
    );
    assert!(r.pending().is_empty());
    assert!(matches!(
        r.answer(&id, &PermissionDecision::Allow { remember: false }),
        Err(AnswerError::NotFound)
    ));
    r.close();
}

#[test]
fn permission_hook_going_away_resolves_elsewhere() {
    let d = sock_dir("gone");
    let (r, rx) = bind(&d.join("h.sock"));
    let mut s = UnixStream::connect(r.path()).unwrap();
    s.write_all(envelope("PermissionRequest", true, perm_payload()).as_bytes())
        .unwrap();
    let AgentEvent::PermissionRequested { request } = next(&rx).kind else {
        panic!()
    };
    drop(s); // the agent killed the hook (user answered in the terminal)
    let ev = next(&rx);
    assert!(
        matches!(ev.kind, AgentEvent::PermissionResolved { ref request_id, outcome: PermissionOutcome::Elsewhere } if Some(request_id) == request.id.as_ref())
    );
    assert!(r.pending().is_empty());
    r.close();
}

#[test]
fn reply_requested_for_non_permission_gets_empty_line() {
    let d = sock_dir("empty");
    let (r, rx) = bind(&d.join("h.sock"));
    let mut s = UnixStream::connect(r.path()).unwrap();
    s.write_all(envelope("Stop", true, json!({"hook_event_name":"Stop"})).as_bytes())
        .unwrap();
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line).unwrap();
    assert_eq!(line, "\n");
    let _ = next(&rx);
    // unknown agent: still answered with an empty line, no event
    let mut s = UnixStream::connect(r.path()).unwrap();
    s.write_all(
        envelope("Stop", true, json!({}))
            .replace("\"claude\"", "\"nope\"")
            .as_bytes(),
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line).unwrap();
    assert_eq!(line, "\n");
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    r.close();
}

#[test]
fn refuses_live_socket_and_replaces_stale_one() {
    let d = sock_dir("stale");
    let p = d.join("h.sock");
    let (r, _rx) = HookReceiver::bind(&p, AdapterRegistry::default()).unwrap();
    assert!(HookReceiver::bind(&p, AdapterRegistry::default()).is_err());
    r.close();
    // a stale socket file (listener gone) is replaced
    let l = std::os::unix::net::UnixListener::bind(&p).unwrap();
    drop(l);
    let (r2, _rx2) = HookReceiver::bind(&p, AdapterRegistry::default()).unwrap();
    r2.close();
    // a regular file is never removed
    std::fs::write(&p, "x").unwrap();
    assert!(HookReceiver::bind(&p, AdapterRegistry::default()).is_err());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn later_tool_event_releases_hook_answered_in_terminal() {
    let d = sock_dir("settle");
    let (r, rx) = bind(&d.join("h.sock"));
    let mut hook = UnixStream::connect(r.path()).unwrap();
    hook.write_all(envelope("PermissionRequest", true, perm_payload()).as_bytes())
        .unwrap();
    let AgentEvent::PermissionRequested { request } = next(&rx).kind else {
        panic!()
    };

    // a different tool finishing does not settle it
    let mut s = UnixStream::connect(r.path()).unwrap();
    s.write_all(envelope("PostToolUse", false, json!({"hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"ls"}})).as_bytes()).unwrap();
    assert!(matches!(next(&rx).kind, AgentEvent::ToolFinished { .. }));
    assert_eq!(r.pending().len(), 1);

    // the guarded call finishing does
    let mut s = UnixStream::connect(r.path()).unwrap();
    s.write_all(envelope("PostToolUse", false, json!({"hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"touch x"}})).as_bytes()).unwrap();
    let ev = next(&rx);
    assert!(
        matches!(ev.kind, AgentEvent::PermissionResolved { ref request_id, outcome: PermissionOutcome::Elsewhere } if Some(request_id) == request.id.as_ref()),
        "{ev:?}"
    );
    assert!(matches!(next(&rx).kind, AgentEvent::ToolFinished { .. }));
    // the waiting hook got an empty line (prints nothing) and was closed
    let mut line = String::new();
    BufReader::new(&hook).read_line(&mut line).unwrap();
    assert_eq!(line, "\n");
    assert!(r.pending().is_empty());
    r.close();
}

#[test]
fn envelopes_without_the_cards_secret_are_dropped() {
    let d = sock_dir("secret");
    let (r, rx) = bind(&d.join("h.sock"));
    let stop = json!({"session_id":"s1","hook_event_name":"Stop"});
    // wrong secret, missing secret, unregistered card: all dropped, waiting hooks released
    for line in [
        envelope("Stop", false, stop.clone()).replace(SECRET, "ffffffffffffffffffffffffffffffff"),
        envelope("Stop", false, stop.clone()).replace(&format!(",\"secret\":\"{SECRET}\""), ""),
        envelope("Stop", false, stop.clone()).replace("\"c1\"", "\"c2\""),
    ] {
        let mut s = UnixStream::connect(r.path()).unwrap();
        s.write_all(line.as_bytes()).unwrap();
    }
    let mut s = UnixStream::connect(r.path()).unwrap();
    s.write_all(
        envelope("PermissionRequest", true, perm_payload())
            .replace(SECRET, "nope")
            .as_bytes(),
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line).unwrap();
    assert_eq!(line, "\n", "a forged permission request gets no answer");
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    assert_eq!(r.dropped_events(), 4);
    assert!(r.pending().is_empty());
    // the right secret still works
    let mut s = UnixStream::connect(r.path()).unwrap();
    s.write_all(envelope("Stop", false, stop).as_bytes())
        .unwrap();
    assert!(matches!(next(&rx).kind, AgentEvent::TurnFinished { .. }));
    // after removal the card is refused again; migration mode accepts unregistered cards
    r.remove_card_secret("c1");
    r.set_require_secret(false);
    let mut s = UnixStream::connect(r.path()).unwrap();
    s.write_all(
        envelope("Stop", false, json!({"hook_event_name":"Stop"}))
            .replace(&format!(",\"secret\":\"{SECRET}\""), "")
            .as_bytes(),
    )
    .unwrap();
    assert!(matches!(next(&rx).kind, AgentEvent::TurnFinished { .. }));
    r.close();
}

#[test]
fn transcript_paths_outside_the_agents_root_are_removed() {
    let d = sock_dir("paths");
    let cfg = d.join("cfg");
    let proj = cfg.join("projects/-w");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("s1.jsonl"), "").unwrap();
    std::fs::write(d.join("secret.txt"), "x").unwrap();
    // a symlink inside the root that points outside it
    std::os::unix::fs::symlink(d.join("secret.txt"), proj.join("evil.jsonl")).unwrap();
    let claude = ClaudeCode {
        program: "claude".into(),
        config_dir: cfg.clone(),
    };
    let registry = AdapterRegistry::empty().with(std::sync::Arc::new(claude));
    let (r, rx) = HookReceiver::bind(&d.join("h.sock"), registry).unwrap();
    r.set_card_secret("c1", SECRET);
    let cases = [
        (proj.join("s1.jsonl"), true),
        (proj.join("new-not-yet-written.jsonl"), true),
        (proj.join("evil.jsonl"), false),
        (d.join("secret.txt"), false),
        (proj.join("../../../secret.txt"), false),
        (proj.join("notes.txt"), false),
    ];
    for (path, ok) in cases {
        let mut s = UnixStream::connect(r.path()).unwrap();
        let p = json!({"session_id":"s1","hook_event_name":"Stop","transcript_path":path});
        s.write_all(envelope("Stop", false, p).as_bytes()).unwrap();
        let ev = next(&rx);
        assert_eq!(ev.transcript_path.is_some(), ok, "{}", path.display());
    }
    r.close();
    let _ = std::fs::remove_dir_all(d);
}
