//! Black-box tests of the `muster-hook` binary with a throwaway socket server.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_muster-hook");

fn dir(tag: &str) -> PathBuf {
    let d = PathBuf::from(format!("/tmp/mht-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str], env: &[(&str, &str)], stdin: &str) -> (String, Duration, i32) {
    let start = Instant::now();
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .env_remove("MUSTER_CARD_ID")
        .env_remove("MUSTER_HOOK_SECRET")
        .env_remove("MUSTER_HOOK_SOCK")
        .env_remove("MUSTER_STATUSLINE_CHAIN");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (String::from_utf8(out.stdout).unwrap(), start.elapsed(), out.status.code().unwrap_or(-1))
}

#[test]
fn no_env_is_a_silent_noop() {
    let (out, _, code) = run(&["hook", "Stop"], &[], "{\"a\":1}");
    assert_eq!((out.as_str(), code), ("", 0));
}

#[test]
fn daemon_down_fails_open_fast() {
    let d = dir("down");
    let sock = d.join("none.sock");
    let (out, took, code) = run(
        &["hook", "PermissionRequest", "--wait", "30"],
        &[("MUSTER_CARD_ID", "c"), ("MUSTER_HOOK_SOCK", sock.to_str().unwrap())],
        "{\"a\":1}",
    );
    assert_eq!((out.as_str(), code), ("", 0));
    assert!(took < Duration::from_secs(2), "took {took:?}");
}

#[test]
fn forwards_one_line_and_prints_reply() {
    let d = dir("reply");
    let sock = d.join("h.sock");
    let _ = std::fs::remove_file(&sock);
    let l = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line).unwrap();
        (&s).write_all(b"{\"decision\":1}\n").unwrap();
        line
    });
    let payload = "{\n  \"hook_event_name\": \"PermissionRequest\",\n  \"tool_input\": {\"command\": \"echo \\\"hi\\\"\\n\"}\n}\n";
    let (out, _, code) = run(
        &["hook", "PermissionRequest", "--wait", "5"],
        &[
            ("MUSTER_CARD_ID", "card \"7\""),
            ("MUSTER_AGENT_ID", "claude"),
            ("MUSTER_HOOK_SOCK", sock.to_str().unwrap()),
            ("MUSTER_HOOK_SECRET", "abc123"),
        ],
        payload,
    );
    assert_eq!((out.as_str(), code), ("{\"decision\":1}\n", 0));
    let line = server.join().unwrap();
    assert!(line.ends_with('\n') && line.matches('\n').count() == 1, "single line: {line:?}");
    let v: serde_like::Value = serde_like::parse(&line);
    assert_eq!(v.get("cardId"), "card \"7\"");
    assert_eq!(v.get("event"), "PermissionRequest");
    assert!(line.contains("\"wantsReply\":true"));
    assert_eq!(v.get("secret"), "abc123");
    assert!(line.contains(r#""command": "echo \"hi\"\n""#), "payload kept verbatim: {line}");
}

#[test]
fn reply_timeout_prints_nothing() {
    let d = dir("timeout");
    let sock = d.join("h.sock");
    let _ = std::fs::remove_file(&sock);
    let l = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        std::thread::sleep(Duration::from_secs(3));
        drop(s);
    });
    let (out, took, code) = run(
        &["hook", "PermissionRequest", "--wait", "1"],
        &[("MUSTER_CARD_ID", "c"), ("MUSTER_HOOK_SOCK", sock.to_str().unwrap())],
        "{}",
    );
    assert_eq!((out.as_str(), code), ("", 0));
    assert!(took >= Duration::from_millis(900) && took < Duration::from_millis(2500), "took {took:?}");
    server.join().unwrap();
}

#[test]
fn statusline_forwards_and_chains() {
    let d = dir("sl");
    let sock = d.join("h.sock");
    let _ = std::fs::remove_file(&sock);
    let l = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line).unwrap();
        line
    });
    let (out, _, _) = run(
        &["statusline"],
        &[
            ("MUSTER_CARD_ID", "c"),
            ("MUSTER_HOOK_SOCK", sock.to_str().unwrap()),
            ("MUSTER_STATUSLINE_CHAIN", "printf 'mine:'; cat"),
        ],
        "{\"model\":{\"id\":\"m\"}}",
    );
    assert_eq!(out, "mine:{\"model\":{\"id\":\"m\"}}");
    let line = server.join().unwrap();
    assert!(line.contains("\"kind\":\"statusline\""));
    // without Muster env the chain still runs
    let (out, _, _) = run(&["statusline"], &[("MUSTER_STATUSLINE_CHAIN", "echo plain")], "{}");
    assert_eq!(out, "plain\n");
}

#[test]
fn non_json_payload_becomes_null() {
    let d = dir("nonjson");
    let sock = d.join("h.sock");
    let _ = std::fs::remove_file(&sock);
    let l = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line).unwrap();
        line
    });
    run(&["hook", "Stop"], &[("MUSTER_CARD_ID", "c"), ("MUSTER_HOOK_SOCK", sock.to_str().unwrap())], "garbage\nmore");
    assert!(server.join().unwrap().ends_with("\"payload\":null}\n"));
}

/// Minimal field extraction so the hook crate keeps zero dependencies, dev included.
mod serde_like {
    pub struct Value(String);
    pub fn parse(s: &str) -> Value {
        Value(s.to_string())
    }
    impl Value {
        /// Top-level string field (enough for the fields the envelope writes first).
        pub fn get(&self, key: &str) -> String {
            let pat = format!("\"{key}\":\"");
            let start = self.0.find(&pat).expect(key) + pat.len();
            let mut out = String::new();
            let mut chars = self.0[start..].chars();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => out.push(chars.next().unwrap()),
                    '"' => break,
                    c => out.push(c),
                }
            }
            out
        }
    }
}
