//! Round 5: the fixes from docs/REVIEW-daemon.md. Each test fails on the code before
//! its fix. Stand-in agents only.

mod common;

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use common::*;
use muster_proto::ErrorCode;
use serde_json::json;

/// A stand-in agent that prints its environment on request and echoes lines.
const FAKE: &str = r#"#!/bin/sh
echo "FAKE-AGENT"
while IFS= read -r line; do
  case "$line" in
    env) echo "ENV token=[$MUSTER_CARD_TOKEN] home=[$MUSTER_HOME] tmux=[$TMUX][$TMUX_PANE] file=[$MUSTER_DAEMON_FILE]" ;;
    cli*) set -- $line; shift; "$MUSTER_CLI" "$@"; echo "CLI-DONE[$?]" ;;
    *) echo "GOT[$line]" ;;
  esac
done
"#;

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

async fn start(c: &mut Client, pid: &str, col: &str, agent: &str) -> String {
    c.call("card.create", json!({"projectId": pid, "columnId": col, "agentId": agent, "start": true})).await.unwrap()
        ["card"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Text between `start` and `]` in the card's output.
fn grab(out: &[u8], start: &str) -> String {
    let s = String::from_utf8_lossy(out);
    let i = s.rfind(start).unwrap() + start.len();
    s[i..].split(']').next().unwrap().to_string()
}

fn setup(env: &mut Env) {
    env.env.push(("MUSTER_CLI_BIN".into(), cli_bin().to_string_lossy().into_owned()));
    install(env, "gemini", FAKE);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn card_tokens_are_scoped_to_their_card() {
    let mut env = Env::new();
    setup(&mut env);
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, col) = project(&mut c, &env).await;
    // The user's client reads the project's documents; they stay inside its folder.
    std::fs::write(env.root.join("project/CLAUDE.md"), "# Rules\n").unwrap();
    std::fs::write(env.root.join("secret.md"), "outside").unwrap();
    let docs = c.call("docs.list", json!({"projectId": pid})).await.unwrap();
    assert_eq!(docs["docs"][0]["path"], "CLAUDE.md");
    assert_eq!(docs["docs"][0]["kind"], "claude");
    let doc = c.call("docs.read", json!({"projectId": pid, "path": "CLAUDE.md"})).await.unwrap();
    assert_eq!(doc["content"], "# Rules\n");
    let saved = c
        .call(
            "docs.write",
            json!({"projectId": pid, "path": "CLAUDE.md", "content": "# New\n", "expectedModifiedAt": doc["modifiedAt"]}),
        )
        .await
        .unwrap();
    assert_eq!(saved["doc"]["size"], 6);
    let made = c
        .call("docs.write", json!({"projectId": pid, "path": "docs/Guide.md", "content": "# Guide\n", "create": true}))
        .await
        .unwrap();
    assert_eq!(made["doc"]["path"], "docs/Guide.md");
    let e = c
        .call("docs.write", json!({"projectId": pid, "path": "../secret.md", "content": "x", "create": true}))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    assert_eq!(std::fs::read_to_string(env.root.join("secret.md")).unwrap(), "outside");
    let e = c.call("docs.read", json!({"projectId": pid, "path": "../secret.md"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    let e = c.call("docs.list", json!({"projectId": "nope"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let a = start(&mut c, &pid, &col, "gemini").await;
    let b = start(&mut c, &pid, &col, "gemini").await;
    let x = start(&mut c, &pid, &col, "gemini").await;
    c.call("link.add", json!({"a": a, "b": b})).await.unwrap();
    c.call("card.update", json!({"id": x, "notes": "X-SECRET"})).await.unwrap();
    c.call("card.update", json!({"id": b, "notes": "B-NOTES"})).await.unwrap();

    // The session gets its own token and no master-token location; no tmux link either.
    c.attach(&a, 200, 30).await;
    c.wait_output("FAKE-AGENT").await;
    c.call("term.write", json!({"cardId": a, "data": "env\r"})).await.unwrap();
    c.wait_output("ENV token=[").await;
    c.wait_output("file=[").await;
    let token = grab(&c.output, "ENV token=[");
    assert_eq!(token.len(), 64);
    assert_eq!(grab(&c.output, "home=["), "");
    assert_eq!(grab(&c.output, "tmux=["), "", "TMUX must not reach card processes");
    assert!(!std::fs::read_to_string(env.home.join("muster.db")).unwrap_or_default().contains(&token));

    let mut k = Client::connect_token(port, &token).await.expect("card token accepted");
    // Allowed, always as card A.
    let l = k.call("context.list", json!({"cardId": x})).await.unwrap();
    assert_eq!(l["cards"].as_array().unwrap().len(), 1, "lists A's links, whatever cardId says");
    let n = k.call("context.read", json!({"cardId": b, "kind": "notes"})).await.unwrap();
    assert_eq!(n["text"], "B-NOTES");
    k.call("card.update", json!({"id": a, "notes": "mine"})).await.unwrap();
    k.call("card.move", json!({"id": a, "columnId": col})).await.unwrap();
    k.call("message.send", json!({"cardId": b, "text": "hi"})).await.unwrap();
    let board = k.call("board.get", json!({"projectId": pid})).await.unwrap();
    assert_eq!(board["cards"].as_array().unwrap().len(), 2, "only A and its linked card");
    // Notes and comments are signed as card A's agent, whatever the caller claims.
    let mine = c.call("card.get", json!({"id": a})).await.unwrap();
    assert_eq!(
        mine["card"]["notesEditedBy"],
        json!({"kind": "agent", "cardId": a, "agentId": "gemini", "cardTitle": mine["card"]["title"]})
    );
    let own = k.call("comment.add", json!({"cardId": a, "text": "on my card"})).await.unwrap();
    assert_eq!(own["comment"]["author"]["kind"], "agent");
    assert_eq!(own["comment"]["author"]["cardId"], a.as_str());
    let other = k
        .call("comment.add", json!({"cardId": b, "text": "on\u{1b}[31m a linked card", "fromCardId": x}))
        .await
        .unwrap();
    assert_eq!(other["comment"]["author"]["cardId"], a.as_str(), "fromCardId cannot be chosen");
    assert_eq!(other["comment"]["text"], "on[31m a linked card", "control characters are dropped");
    let ev = c.wait_event_named("comment.changed").await;
    assert_eq!(ev["comment"]["text"], "on my card");
    let listed = k.call("comment.list", json!({"cardId": b})).await.unwrap();
    assert_eq!(listed["comments"].as_array().unwrap().len(), 1);
    let read = k.call("context.read", json!({"cardId": b, "kind": "comments"})).await.unwrap();
    assert!(read["text"].as_str().unwrap().contains(&format!("agent gemini (card {a}")), "{read}");
    // An agent's comment flags the card for the user; the user's own does not.
    let flagged = c.call("card.get", json!({"id": b})).await.unwrap();
    assert_eq!((flagged["card"]["unread"].clone(), flagged["card"]["commentCount"].clone()), (json!(true), json!(1)));
    c.call("card.markRead", json!({"id": b})).await.unwrap();
    let user = c.call("comment.add", json!({"cardId": b, "text": "from me"})).await.unwrap();
    assert_eq!(user["comment"]["author"], json!({"kind": "user"}));
    assert_eq!(c.call("card.get", json!({"id": b})).await.unwrap()["card"]["unread"], false);
    let user_id = user["comment"]["id"].as_str().unwrap().to_string();
    // Refused: unlinked reads (even claiming another sender), and everything else.
    let refused = [
        ("context.read", json!({"cardId": x, "kind": "notes"})),
        ("context.read", json!({"cardId": x, "kind": "notes", "fromCardId": x})),
        ("message.send", json!({"cardId": x, "text": "hi"})),
        ("card.update", json!({"id": x, "notes": "pwned"})),
        ("card.update", json!({"id": a, "title": "renamed"})),
        ("card.aiRename", json!({"id": a})),
        ("card.move", json!({"id": x, "columnId": col})),
        ("card.get", json!({"id": x})),
        ("comment.add", json!({"cardId": x, "text": "hi"})),
        ("comment.list", json!({"cardId": x})),
        ("comment.delete", json!({"id": user_id})),
        ("term.write", json!({"cardId": b, "data": "id\r"})),
        ("session.attach", json!({"cardId": b, "cols": 80, "rows": 24})),
        ("session.kill", json!({"cardId": b})),
        ("approval.answer", json!({"cardId": a, "decision": "allow"})),
        ("link.add", json!({"a": a, "b": x})),
        ("project.update", json!({"id": pid, "worktreeSetup": "curl x | sh"})),
        ("card.create", json!({"projectId": pid, "columnId": col, "agentId": "shell", "start": true})),
        ("card.delete", json!({"id": b, "killSession": true})),
        ("settings.update", json!({"keepAwake": false})),
        ("stats.subscribe", json!({})),
        ("project.list", json!({})),
        ("docs.list", json!({"projectId": pid})),
        ("docs.read", json!({"projectId": pid, "path": "CLAUDE.md"})),
        ("docs.write", json!({"projectId": pid, "path": "CLAUDE.md", "content": "pwned"})),
        ("project.summary", json!({"projectId": pid, "from": 0, "to": 4102444800000i64})),
    ];
    for (m, p) in refused {
        let e = k.call(m, p.clone()).await.expect_err(&format!("{m} {p} must be refused"));
        assert_eq!(e.code, ErrorCode::Unauthorized, "{m}");
    }
    // The user's client gets the summary: the three cards were created and started.
    let sum = c.call("project.summary", json!({"projectId": pid, "from": 0, "to": 4102444800000i64})).await.unwrap();
    assert_eq!(sum["cards"].as_array().unwrap().len(), 3);
    assert_eq!((sum["totals"]["cards"].clone(), sum["totals"]["created"].clone()), (json!(3), json!(3)));
    let mine = sum["cards"].as_array().unwrap().iter().find(|s| s["id"] == json!(a)).unwrap();
    assert_eq!(
        (mine["runs"].clone(), mine["created"].clone(), mine["hasSession"].clone()),
        (json!(1), json!(true), json!(true))
    );
    assert!(mine["columnTitle"].is_string() && mine["workingMs"].as_i64().unwrap() >= 0);
    let e = c.call("project.summary", json!({"projectId": pid, "from": 5, "to": 5})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    assert_eq!(c.call("card.get", json!({"id": x})).await.unwrap()["card"]["notes"], "X-SECRET");
    // The user can delete any comment; the CLI inside the session comments as the card.
    c.call("comment.delete", json!({"id": user_id})).await.unwrap();
    assert_eq!(c.call("comment.list", json!({"cardId": b})).await.unwrap()["comments"].as_array().unwrap().len(), 1);
    c.call("term.write", json!({"cardId": a, "data": "cli comment add from the cli\r"})).await.unwrap();
    c.wait_output("CLI-DONE[0]").await;
    let mine = c.call("comment.list", json!({"cardId": a})).await.unwrap();
    let last = mine["comments"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((last["text"].clone(), last["author"]["cardId"].clone()), (json!("from the cli"), json!(a)));
    // The CLI inside the session uses the card token: --any does not help.
    c.call("term.write", json!({"cardId": a, "data": format!("cli context read {x} --notes --any\r")})).await.unwrap();
    c.wait_output("CLI-DONE[1]").await;
    assert!(!String::from_utf8_lossy(&c.output).contains("X-SECRET"));
    // The token dies with the session, also on a connection opened before.
    c.call("session.kill", json!({"cardId": a})).await.unwrap();
    c.wait_event("session.exit", &a).await;
    let e = k.call("context.list", json!({"cardId": a})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Unauthorized);
    drop(k);
    let e = Client::connect_token(port, &token).await.err().expect("revoked");
    assert_eq!(e.code, ErrorCode::Unauthorized);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn message_limits() {
    let mut env = Env::new();
    setup(&mut env);
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, col) = project(&mut c, &env).await;
    let a = start(&mut c, &pid, &col, "gemini").await;
    let long = "x".repeat(8001);
    let e = c.call("message.send", json!({"cardId": a, "text": long})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    let e = c.call("message.send", json!({"cardId": a, "text": "\u{1b}\r\u{7}"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid, "only control characters is empty");
    for i in 0..10 {
        c.call("message.send", json!({"cardId": a, "text": format!("m{i}")})).await.unwrap();
    }
    let e = c.call("message.send", json!({"cardId": a, "text": "one too many"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Unavailable);
    // Handoffs start agent sessions: limited per card.
    for _ in 0..3 {
        c.call("card.handoff", json!({"id": a})).await.unwrap();
    }
    let e = c.call("card.handoff", json!({"id": a})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Unavailable);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forged_hook_events_are_dropped_and_reported() {
    let mut env = Env::new();
    env.env.push(("MUSTER_HOOK_BIN".into(), hook_bin().to_string_lossy().into_owned()));
    install(&env, "claude", "#!/bin/sh\necho FAKE\nexec sleep 60\n");
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, col) = project(&mut c, &env).await;
    let a = start(&mut c, &pid, &col, "claude").await;
    // Another process posts an event for card A without A's secret.
    let mut s = UnixStream::connect(env.home.join("hook.sock")).unwrap();
    let env_line = json!({"v": 1, "kind": "hook", "event": "Stop", "cardId": a, "agentId": "claude",
        "secret": "0".repeat(64), "payload": {"hook_event_name": "Stop", "session_id": "x",
        "transcript_path": "/etc/hosts"}});
    use std::io::Write;
    s.write_all(format!("{env_line}\n").as_bytes()).unwrap();
    drop(s);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let st = c.call("daemon.status", json!({})).await.unwrap();
        if st["notices"].as_array().unwrap().iter().any(|n| n["id"] == "hook_events_dropped") {
            break;
        }
        assert!(Instant::now() < deadline, "no hook_events_dropped notice: {st}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let card = c.call("card.get", json!({"id": a})).await.unwrap()["card"].clone();
    assert_ne!(card["status"], "done", "the forged Stop was not applied");
    let r = c.call("context.read", json!({"cardId": a, "kind": "transcript"})).await.unwrap();
    assert_eq!(r["text"], "(no transcript)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn large_input_without_a_client_and_card_files_removed() {
    let mut env = Env::new();
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, col) = project(&mut c, &env).await;
    let sh = start(&mut c, &pid, &col, "shell").await;
    let out = env.root.join("project/big.txt");
    c.call("term.write", json!({"cardId": sh, "data": "stty -echo; cat > big.txt\r"})).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let line = format!("{}\n", "y".repeat(63));
    let data = line.repeat(5000); // 320 KB, one tmux argument per byte before the fix
    c.call("term.write", json!({"cardId": sh, "data": data})).await.expect("large write with no client");
    c.call("term.write", json!({"cardId": sh, "data": "\u{4}"})).await.unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) < 320_000 {
        assert!(Instant::now() < deadline, "only {:?} bytes arrived", std::fs::metadata(&out).map(|m| m.len()));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Files kept per card disappear with the card.
    let exit = env.home.join(format!("exit/m-{sh}"));
    let setup = env.home.join(format!("setup/{sh}.run.sh"));
    std::fs::create_dir_all(setup.parent().unwrap()).unwrap();
    std::fs::write(&exit, "0:").unwrap();
    std::fs::write(&setup, "#").unwrap();
    c.call("session.detach", json!({"cardId": sh})).await.ok();
    c.call("card.delete", json!({"id": sh, "killSession": true})).await.unwrap();
    assert!(!exit.exists() && !setup.exists());
    assert!(!env.home.join(format!("scrollback/{sh}.ans")).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clipboard_off_by_default_and_hello_lockout() {
    let mut env = Env::new();
    env.env.push(("MUSTER_HELLO_WINDOW_MS".into(), "1500".into()));
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, col) = project(&mut c, &env).await;
    start(&mut c, &pid, &col, "shell").await;
    let show = || {
        let o = Command::new("tmux").args(["-L", &env.socket, "show-options", "-g", "set-clipboard"]).output().unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    assert_eq!(show(), "set-clipboard off");
    c.call("settings.update", json!({"clipboardPassthrough": true})).await.unwrap();
    assert_eq!(show(), "set-clipboard external");

    // Ten bad hellos slow down further guesses, but never lock out a valid token
    // (otherwise anyone could keep the app out by sending bad tokens).
    for _ in 0..10 {
        assert!(Client::connect_token(port, "wrong").await.is_err());
    }
    let e = Client::connect_token(port, "wrong").await.err().unwrap();
    assert!(e.msg.contains("too many"), "{}", e.msg);
    let token = std::fs::read_to_string(env.home.join("token")).unwrap();
    Client::connect_token(port, token.trim()).await.expect("a valid token is never locked out");
    tokio::time::sleep(Duration::from_millis(1700)).await;
    let e = Client::connect_token(port, "wrong").await.err().unwrap();
    assert_eq!(e.msg, "bad token", "the window has passed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_stays_up_without_tmux_and_log_rotates() {
    let mut env = Env::new();
    env.env.push(("MUSTER_TMUX".into(), "/nonexistent/tmux".into()));
    env.env.push(("MUSTER_LOG_MAX_BYTES".into(), "100000".into()));
    let log = env.root.join("musterd.log");
    std::fs::write(&log, "x".repeat(200_000)).unwrap();
    env.log_file = Some(log.clone());
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let st = c.call("daemon.status", json!({})).await.unwrap();
    assert!(st["notices"].as_array().unwrap().iter().any(|n| n["id"] == "tmux_missing"), "{st}");
    let (pid, col) = project(&mut c, &env).await;
    let e = c
        .call("card.create", json!({"projectId": pid, "columnId": col, "agentId": "shell", "start": true}))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Unavailable);
    // The oversized log was rotated at startup.
    let mut old = log.clone().into_os_string();
    old.push(".1");
    assert!(Path::new(&old).exists());
    assert!(std::fs::metadata(&log).unwrap().len() < 100_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_huge_transcript_does_not_stall_terminals() {
    let mut env = Env::new();
    env.env.push(("MUSTER_HOOK_BIN".into(), hook_bin().to_string_lossy().into_owned()));
    // A transcript whose newest 30 MB are lines the chat reader must scan and skip.
    let dir = env.root.join("claude-config/projects/big");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.jsonl");
    {
        use std::io::Write;
        let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        writeln!(f, r#"{{"type":"user","uuid":"u1","message":{{"role":"user","content":"hello"}}}}"#).unwrap();
        let junk = format!(r#"{{"type":"attachment","data":"{}"}}"#, "z".repeat(4000));
        for _ in 0..7500 {
            writeln!(f, "{junk}").unwrap();
        }
    }
    install(&env, "claude", "#!/bin/sh\necho FAKE\nexec sleep 120\n");
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, col) = project(&mut c, &env).await;
    let card = start(&mut c, &pid, &col, "claude").await;
    let sh = start(&mut c, &pid, &col, "shell").await;
    // Point the card at the transcript the way a hook would (through its persisted state).
    let sid =
        c.call("card.get", json!({"id": card})).await.unwrap()["card"]["agentSessionId"].as_str().unwrap().to_string();
    let named = dir.join(format!("{sid}.jsonl"));
    std::fs::rename(&path, &named).unwrap();

    // Client B streams a busy terminal.
    let mut b = Client::connect(port, &env.home).await;
    b.attach(&sh, 100, 30).await;
    b.call("term.write", json!({"cardId": sh, "data": "while :; do echo tick; sleep 0.02; done\r"})).await.unwrap();
    b.wait_output("tick").await;

    // Client A asks for the chat page (a long scan) while B's output is timed.
    let mut a = Client::connect(port, &env.home).await;
    let started = Instant::now();
    let id = a.send("chat.page", json!({"cardId": card, "limit": 50})).await;
    let mut gaps = Duration::ZERO;
    let mut last = Instant::now();
    let mut done = false;
    while !done {
        tokio::select! {
            r = a.pump() => if let Some(r) = r { assert_eq!(r.id, id); done = true; },
            _ = b.pump() => { let now = Instant::now(); gaps = gaps.max(now - last); last = now; }
        }
    }
    let took = started.elapsed();
    eprintln!("chat.page took {took:?}; longest gap in terminal output {gaps:?}");
    assert!(gaps < Duration::from_millis(500), "terminal output stalled for {gaps:?} (chat.page took {took:?})");
}
