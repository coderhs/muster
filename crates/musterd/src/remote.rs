//! Remote access: phones, tablets and other computers reach the daemon through
//! `tailscale serve`.
//!
//! When it is switched on, a second listener runs on a fixed loopback port. Tailscale
//! terminates HTTPS on the Mac's tailnet name and proxies to it. The listener serves the
//! built front end, a `/pair` endpoint and the same WebSocket API as the local one. Only
//! device tokens are accepted there: the master token and card tokens stay on the Mac.
//!
//! A device is paired by opening a one-time link (shown as a QR code in Settings). The
//! page trades the code for its own token, which can be revoked on its own.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use muster_core::util::{err, hex, invalid, now_ms, random_bytes, sha256};
use muster_core::Result;
use muster_proto::{ErrorCode, Event, RemotePairResult, RemoteStatus, TailscaleInfo};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role as WsRole, WebSocketConfig};
use tokio_tungstenite::WebSocketStream;

use crate::log;
use crate::state::{Conn, State};

const META_KEY: &str = "remote";
const DEFAULT_PORT: u16 = 7318;
const DEFAULT_HTTPS_PORT: u16 = 8443;
const PAIR_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 4 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_DEVICE_NAME: usize = 60;
/// The same policy as the desktop window, for the page served to devices.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
                   font-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    enabled: bool,
    port: u16,
    https_port: u16,
}

impl Default for Config {
    fn default() -> Self {
        // Tests run on a free port.
        let port = std::env::var("MUSTER_REMOTE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
        Config { enabled: false, port, https_port: DEFAULT_HTTPS_PORT }
    }
}

#[derive(Default)]
pub struct Remote {
    config: Mutex<Config>,
    listener: Mutex<Option<tokio::task::JoinHandle<()>>>,
    tailscale: Mutex<TailscaleInfo>,
    error: Mutex<Option<String>>,
    /// One-time pairing codes and when they were made.
    pair_codes: Mutex<HashMap<String, Instant>>,
    /// Open device connections: connection id -> (device id, connection).
    conns: Mutex<HashMap<u64, (String, Conn)>>,
    /// Serialises switching on and off.
    switching: tokio::sync::Mutex<()>,
}

impl Remote {
    /// The origin a device's page has: `https://<tailnet name>[:port]`.
    pub fn origin(&self) -> Option<String> {
        let dns = self.tailscale.lock().unwrap().dns_name.clone()?;
        let port = self.config.lock().unwrap().https_port;
        Some(if port == 443 { format!("https://{dns}") } else { format!("https://{dns}:{port}") })
    }

    fn origin_allowed(&self, origin: Option<&str>) -> bool {
        let Some(origin) = origin else { return false };
        if self.origin().as_deref() == Some(origin) {
            return true;
        }
        // For testing without Tailscale: exact extra origins.
        std::env::var("MUSTER_REMOTE_ORIGINS")
            .map(|v| v.split(',').map(str::trim).any(|o| !o.is_empty() && o == origin))
            .unwrap_or(false)
    }

    pub fn register_conn(&self, conn_id: u64, device_id: &str, conn: Conn) {
        self.conns.lock().unwrap().insert(conn_id, (device_id.to_string(), conn));
    }

    pub fn unregister_conn(&self, conn_id: u64) {
        self.conns.lock().unwrap().remove(&conn_id);
    }

    /// Closes the open connections of one device, or of all devices.
    fn close_conns(&self, device_id: Option<&str>) {
        let conns = self.conns.lock().unwrap();
        for (dev, conn) in conns.values() {
            if device_id.is_none_or(|d| d == dev) {
                conn.kill.notify_one();
            }
        }
    }
}

pub fn status(st: &State) -> RemoteStatus {
    let r = &st.remote;
    let cfg = r.config.lock().unwrap().clone();
    let tailscale = r.tailscale.lock().unwrap().clone();
    let url = if cfg.enabled && tailscale.https { r.origin().map(|o| format!("{o}/")) } else { None };
    RemoteStatus {
        enabled: cfg.enabled,
        port: cfg.port,
        https_port: cfg.https_port,
        url,
        web_ui: web_dir().is_some(),
        tailscale,
        error: r.error.lock().unwrap().clone(),
        devices: st.store.lock().unwrap().list_devices().unwrap_or_default(),
    }
}

fn broadcast(st: &State) {
    st.clients.broadcast(&Event::RemoteChanged(status(st)));
}

/// Reads the saved configuration and, when remote access was on, starts it again.
pub async fn init(st: &Arc<State>) {
    let saved = st.store.lock().unwrap().meta_get(META_KEY).ok().flatten();
    let mut cfg: Config = saved.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    if let Some(port) = std::env::var("MUSTER_REMOTE_PORT").ok().and_then(|p| p.parse().ok()) {
        cfg.port = port;
    }
    let enabled = cfg.enabled;
    *st.remote.config.lock().unwrap() = cfg;
    if enabled {
        let st = st.clone();
        tokio::spawn(async move {
            let _ = set_enabled(&st, true).await;
        });
    }
}

/// Switches remote access on or off: the listener, `tailscale serve`, open device
/// connections. A Tailscale problem does not fail the call; it is reported in
/// `RemoteStatus::error`.
///
/// Boxed: the listener it starts leads back to the request handler, which calls this.
pub fn set_enabled(st: &Arc<State>, on: bool) -> Pin<Box<dyn Future<Output = Result<RemoteStatus>> + Send + '_>> {
    Box::pin(set_enabled_inner(st, on))
}

async fn set_enabled_inner(st: &Arc<State>, on: bool) -> Result<RemoteStatus> {
    let r = &st.remote;
    let _guard = r.switching.lock().await;
    let cfg = {
        let mut c = r.config.lock().unwrap();
        c.enabled = on;
        c.clone()
    };
    let saved = serde_json::to_string(&cfg).expect("config serialises");
    st.store.lock().unwrap().meta_set(META_KEY, &saved)?;
    *r.error.lock().unwrap() = None;
    let ts = tailscale_path();

    if on {
        if r.listener.lock().unwrap().is_none() {
            match TcpListener::bind(("127.0.0.1", cfg.port)).await {
                Ok(l) => {
                    log!("remote access: listening on 127.0.0.1:{}", cfg.port);
                    *r.listener.lock().unwrap() = Some(tokio::spawn(serve(st.clone(), l)));
                }
                Err(e) => {
                    *r.error.lock().unwrap() = Some(format!("cannot listen on 127.0.0.1:{}: {e}", cfg.port));
                }
            }
        }
        let info = tailscale_info(ts.as_deref()).await;
        *r.tailscale.lock().unwrap() = info.clone();
        if r.error.lock().unwrap().is_none() {
            let problem = if web_dir().is_none() {
                Some("the web app was not found next to the daemon; rebuild Muster".to_string())
            } else if !info.found {
                Some("Tailscale is not installed".to_string())
            } else if !info.running {
                Some("Tailscale is not connected; open Tailscale and log in".to_string())
            } else if !info.https {
                Some(
                    "HTTPS certificates are off for your tailnet; turn on MagicDNS and HTTPS in the \
                     Tailscale admin console (DNS page)"
                        .to_string(),
                )
            } else {
                serve_on(ts.as_deref().unwrap(), cfg.https_port, cfg.port).await.err()
            };
            *r.error.lock().unwrap() = problem;
        }
    } else {
        if let Some(h) = r.listener.lock().unwrap().take() {
            h.abort();
            log!("remote access: stopped");
        }
        r.close_conns(None);
        r.pair_codes.lock().unwrap().clear();
        if let Some(ts) = ts.as_deref() {
            if let Err(e) = serve_off(ts, cfg.https_port).await {
                log!("remote access: {e}");
            }
        }
    }
    broadcast(st);
    Ok(status(st))
}

/// Makes a one-time pairing link.
pub fn pair(st: &State) -> Result<RemotePairResult> {
    let r = &st.remote;
    if !r.config.lock().unwrap().enabled {
        return Err(invalid("remote access is off"));
    }
    let Some(origin) = r.origin().filter(|_| r.tailscale.lock().unwrap().https) else {
        return Err(err(ErrorCode::Unavailable, "Tailscale is not serving Muster yet"));
    };
    let code = hex(&random_bytes(16));
    let mut codes = r.pair_codes.lock().unwrap();
    codes.retain(|_, t| t.elapsed() < PAIR_TTL);
    codes.insert(code.clone(), Instant::now());
    Ok(RemotePairResult { url: format!("{origin}/#pair={code}"), expires_at: now_ms() + PAIR_TTL.as_millis() as i64 })
}

/// Forgets a device and closes its connections.
pub fn revoke(st: &State, id: &str) -> Result<RemoteStatus> {
    if !st.store.lock().unwrap().delete_device(id)? {
        return Err(err(ErrorCode::NotFound, "no such device"));
    }
    st.remote.close_conns(Some(id));
    log!("remote access: revoked device {id}");
    broadcast(st);
    Ok(status(st))
}

/// The device a token belongs to.
pub fn device_for_token(st: &State, token: &str) -> Option<(String, String)> {
    if token.is_empty() {
        return None;
    }
    let hash = hex(&sha256(token.as_bytes()));
    let id = st.store.lock().unwrap().device_for_token_hash(&hash).ok().flatten()?;
    Some((id, hash))
}

/// Whether a device id still owns this token hash (it was not revoked).
pub fn device_valid(st: &State, id: &str, hash: &str) -> bool {
    st.store.lock().unwrap().device_for_token_hash(hash).ok().flatten().as_deref() == Some(id)
}

pub fn enabled(st: &State) -> bool {
    st.remote.config.lock().unwrap().enabled
}

pub fn device_seen(st: &State, id: &str) {
    let _ = st.store.lock().unwrap().touch_device(id, now_ms());
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

async fn serve(st: Arc<State>, listener: TcpListener) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                log!("remote accept: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        // `tailscale serve` connects from loopback; nothing else should reach this port.
        if !peer.ip().is_loopback() {
            continue;
        }
        let _ = stream.set_nodelay(true);
        let st = st.clone();
        tokio::spawn(async move {
            if let Err(e) = http(st, stream).await {
                log!("remote connection: {e}");
            }
        });
    }
}

struct Head {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    /// Bytes read past the end of the head.
    rest: Vec<u8>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn has_token(&self, name: &str, token: &str) -> bool {
        self.header(name).is_some_and(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
    }
}

async fn read_head(stream: &mut TcpStream) -> std::result::Result<Option<Head>, String> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    loop {
        let n = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(&buf) {
            Ok(httparse::Status::Complete(len)) => {
                return Ok(Some(Head {
                    method: req.method.unwrap_or("").to_string(),
                    path: req.path.unwrap_or("/").to_string(),
                    headers: req
                        .headers
                        .iter()
                        .map(|h| (h.name.to_string(), String::from_utf8_lossy(h.value).into_owned()))
                        .collect(),
                    rest: buf[len..].to_vec(),
                }))
            }
            Ok(httparse::Status::Partial) if buf.len() < MAX_HEAD => continue,
            Ok(httparse::Status::Partial) => return Err("request head too large".into()),
            Err(e) => return Err(format!("bad request: {e}")),
        }
    }
}

async fn http(st: Arc<State>, mut stream: TcpStream) -> std::result::Result<(), String> {
    let head = match tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut stream)).await {
        Ok(Ok(Some(h))) => h,
        Ok(Ok(None)) | Err(_) => return Ok(()),
        Ok(Err(e)) => {
            respond(&mut stream, 400, "text/plain", &[], b"bad request").await;
            return Err(e);
        }
    };
    let path = head.path.split(['?', '#']).next().unwrap_or("/").to_string();
    match (head.method.as_str(), path.as_str()) {
        ("GET", "/ws") if head.has_token("upgrade", "websocket") && head.has_token("connection", "upgrade") => {
            websocket(st, stream, head).await
        }
        ("POST", "/pair") => {
            pair_request(&st, &mut stream, head).await;
            Ok(())
        }
        ("GET" | "HEAD", _) => {
            static_file(&mut stream, &path, head.method == "HEAD").await;
            Ok(())
        }
        _ => {
            respond(&mut stream, 405, "text/plain", &[], b"method not allowed").await;
            Ok(())
        }
    }
}

async fn websocket(st: Arc<State>, mut stream: TcpStream, head: Head) -> std::result::Result<(), String> {
    if !st.remote.origin_allowed(head.header("origin")) {
        respond(&mut stream, 403, "text/plain", &[], b"origin not allowed").await;
        return Ok(());
    }
    let Some(key) = head.header("sec-websocket-key") else {
        respond(&mut stream, 400, "text/plain", &[], b"missing Sec-WebSocket-Key").await;
        return Ok(());
    };
    let resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\r\n",
        derive_accept_key(key.as_bytes())
    );
    stream.write_all(resp.as_bytes()).await.map_err(|e| e.to_string())?;
    let mut cfg = WebSocketConfig::default();
    cfg.max_message_size = Some(16 << 20);
    cfg.max_frame_size = Some(16 << 20);
    let ws = WebSocketStream::from_partially_read(stream, head.rest, WsRole::Server, Some(cfg)).await;
    crate::server::connection(st, ws, crate::server::Listener::Remote).await
}

#[derive(Deserialize)]
struct PairRequest {
    code: String,
    #[serde(default)]
    name: String,
}

#[derive(Serialize)]
struct PairResponse {
    token: String,
    #[serde(rename = "deviceId")]
    device_id: String,
}

async fn pair_request(st: &Arc<State>, stream: &mut TcpStream, head: Head) {
    if !st.remote.origin_allowed(head.header("origin")) {
        return respond(stream, 403, "text/plain", &[], b"origin not allowed").await;
    }
    let len: usize = head.header("content-length").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    if len > MAX_BODY {
        return respond(stream, 413, "text/plain", &[], b"too large").await;
    }
    let mut body = head.rest;
    while body.len() < len {
        let mut chunk = vec![0u8; len - body.len()];
        match tokio::time::timeout(HEAD_TIMEOUT, stream.read(&mut chunk)).await {
            Ok(Ok(n)) if n > 0 => body.extend_from_slice(&chunk[..n]),
            _ => return,
        }
    }
    body.truncate(len);
    let Ok(req) = serde_json::from_slice::<PairRequest>(&body) else {
        return respond(stream, 400, "text/plain", &[], b"bad request").await;
    };
    if st.hello_locked() {
        tokio::time::sleep(Duration::from_secs(1)).await;
        return respond(stream, 429, "text/plain", &[], b"too many attempts; try again later").await;
    }
    let valid = {
        let mut codes = st.remote.pair_codes.lock().unwrap();
        codes.retain(|_, t| t.elapsed() < PAIR_TTL);
        codes.remove(&req.code).is_some()
    };
    if !valid {
        st.hello_failed();
        return respond(stream, 403, "text/plain", &[], b"this pairing link has expired or was already used").await;
    }
    let name: String = req.name.trim().chars().filter(|c| !c.is_control()).take(MAX_DEVICE_NAME).collect();
    let name = if name.is_empty() { "Device".to_string() } else { name };
    let token = hex(&random_bytes(32));
    let added = st.store.lock().unwrap().add_device(&name, &hex(&sha256(token.as_bytes())));
    let device = match added {
        Ok(d) => d,
        Err(e) => {
            log!("remote access: saving device: {}", e.msg);
            return respond(stream, 500, "text/plain", &[], b"could not save the device").await;
        }
    };
    log!("remote access: paired device {} ({})", device.id, device.name);
    broadcast(st);
    let body = serde_json::to_vec(&PairResponse { token, device_id: device.id }).expect("serialise");
    respond(stream, 200, "application/json", &[("Cache-Control", "no-store")], &body).await;
}

/// Where the built front end is: `MUSTER_WEB_DIR`, the app bundle's resources, or the
/// repository's `app/dist` for development builds.
pub fn web_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(d) = std::env::var_os("MUSTER_WEB_DIR").filter(|d| !d.is_empty()) {
        candidates.push(PathBuf::from(d));
    }
    if let Some(exe) = std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()) {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("../Resources/web"));
            // The Debian package: binaries in /usr/bin, resources in /usr/lib/Muster.
            candidates.push(dir.join("../lib/Muster/web"));
            candidates.push(dir.join("web"));
            candidates.push(dir.join("../../app/dist"));
        }
    }
    candidates.into_iter().find(|d| d.join("index.html").is_file()).and_then(|d| d.canonicalize().ok())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Resolves a request path to a file inside `root`. Anything unusual is refused rather
/// than decoded: the front end's own file names are plain.
fn resolve_static(root: &Path, path: &str) -> Option<PathBuf> {
    let rel = path.trim_start_matches('/');
    if rel.contains("..") || rel.contains('\\') || rel.contains('%') || rel.contains('\0') {
        return None;
    }
    let file = if rel.is_empty() { root.join("index.html") } else { root.join(rel) };
    let file = file.canonicalize().ok()?;
    (file.starts_with(root) && file.is_file()).then_some(file)
}

async fn static_file(stream: &mut TcpStream, path: &str, head_only: bool) {
    let Some(root) = web_dir() else {
        return respond(stream, 503, "text/plain", &[], b"the Muster web app is not installed").await;
    };
    let file = match resolve_static(&root, path) {
        Some(f) => f,
        // An unknown path without an extension is a page of the app.
        None if !path.rsplit('/').next().unwrap_or("").contains('.') => root.join("index.html"),
        None => return respond(stream, 404, "text/plain", &[], b"not found").await,
    };
    let read = tokio::task::spawn_blocking({
        let file = file.clone();
        move || std::fs::read(file)
    });
    let Ok(Ok(body)) = read.await else {
        return respond(stream, 404, "text/plain", &[], b"not found").await;
    };
    let cache = if path.starts_with("/assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    let ctype = content_type(&file);
    let headers = [("Cache-Control", cache), ("Content-Security-Policy", CSP)];
    if head_only {
        return respond_len(stream, 200, ctype, &headers, body.len(), &[]).await;
    }
    respond(stream, 200, ctype, &headers, &body).await
}

async fn respond(stream: &mut TcpStream, code: u16, ctype: &str, extra: &[(&str, &str)], body: &[u8]) {
    respond_len(stream, code, ctype, extra, body.len(), body).await
}

async fn respond_len(stream: &mut TcpStream, code: u16, ctype: &str, extra: &[(&str, &str)], len: usize, body: &[u8]) {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let mut head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {len}\r\nConnection: close\r\n\
         X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\n"
    );
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body).await;
    let _ = stream.flush().await;
}

// ---------------------------------------------------------------------------
// Tailscale
// ---------------------------------------------------------------------------

/// The `tailscale` command. launchd gives the daemon a minimal PATH, so known places
/// are tried; `MUSTER_TAILSCALE` overrides them.
fn tailscale_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MUSTER_TAILSCALE").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p)).filter(|p| p.is_file());
    }
    [
        "/usr/local/bin/tailscale",
        "/usr/bin/tailscale",
        "/opt/homebrew/bin/tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

async fn run_tailscale(ts: &Path, args: &[&str]) -> std::result::Result<String, String> {
    let mut cmd = tokio::process::Command::new(ts);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| format!("running tailscale: {e}"))?;
    // `tailscale serve` waits, printing a link, when Serve is not enabled for the tailnet.
    match tokio::time::timeout(Duration::from_secs(20), child.wait_with_output()).await {
        Ok(Ok(out)) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(Ok(out)) => {
            let text = format!("{}{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
            Err(format!("tailscale {}: {}", args.first().unwrap_or(&""), text.trim()))
        }
        Ok(Err(e)) => Err(format!("running tailscale: {e}")),
        Err(_) => Err(format!(
            "tailscale {} did not finish; Serve may need to be enabled for your tailnet (run `tailscale serve` \
             in a terminal to see the link)",
            args.first().unwrap_or(&"")
        )),
    }
}

async fn tailscale_info(ts: Option<&Path>) -> TailscaleInfo {
    let Some(ts) = ts else { return TailscaleInfo::default() };
    let mut info = TailscaleInfo { found: true, ..Default::default() };
    let Ok(out) = run_tailscale(ts, &["status", "--json"]).await else { return info };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&out) else { return info };
    info.running = v["BackendState"] == "Running";
    info.dns_name =
        v["Self"]["DNSName"].as_str().map(|s| s.trim_end_matches('.').to_string()).filter(|s| !s.is_empty());
    info.https = v["CertDomains"].as_array().is_some_and(|a| !a.is_empty());
    info
}

async fn serve_on(ts: &Path, https_port: u16, port: u16) -> std::result::Result<(), String> {
    let https = format!("--https={https_port}");
    let target = format!("http://127.0.0.1:{port}");
    run_tailscale(ts, &["serve", "--bg", &https, &target]).await.map(|_| ())
}

async fn serve_off(ts: &Path, https_port: u16) -> std::result::Result<(), String> {
    let https = format!("--https={https_port}");
    run_tailscale(ts, &["serve", &https, "off"]).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_paths_stay_inside_the_root() {
        let dir = std::env::temp_dir().join(format!("muster-web-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("index.html"), "<html>").unwrap();
        std::fs::write(dir.join("assets/app.js"), "x").unwrap();
        let root = dir.canonicalize().unwrap();
        assert_eq!(resolve_static(&root, "/"), Some(root.join("index.html")));
        assert_eq!(resolve_static(&root, "/assets/app.js"), Some(root.join("assets/app.js")));
        assert_eq!(resolve_static(&root, "/../etc/passwd"), None);
        assert_eq!(resolve_static(&root, "/assets/%2e%2e/x"), None);
        assert_eq!(resolve_static(&root, "/missing.js"), None);
        assert_eq!(resolve_static(&root, "/assets"), None);
    }

    #[test]
    fn content_types() {
        assert_eq!(content_type(Path::new("a.js")), "text/javascript; charset=utf-8");
        assert_eq!(content_type(Path::new("m.webmanifest")), "application/manifest+json");
        assert_eq!(content_type(Path::new("x")), "application/octet-stream");
    }
}
