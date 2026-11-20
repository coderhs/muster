//! Remote access: the listener behind `tailscale serve`, pairing, device tokens and
//! revocation. A fake `tailscale` stands in for the real one.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::time::{Duration, Instant};

use common::*;
use muster_proto::ErrorCode;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const DNS: &str = "mac.example.ts.net";

const FAKE_TAILSCALE: &str = r#"#!/bin/sh
echo "$@" >> "$(dirname "$0")/tailscale.log"
case "$1" in
  status) echo '{"BackendState":"Running","Self":{"DNSName":"mac.example.ts.net."},"CertDomains":["mac.example.ts.net"]}' ;;
  serve) exit 0 ;;
esac
"#;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn setup() -> (Env, u16) {
    let mut env = Env::new();
    let bin = env.root.join("ts");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("tailscale"), FAKE_TAILSCALE).unwrap();
    assert!(Command::new("chmod").arg("755").arg(bin.join("tailscale")).status().unwrap().success());
    let web = env.root.join("web");
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html><title>Muster</title>").unwrap();
    std::fs::write(web.join("assets/app-1234.js"), "console.log(1)").unwrap();
    let port = free_port();
    env.env.push(("MUSTER_TAILSCALE".into(), bin.join("tailscale").to_string_lossy().into_owned()));
    env.env.push(("MUSTER_WEB_DIR".into(), web.to_string_lossy().into_owned()));
    env.env.push(("MUSTER_REMOTE_PORT".into(), port.to_string()));
    (env, port)
}

fn ts_log(env: &Env) -> String {
    std::fs::read_to_string(env.root.join("ts/tailscale.log")).unwrap_or_default()
}

/// One HTTP/1.1 request; returns (status, headers, body).
fn http(port: u16, req: &str) -> (u16, String, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).unwrap();
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&out[..split]).into_owned();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, head, out[split + 4..].to_vec())
}

fn origin() -> String {
    format!("https://{DNS}:8443")
}

fn pair_post(port: u16, code: &str, origin: &str) -> (u16, Value) {
    let body = json!({"code": code, "name": "Test iPad"}).to_string();
    let (status, _, body) = http(
        port,
        &format!(
            "POST /pair HTTP/1.1\r\nHost: {DNS}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    );
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

/// Opens a WebSocket on the remote port with an Origin header and says hello.
async fn remote_client(port: u16, origin: Option<&str>, token: &str) -> Result<Client, String> {
    let mut req = format!("ws://127.0.0.1:{port}/ws").into_client_request().unwrap();
    if let Some(o) = origin {
        req.headers_mut().insert("Origin", o.parse().unwrap());
    }
    let (ws, _) = tokio_tungstenite::connect_async(req).await.map_err(|e| e.to_string())?;
    let mut c = Client { ws, next: 1, events: vec![], output: vec![], focus: None };
    c.call("hello", json!({"token": token, "client": "mobile", "protocol": 1})).await.map_err(|e| e.msg)?;
    Ok(c)
}

#[tokio::test]
async fn pair_connect_and_revoke() {
    let (mut env, rport) = setup();
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;

    let st = c.call("remote.status", json!({})).await.unwrap();
    assert_eq!(st["enabled"], false);
    assert_eq!(st["webUi"], true);
    assert_eq!(c.call("remote.pair", json!({})).await.unwrap_err().code, ErrorCode::Invalid);
    assert!(TcpStream::connect(("127.0.0.1", rport)).is_err(), "no listener while off");

    // On: listener up, Tailscale asked to serve it.
    let st = c.call("remote.set", json!({"enabled": true})).await.unwrap();
    assert_eq!(st["enabled"], true);
    assert_eq!(st["url"], format!("{}/", origin()));
    assert!(st.get("error").is_none(), "{st}");
    assert!(ts_log(&env).contains(&format!("serve --bg --https=8443 http://127.0.0.1:{rport}")), "{}", ts_log(&env));

    // The web app is served; paths cannot leave its folder.
    let (status, head, body) = http(rport, "GET / HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(status, 200);
    assert!(head.contains("text/html"));
    assert!(String::from_utf8_lossy(&body).contains("<title>Muster</title>"));
    let (status, head, _) = http(rport, "GET /assets/app-1234.js HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(status, 200);
    assert!(head.contains("immutable"));
    assert_eq!(http(rport, "GET /assets/missing.js HTTP/1.1\r\nHost: x\r\n\r\n").0, 404);
    assert_eq!(http(rport, "GET /..%2fhome/token.js HTTP/1.1\r\nHost: x\r\n\r\n").0, 404);

    // The master token never works remotely, and the origin must be the tailnet page.
    let token = std::fs::read_to_string(env.home.join("token")).unwrap();
    assert!(remote_client(rport, None, token.trim()).await.is_err());
    assert!(remote_client(rport, Some("https://evil.example"), token.trim()).await.is_err());
    assert!(remote_client(rport, Some(&origin()), token.trim()).await.is_err());

    // Pairing: a one-time code traded for a device token.
    let link = c.call("remote.pair", json!({})).await.unwrap();
    let url = link["url"].as_str().unwrap();
    let code = url.split("#pair=").nth(1).unwrap();
    assert!(url.starts_with(&origin()));
    assert_eq!(pair_post(rport, code, "https://evil.example").0, 403, "wrong origin");
    let (status, paired) = pair_post(rport, code, &origin());
    assert_eq!(status, 200, "{paired}");
    let dtoken = paired["token"].as_str().unwrap().to_string();
    assert_eq!(pair_post(rport, code, &origin()).0, 403, "a code works once");

    // The device works like the app, except for managing remote access.
    let mut d = remote_client(rport, Some(&origin()), &dtoken).await.unwrap();
    d.call("project.list", json!({})).await.unwrap();
    assert_eq!(d.call("remote.pair", json!({})).await.unwrap_err().code, ErrorCode::Unauthorized);
    // ...and its token is useless on the local port.
    assert!(Client::connect_token(port, &dtoken).await.is_err());

    // The file viewer: inside the card's project for both; a file elsewhere on the Mac
    // only for the app, and only once the user allows it there.
    std::fs::create_dir_all(env.root.join("project/src")).unwrap();
    std::fs::write(env.root.join("project/src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(env.root.join("elsewhere.txt"), "private\n").unwrap();
    let pid = c.call("project.add", json!({"path": env.root.join("project")})).await.unwrap()["project"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let col =
        c.call("board.get", json!({"projectId": pid})).await.unwrap()["columns"][0]["id"].as_str().unwrap().to_string();
    let card = c.call("card.create", json!({"projectId": pid, "columnId": col, "agentId": "shell"})).await.unwrap()
        ["card"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let inside = json!({"id": card, "path": "src/main.rs"});
    let outside = json!({"id": card, "path": env.root.join("elsewhere.txt")});
    for client in [&mut c, &mut d] {
        let f = client.call("card.file", inside.clone()).await.unwrap();
        assert_eq!((f["path"].as_str(), f["content"].as_str()), (Some("src/main.rs"), Some("fn main() {}\n")));
        assert_eq!(client.call("card.file", outside.clone()).await.unwrap_err().code, ErrorCode::Invalid);
    }
    let e = d.call("settings.update", json!({"anyFilePath": true})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Unauthorized);
    let s = c.call("settings.update", json!({"anyFilePath": true})).await.unwrap();
    assert_eq!(s["settings"]["anyFilePath"], true);
    assert_eq!(c.call("card.file", outside.clone()).await.unwrap()["content"], "private\n");
    assert_eq!(d.call("card.file", outside.clone()).await.unwrap_err().code, ErrorCode::Invalid);

    let st = c.call("remote.status", json!({})).await.unwrap();
    let devices = st["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["name"], "Test iPad");
    assert!(devices[0]["lastSeenAt"].is_i64());
    let dev_id = devices[0]["id"].as_str().unwrap().to_string();

    // Revoking closes the open connection and refuses the token.
    c.call("remote.revoke", json!({"id": dev_id})).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        use futures_util::StreamExt;
        match tokio::time::timeout(Duration::from_millis(200), d.ws.next()).await {
            Ok(None) | Ok(Some(Err(_))) => break,
            Ok(Some(Ok(m))) if m.is_close() => break,
            _ => assert!(Instant::now() < deadline, "revoked device stayed connected"),
        }
    }
    assert!(remote_client(rport, Some(&origin()), &dtoken).await.is_err());

    // Off: Tailscale stops serving and the port closes.
    c.call("remote.set", json!({"enabled": false})).await.unwrap();
    assert!(ts_log(&env).contains("serve --https=8443 off"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect(("127.0.0.1", rport)).is_ok() {
        assert!(Instant::now() < deadline, "remote listener still open");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn remote_access_comes_back_after_restart() {
    let (mut env, rport) = setup();
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    c.call("remote.set", json!({"enabled": true})).await.unwrap();
    drop(c);
    env.crash_daemon();
    let port = env.start_daemon();
    let mut c = Client::connect(port, &env.home).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(("127.0.0.1", rport)).is_err() {
        assert!(Instant::now() < deadline, "remote listener did not come back");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(c.call("remote.status", json!({})).await.unwrap()["enabled"], true);
}
