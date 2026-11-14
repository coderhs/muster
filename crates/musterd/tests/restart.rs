//! End-to-end: a shell card's tmux session survives a daemon crash and
//! re-attaching after the restart reproduces the screen.
//!
//! Uses a temporary MUSTER_HOME and a unique `muster-test-*` tmux socket,
//! and kills only that tmux server.


mod common;

use std::path::PathBuf;
use std::process::{Command, Stdio};

use common::*;
use futures_util::{SinkExt, StreamExt};
use muster_proto::*;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test(flavor = "current_thread")]
async fn session_survives_daemon_restart() {
    let mut env = Env::new();
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;

    // A second daemon for the same MUSTER_HOME refuses to start.
    let second = muster_command(env!("CARGO_BIN_EXE_musterd"))
        .env("MUSTER_HOME", &env.home)
        .env("MUSTER_TMUX_SOCKET", &env.socket)
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!second.success());
    let token_mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(env.home.join("token")).unwrap().permissions());
    assert_eq!(token_mode & 0o777, 0o600);

    // Before hello, anything else is refused (checked on a second socket).
    {
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await.unwrap();
        ws.send(Message::text(json!({"t":"req","id":1,"m":"project.list","p":{}}).to_string())).await.unwrap();
        let m = ws.next().await.unwrap().unwrap();
        let r: ResponseFrame = serde_json::from_str(m.to_text().unwrap()).unwrap();
        assert_eq!(r.e.unwrap().code, ErrorCode::Unauthorized);
    }

    let project = c.call("project.add", json!({"path": env.root.join("project")})).await.unwrap();
    let project_id = project["project"]["id"].as_str().unwrap().to_string();
    let board = c.call("board.get", json!({"projectId": project_id})).await.unwrap();
    assert_eq!(board["columns"].as_array().unwrap().len(), 3);
    let col = board["columns"][0]["id"].as_str().unwrap().to_string();

    let card = c
        .call("card.create", json!({"projectId": project_id, "columnId": col, "agentId": "shell", "start": true}))
        .await
        .unwrap();
    let card_id = card["card"]["id"].as_str().unwrap().to_string();
    assert_eq!(card["card"]["status"], "running");
    assert_eq!(card["card"]["hasSession"], true);
    assert!(env.tmux_sessions().contains(&format!("m-{card_id}")));

    let size = c.attach(&card_id, 80, 24).await;
    assert_eq!(size, json!({"cols": 80, "rows": 24}));
    // The quotes and arithmetic make the output differ from the typed echo.
    c.call("term.write", json!({"cardId": card_id, "data": "echo MUSTER_CARD_ID=$MUSTER_CARD_ID hel''lo-$((6*7))\r"}))
        .await
        .unwrap();
    c.wait_output("hello-42").await;
    c.wait_output(&format!("MUSTER_CARD_ID={card_id} hello-42")).await;

    // If the terminal client dies while the session lives on, a new one is started and
    // output keeps flowing (beginning with a repaint).
    let clients = Command::new("tmux")
        .args(["-L", &env.socket, "list-clients", "-t", &format!("=m-{card_id}"), "-F", "#{client_pid}"])
        .output()
        .unwrap();
    let pid = String::from_utf8_lossy(&clients.stdout).trim().to_string();
    assert!(!pid.is_empty(), "no tmux client attached");
    assert!(Command::new("kill").arg("-9").arg(&pid).status().unwrap().success());
    c.output.clear();
    c.wait_output("hello-42").await; // the new client's repaint
    c.call("term.write", json!({"cardId": card_id, "data": "echo revived-$((2*3))\r"})).await.unwrap();
    c.wait_output("revived-6").await;

    // A second client joining an attached session gets the screen too, and
    // the size becomes the smaller of the two.
    let mut c2 = Client::connect(port, &env.home).await;
    let size2 = c2.attach(&card_id, 70, 30).await;
    assert_eq!(size2, json!({"cols": 70, "rows": 24}));
    c2.wait_output("hello-42").await;
    let ev = c2.wait_event("session.size", &card_id).await;
    assert_eq!((ev["cols"].as_u64(), ev["rows"].as_u64()), (Some(70), Some(24)));
    let r = c2.call("term.resize", json!({"cardId": card_id, "cols": 100, "rows": 40})).await.unwrap();
    assert_eq!(r, json!({"cols": 80, "rows": 24}));
    drop(c2);

    // Crash the daemon. The tmux session must survive.
    env.crash_daemon();
    drop(c);
    assert!(env.tmux_sessions().contains(&format!("m-{card_id}")), "session died with the daemon");

    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let board = c.call("board.get", json!({"projectId": project_id})).await.unwrap();
    let card = &board["cards"][0];
    assert_eq!(card["id"], card_id.as_str());
    assert_eq!(card["hasSession"], true);
    assert_eq!(card["status"], "running");

    // Re-attach from an empty terminal: the first bytes repaint the screen,
    // which still shows the earlier output.
    c.attach(&card_id, 80, 24).await;
    c.wait_output("hello-42").await;
    c.output.clear();
    c.call("term.write", json!({"cardId": card_id, "data": "echo still-$((40+2))\r"})).await.unwrap();
    c.wait_output("still-42").await;

    // Ending the shell reports the exit code and marks the card exited.
    c.call("term.write", json!({"cardId": card_id, "data": "exit 5\r"})).await.unwrap();
    let exit = c.wait_event("session.exit", &card_id).await;
    assert_eq!(exit["exitCode"], 5);
    let board = c.call("board.get", json!({"projectId": project_id})).await.unwrap();
    assert_eq!(board["cards"][0]["status"], "exited");
    assert_eq!(board["cards"][0]["hasSession"], false);
    // The output did not end with tmux's screen-clearing exit sequence.
    assert!(!String::from_utf8_lossy(&c.output).contains("[exited]"));

    // Starting it again launches a new session.
    let card = c.call("card.start", json!({"id": card_id})).await.unwrap();
    assert_eq!(card["card"]["status"], "running");
    c.call("card.delete", json!({"id": card_id})).await.unwrap_err();
    c.call("card.delete", json!({"id": card_id, "killSession": true})).await.unwrap();
    let board = c.call("board.get", json!({"projectId": project_id})).await.unwrap();
    assert!(board["cards"].as_array().unwrap().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn board_methods_and_events() {
    let mut env = Env::new();
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let mut watcher = Client::connect(port, &env.home).await;

    // A git repository for the worktree case.
    let repo = env.root.join("project");
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "init"],
    ] {
        assert!(Command::new("git").arg("-C").arg(&repo).args(&args).status().unwrap().success());
    }

    let p = c.call("project.add", json!({"path": repo, "name": "Demo"})).await.unwrap()["project"].clone();
    let pid = p["id"].as_str().unwrap().to_string();
    assert_eq!(p["name"], "Demo");
    let ev = watcher.wait_event_named("project.changed").await;
    assert_eq!(ev["project"]["id"], pid.as_str());
    assert_eq!(c.call("project.add", json!({"path": repo})).await.unwrap_err().code, ErrorCode::Conflict);
    let p = c.call("project.update", json!({"id": pid, "color": "#123456"})).await.unwrap();
    assert_eq!(p["project"]["color"], "#123456");

    let board = c.call("board.get", json!({"projectId": pid})).await.unwrap();
    let cols: Vec<String> = board["columns"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap().into()).collect();
    let review = c.call("column.create", json!({"projectId": pid, "title": "Review", "category": "started"})).await.unwrap();
    let review_id = review["column"]["id"].as_str().unwrap().to_string();
    assert_eq!(review["column"]["position"], 3);
    let moved = c.call("column.update", json!({"id": review_id, "position": 1, "title": "Review!"})).await.unwrap();
    assert_eq!(moved["column"]["position"], 1);
    assert_eq!(moved["column"]["title"], "Review!");

    let mk = |title: &str| json!({"projectId": pid, "columnId": cols[0], "agentId": "claude", "title": title});
    let a = c.call("card.create", mk("a")).await.unwrap()["card"].clone();
    let b = c.call("card.create", mk("b")).await.unwrap()["card"].clone();
    assert_eq!(a["status"], "draft");
    assert_eq!(a["hasSession"], false);
    assert_eq!(a["cwd"], p["project"]["path"]);
    assert!(a["rank"].as_str() < b["rank"].as_str());
    let a_id = a["id"].as_str().unwrap();
    let b_id = b["id"].as_str().unwrap();
    let r = c.call("card.move", json!({"id": b_id, "columnId": cols[0], "before": a_id})).await.unwrap();
    assert!(r["card"]["rank"].as_str() < a["rank"].as_str());
    let ev = watcher.wait_event_named("card.changed").await;
    assert!(ev["card"].is_object());
    let r = c.call("card.move", json!({"id": a_id, "columnId": review_id})).await.unwrap();
    assert_eq!(r["card"]["columnId"], review_id.as_str());
    assert_eq!(r["card"]["status"], "draft", "moving never starts a session");

    let u = c.call("card.update", json!({"id": a_id, "notes": "n", "priority": "urgent", "dueAt": 5})).await.unwrap();
    assert_eq!((u["card"]["notes"].as_str(), u["card"]["priority"].as_str()), (Some("n"), Some("urgent")));
    let u = c.call("card.update", json!({"id": a_id, "notes": null})).await.unwrap();
    assert!(u["card"].get("notes").is_none());
    c.call("card.markRead", json!({"id": a_id})).await.unwrap();

    let wt = c
        .call("card.create", json!({"projectId": pid, "columnId": cols[0], "agentId": "shell", "worktree": {"branch": "feat/x"}}))
        .await
        .unwrap();
    assert_eq!(wt["card"]["branch"], "feat/x");
    let wt_cwd = PathBuf::from(wt["card"]["cwd"].as_str().unwrap());
    assert!(wt_cwd.ends_with("project-feat-x") && wt_cwd.join(".git").exists());

    // An agent card runs the agent's command with the prompt as an argument,
    // in the card's cwd, with the Muster variables set. A stand-in `codex`
    // on PATH records what it was given.
    let bin = env.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        bin.join("codex"),
        "#!/bin/sh\nfor a; do last=$a; done\necho \"FAKE-CODEX[$last] first=$1 agent=$MUSTER_AGENT_ID dir=$(basename \"$PWD\")\"\nexec sleep 30\n",
    )
    .unwrap();
    Command::new("chmod").arg("755").arg(bin.join("codex")).status().unwrap();
    let ag = c
        .call("card.create", json!({"projectId": pid, "columnId": cols[0], "agentId": "codex", "prompt": "it's a \"test\"", "start": true}))
        .await
        .unwrap();
    let ag_id = ag["card"]["id"].as_str().unwrap().to_string();
    assert_eq!(ag["card"]["title"], "it's a \"test\"");
    c.attach(&ag_id, 100, 30).await;
    // Codex goes through its adapter: instructions as `-c developer_instructions=...`,
    // the prompt last.
    c.wait_output("FAKE-CODEX[it's a \"test\"] first=-c agent=codex dir=project").await;
    c.call("session.detach", json!({"cardId": ag_id})).await.unwrap();
    c.call("card.delete", json!({"id": ag_id, "killSession": true})).await.unwrap();

    let agents = c.call("agent.list", json!({})).await.unwrap();
    let shell = agents["agents"].as_array().unwrap().iter().find(|a| a["id"] == "shell").unwrap().clone();
    assert_eq!(shell["installed"], true);
    let codex = agents["agents"].as_array().unwrap().iter().find(|a| a["id"] == "codex").unwrap().clone();
    assert_eq!(codex["installed"], true, "found through the login shell's PATH");

    // Errors.
    assert_eq!(c.call("card.start", json!({"id": "nope"})).await.unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(c.call("nope.nope", json!({})).await.unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(c.call("card.move", json!({"id": a_id})).await.unwrap_err().code, ErrorCode::Invalid);
    assert_eq!(
        c.call("session.attach", json!({"cardId": a_id, "cols": 80, "rows": 24})).await.unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        c.call("card.create", json!({"projectId": pid, "columnId": cols[0], "agentId": "custom:x"})).await.unwrap_err().code,
        ErrorCode::Invalid
    );

    // A live session blocks project removal; killing it unblocks.
    let s = c.call("card.create", json!({"projectId": pid, "columnId": cols[0], "agentId": "shell", "start": true})).await.unwrap();
    let s_id = s["card"]["id"].as_str().unwrap().to_string();
    assert_eq!(c.call("project.remove", json!({"id": pid})).await.unwrap_err().code, ErrorCode::Conflict);
    // Input to a session nobody is attached to goes straight to tmux.
    c.call("term.write", json!({"cardId": s_id, "data": "echo unattached-$((1+2))\r"})).await.unwrap();
    let mut viewer = Client::connect(port, &env.home).await;
    let id = viewer.send("session.attach", json!({"cardId": s_id, "cols": 90, "rows": 20, "watch": true})).await;
    let r = loop {
        if let Some(r) = viewer.pump().await {
            break r;
        }
    };
    assert_eq!(r.id, id);
    // A watch-only client keeps the session's existing size.
    assert_eq!(r.r.unwrap(), json!({"cols": 120, "rows": 36}));
    viewer.wait_output("unattached-3").await;
    drop(viewer);
    c.call("session.kill", json!({"cardId": s_id})).await.unwrap();
    let exit = c.wait_event("session.exit", &s_id).await;
    assert!(exit.get("exitCode").is_none());

    c.call("column.delete", json!({"id": review_id, "moveCardsTo": cols[2]})).await.unwrap();
    let board = c.call("board.get", json!({"projectId": pid})).await.unwrap();
    assert_eq!(board["columns"].as_array().unwrap().len(), 3);
    let a_now = board["cards"].as_array().unwrap().iter().find(|c| c["id"] == a_id).unwrap().clone();
    assert_eq!(a_now["columnId"], cols[2].as_str());

    c.call("project.remove", json!({"id": pid})).await.unwrap();
    let ev = watcher.wait_event_named("project.changed").await;
    let _ = ev;
    assert!(c.call("project.list", json!({})).await.unwrap()["projects"].as_array().unwrap().is_empty());
}


#[tokio::test(flavor = "current_thread")]
async fn origin_check_and_heartbeat() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut env = Env::new();
    env.env = vec![("MUSTER_PING_MS".into(), "200".into())];
    let port = env.start_daemon();
    let url = format!("ws://127.0.0.1:{port}/ws");
    for (origin, ok) in [
        ("tauri://localhost", true),
        ("http://localhost:1420", true),
        ("http://127.0.0.1:1420", true),
        // Only the exact dev origins: no other port, no portless origin.
        ("http://localhost", false),
        ("http://localhost:3000", false),
        ("https://evil.example", false),
        ("http://localhost.evil.example", false),
        ("http://127.0.0.1:80@evil.example", false),
        ("null", false),
    ] {
        let mut req = url.as_str().into_client_request().unwrap();
        req.headers_mut().insert("Origin", origin.parse().unwrap());
        let res = tokio_tungstenite::connect_async(req).await;
        assert_eq!(res.is_ok(), ok, "origin {origin}");
    }

    // A client that keeps reading answers pings and stays connected.
    let mut live = Client::connect(port, &env.home).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    for _ in 0..8 {
        live.call("project.list", json!({})).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    // A peer that never answers is dropped after two ping periods.
    let (mut dead, _) = tokio_tungstenite::connect_async(url.as_str()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    let mut pings = 0;
    let mut closed = false;
    while let Ok(Some(m)) = tokio::time::timeout(std::time::Duration::from_secs(2), dead.next()).await {
        match m {
            Ok(Message::Ping(_)) => pings += 1,
            Ok(Message::Close(_)) | Err(_) => {
                closed = true;
                break;
            }
            _ => {}
        }
    }
    assert!(pings >= 1 && closed, "pings {pings}, closed {closed}");
}
