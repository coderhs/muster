//! `muster attach`: raw-mode passthrough between this terminal and a session.

use std::io::Write;

use muster_proto::*;
use tokio::io::AsyncReadExt;
use tokio::signal::unix::{signal, SignalKind};

use crate::client::{Client, Incoming};

const DETACH_KEY: u8 = 0x1d; // Ctrl-]

struct RawMode {
    saved: Option<libc::termios>,
}

impl RawMode {
    fn enable() -> RawMode {
        unsafe {
            if libc::isatty(0) == 0 {
                return RawMode { saved: None };
            }
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut t) != 0 {
                return RawMode { saved: None };
            }
            let saved = t;
            libc::cfmakeraw(&mut t);
            libc::tcsetattr(0, libc::TCSANOW, &t);
            RawMode { saved: Some(saved) }
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if let Some(t) = self.saved {
            unsafe {
                libc::tcsetattr(0, libc::TCSANOW, &t);
            }
        }
    }
}

fn term_size() -> (u16, u16) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
            (ws.ws_col, ws.ws_row)
        } else {
            (80, 24)
        }
    }
}

/// Takes the longest valid UTF-8 prefix out of `buf`, leaving an incomplete
/// trailing sequence for the next read. Invalid bytes are replaced.
fn take_utf8(buf: &mut Vec<u8>) -> String {
    match std::str::from_utf8(buf) {
        Ok(s) => {
            let s = s.to_string();
            buf.clear();
            s
        }
        Err(e) if e.error_len().is_none() => {
            let n = e.valid_up_to();
            let s = String::from_utf8_lossy(&buf[..n]).into_owned();
            buf.drain(..n);
            s
        }
        Err(_) => {
            let s = String::from_utf8_lossy(buf).into_owned();
            buf.clear();
            s
        }
    }
}

pub async fn attach(mut c: Client, card_id: &str, watch: bool) -> Result<i32, String> {
    let (cols, rows) = term_size();
    let _: SizeResult =
        c.call(Method::SessionAttach(SessionAttachParams { card_id: card_id.to_string(), cols, rows, watch })).await?;
    let raw = RawMode::enable();
    let mut stdout = std::io::stdout();
    let mut stdin = tokio::io::stdin();
    let mut winch = signal(SignalKind::window_change()).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 4096];
    let mut message = String::new();
    let mut code = 0;
    let mut carry: Vec<u8> = Vec::new();

    loop {
        tokio::select! {
            n = stdin.read(&mut buf) => {
                let n = n.unwrap_or(0);
                if n == 0 {
                    break;
                }
                let data = &buf[..n];
                if let Some(i) = data.iter().position(|&b| b == DETACH_KEY) {
                    if !watch && i > 0 {
                        let s = String::from_utf8_lossy(&data[..i]).into_owned();
                        c.send(Method::TermWrite(TermWriteParams { card_id: card_id.to_string(), data: s, raw: false })).await?;
                    }
                    message = "[detached]".into();
                    let _ = c.send(Method::SessionDetach(CardIdParams { card_id: card_id.to_string() })).await;
                    break;
                }
                if !watch {
                    carry.extend_from_slice(data);
                    let s = take_utf8(&mut carry);
                    c.send(Method::TermWrite(TermWriteParams { card_id: card_id.to_string(), data: s, raw: false })).await?;
                }
            }
            _ = winch.recv() => {
                let (cols, rows) = term_size();
                c.send(Method::TermResize(TermResizeParams { card_id: card_id.to_string(), cols, rows })).await?;
            }
            inc = c.next() => match inc {
                Incoming::Output(sid, data) if sid == card_id => {
                    let _ = stdout.write_all(&data);
                    let _ = stdout.flush();
                }
                Incoming::Event(e) if e.m == "session.exit" && e.p["cardId"] == card_id => {
                    message = match e.p["exitCode"].as_i64() {
                        Some(code) => format!("[session exited with code {code}]"),
                        None => "[session ended]".into(),
                    };
                    break;
                }
                Incoming::Response(r) if !r.ok => {
                    if let Some(e) = r.e {
                        message = format!("[error: {}]", e.msg);
                        code = 1;
                        if e.code != ErrorCode::Invalid {
                            break;
                        }
                    }
                }
                Incoming::Closed => {
                    message = "[daemon connection closed]".into();
                    code = 1;
                    break;
                }
                _ => {}
            }
        }
    }
    drop(raw);
    // Leave the alternate screen and reset attributes the session may have set.
    let _ = stdout.write_all(b"\x1b[?1049l\x1b[0m\x1b[?25h\x1b[?1000l\x1b[?1002l\x1b[?1006l\x1b[?2004l\r\n");
    if !message.is_empty() {
        let _ = writeln!(stdout, "{message}");
    }
    let _ = stdout.flush();
    Ok(code)
}
