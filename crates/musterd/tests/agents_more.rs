//! Round 4: coarse activity status, Codex (session log), opencode (activity + CLI
//! queries), agent capabilities, daemon status notices, and dev/stable side by side.
//! Stand-in agent scripts only; no real agent CLI is run.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::*;
use serde_json::{json, Value};

fn install(env: &Env, name: &str, script: &str) {
    let bin = env.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join(name), script).unwrap();
    assert!(Command::new("chmod").arg("755").arg(bin.join(name)).status().unwrap().success());
}

async fn project(c: &mut Client, env: &Env) -> (String, String) {
    let pid = c.call("project.add", json!({"path": env.root.join("project")})).await.unwrap()["project"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let col =
        c.call("board.get", json!({"projectId": pid})).await.unwrap()["columns"][0]["id"].as_str().unwrap().to_string();
    (pid, col)
}

async fn start(c: &mut Client, pid: &str, col: &str, agent: &str) -> Value {
    c.call("card.create", json!({"projectId": pid, "columnId": col, "agentId": agent, "start": true})).await.unwrap()
        ["card"]
        .clone()
}

async fn wait_card(c: &mut Client, card: &str, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        while let Some(i) = c.events.iter().position(|e| e.m == "card.changed" && e.p["card"]["id"] == card) {
            let ev = c.events.remove(i);
            if pred(&ev.p["card"]) {
                return ev.p["card"].clone();
            }
        }
        assert!(Instant::now() < deadline, "card never became {what}");
        c.pump().await;
    }
}

async fn card_now(c: &mut Client, id: &str) -> Value {
    c.call("card.get", json!({"id": id})).await.unwrap()["card"].clone()
}

const FAKE_TERMINAL_AGENT: &str = r#"#!/bin/sh
echo "FAKE-AGENT $(basename "$0") $*"
while IFS= read -r line; do
  case "$line" in
    busy) i=0; while [ $i -lt 12 ]; do echo tick; sleep 0.25; i=$((i+1)); done ;;
    *) echo "GOT[$line]" ;;
  esac
done
"#;

#[tokio::test(flavor = "current_thread")]
async fn activity_status_and_messages_for_hookless_cards() {
    let mut env = Env::new();
    env.env.push(("MUSTER_QUIET_MS".into(), "1000".into()));
    install(&env, "gemini", FAKE_TERMINAL_AGENT);
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, col) = project(&mut c, &env).await;

    // A shell card goes idle after a quiet second, and running again on output.
    let sh = start(&mut c, &pid, &col, "shell").await;
    let sh_id = sh["id"].as_str().unwrap().to_string();
    assert_eq!(sh["status"], "running");
    wait_card(&mut c, &sh_id, "idle", |k| k["status"] == "idle").await;
    c.call("term.write", json!({"cardId": sh_id, "data": "echo hi\r"})).await.unwrap();
    let w = wait_card(&mut c, &sh_id, "running on output", |k| k["status"] == "running").await;
    assert!(w["turnStartedAt"].as_i64().is_some());
    wait_card(&mut c, &sh_id, "idle again", |k| k["status"] == "idle").await;
    // Messages are never typed into a plain shell.
    let m = c.call("message.send", json!({"cardId": sh_id, "text": "ls"})).await.unwrap()["message"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(card_now(&mut c, &sh_id).await["queuedMessages"].as_array().unwrap().len(), 1);
    c.call("message.cancel", json!({"id": m})).await.unwrap();

    // An agent without hooks: a message waits while it works and arrives at idle.
    let g = start(&mut c, &pid, &col, "gemini").await;
    let g_id = g["id"].as_str().unwrap().to_string();
    c.attach(&g_id, 160, 30).await;
    c.wait_output("FAKE-AGENT gemini").await;
    wait_card(&mut c, &g_id, "idle", |k| k["status"] == "idle").await;
    c.call("term.write", json!({"cardId": g_id, "data": "busy\r"})).await.unwrap();
    wait_card(&mut c, &g_id, "running", |k| k["status"] == "running").await;
    c.call("message.send", json!({"cardId": g_id, "text": "hello gemini"})).await.unwrap();
    assert!(!String::from_utf8_lossy(&c.output).contains("hello gemini"));
    // One bracketed paste: a fixed header line, then the text.
    c.wait_output("[Muster message from the user]").await;
    c.wait_output("hello gemini").await;
    wait_card(&mut c, &g_id, "delivered", |k| k.get("queuedMessages").is_none()).await;

    // The silence window is a setting.
    let e = c.call("settings.update", json!({"silenceSeconds": 0})).await.unwrap_err();
    assert_eq!(e.code, muster_proto::ErrorCode::Invalid);
    let s = c.call("settings.update", json!({"silenceSeconds": 7})).await.unwrap();
    assert_eq!(s["settings"]["silenceSeconds"], 7);
}

fn codex_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../muster-agents/tests/fixtures/codex/rollout-redacted.jsonl")
}

const FAKE_CODEX: &str = r##"#!/bin/sh
for a; do last=$a; done
echo "FAKE-CODEX[$last] first=$1"
D="$CODEX_HOME/sessions/2026/11/14"; mkdir -p "$D"
F="$D/rollout-2026-11-14T00-00-00-019eb02d-481e-7b70-a821-86533beda9f3.jsonl"
if [ "$1" != resume ]; then
  sed -n 1p "$FAKE_CODEX_FIXTURE" | sed "s#\"cwd\": \"[^\"]*\"#\"cwd\": \"$(pwd -P)\"#" > "$F"
fi
echo "ROLLOUT-READY"
while IFS= read -r line; do
  set -- $line
  case "$1" in
    lines) sed -n "$2,$3p" "$FAKE_CODEX_FIXTURE" >> "$F"; echo "APPENDED[$2-$3]" ;;
  esac
done
"##;

#[tokio::test(flavor = "current_thread")]
async fn codex_status_from_its_session_log() {
    let mut env = Env::new();
    env.env.push(("MUSTER_QUIET_MS".into(), "1000".into()));
    env.env.push(("FAKE_CODEX_FIXTURE".into(), codex_fixture().to_string_lossy().into_owned()));
    install(&env, "codex", FAKE_CODEX);
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, col) = project(&mut c, &env).await;
    let card = c
        .call(
            "card.create",
            json!({"projectId": pid, "columnId": col, "agentId": "codex", "prompt": "do it", "start": true}),
        )
        .await
        .unwrap()["card"]
        .clone();
    let id = card["id"].as_str().unwrap().to_string();
    assert!(card.get("agentSessionId").is_none(), "codex picks its own id");
    // The session file is found right after launch and its id stored.
    let w = wait_card(&mut c, &id, "session id", |k| k.get("agentSessionId").is_some()).await;
    let sid = "019eb02d-481e-7b70-a821-86533beda9f3";
    assert_eq!(w["agentSessionId"], sid);
    c.attach(&id, 160, 30).await;
    c.wait_output("ROLLOUT-READY").await;

    // The turn runs (log written), then ends; the quiet terminal settles it from the log.
    c.call("term.write", json!({"cardId": id, "data": "lines 2 36\r"})).await.unwrap();
    c.wait_output("APPENDED[2-36]").await;
    let u = c.wait_event_named("usage.changed").await;
    assert_eq!(u["usage"]["agentId"], "codex");
    assert_eq!(u["usage"]["windows"][0]["name"], "primary");
    assert_eq!(u["usage"]["windows"][0]["usedPercent"], 5.0);
    assert_eq!(u["usage"]["windows"][0]["windowMinutes"], 43200);
    c.call("term.write", json!({"cardId": id, "data": "lines 37 37\r"})).await.unwrap();
    let w = wait_card(&mut c, &id, "done", |k| k["status"] == "done").await;
    assert_eq!(w["unread"], true);
    assert!(card_now(&mut c, &id).await["context"]["usedTokens"].as_u64().unwrap() > 0);

    // Chat from the rollout.
    let page = c.call("chat.page", json!({"cardId": id, "limit": 20})).await.unwrap();
    assert!(!page["messages"].as_array().unwrap().is_empty(), "{page}");

    // Resume uses the stored id.
    c.call("session.kill", json!({"cardId": id})).await.unwrap();
    wait_card(&mut c, &id, "exited", |k| k["status"] == "exited").await;
    c.call("card.start", json!({"id": id})).await.unwrap();
    c.attach(&id, 160, 30).await;
    c.wait_output(&format!("FAKE-CODEX[{sid}] first=resume")).await;
}

const FAKE_OPENCODE: &str = r#"#!/bin/sh
if [ "$1" = api ]; then
  case "$3" in
    */message) cat "$FAKE_OC/messages.json" ;;
    *) cat "$FAKE_OC/session.json" ;;
  esac
  exit 0
fi
echo "FAKE-OPENCODE $*"
while IFS= read -r line; do echo "OC-GOT[$line]"; done
"#;

fn oc_session(dir: &Path, sid: &str, busy: bool) {
    let (updated, idle) = if busy { (2000, 1000) } else { (2000, 3000) };
    let j = json!({"data": {"id": sid, "time": {"updated": updated, "idle": idle},
        "tokens": {"input": 500, "cache": {"read": 20, "write": 0}}}});
    std::fs::write(dir.join("session.json"), j.to_string()).unwrap();
}

fn oc_messages(dir: &Path, extra: bool) {
    let mut msgs = vec![json!({"id": "msg_1", "type": "user", "text": "\"hello\""})];
    if extra {
        msgs.insert(0, json!({"id": "msg_2", "type": "assistant", "content": [{"type": "text", "text": "hi there"}]}));
    }
    std::fs::write(dir.join("messages.json"), json!({"data": msgs}).to_string()).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn opencode_activity_queries_and_chat() {
    let mut env = Env::new();
    let oc = env.root.join("oc");
    std::fs::create_dir_all(&oc).unwrap();
    env.env.push(("MUSTER_QUIET_MS".into(), "1000".into()));
    env.env.push(("FAKE_OC".into(), oc.to_string_lossy().into_owned()));
    install(&env, "opencode", FAKE_OPENCODE);
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, col) = project(&mut c, &env).await;

    let agents = c.call("agent.list", json!({})).await.unwrap();
    let caps =
        |id: &str| agents["agents"].as_array().unwrap().iter().find(|a| a["id"] == id).unwrap()["capabilities"].clone();
    assert_eq!(caps("claude")["statusSource"], "hooks");
    assert_eq!(caps("claude")["questions"], true);
    assert_eq!(caps("codex")["statusSource"], "log");
    assert_eq!(caps("codex")["approvals"], false);
    assert_eq!(caps("opencode")["statusSource"], "activity");
    assert_eq!(caps("opencode")["chat"], true);
    assert_eq!(caps("shell")["statusSource"], "activity");
    assert_eq!(caps("shell")["resume"], false);

    let card = start(&mut c, &pid, &col, "opencode").await;
    let id = card["id"].as_str().unwrap().to_string();
    let sid = card["agentSessionId"].as_str().expect("preset opencode session id").to_string();
    oc_session(&oc, &sid, true);
    oc_messages(&oc, false);
    c.attach(&id, 160, 30).await;
    c.wait_output(&format!("--session {sid}")).await;
    // Quiet terminal, but opencode says it is busy: still running, with token usage.
    let w = wait_card(&mut c, &id, "context from opencode", |k| k.get("context").is_some()).await;
    assert_eq!(w["status"], "running");
    assert_eq!(w["context"]["usedTokens"], 520);
    // Output, then quiet again with opencode idle: idle.
    oc_session(&oc, &sid, false);
    c.call("term.write", json!({"cardId": id, "data": "go\r"})).await.unwrap();
    wait_card(&mut c, &id, "idle", |k| k["status"] == "idle").await;

    // Chat comes from `opencode api`, and an open chat view gets new messages.
    let page = c.call("chat.page", json!({"cardId": id, "limit": 20})).await.unwrap();
    assert_eq!(page["messages"].as_array().unwrap().len(), 1);
    assert_eq!(page["messages"][0]["parts"][0]["text"], "hello");
    oc_messages(&oc, true);
    let ev = c.wait_event("chat.appended", &id).await;
    assert_eq!(ev["messages"][0]["parts"][0]["text"], "hi there");
}

#[tokio::test(flavor = "current_thread")]
async fn daemon_status_notices() {
    // Claude settings with a PermissionRequest hook of the user's own.
    let mut env = Env::new();
    install(&env, "claude", "#!/bin/sh\necho FAKE\nexec sleep 30\n");
    let cfg = env.root.join("claude-config");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(
        cfg.join("settings.json"),
        r#"{"hooks":{"PermissionRequest":[{"hooks":[{"type":"command","command":"other-tool"}]}]}}"#,
    )
    .unwrap();
    env.env.push(("MUSTER_HOOK_BIN".into(), hook_bin().to_string_lossy().into_owned()));
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let s = c.call("daemon.status", json!({})).await.unwrap();
    assert_eq!(s["hooksAvailable"], true);
    assert_eq!(s["notices"], json!([]));
    assert_eq!(s["tmuxSocket"], env.socket.as_str());
    let (pid, col) = project(&mut c, &env).await;
    start(&mut c, &pid, &col, "claude").await;
    let ev = c.wait_event_named("daemon.status").await;
    assert_eq!(ev["notices"][0]["id"], "claude_permission_hooks");
    assert!(ev["notices"][0]["message"].as_str().unwrap().contains("pre-empted"));

    // A MUSTER_HOME too deep for the hook socket: visible, not just logged.
    let mut deep = Env::new();
    deep.env.push(("MUSTER_HOOK_BIN".into(), hook_bin().to_string_lossy().into_owned()));
    deep.home = deep.root.join("x".repeat(90)).join("home");
    std::fs::create_dir_all(&deep.home).unwrap();
    let port = deep.start_daemon();
    let mut c = Client::connect(port, &deep.home).await;
    let s = c.call("daemon.status", json!({})).await.unwrap();
    assert_eq!(s["hooksAvailable"], false);
    assert_eq!(s["notices"][0]["id"], "hook_socket");
    assert!(s["notices"][0]["message"].as_str().unwrap().contains("104"));
}

#[tokio::test(flavor = "current_thread")]
async fn dev_and_stable_daemons_side_by_side() {
    let mut stable = Env::new();
    let mut dev = Env::new();
    let (p1, p2) = (stable.start_daemon(), dev.start_daemon());
    assert_ne!(p1, p2);
    assert_ne!(stable.socket, dev.socket);
    let mut a = Client::connect(p1, &stable.home).await;
    let mut b = Client::connect(p2, &dev.home).await;
    let (pa, ca) = project(&mut a, &stable).await;
    let (pb, cb) = project(&mut b, &dev).await;
    let x = start(&mut a, &pa, &ca, "shell").await["id"].as_str().unwrap().to_string();
    let y = start(&mut b, &pb, &cb, "shell").await["id"].as_str().unwrap().to_string();
    // Separate tmux servers, separate state.
    assert!(stable.tmux_sessions().contains(&x) && !stable.tmux_sessions().contains(&y));
    assert!(dev.tmux_sessions().contains(&y) && !dev.tmux_sessions().contains(&x));
    assert!(a.call("card.get", json!({"id": y})).await.is_err());
    let ta = std::fs::read_to_string(stable.home.join("token")).unwrap();
    let tb = std::fs::read_to_string(dev.home.join("token")).unwrap();
    assert_ne!(ta, tb);
    // Stopping one leaves the other running.
    stable.crash_daemon();
    b.call("project.list", json!({})).await.unwrap();
    assert!(dev.tmux_sessions().contains(&y));
}

const FAKE_CURSOR: &str = r#"#!/bin/sh
echo "FAKE-CURSOR $*"
while IFS= read -r line; do
  set -- $line
  case "$1" in
    ev) "$MUSTER_HOOK_BIN" hook "$2" < "$3"; echo "SENT[$2]" ;;
  esac
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cursor_status_from_its_hooks() {
    let mut env = Env::new();
    env.env.push(("MUSTER_HOOK_BIN".into(), hook_bin().to_string_lossy().into_owned()));
    install(&env, "cursor-agent", FAKE_CURSOR);
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../muster-agents/tests/fixtures/cursor/events-interactive.jsonl");
    let dir = env.root.join("p");
    std::fs::create_dir_all(&dir).unwrap();
    let events: Vec<(String, String)> = std::fs::read_to_string(&fixture)
        .unwrap()
        .lines()
        .enumerate()
        .map(|(i, l)| {
            let v: Value = serde_json::from_str(l).unwrap();
            let mut p = v["payload"].clone();
            // Transcript paths outside this test's own directory would be stripped anyway.
            if let Some(o) = p.as_object_mut() {
                o.remove("transcript_path");
            }
            let f = dir.join(format!("c{i}.json"));
            std::fs::write(&f, p.to_string()).unwrap();
            (v["event"].as_str().unwrap().to_string(), f.to_string_lossy().into_owned())
        })
        .collect();
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, col) = project(&mut c, &env).await;
    let card = start(&mut c, &pid, &col, "cursor").await;
    let id = card["id"].as_str().unwrap().to_string();
    assert!(card.get("agentSessionId").is_none(), "cursor picks its own id");
    c.attach(&id, 200, 30).await;
    c.wait_output("FAKE-CURSOR --plugin-dir").await;
    // Prompt, session start, a shell tool call, stop.
    for (ev, file) in &events[..8] {
        c.call("term.write", json!({"cardId": id, "data": format!("ev {ev} {file}\r")})).await.unwrap();
        c.wait_output(&format!("SENT[{ev}]")).await;
    }
    let w = wait_card(&mut c, &id, "done", |k| k["status"] == "done").await;
    assert!(w["agentSessionId"].as_str().unwrap().starts_with("c4ba7951"), "{w}");
    // Cursor's `stop` carries the token count too, but the daemon applies it after the
    // status, in a second change: wait for it instead of reading the card at once.
    let counted = |k: &Value| k["context"]["usedTokens"] == 38532;
    let k = if counted(&w) { w } else { wait_card(&mut c, &id, "counted", counted).await };
    // Cursor never asks for approvals from the board.
    assert!(k.get("pendingApproval").is_none());
}
