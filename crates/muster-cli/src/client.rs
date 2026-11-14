//! Minimal protocol client.

use std::path::PathBuf;

use futures_util::{SinkExt, StreamExt};
use muster_proto::*;
use serde::de::DeserializeOwned;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct Client {
    pub ws: Ws,
    next_id: u64,
    pub daemon_version: String,
    pub pid: u64,
    pub port: u16,
}

pub fn muster_home() -> PathBuf {
    if let Some(h) = std::env::var_os("MUSTER_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(h);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join("Library/Application Support/Muster")
}

/// Incoming traffic other than the response being waited for.
pub enum Incoming {
    Response(ResponseFrame),
    Event(EventFrame),
    Output(String, Vec<u8>),
    Closed,
}

impl Client {
    pub async fn connect() -> Result<Client, String> {
        // Inside a card: the card's own token and the daemon file it was given. That
        // token only allows acting for that card. Elsewhere: the master token.
        let card_token = std::env::var("MUSTER_CARD_TOKEN").ok().filter(|t| !t.is_empty());
        let home = muster_home();
        let daemon_file = match (&card_token, std::env::var_os("MUSTER_DAEMON_FILE")) {
            (Some(_), Some(f)) => PathBuf::from(f),
            _ => home.join("daemon.json"),
        };
        let info = std::fs::read_to_string(&daemon_file)
            .map_err(|_| format!("daemon not running (no {})", daemon_file.display()))?;
        let info: serde_json::Value = serde_json::from_str(&info).map_err(|e| format!("daemon.json: {e}"))?;
        let port = info["port"].as_u64().ok_or("daemon.json has no port")? as u16;
        let pid = info["pid"].as_u64().unwrap_or(0);
        let token = match card_token {
            Some(t) => t,
            None => std::fs::read_to_string(home.join("token")).map_err(|e| format!("token: {e}"))?,
        };
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws"))
            .await
            .map_err(|e| format!("cannot reach daemon on port {port}: {e}"))?;
        let mut c = Client { ws, next_id: 1, daemon_version: String::new(), pid, port };
        let hello: HelloResult = c
            .call(Method::Hello(HelloParams {
                token: token.trim().to_string(),
                client: ClientKind::Cli,
                protocol: PROTOCOL_VERSION,
            }))
            .await?;
        c.daemon_version = hello.daemon_version;
        Ok(c)
    }

    /// Sends a request without waiting; returns its id.
    pub async fn send(&mut self, m: Method) -> Result<u64, String> {
        let id = self.next_id;
        self.next_id += 1;
        let text = serde_json::to_string(&m.to_frame(id)).map_err(|e| e.to_string())?;
        self.ws.send(Message::text(text)).await.map_err(|e| e.to_string())?;
        Ok(id)
    }

    pub async fn next(&mut self) -> Incoming {
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Text(t))) => match serde_json::from_str::<ServerFrame>(t.as_str()) {
                    Ok(ServerFrame::Response(r)) => return Incoming::Response(r),
                    Ok(ServerFrame::Event(e)) => return Incoming::Event(e),
                    Err(_) => continue,
                },
                Some(Ok(Message::Binary(b))) => {
                    if let Some((sid, data)) = decode_term_frame(&b) {
                        return Incoming::Output(sid.to_string(), data.to_vec());
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return Incoming::Closed,
                Some(Ok(_)) => continue,
            }
        }
    }

    /// Sends a request and waits for its response, discarding other traffic.
    pub async fn call<T: DeserializeOwned>(&mut self, m: Method) -> Result<T, String> {
        let name = m.name();
        let id = self.send(m).await?;
        loop {
            match self.next().await {
                Incoming::Response(r) if r.id == id => {
                    return if r.ok {
                        serde_json::from_value(r.r.unwrap_or(serde_json::Value::Null))
                            .map_err(|e| format!("{name}: unexpected result: {e}"))
                    } else {
                        let e = r.e.unwrap_or(ErrorBody::new(ErrorCode::Internal, "no error body"));
                        Err(format!("{name}: {} ({:?})", e.msg, e.code))
                    };
                }
                Incoming::Closed => return Err("connection closed".into()),
                _ => continue,
            }
        }
    }
}
