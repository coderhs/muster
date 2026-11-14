//! Shared paging for JSONL transcripts: an adapter supplies a line parser, this module
//! does the backwards paging, merging and byte budget (same contract as the Claude Code
//! reader: newest page first, `older` / `end` offsets).

use std::io;
use std::path::Path;

use crate::tail::{lines_after, ReverseLines};
use crate::types::*;

/// Upper bound of bytes read from disk for one page.
pub(crate) const SCAN_BUDGET: u64 = 32 * 1024 * 1024;

/// One transcript line reduced to chat content. Lines with the same `key` and role that
/// are adjacent merge into one message.
pub(crate) struct Line {
    pub key: String,
    pub role: ChatRole,
    pub timestamp: Option<String>,
    pub model: Option<String>,
    pub parts: Vec<ChatPart>,
}

pub(crate) type Parser<'a> = &'a dyn Fn(&[u8], u64) -> Option<Line>;

fn merges(msgs: &[ChatMessage], l: &Line) -> bool {
    msgs.last()
        .is_some_and(|m| m.id == l.key && m.role == l.role)
}

fn to_msg(l: Line) -> ChatMessage {
    ChatMessage {
        id: l.key,
        role: l.role,
        timestamp: l.timestamp,
        model: l.model,
        parts: l.parts,
    }
}

pub(crate) fn read_page(
    path: &Path,
    before: Option<u64>,
    max_messages: usize,
    parse: Parser,
) -> io::Result<ChatPage> {
    let mut r = ReverseLines::open(path, before)?;
    let end = if before.is_none() {
        r.end
    } else {
        std::fs::metadata(path)?.len()
    };
    let mut msgs: Vec<ChatMessage> = Vec::new();
    let mut older = None;
    let max = max_messages.max(1);
    while let Some((off, line)) = r.next_line()? {
        if let Some(l) = parse(&line, off).filter(|l| !l.parts.is_empty()) {
            if merges(&msgs, &l) {
                let last = msgs.last_mut().unwrap();
                let mut parts = l.parts;
                parts.append(&mut last.parts);
                last.parts = parts;
                if l.timestamp.is_some() {
                    last.timestamp = l.timestamp;
                }
            } else if msgs.len() >= max {
                older = Some(off + line.len() as u64 + 1);
                break;
            } else {
                msgs.push(to_msg(l));
            }
        }
        if r.bytes_read > SCAN_BUDGET {
            older = Some(off);
            break;
        }
    }
    msgs.reverse();
    Ok(ChatPage {
        messages: msgs,
        older,
        end,
    })
}

pub(crate) fn read_since(
    path: &Path,
    after: u64,
    max_messages: usize,
    parse: Parser,
) -> io::Result<ChatPage> {
    let (lines, end) = lines_after(path, after, SCAN_BUDGET)?;
    let mut msgs: Vec<ChatMessage> = Vec::new();
    let mut stop_at = end;
    for (off, line) in lines {
        if let Some(l) = parse(&line, off).filter(|l| !l.parts.is_empty()) {
            if merges(&msgs, &l) {
                msgs.last_mut().unwrap().parts.extend(l.parts);
            } else if msgs.len() >= max_messages.max(1) {
                stop_at = off;
                break;
            } else {
                msgs.push(to_msg(l));
            }
        }
    }
    Ok(ChatPage {
        messages: msgs,
        older: None,
        end: stop_at,
    })
}

/// Seconds-since-epoch → ms, for agents that write unix seconds.
pub(crate) fn secs_ms(v: Option<&serde_json::Value>) -> Option<u64> {
    v.and_then(serde_json::Value::as_f64)
        .filter(|x| *x > 0.0)
        .map(|x| (x * 1000.0) as u64)
}
