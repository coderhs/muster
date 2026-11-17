//! Claude Code integration through the real `muster-hook` binary.
//!
//! A stand-in `claude` script on PATH parses the arguments the adapter passes
//! (`--session-id`, `--resume`, `--settings`, `-- <prompt>`), finds the hook binary in
//! the generated settings file and, on command lines typed into its terminal, feeds
//! captured hook payloads (from `crates/muster-agents/tests/fixtures`) through
//! `muster-hook`, exactly as Claude Code would. The real `claude` CLI is never run.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use common::*;
use muster_proto::ErrorCode;
use serde_json::{json, Value};

const FAKE_CLAUDE: &str = r#"#!/bin/sh
SID=; RESUMED=no; SETTINGS=; PROMPT=
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) SID="$2"; shift ;;
    --resume) SID="$2"; RESUMED=yes; shift ;;
    --settings) SETTINGS="$2"; shift ;;
    --append-system-prompt-file) INSTRUCTIONS="$2"; shift ;;
    --plugin-dir) PLUGIN="$2"; shift ;;
    --) shift; PROMPT="$*"; break ;;
  esac
  shift
done
HOOK=$(sed -n 's/^ *"command": *"\(.*muster-hook\)",* *$/\1/p' "$SETTINGS" | head -1)
echo "FAKE-CLAUDE sid=$SID resumed=$RESUMED prompt=$PROMPT hook=${HOOK:+found}"
[ -f "$INSTRUCTIONS" ] && grep -q MUSTER_CARD_ID "$INSTRUCTIONS" && echo "INSTRUCTIONS-OK"
[ -f "$PLUGIN/.claude-plugin/plugin.json" ] && [ -f "$PLUGIN/skills/muster/SKILL.md" ] && echo "SKILL-OK"
while IFS= read -r line; do
  set -- $line
  case "$1" in
    ev) "$HOOK" hook "$2" < "$3"; echo "SENT[$2]" ;;
    wait) out=$("$HOOK" hook "$2" --wait "$4" < "$3"); echo "DECISION[$out]" ;;
    sl) "$HOOK" statusline < "$2" > /dev/null; echo "SENT[statusline]" ;;
    cli) shift; "$MUSTER_CLI" "$@"; echo "CLI-DONE[$?]" ;;
    /rename)
      echo "$line" >> "$CLAUDE_CONFIG_DIR/typed.log"; sleep 1
      echo '{"type":"custom-title","customTitle":"tidy-the-parser","sessionId":"s"}' >> "$CLAUDE_CONFIG_DIR/projects/muster-test/t.jsonl"
      echo "RENAMED" ;;
    quit) exit 0 ;;
  esac
done
"#;

/// Transcripts must live under Claude's config dir (`$CLAUDE_CONFIG_DIR/projects`): the
/// daemon ignores transcript paths from hooks that point anywhere else.
fn transcript_dir(env: &Env) -> PathBuf {
    let d = env.root.join("claude-config/projects/muster-test");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../muster-agents/tests/fixtures/claude")
}

/// Payloads of the captured interactive run, one per line of the fixture.
fn fixture_payloads() -> Vec<Value> {
    std::fs::read_to_string(fixtures().join("events-interactive.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap()["payload"].clone())
        .collect()
}

struct Scenario {
    dir: PathBuf,
    payloads: Vec<Value>,
    transcript: PathBuf,
    cwd: PathBuf,
}

impl Scenario {
    /// Writes payload `i` with the card's session id and our transcript copy (events
    /// after `/clear` keep their own new session id) and returns its path.
    fn payload(&self, i: usize, sid: &str) -> String {
        let mut p = self.payloads[i].clone();
        let original = "bbfbc0e6-2aee-4a10-af13-d75b744ca2ff";
        if p["session_id"] == original {
            p["session_id"] = json!(sid);
            p["transcript_path"] = json!(self.transcript);
        } else {
            p["transcript_path"] = json!(self.dir.join("cleared.jsonl"));
        }
        p["cwd"] = json!(self.cwd);
        let path = self.dir.join(format!("p{i}.json"));
        std::fs::write(&path, serde_json::to_string(&p).unwrap()).unwrap();
        path.to_string_lossy().into_owned()
    }
}

async fn card_of(c: &mut Client, project: &str, card: &str) -> Value {
    let b = c.call("board.get", json!({"projectId": project})).await.unwrap();
    b["cards"].as_array().unwrap().iter().find(|x| x["id"] == card).unwrap().clone()
}

/// Waits until the card (via `card.changed` events) satisfies `pred`.
async fn wait_card(c: &mut Client, card: &str, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + TIMEOUT;
    let mut last = Value::Null;
    loop {
        while let Some(i) = c.events.iter().position(|e| e.m == "card.changed" && e.p["card"]["id"] == card) {
            let ev = c.events.remove(i);
            if pred(&ev.p["card"]) {
                return ev.p["card"].clone();
            }
            last = ev.p["card"].clone();
        }
        assert!(
            Instant::now() < deadline,
            "card never became {what}; last seen {last}; terminal: {}",
            String::from_utf8_lossy(&c.output)
                .chars()
                .rev()
                .take(600)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        );
        c.pump().await;
    }
}

async fn type_line(c: &mut Client, card: &str, line: &str) {
    c.call("term.write", json!({"cardId": card, "data": format!("{line}\r")})).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn claude_status_approvals_chat_usage_and_restart() {
    let mut env = Env::new();
    env.env = vec![
        ("MUSTER_HOOK_BIN".into(), hook_bin().to_string_lossy().into_owned()),
        ("MUSTER_SESSION_START_TIMEOUT_MS".into(), "1500".into()),
        ("MUSTER_SETTLE_MS".into(), "300".into()),
        // The quiet-terminal check is covered by its own test; keep it out of the way.
        ("MUSTER_QUIET_MS".into(), "600000".into()),
        ("CLAUDE_CONFIG_DIR".into(), env.root.join("claude-config").to_string_lossy().into_owned()),
    ];
    let bin = env.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("claude"), FAKE_CLAUDE).unwrap();
    assert!(Command::new("chmod").arg("755").arg(bin.join("claude")).status().unwrap().success());
    let dir = env.root.join("p");
    std::fs::create_dir_all(&dir).unwrap();

    // The transcript starts with the first turn; the rest is appended later.
    let full = std::fs::read_to_string(fixtures().join("transcript-basic.jsonl")).unwrap();
    let lines: Vec<&str> = full.lines().collect();
    let transcript = transcript_dir(&env).join("transcript.jsonl");
    std::fs::write(&transcript, lines[..45].join("\n") + "\n").unwrap();
    let sc = Scenario {
        dir: dir.clone(),
        payloads: fixture_payloads(),
        transcript: transcript.clone(),
        cwd: env.root.join("project"),
    };

    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let pid = c.call("project.add", json!({"path": env.root.join("project")})).await.unwrap()["project"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let board = c.call("board.get", json!({"projectId": pid})).await.unwrap();
    let col = board["columns"][0]["id"].as_str().unwrap().to_string();

    // Launch: Muster picks the session id and passes hooks through --settings.
    let card = c
        .call(
            "card.create",
            json!({"projectId": pid, "columnId": col, "agentId": "claude", "prompt": "do it", "start": true}),
        )
        .await
        .unwrap()["card"]
        .clone();
    let id = card["id"].as_str().unwrap().to_string();
    let sid = card["agentSessionId"].as_str().expect("session id chosen at launch").to_string();
    assert_eq!(sid.len(), 36);
    assert_eq!(card["status"], "running");
    c.attach(&id, 120, 30).await;
    c.wait_output(&format!("FAKE-CLAUDE sid={sid} resumed=no prompt=do it hook=found")).await;

    // (a) No SessionStart within the window: waiting at a startup prompt.
    let w = wait_card(&mut c, &id, "needs_you/startup_prompt", |k| k["status"] == "needs_you").await;
    assert_eq!(w["needsYouReason"], "startup_prompt");
    type_line(&mut c, &id, &format!("ev SessionStart {}", sc.payload(0, &sid))).await;
    let w = wait_card(&mut c, &id, "idle", |k| k["status"] == "idle").await;
    assert!(w.get("needsYouReason").is_none());

    // Usage from the status line feed: rate limits and the context meter.
    type_line(&mut c, &id, &format!("sl {}", sc.payload(5, &sid))).await;
    let u = c.wait_event_named("usage.changed").await;
    assert_eq!(u["usage"]["fiveHour"]["usedPercent"], 25.0);
    assert_eq!(u["usage"]["fiveHour"]["resetsAt"], 1_794_591_600_000i64);
    assert_eq!(u["usage"]["sevenDay"]["usedPercent"], 12.0);
    let w = wait_card(&mut c, &id, "with context", |k| k.get("context").is_some()).await;
    assert_eq!(w["context"]["windowTokens"], 1_000_000);
    let g = c.call("usage.get", json!({})).await.unwrap();
    assert_eq!(g["usage"]["sevenDay"]["usedPercent"], 12.0);

    // A turn: running, then done (unread), then read.
    type_line(&mut c, &id, &format!("ev UserPromptSubmit {}", sc.payload(3, &sid))).await;
    let w = wait_card(&mut c, &id, "running", |k| k["status"] == "running").await;
    assert!(w["turnStartedAt"].as_i64().is_some());

    // The chat view: newest page, then live appends as the transcript grows.
    let page = c.call("chat.page", json!({"cardId": id, "limit": 50})).await.unwrap();
    let msgs = page["messages"].as_array().unwrap();
    assert!(!msgs.is_empty(), "{page}");
    assert_eq!(msgs[0]["role"], "user");
    assert!(msgs.iter().any(|m| m["role"] == "assistant"));
    let small = c.call("chat.page", json!({"cardId": id, "limit": 1})).await.unwrap();
    assert_eq!(small["messages"].as_array().unwrap().len(), 1);
    let older = small["older"].as_u64().expect("an older page exists");
    let prev = c.call("chat.page", json!({"cardId": id, "before": older, "limit": 50})).await.unwrap();
    assert!(!prev["messages"].as_array().unwrap().is_empty());
    std::fs::write(&transcript, full.clone()).unwrap();
    type_line(&mut c, &id, &format!("ev Stop {}", sc.payload(4, &sid))).await;
    let w = wait_card(&mut c, &id, "done", |k| k["status"] == "done").await;
    assert_eq!(w["unread"], true);
    // The reply the turn ended with, for notifications.
    let said = sc.payloads[4]["last_assistant_message"].as_str().unwrap().trim().to_string();
    assert!(!said.is_empty());
    assert_eq!(w["lastMessage"], said.as_str());
    let appended = c.wait_event("chat.appended", &id).await;
    assert!(!appended["messages"].as_array().unwrap().is_empty());
    let r = c.call("card.markRead", json!({"id": id})).await.unwrap();
    assert_eq!(r["card"]["status"], "idle");
    assert_eq!(r["card"]["unread"], false);
    assert_eq!(r["card"]["lastMessage"], said.as_str(), "kept after the turn");

    // A permission request answered "allow" from the board, through the hook.
    type_line(&mut c, &id, &format!("ev UserPromptSubmit {}", sc.payload(6, &sid))).await;
    type_line(&mut c, &id, &format!("ev PreToolUse {}", sc.payload(7, &sid))).await;
    type_line(&mut c, &id, &format!("wait PermissionRequest {} 30", sc.payload(8, &sid))).await;
    let w = wait_card(&mut c, &id, "needs_you/permission", |k| k["status"] == "needs_you").await;
    assert_eq!(w["needsYouReason"], "permission");
    let pa = &w["pendingApproval"];
    assert_eq!(pa["tool"], "Bash");
    assert!(pa["summary"].as_str().unwrap().contains("touch muster-perm-test.txt"));
    let rid = pa["requestId"].as_str().expect("answerable through the hook").to_string();
    let r = c.call("approval.answer", json!({"cardId": id, "requestId": rid, "decision": "allow"})).await.unwrap();
    assert_eq!(r["via"], "hook");
    c.wait_output(r#""behavior":"allow""#).await;
    let w = wait_card(&mut c, &id, "running", |k| k["status"] == "running").await;
    assert!(w.get("pendingApproval").is_none());
    type_line(&mut c, &id, &format!("ev PostToolUse {}", sc.payload(11, &sid))).await;
    type_line(&mut c, &id, &format!("ev Stop {}", sc.payload(13, &sid))).await;
    wait_card(&mut c, &id, "done", |k| k["status"] == "done").await;

    // ... and one answered "deny" with a message.
    type_line(&mut c, &id, &format!("ev UserPromptSubmit {}", sc.payload(16, &sid))).await;
    type_line(&mut c, &id, &format!("wait PermissionRequest {} 30", sc.payload(18, &sid))).await;
    let w = wait_card(&mut c, &id, "needs_you", |k| k["status"] == "needs_you").await;
    let rid = w["pendingApproval"]["requestId"].as_str().unwrap().to_string();
    let r = c
        .call("approval.answer", json!({"cardId": id, "requestId": rid, "decision": "deny", "message": "not now"}))
        .await
        .unwrap();
    assert_eq!(r["via"], "hook");
    c.wait_output(r#""behavior":"deny","message":"not now""#).await;
    wait_card(&mut c, &id, "running", |k| k["status"] == "running").await;
    // A request id that is no longer pending is answered in the terminal only when
    // the card waits; it does not now.
    let e = c.call("approval.answer", json!({"cardId": id, "requestId": rid, "decision": "allow"})).await.unwrap_err();
    assert_eq!(e.code, muster_proto::ErrorCode::Conflict);

    // (b) The prompt is settled in the terminal (Escape) with no Stop: the hook goes away
    // without our answer, and the transcript shows the interrupted turn.
    let cut = lines.iter().position(|l| l.contains("[Request interrupted by user")).unwrap();
    std::fs::write(&transcript, lines[..=cut + 1].join("\n") + "\n").unwrap();
    type_line(&mut c, &id, &format!("wait PermissionRequest {} 1", sc.payload(18, &sid))).await;
    let w = wait_card(&mut c, &id, "needs_you", |k| k["status"] == "needs_you").await;
    assert!(w["pendingApproval"]["requestId"].is_string());
    c.wait_output("DECISION[]").await; // the hook's wait ran out; it printed nothing
    let w = wait_card(&mut c, &id, "idle after the interrupt", |k| k["status"] == "idle").await;
    assert!(w.get("pendingApproval").is_none());

    // The terminal-key fallback: with no answerable request, "allow" types the key.
    type_line(&mut c, &id, &format!("ev UserPromptSubmit {}", sc.payload(16, &sid))).await;
    let p = lines.iter().position(|l| l.contains("touch muster-perm-test.txt ; then")).unwrap();
    std::fs::write(&transcript, lines[..=p].join("\n") + "\n").unwrap(); // turn in progress
    c.output.clear();
    type_line(&mut c, &id, &format!("wait PermissionRequest {} 1", sc.payload(18, &sid))).await;
    wait_card(&mut c, &id, "needs_you", |k| k["status"] == "needs_you").await;
    c.wait_output("DECISION[]").await;
    let r = c.call("approval.answer", json!({"cardId": id, "decision": "allow"})).await.unwrap();
    assert_eq!(r["via"], "keys");
    c.wait_output("1").await; // echoed by the terminal
    c.call("term.write", json!({"cardId": id, "data": "\u{15}"})).await.unwrap(); // clear the line
    std::fs::write(&transcript, full.clone()).unwrap();
    type_line(&mut c, &id, &format!("ev Stop {}", sc.payload(13, &sid))).await;
    wait_card(&mut c, &id, "done", |k| k["status"] == "done").await;

    // /clear: the agent continues under a new session id, which the card stores.
    type_line(&mut c, &id, &format!("ev SessionEnd {}", sc.payload(21, &sid))).await;
    type_line(&mut c, &id, &format!("ev SessionStart {}", sc.payload(22, &sid))).await;
    let w = wait_card(&mut c, &id, "with the new session id", |k| k["agentSessionId"] != sid.as_str()).await;
    let new_sid = w["agentSessionId"].as_str().unwrap().to_string();
    assert!(new_sid.starts_with("5c21a797"));
    assert_eq!(w["status"], "idle");
    type_line(&mut c, &id, &format!("ev UserPromptSubmit {}", sc.payload(3, &new_sid))).await;
    let before = wait_card(&mut c, &id, "running", |k| k["status"] == "running").await;

    // Restart: the status and its timestamps come back; hooks keep working.
    env.crash_daemon();
    drop(c);
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let k = card_of(&mut c, &pid, &id).await;
    assert_eq!(k["status"], "running");
    assert_eq!(k["statusSince"], before["statusSince"]);
    assert_eq!(k["turnStartedAt"], before["turnStartedAt"]);
    assert_eq!(k["agentSessionId"], new_sid.as_str());
    assert_eq!(k["hasSession"], true);
    let g = c.call("usage.get", json!({})).await.unwrap();
    assert_eq!(g["usage"]["fiveHour"]["usedPercent"], 25.0);
    c.attach(&id, 120, 30).await;
    type_line(&mut c, &id, &format!("ev Stop {}", sc.payload(4, &new_sid))).await;
    wait_card(&mut c, &id, "done after restart", |k| k["status"] == "done").await;

    // card.start on a card with a stored session id resumes it.
    c.call("session.kill", json!({"cardId": id})).await.unwrap();
    wait_card(&mut c, &id, "exited", |k| k["status"] == "exited").await;
    let r = c.call("card.start", json!({"id": id})).await.unwrap();
    assert_eq!(r["card"]["agentSessionId"], new_sid.as_str());
    c.attach(&id, 120, 30).await;
    c.wait_output(&format!("FAKE-CLAUDE sid={new_sid} resumed=yes prompt= hook=found")).await;

    // Nothing in the user's real Claude config was touched: generated settings live
    // under MUSTER_HOME.
    let settings = std::fs::read_dir(env.home.join("agents/claude")).unwrap().count();
    assert!(settings >= 1);
}

#[tokio::test(flavor = "current_thread")]
async fn quiet_terminal_settles_turn_and_shell_activity() {
    let mut env = Env::new();
    env.env = vec![
        ("MUSTER_HOOK_BIN".into(), hook_bin().to_string_lossy().into_owned()),
        ("MUSTER_QUIET_MS".into(), "500".into()),
        ("CLAUDE_CONFIG_DIR".into(), env.root.join("claude-config").to_string_lossy().into_owned()),
    ];
    let bin = env.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("claude"), FAKE_CLAUDE).unwrap();
    assert!(Command::new("chmod").arg("755").arg(bin.join("claude")).status().unwrap().success());
    let dir = env.root.join("p");
    std::fs::create_dir_all(&dir).unwrap();
    // A transcript whose last turn has ended, but no Stop will be sent.
    let transcript = transcript_dir(&env).join("transcript.jsonl");
    std::fs::copy(fixtures().join("transcript-basic.jsonl"), &transcript).unwrap();
    let sc = Scenario { dir: dir.clone(), payloads: fixture_payloads(), transcript, cwd: env.root.join("project") };

    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let pid = c.call("project.add", json!({"path": env.root.join("project")})).await.unwrap()["project"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let col =
        c.call("board.get", json!({"projectId": pid})).await.unwrap()["columns"][0]["id"].as_str().unwrap().to_string();
    let card = c
        .call("card.create", json!({"projectId": pid, "columnId": col, "agentId": "claude", "start": true}))
        .await
        .unwrap()["card"]
        .clone();
    let id = card["id"].as_str().unwrap().to_string();
    let sid = card["agentSessionId"].as_str().unwrap().to_string();
    c.attach(&id, 100, 30).await;
    c.wait_output("hook=found").await;
    type_line(&mut c, &id, &format!("ev SessionStart {}", sc.payload(0, &sid))).await;
    wait_card(&mut c, &id, "idle", |k| k["status"] == "idle").await;
    type_line(&mut c, &id, &format!("ev UserPromptSubmit {}", sc.payload(3, &sid))).await;
    wait_card(&mut c, &id, "running", |k| k["status"] == "running").await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await; // past the notice throttle
    c.call("term.write", json!({"cardId": id, "data": "some output"})).await.unwrap();
    // Output, then silence: the transcript says the turn ended.
    wait_card(&mut c, &id, "done after the terminal went quiet", |k| k["status"] == "done").await;

    // Shell cards: lastActivityAt follows terminal output, while attached...
    let sh = c
        .call("card.create", json!({"projectId": pid, "columnId": col, "agentId": "shell", "start": true}))
        .await
        .unwrap()["card"]
        .clone();
    let sh_id = sh["id"].as_str().unwrap().to_string();
    assert!(sh.get("lastActivityAt").is_none());
    c.attach(&sh_id, 100, 30).await;
    let w = wait_card(&mut c, &sh_id, "active", |k| k.get("lastActivityAt").is_some()).await;
    assert!(w["lastActivityAt"].as_i64().unwrap() > 0);
    // ... and from tmux's record when nobody is attached.
    let sh2 = c
        .call("card.create", json!({"projectId": pid, "columnId": col, "agentId": "shell", "start": true}))
        .await
        .unwrap()["card"]
        .clone();
    let sh2_id = sh2["id"].as_str().unwrap().to_string();
    c.call("term.write", json!({"cardId": sh2_id, "data": "echo hi\r"})).await.unwrap();
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let k = card_of(&mut c, &pid, &sh2_id).await;
        if k["lastActivityAt"].as_i64().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "no lastActivityAt: {k}");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

fn fake_claude_env(env: &mut Env) {
    env.env = vec![
        ("CODEX_HOME".into(), env.root.join("codex-home").to_string_lossy().into_owned()),
        ("MUSTER_CLI_BIN".into(), cli_bin().to_string_lossy().into_owned()),
        ("MUSTER_HOOK_BIN".into(), hook_bin().to_string_lossy().into_owned()),
        ("MUSTER_QUIET_MS".into(), "600000".into()),
        ("CLAUDE_CONFIG_DIR".into(), env.root.join("claude-config").to_string_lossy().into_owned()),
    ];
    let bin = env.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("claude"), FAKE_CLAUDE).unwrap();
    assert!(Command::new("chmod").arg("755").arg(bin.join("claude")).status().unwrap().success());
}

#[tokio::test(flavor = "current_thread")]
async fn automatic_titles() {
    let mut env = Env::new();
    fake_claude_env(&mut env);
    let dir = env.root.join("p");
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = transcript_dir(&env).join("t.jsonl");
    std::fs::write(&transcript, "").unwrap();
    let sc = Scenario { dir, payloads: fixture_payloads(), transcript, cwd: env.root.join("project") };
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let pid = c.call("project.add", json!({"path": env.root.join("project")})).await.unwrap()["project"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let col =
        c.call("board.get", json!({"projectId": pid})).await.unwrap()["columns"][0]["id"].as_str().unwrap().to_string();

    // Shell card without a title: the cwd's name, maintained by the daemon.
    let sh = c.call("card.create", json!({"projectId": pid, "columnId": col, "agentId": "shell"})).await.unwrap()
        ["card"]
        .clone();
    assert_eq!((sh["title"].as_str(), sh["titleAuto"].as_bool()), (Some("project"), Some(true)));
    let sh_id = sh["id"].as_str().unwrap();
    let u = c.call("card.update", json!({"id": sh_id, "title": "Mine"})).await.unwrap()["card"].clone();
    assert_eq!((u["title"].as_str(), u["titleAuto"].as_bool()), (Some("Mine"), Some(false)));
    let u = c.call("card.update", json!({"id": sh_id, "titleAuto": true})).await.unwrap()["card"].clone();
    assert_eq!((u["title"].as_str(), u["titleAuto"].as_bool()), (Some("Mine"), Some(true)));
    // A typed title is never automatic.
    let t = c
        .call("card.create", json!({"projectId": pid, "columnId": col, "agentId": "shell", "title": "Typed"}))
        .await
        .unwrap()["card"]
        .clone();
    assert_eq!(t["titleAuto"], false);

    // Agent card started with no prompt: placeholder, then the first prompt, then the
    // agent's own session name.
    let card = c
        .call("card.create", json!({"projectId": pid, "columnId": col, "agentId": "claude", "start": true}))
        .await
        .unwrap()["card"]
        .clone();
    assert_eq!((card["title"].as_str(), card["titleAuto"].as_bool()), (Some("Claude Code"), Some(true)));
    let id = card["id"].as_str().unwrap().to_string();
    let sid = card["agentSessionId"].as_str().unwrap().to_string();
    c.attach(&id, 100, 30).await;
    c.wait_output("hook=found").await;
    type_line(&mut c, &id, &format!("ev SessionStart {}", sc.payload(0, &sid))).await;
    type_line(&mut c, &id, &format!("ev UserPromptSubmit {}", sc.payload(3, &sid))).await;
    wait_card(&mut c, &id, "titled from the prompt", |k| k["title"] == "reply with the single word ok").await;
    // A later prompt does not retitle.
    type_line(&mut c, &id, &format!("ev UserPromptSubmit {}", sc.payload(6, &sid))).await;
    type_line(&mut c, &id, &format!("sl {}", sc.payload(5, &sid))).await;
    let w = wait_card(&mut c, &id, "titled by the agent", |k| k["title"] == "Single word reply").await;
    assert_eq!(w["titleAuto"], true);
    // Once the user names it, the agent's name no longer applies.
    c.call("card.update", json!({"id": id, "title": "My task"})).await.unwrap();
    type_line(&mut c, &id, &format!("sl {}", sc.payload(9, &sid))).await;
    c.wait_output("SENT[statusline]").await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(card_of(&mut c, &pid, &id).await["title"], "My task");
}

#[tokio::test(flavor = "current_thread")]
async fn rename_with_ai() {
    let mut env = Env::new();
    fake_claude_env(&mut env);
    let dir = env.root.join("p");
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = transcript_dir(&env).join("t.jsonl");
    std::fs::write(&transcript, "{\"type\":\"custom-title\",\"customTitle\":\"an-older-name\"}\n").unwrap();
    let sc = Scenario { dir, payloads: fixture_payloads(), transcript, cwd: env.root.join("project") };
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, cols) = setup_project(&mut c, &env).await;

    let agents = c.call("agent.list", json!({})).await.unwrap();
    let can = |id: &str| {
        agents["agents"].as_array().unwrap().iter().find(|a| a["id"] == id).unwrap()["capabilities"]["aiRename"].clone()
    };
    assert_eq!((can("claude"), can("codex"), can("shell")), (json!(true), json!(false), json!(false)));

    // Never typed into a shell, nor into a card without a session.
    let sh = new_card(&mut c, &pid, &cols[0], "shell").await;
    let e = c.call("card.aiRename", json!({"id": sh["id"]})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    let draft =
        c.call("card.create", json!({"projectId": pid, "columnId": cols[0], "agentId": "claude"})).await.unwrap()
            ["card"]
            .clone();
    let e = c.call("card.aiRename", json!({"id": draft["id"]})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);

    let card = new_card(&mut c, &pid, &cols[0], "claude").await;
    let id = card["id"].as_str().unwrap().to_string();
    let sid = card["agentSessionId"].as_str().unwrap().to_string();
    c.attach(&id, 100, 30).await;
    c.wait_output("hook=found").await;
    type_line(&mut c, &id, &format!("ev SessionStart {}", sc.payload(0, &sid))).await;
    wait_card(&mut c, &id, "idle", |k| k["status"] == "idle").await;

    // Mid-turn the command would be queued as a prompt: refused.
    type_line(&mut c, &id, &format!("ev UserPromptSubmit {}", sc.payload(3, &sid))).await;
    wait_card(&mut c, &id, "running", |k| k["status"] == "running").await;
    let e = c.call("card.aiRename", json!({"id": id})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    type_line(&mut c, &id, &format!("ev Stop {}", sc.payload(4, &sid))).await;
    wait_card(&mut c, &id, "between turns", |k| k["status"] == "done" || k["status"] == "idle").await;

    // A card the user named follows the agent again; asking twice types the command once.
    c.call("card.update", json!({"id": id, "title": "My task"})).await.unwrap();
    let r = c.call("card.aiRename", json!({"id": id})).await.unwrap()["card"].clone();
    assert_eq!((r["title"].as_str(), r["titleAuto"].as_bool()), (Some("My task"), Some(true)));
    c.call("card.aiRename", json!({"id": id})).await.unwrap();
    let w = wait_card(&mut c, &id, "named by the agent", |k| k["title"] == "Tidy the parser").await;
    assert_eq!(w["titleAuto"], true);
    c.wait_output("RENAMED").await;
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let typed = std::fs::read_to_string(env.root.join("claude-config/typed.log")).unwrap();
    assert_eq!(typed, "/rename\n");

    // A title typed afterwards stays.
    let u = c.call("card.update", json!({"id": id, "title": "Mine"})).await.unwrap()["card"].clone();
    assert_eq!((u["title"].as_str(), u["titleAuto"].as_bool()), (Some("Mine"), Some(false)));
}

fn git_repo(path: &Path) {
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "init"],
    ] {
        assert!(Command::new("git").arg("-C").arg(path).args(&args).status().unwrap().success());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn worktree_setup_runs_in_the_session() {
    let mut env = Env::new();
    fake_claude_env(&mut env);
    git_repo(&env.root.join("project"));
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let p = c.call("project.add", json!({"path": env.root.join("project")})).await.unwrap()["project"].clone();
    let pid = p["id"].as_str().unwrap().to_string();
    assert!(p.get("worktreeSetup").is_none());
    let col =
        c.call("board.get", json!({"projectId": pid})).await.unwrap()["columns"][0]["id"].as_str().unwrap().to_string();
    let script = "echo SETUP-RAN \"$MUSTER_BRANCH\" \"$(basename \"$MUSTER_WORKTREE_PATH\")\" \"$(basename \"$MUSTER_PROJECT_PATH\")\"\ntouch setup-marker\n";
    let u = c.call("project.update", json!({"id": pid, "worktreeSetup": script})).await.unwrap();
    assert_eq!(u["project"]["worktreeSetup"], script);

    // Success: setup output in the card's terminal, then the shell starts.
    let card = c
        .call("card.create", json!({"projectId": pid, "columnId": col, "agentId": "shell", "start": true, "worktree": {"branch": "feat/a"}}))
        .await
        .unwrap()["card"]
        .clone();
    let id = card["id"].as_str().unwrap().to_string();
    assert_eq!(card["setup"]["state"], "running");
    assert_eq!(card["status"], "running");
    let w = wait_card(&mut c, &id, "set up", |k| k.get("setup").is_none()).await;
    assert_eq!(w["status"], "running");
    let wt = PathBuf::from(w["cwd"].as_str().unwrap());
    assert!(wt.join("setup-marker").exists());
    c.attach(&id, 100, 30).await;
    c.wait_output("SETUP-RAN feat/a project-feat-a project").await;
    c.call("term.write", json!({"cardId": id, "data": "echo shell-$((1+1))\r"})).await.unwrap();
    c.wait_output("shell-2").await;

    // Failure: the agent does not start, the shell stays, the card needs the user.
    c.call("project.update", json!({"id": pid, "worktreeSetup": "echo BROKEN\nexit 3\n"})).await.unwrap();
    let card = c
        .call("card.create", json!({"projectId": pid, "columnId": col, "agentId": "claude", "start": true, "worktree": {"branch": "feat/b"}}))
        .await
        .unwrap()["card"]
        .clone();
    let bad = card["id"].as_str().unwrap().to_string();
    let w = wait_card(&mut c, &bad, "failed setup", |k| k["status"] == "needs_you").await;
    assert_eq!(w["needsYouReason"], "setup_failed");
    assert_eq!(w["setup"], json!({"state": "failed", "exitCode": 3}));
    assert_eq!(w["hasSession"], true);
    c.attach(&bad, 100, 30).await;
    c.wait_output("worktree setup failed (exit 3)").await;
    assert!(!String::from_utf8_lossy(&c.output).contains("FAKE-CLAUDE"));
    // The user fixes the script and starts again: setup reruns, then the agent starts.
    c.call("project.update", json!({"id": pid, "worktreeSetup": "echo FIXED\n"})).await.unwrap();
    c.call("card.start", json!({"id": bad})).await.unwrap();
    let w =
        wait_card(&mut c, &bad, "set up after the fix", |k| k.get("setup").is_none() && k["status"] == "running").await;
    assert!(w.get("needsYouReason").is_none());
    c.attach(&bad, 100, 30).await;
    c.wait_output("hook=found").await;

    // No script: nothing runs, no setup state.
    let u = c.call("project.update", json!({"id": pid, "worktreeSetup": null})).await.unwrap();
    assert!(u["project"].get("worktreeSetup").is_none());
    let card = c
        .call("card.create", json!({"projectId": pid, "columnId": col, "agentId": "shell", "start": true, "worktree": {"branch": "feat/c"}}))
        .await
        .unwrap()["card"]
        .clone();
    assert!(card.get("setup").is_none());
    assert_eq!(card["status"], "running");
}

#[tokio::test(flavor = "current_thread")]
async fn quiet_terminal_settles_turn_with_nobody_attached() {
    let mut env = Env::new();
    fake_claude_env(&mut env);
    env.env.retain(|(k, _)| k != "MUSTER_QUIET_MS");
    env.env.push(("MUSTER_QUIET_MS".into(), "1000".into()));
    let dir = env.root.join("p");
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = transcript_dir(&env).join("t.jsonl");
    std::fs::copy(fixtures().join("transcript-basic.jsonl"), &transcript).unwrap(); // turn ended
    let sc = Scenario { dir, payloads: fixture_payloads(), transcript, cwd: env.root.join("project") };
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let pid = c.call("project.add", json!({"path": env.root.join("project")})).await.unwrap()["project"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let col =
        c.call("board.get", json!({"projectId": pid})).await.unwrap()["columns"][0]["id"].as_str().unwrap().to_string();
    let card = c
        .call("card.create", json!({"projectId": pid, "columnId": col, "agentId": "claude", "start": true}))
        .await
        .unwrap()["card"]
        .clone();
    let id = card["id"].as_str().unwrap().to_string();
    let sid = card["agentSessionId"].as_str().unwrap().to_string();
    // Never attached: input goes in through tmux directly.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    type_line(&mut c, &id, &format!("ev SessionStart {}", sc.payload(0, &sid))).await;
    wait_card(&mut c, &id, "idle", |k| k["status"] == "idle").await;
    type_line(&mut c, &id, &format!("ev UserPromptSubmit {}", sc.payload(3, &sid))).await;
    wait_card(&mut c, &id, "running", |k| k["status"] == "running").await;
    // No Stop arrives; once the pane is silent, tmux tells the daemon, which reads the
    // transcript and settles the turn.
    let w = wait_card(&mut c, &id, "done after silence", |k| k["status"] == "done").await;
    assert_eq!(w["unread"], true);
}

// ---------------------------------------------------------------------------
// Round 3: stats, keep-awake, restore, context sharing
// ---------------------------------------------------------------------------

async fn setup_project(c: &mut Client, env: &Env) -> (String, Vec<String>) {
    let pid = c.call("project.add", json!({"path": env.root.join("project")})).await.unwrap()["project"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let cols = c.call("board.get", json!({"projectId": pid})).await.unwrap()["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    (pid, cols)
}

async fn new_card(c: &mut Client, pid: &str, col: &str, agent: &str) -> Value {
    c.call("card.create", json!({"projectId": pid, "columnId": col, "agentId": agent, "start": true})).await.unwrap()
        ["card"]
        .clone()
}

fn daemon_pid(env: &Env) -> u64 {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(env.home.join("daemon.json")).unwrap()).unwrap();
    v["pid"].as_u64().unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn stats_only_while_subscribed() {
    let mut env = Env::new();
    fake_claude_env(&mut env);
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, cols) = setup_project(&mut c, &env).await;
    let card = new_card(&mut c, &pid, &cols[0], "shell").await;
    let id = card["id"].as_str().unwrap().to_string();
    c.call("term.write", json!({"cardId": id, "data": "sleep 60 & sleep 61 &\r"})).await.unwrap();

    let r = c.call("stats.subscribe", json!({"intervalMs": 100})).await.unwrap();
    assert_eq!(r["intervalMs"], 2000, "the interval has a floor");
    // Wait for a sample that sees the shell and both children (the shell may still be
    // starting on a busy machine).
    let deadline = Instant::now() + TIMEOUT;
    let (s, mine) = loop {
        let s = c.wait_event_named("stats.changed").await;
        let mine = s["cards"].as_array().unwrap().iter().find(|x| x["cardId"] == id.as_str()).cloned();
        if let Some(m) = mine.filter(|m| m["processes"].as_u64().unwrap_or(0) >= 3) {
            break (s, m);
        }
        assert!(Instant::now() < deadline, "never saw the shell's children: {s}");
    };
    assert!(mine["memBytes"].as_u64().unwrap() > 0);
    // The shell and its two background children.
    assert!(mine["processes"].as_u64().unwrap() >= 3, "{mine}");
    assert!(s["total"]["memBytes"].as_u64().unwrap() >= mine["memBytes"].as_u64().unwrap());
    assert!(s["daemon"]["memBytes"].as_u64().unwrap() > 0);
    let s2 = c.wait_event_named("stats.changed").await;
    assert!(s2["at"].as_i64().unwrap() - s["at"].as_i64().unwrap() >= 1900);

    // Unsubscribed: nothing more arrives.
    c.call("stats.unsubscribe", json!({})).await.unwrap();
    c.events.clear();
    let deadline = Instant::now() + std::time::Duration::from_millis(2600);
    while Instant::now() < deadline {
        c.call("project.list", json!({})).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    assert!(!c.events.iter().any(|e| e.m == "stats.changed"));
}

/// The daemon holds its assertion through `caffeinate -i -w <daemon pid>`; check that
/// the process exists and that macOS lists its assertion.
fn assertion_held(pid: u64) -> bool {
    let out = Command::new("/usr/bin/pgrep").args(["-f", &format!("caffeinate -i -w {pid}$")]).output().unwrap();
    let child = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if child.is_empty() {
        return false;
    }
    let a = Command::new("/usr/bin/pmset").args(["-g", "assertions"]).output().unwrap();
    String::from_utf8_lossy(&a.stdout).contains(&format!("pid {child}(caffeinate)"))
}

fn low_battery() -> bool {
    let out = Command::new("/usr/bin/pmset").args(["-g", "batt"]).output().unwrap();
    let t = String::from_utf8_lossy(&out.stdout).into_owned();
    t.contains("'Battery Power'")
        && t.split('%')
            .next()
            .and_then(|a| a.rsplit(|c: char| !c.is_ascii_digit()).next())
            .and_then(|d| d.parse::<u32>().ok())
            .is_some_and(|p| p < 10)
}

async fn wait_for(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn keep_awake_follows_running_cards_and_settings() {
    if low_battery() {
        eprintln!("skipping: on battery below 10%, the assertion is deliberately not taken");
        return;
    }
    let mut env = Env::new();
    fake_claude_env(&mut env);
    let dir = env.root.join("p");
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = transcript_dir(&env).join("t.jsonl");
    std::fs::write(&transcript, "").unwrap();
    let sc = Scenario { dir, payloads: fixture_payloads(), transcript, cwd: env.root.join("project") };
    let port = env.start_daemon();
    let dpid = daemon_pid(&env);
    let mut c = Client::connect(port, &env.home).await;
    let s = c.call("settings.get", json!({})).await.unwrap();
    assert_eq!(
        s["settings"],
        json!({
            "keepAwake": true,
            "autoRestore": false,
            "silenceSeconds": 5,
            "clipboardPassthrough": false,
            "idleStopMinutes": 0
        })
    );
    assert!(!assertion_held(dpid));
    let (pid, cols) = setup_project(&mut c, &env).await;
    let card = new_card(&mut c, &pid, &cols[0], "claude").await;
    let id = card["id"].as_str().unwrap().to_string();
    let sid = card["agentSessionId"].as_str().unwrap().to_string();
    wait_for("assertion while running", || assertion_held(dpid)).await;
    type_line(&mut c, &id, &format!("ev SessionStart {}", sc.payload(0, &sid))).await;
    wait_card(&mut c, &id, "idle", |k| k["status"] == "idle").await;
    wait_for("release when idle", || !assertion_held(dpid)).await;
    type_line(&mut c, &id, &format!("ev UserPromptSubmit {}", sc.payload(3, &sid))).await;
    wait_for("assertion again", || assertion_held(dpid)).await;
    // Turning the setting off releases it at once.
    let r = c.call("settings.update", json!({"keepAwake": false})).await.unwrap();
    assert_eq!(r["settings"]["keepAwake"], false);
    let ev = c.wait_event_named("settings.changed").await;
    assert_eq!(ev["settings"]["keepAwake"], false);
    wait_for("release when disabled", || !assertion_held(dpid)).await;
    c.call("settings.update", json!({"keepAwake": true})).await.unwrap();
    wait_for("re-enabled", || assertion_held(dpid)).await;
    type_line(&mut c, &id, &format!("ev Stop {}", sc.payload(4, &sid))).await;
    wait_for("release when done", || !assertion_held(dpid)).await;
    // The setting persists across restarts.
    c.call("settings.update", json!({"autoRestore": true})).await.unwrap();
    env.crash_daemon();
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let s = c.call("settings.get", json!({})).await.unwrap();
    assert_eq!(s["settings"]["autoRestore"], true);
}

fn kill_tmux_server(env: &Env) {
    assert!(env.socket.starts_with("muster-test-"));
    kill_tmux(&env.socket);
}

#[tokio::test(flavor = "current_thread")]
async fn restore_after_tmux_dies() {
    let mut env = Env::new();
    fake_claude_env(&mut env);
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, cols) = setup_project(&mut c, &env).await;
    let sh = new_card(&mut c, &pid, &cols[0], "shell").await;
    let sh_id = sh["id"].as_str().unwrap().to_string();
    let cl = new_card(&mut c, &pid, &cols[0], "claude").await;
    let cl_id = cl["id"].as_str().unwrap().to_string();
    let sid = cl["agentSessionId"].as_str().unwrap().to_string();

    c.attach(&sh_id, 100, 30).await;
    c.call("term.write", json!({"cardId": sh_id, "data": "echo RESTORE-ME-$((40+2))\r"})).await.unwrap();
    c.wait_output("RESTORE-ME-42").await;
    c.call("session.detach", json!({"cardId": sh_id})).await.unwrap(); // saves scrollback
    assert!(env.home.join(format!("scrollback/{sh_id}.ans")).is_file());
    c.attach(&cl_id, 100, 30).await;
    c.wait_output("hook=found").await;
    c.call("session.detach", json!({"cardId": cl_id})).await.unwrap();

    // tmux dies: both cards become restorable (no exit status recorded).
    kill_tmux_server(&env);
    let w = wait_card(&mut c, &sh_id, "restorable", |k| k["restorable"] == true).await;
    assert_eq!(w["status"], "exited");
    assert_eq!(w["hasSession"], false);
    wait_card(&mut c, &cl_id, "restorable", |k| k["restorable"] == true).await;

    // Starting replays the saved screen, then the shell (or the agent, resumed) runs.
    let r = c.call("card.start", json!({"id": sh_id})).await.unwrap();
    assert_eq!(r["card"]["restorable"], false);
    c.attach(&sh_id, 100, 30).await;
    c.wait_output("RESTORE-ME-42").await;
    c.wait_output("[muster] restored").await;
    c.call("term.write", json!({"cardId": sh_id, "data": "echo alive-$((1+1))\r"})).await.unwrap();
    c.wait_output("alive-2").await;
    c.call("card.start", json!({"id": cl_id})).await.unwrap();
    c.attach(&cl_id, 100, 30).await;
    c.wait_output(&format!("FAKE-CLAUDE sid={sid} resumed=yes")).await;

    // A session ended on purpose is not restorable.
    c.call("session.kill", json!({"cardId": cl_id})).await.unwrap();
    let w = wait_card(&mut c, &cl_id, "exited", |k| k["status"] == "exited").await;
    assert_eq!(w["restorable"], false);

    // Automatic restore at startup after the daemon and tmux both went away.
    c.call("settings.update", json!({"autoRestore": true})).await.unwrap();
    // Still attached to the shell from above; watch its output again.
    c.focus = Some(sh_id.clone());
    c.output.clear();
    c.call("term.write", json!({"cardId": sh_id, "data": "echo SECOND-$((5*5))\r"})).await.unwrap();
    c.wait_output("SECOND-25").await;
    c.call("session.detach", json!({"cardId": sh_id})).await.unwrap();
    env.crash_daemon();
    drop(c);
    kill_tmux_server(&env);
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let k = card_of(&mut c, &pid, &sh_id).await;
    assert_eq!(k["hasSession"], true, "{k}");
    assert_eq!(k["restorable"], false);
    c.attach(&sh_id, 100, 30).await;
    c.wait_output("SECOND-25").await;
}

fn cli(env: &Env, me: Option<&str>, args: &[&str]) -> (bool, String) {
    let mut cmd = muster_command(cli_bin());
    cmd.env("MUSTER_HOME", &env.home).args(args);
    if let Some(m) = me {
        cmd.env("MUSTER_CARD_ID", m);
    }
    let out = cmd.output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

#[tokio::test(flavor = "current_thread")]
async fn context_links_messages_and_handoff() {
    let mut env = Env::new();
    fake_claude_env(&mut env);
    let dir = env.root.join("p");
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = transcript_dir(&env).join("t.jsonl");
    std::fs::copy(fixtures().join("transcript-basic.jsonl"), &transcript).unwrap();
    let sc = Scenario { dir, payloads: fixture_payloads(), transcript, cwd: env.root.join("project") };
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    assert!(env.home.join("agents/muster-instructions.md").is_file());
    assert!(env.home.join("agents/plugin/skills/muster/SKILL.md").is_file());
    let (pid, cols) = setup_project(&mut c, &env).await;
    let a = new_card(&mut c, &pid, &cols[0], "claude").await;
    let b = new_card(&mut c, &pid, &cols[0], "claude").await;
    let x = new_card(&mut c, &pid, &cols[0], "shell").await;
    let (a_id, b_id, x_id) = (
        a["id"].as_str().unwrap().to_string(),
        b["id"].as_str().unwrap().to_string(),
        x["id"].as_str().unwrap().to_string(),
    );
    let b_sid = b["agentSessionId"].as_str().unwrap().to_string();
    let a_sid = a["agentSessionId"].as_str().unwrap().to_string();

    // Links are undirected, on the board and announced.
    c.call("link.add", json!({"a": b_id, "b": a_id})).await.unwrap();
    let ev = c.wait_event_named("link.changed").await;
    assert_eq!(ev["linked"], true);
    let board = c.call("board.get", json!({"projectId": pid})).await.unwrap();
    assert_eq!(board["links"].as_array().unwrap().len(), 1);

    // B has a transcript, notes and a screen.
    type_line(&mut c, &b_id, &format!("ev SessionStart {}", sc.payload(0, &b_sid))).await;
    wait_card(&mut c, &b_id, "idle", |k| k["status"] == "idle").await;
    c.call("card.update", json!({"id": b_id, "notes": "B-NOTES-xyz"})).await.unwrap();
    c.call("card.update", json!({"id": x_id, "notes": "X-NOTES"})).await.unwrap();

    // The CLI as agent A.
    let me = Some(a_id.as_str());
    let (ok, out) = cli(&env, me, &["context", "list"]);
    assert!(ok && out.contains(&b_id) && !out.contains(&x_id), "{out}");
    let (ok, out) = cli(&env, me, &["context", "read", &b_id, "--notes"]);
    assert!(ok && out.contains("B-NOTES-xyz"), "{out}");
    let (ok, out) = cli(&env, me, &["context", "read", &b_id, "--transcript", "5"]);
    assert!(ok && out.contains("user: ") && out.contains("assistant: "), "{out}");
    let (ok, out) = cli(&env, me, &["context", "read", &b_id, "--screen"]);
    assert!(ok && out.contains("FAKE-CLAUDE"), "{out}");
    let (ok, out) = cli(&env, me, &["context", "read", &b_id]);
    assert!(ok && out.contains("status: idle") && out.contains("last assistant reply"), "{out}");
    let (ok, out) = cli(&env, me, &["context", "read", &x_id, "--notes"]);
    assert!(!ok && out.contains("not linked"), "{out}");
    let (ok, _) = cli(&env, me, &["context", "read", &x_id, "--notes", "--any"]);
    assert!(!ok, "--any is refused inside a card");
    // A human outside any card needs --any.
    let (ok, _) = cli(&env, None, &["context", "read", &x_id, "--notes"]);
    assert!(!ok);
    let (ok, out) = cli(&env, None, &["context", "read", &x_id, "--notes", "--any"]);
    assert!(ok && out.contains("X-NOTES"), "{out}");
    // Notes and moving my own card.
    assert!(cli(&env, me, &["notes", "set", "first"]).0);
    assert!(cli(&env, me, &["notes", "append", "second"]).0);
    assert!(cli(&env, me, &["card", "move", "--me", "Done"]).0);
    let k = card_of(&mut c, &pid, &a_id).await;
    assert_eq!(k["notes"], "first\nsecond");
    assert_eq!(k["columnId"], cols[3].as_str());

    // The CLI from inside A's own session (MUSTER_HOME, MUSTER_CARD_ID, MUSTER_CLI set).
    c.attach(&a_id, 120, 30).await;
    c.wait_output("hook=found").await;
    // The launch passed the instructions file and the skill directory.
    c.wait_output("INSTRUCTIONS-OK").await;
    c.wait_output("SKILL-OK").await;
    type_line(&mut c, &a_id, "cli context list").await;
    c.wait_output("CLI-DONE[0]").await;
    assert!(String::from_utf8_lossy(&c.output).contains(&b_id));

    // Messages: queued while B works, delivered when it is done.
    type_line(&mut c, &b_id, &format!("ev UserPromptSubmit {}", sc.payload(3, &b_sid))).await;
    wait_card(&mut c, &b_id, "running", |k| k["status"] == "running").await;
    let (ok, out) = cli(&env, me, &["send", &b_id, "please", "check", "the", "build"]);
    assert!(ok && out.contains("queued"), "{out}");
    let w = wait_card(&mut c, &b_id, "with a queued message", |k| {
        k["queuedMessages"].as_array().is_some_and(|q| q.len() == 1)
    })
    .await;
    assert_eq!(w["queuedMessages"][0]["fromCardId"], a_id.as_str());
    let m2 = c.call("message.send", json!({"cardId": b_id, "text": "never mind"})).await.unwrap()["message"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    c.call("message.cancel", json!({"id": m2})).await.unwrap();
    wait_card(&mut c, &b_id, "one queued", |k| k["queuedMessages"].as_array().is_some_and(|q| q.len() == 1)).await;
    // Not delivered while running; then the turn ends.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(card_of(&mut c, &pid, &b_id).await["queuedMessages"].as_array().unwrap().len(), 1);
    c.attach(&b_id, 160, 30).await;
    type_line(&mut c, &b_id, &format!("ev Stop {}", sc.payload(4, &b_sid))).await;
    c.wait_output(&format!("[Muster message from card {a_id}")).await;
    c.wait_output("please check the build").await;
    wait_card(&mut c, &b_id, "delivered", |k| k.get("queuedMessages").is_none()).await;

    // Handoff: a new linked card, told how to read A's context.
    let h = c.call("card.handoff", json!({"id": a_id})).await.unwrap()["card"].clone();
    let h_id = h["id"].as_str().unwrap().to_string();
    assert_eq!(h["columnId"], cols[3].as_str());
    assert_eq!(h["cwd"], a["cwd"]);
    assert_eq!(h["status"], "running");
    assert!(h["prompt"].as_str().unwrap().contains(&format!("muster context read {a_id}")));
    c.attach(&h_id, 300, 30).await;
    c.wait_output("muster context read").await;
    let links = c.call("board.get", json!({"projectId": pid})).await.unwrap()["links"].clone();
    assert!(links.as_array().unwrap().iter().any(|l| (l["a"] == a_id.as_str() || l["b"] == a_id.as_str())
        && (l["a"] == h_id.as_str() || l["b"] == h_id.as_str())));
    let _ = a_sid;

    c.call("link.remove", json!({"a": a_id, "b": b_id})).await.unwrap();
    let (ok, _) = cli(&env, me, &["context", "read", &b_id, "--notes"]);
    assert!(!ok, "unlinked again");
}

/// A payload from another fixture file, rewritten to the card's session and paths.
fn payload_from(sc: &Scenario, file: &str, index: usize, sid: &str) -> String {
    let line = std::fs::read_to_string(fixtures().join(file)).unwrap().lines().nth(index).unwrap().to_string();
    let mut p = serde_json::from_str::<Value>(&line).unwrap()["payload"].clone();
    p["session_id"] = json!(sid);
    p["transcript_path"] = json!(sc.transcript);
    p["cwd"] = json!(sc.cwd);
    let path = sc.dir.join(format!("{file}-{index}.json"));
    std::fs::write(&path, serde_json::to_string(&p).unwrap()).unwrap();
    path.to_string_lossy().into_owned()
}

#[tokio::test(flavor = "current_thread")]
async fn questions_and_plans_answered_from_the_board() {
    let mut env = Env::new();
    fake_claude_env(&mut env);
    let dir = env.root.join("p");
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = transcript_dir(&env).join("t.jsonl");
    std::fs::write(&transcript, "").unwrap();
    let sc = Scenario { dir, payloads: fixture_payloads(), transcript, cwd: env.root.join("project") };
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let (pid, cols) = setup_project(&mut c, &env).await;
    let card = new_card(&mut c, &pid, &cols[0], "claude").await;
    let id = card["id"].as_str().unwrap().to_string();
    let sid = card["agentSessionId"].as_str().unwrap().to_string();
    c.attach(&id, 200, 40).await;
    c.wait_output("hook=found").await;
    type_line(&mut c, &id, &format!("ev SessionStart {}", sc.payload(0, &sid))).await;
    type_line(&mut c, &id, &format!("ev UserPromptSubmit {}", sc.payload(3, &sid))).await;
    wait_card(&mut c, &id, "running", |k| k["status"] == "running").await;

    // A multiple-choice question.
    let q = payload_from(&sc, "events-question-subagent-compact.jsonl", 5, &sid);
    type_line(&mut c, &id, &format!("wait PermissionRequest {q} 30")).await;
    let w = wait_card(&mut c, &id, "question", |k| k["status"] == "needs_you").await;
    assert_eq!(w["needsYouReason"], "question");
    let pa = &w["pendingApproval"];
    assert_eq!(pa["kind"], "question");
    assert_eq!(pa["questions"][0]["question"], "Which color do you prefer?");
    assert_eq!(pa["questions"][0]["header"], "Color");
    assert_eq!(pa["questions"][0]["multiSelect"], false);
    assert_eq!(pa["questions"][0]["options"][1], json!({"label": "Blue", "description": "You prefer blue."}));
    let rid = pa["requestId"].as_str().unwrap().to_string();
    let e = c.call("approval.answer", json!({"cardId": id, "requestId": rid, "decision": "answer"})).await.unwrap_err();
    assert_eq!(e.code, muster_proto::ErrorCode::Invalid);
    let r = c
        .call(
            "approval.answer",
            json!({"cardId": id, "requestId": rid, "decision": "answer", "answers": {"Which color do you prefer?": ["Blue"]}}),
        )
        .await
        .unwrap();
    assert_eq!(r["via"], "hook");
    c.wait_output("DECISION[").await;
    c.wait_output("Blue").await;
    wait_card(&mut c, &id, "running", |k| k["status"] == "running").await;

    // A plan: approve it, continuing in accept-edits mode.
    let p = payload_from(&sc, "events-plan.jsonl", 6, &sid);
    c.output.clear();
    type_line(&mut c, &id, &format!("wait PermissionRequest {p} 30")).await;
    let w = wait_card(&mut c, &id, "plan", |k| k["status"] == "needs_you").await;
    assert_eq!(w["needsYouReason"], "plan");
    assert_eq!(w["pendingApproval"]["kind"], "plan");
    assert!(w["pendingApproval"]["plan"].as_str().unwrap().contains("plan-test.txt"));
    assert!(w["pendingApproval"]["planFilePath"].as_str().unwrap().ends_with(".md"));
    let rid = w["pendingApproval"]["requestId"].as_str().unwrap().to_string();
    let r = c
        .call(
            "approval.answer",
            json!({"cardId": id, "requestId": rid, "decision": "approve_plan", "acceptEdits": true}),
        )
        .await
        .unwrap();
    assert_eq!(r["via"], "hook");
    c.wait_output(r#""behavior":"allow""#).await;
    wait_card(&mut c, &id, "running", |k| k["status"] == "running").await;

    // Sending a plan back is a deny with a message.
    c.output.clear();
    type_line(&mut c, &id, &format!("wait PermissionRequest {p} 30")).await;
    let w = wait_card(&mut c, &id, "plan again", |k| k["status"] == "needs_you").await;
    let rid = w["pendingApproval"]["requestId"].as_str().unwrap().to_string();
    c.call("approval.answer", json!({"cardId": id, "requestId": rid, "decision": "deny", "message": "smaller steps"}))
        .await
        .unwrap();
    c.wait_output("smaller steps").await;
    wait_card(&mut c, &id, "running", |k| k["status"] == "running").await;

    // Questions have no terminal-key fallback.
    type_line(&mut c, &id, &format!("wait PermissionRequest {q} 1")).await;
    wait_card(&mut c, &id, "question", |k| k["status"] == "needs_you").await;
    c.wait_output("DECISION[]").await;
    let e = c
        .call(
            "approval.answer",
            json!({"cardId": id, "decision": "answer", "answers": {"Which color do you prefer?": ["Red"]}}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, muster_proto::ErrorCode::Unavailable);
}
