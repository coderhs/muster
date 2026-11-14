//! Claude Code transcript (`<session>.jsonl`) reading.
//!
//! The file mixes conversation lines (`type: "user" | "assistant" | "system"`) with
//! bookkeeping lines (`attachment`, `api-request*`, `file-history-*`, `cost-state`, ...),
//! some of them hundreds of kilobytes. Everything here reads from the end, pre-filters
//! lines by substring before parsing, and stops after a byte budget.

use std::io;
use std::path::Path;

use serde_json::Value;

use super::{one_line, tool_summary};
use crate::tail::{contains, lines_after, ReverseLines};
use crate::types::*;

/// Upper bound of bytes read from disk for one page or one tail query.
const SCAN_BUDGET: u64 = 32 * 1024 * 1024;
const RESULT_SUMMARY_CHARS: usize = 200;
const INTERRUPT_PREFIX: &str = "[Request interrupted by user";

fn is_conversation_line(line: &[u8]) -> bool {
    contains(line, b"\"type\":\"user\"") || contains(line, b"\"type\":\"assistant\"")
}

/// A parsed conversation line reduced to what the chat view needs.
struct Parsed {
    key: String,
    role: ChatRole,
    timestamp: Option<String>,
    model: Option<String>,
    parts: Vec<ChatPart>,
}

fn parse_line(line: &[u8]) -> Option<Parsed> {
    if !is_conversation_line(line) {
        return None;
    }
    let v: Value = serde_json::from_slice(line).ok()?;
    let ty = v.get("type")?.as_str()?;
    if v.get("isSidechain").and_then(Value::as_bool) == Some(true)
        || v.get("isMeta").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let msg = v.get("message")?;
    let uuid = v
        .get("uuid")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let timestamp = v
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    match ty {
        "assistant" => {
            let model = msg.get("model").and_then(Value::as_str).map(str::to_string);
            if model.as_deref() == Some("<synthetic>") {
                return None;
            }
            let mut parts = Vec::new();
            for b in msg
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        let t = b.get("text").and_then(Value::as_str).unwrap_or_default();
                        if !t.trim().is_empty() {
                            parts.push(ChatPart::Text {
                                text: t.to_string(),
                            });
                        }
                    }
                    Some("thinking" | "redacted_thinking") => parts.push(ChatPart::Thinking),
                    Some("tool_use" | "server_tool_use") => {
                        let name = b
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let input = b.get("input").cloned().unwrap_or(Value::Null);
                        parts.push(ChatPart::ToolCall {
                            id: b
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            summary: tool_summary(&name, &input),
                            name,
                        });
                    }
                    _ => {}
                }
            }
            // Claude Code writes one line per content block; lines of one API message share
            // `message.id`, which is used as the key so they merge.
            let key = msg
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or(uuid);
            Some(Parsed {
                key,
                role: ChatRole::Assistant,
                timestamp,
                model,
                parts,
            })
        }
        "user" => {
            let mut parts = Vec::new();
            match msg.get("content") {
                Some(Value::String(t)) => user_text(t, &mut parts),
                Some(Value::Array(blocks)) => {
                    for b in blocks {
                        match b.get("type").and_then(Value::as_str) {
                            Some("text") => user_text(
                                b.get("text").and_then(Value::as_str).unwrap_or_default(),
                                &mut parts,
                            ),
                            Some("image") => parts.push(ChatPart::Image),
                            Some("tool_result") => parts.push(ChatPart::ToolResult {
                                tool_use_id: b
                                    .get("tool_use_id")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                                is_error: b
                                    .get("is_error")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false),
                                summary: one_line(
                                    &result_text(b.get("content")),
                                    RESULT_SUMMARY_CHARS,
                                ),
                            }),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            let role = if !parts.is_empty()
                && parts
                    .iter()
                    .all(|p| matches!(p, ChatPart::ToolResult { .. }))
            {
                ChatRole::Tool
            } else {
                ChatRole::User
            };
            Some(Parsed {
                key: uuid,
                role,
                timestamp,
                model: None,
                parts,
            })
        }
        _ => None,
    }
}

fn user_text(t: &str, parts: &mut Vec<ChatPart>) {
    let trimmed = t.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("<local-command-stdout>")
        || trimmed.starts_with("<local-command-stderr>")
        || trimmed.starts_with("<local-command-caveat>")
        || trimmed.starts_with("<system-reminder>")
    {
        return;
    }
    if trimmed.starts_with(INTERRUPT_PREFIX) {
        parts.push(ChatPart::Interrupted);
        return;
    }
    if let Some(name) = between(trimmed, "<command-name>", "</command-name>") {
        let args = between(trimmed, "<command-args>", "</command-args>").unwrap_or("");
        let text = if args.trim().is_empty() {
            name.to_string()
        } else {
            format!("{name} {}", args.trim())
        };
        parts.push(ChatPart::Text { text });
        return;
    }
    if trimmed.starts_with("<command-message>") {
        return;
    }
    parts.push(ChatPart::Text {
        text: t.to_string(),
    });
}

fn between<'a>(s: &'a str, a: &str, b: &str) -> Option<&'a str> {
    let start = s.find(a)? + a.len();
    let end = s[start..].find(b)? + start;
    Some(&s[start..end])
}

fn result_text(c: Option<&Value>) -> String {
    match c {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Collects parsed lines into messages, merging consecutive lines with the same key.
#[derive(Default)]
struct Builder {
    msgs: Vec<ChatMessage>,
}

impl Builder {
    /// Add a line that is *older* than everything added so far.
    fn push_older(&mut self, p: Parsed) {
        if p.parts.is_empty() {
            return;
        }
        if let Some(last) = self.msgs.last_mut() {
            if last.id == p.key && last.role == p.role {
                let mut parts = p.parts;
                parts.append(&mut last.parts);
                last.parts = parts;
                last.timestamp = p.timestamp.or(last.timestamp.take());
                return;
            }
        }
        self.msgs.push(ChatMessage {
            id: p.key,
            role: p.role,
            timestamp: p.timestamp,
            model: p.model,
            parts: p.parts,
        });
    }

    /// Add a line that is *newer* than everything added so far.
    fn push_newer(&mut self, p: Parsed) {
        if p.parts.is_empty() {
            return;
        }
        if let Some(last) = self.msgs.last_mut() {
            if last.id == p.key && last.role == p.role {
                last.parts.extend(p.parts);
                return;
            }
        }
        self.msgs.push(ChatMessage {
            id: p.key,
            role: p.role,
            timestamp: p.timestamp,
            model: p.model,
            parts: p.parts,
        });
    }
}

pub fn read_page(path: &Path, before: Option<u64>, max_messages: usize) -> io::Result<ChatPage> {
    let mut r = ReverseLines::open(path, before)?;
    let end = if before.is_none() {
        r.end
    } else {
        std::fs::metadata(path)?.len()
    };
    let mut b = Builder::default();
    let mut older = None;
    let max = max_messages.max(1);
    while let Some((off, line)) = r.next_line()? {
        if let Some(p) = parse_line(&line) {
            let merges = b
                .msgs
                .last()
                .is_some_and(|m| m.id == p.key && m.role == p.role);
            if b.msgs.len() >= max && !merges && !p.parts.is_empty() {
                // This line belongs to the next page; resume at its end.
                older = Some(off + line.len() as u64 + 1);
                break;
            }
            b.push_older(p);
        }
        if r.bytes_read > SCAN_BUDGET {
            older = Some(off);
            break;
        }
    }
    let mut messages = b.msgs;
    messages.reverse();
    Ok(ChatPage {
        messages,
        older,
        end,
    })
}

pub fn read_since(path: &Path, after: u64, max_messages: usize) -> io::Result<ChatPage> {
    let (lines, end) = lines_after(path, after, SCAN_BUDGET)?;
    let mut b = Builder::default();
    let mut stop_at = end;
    for (off, line) in lines {
        if let Some(p) = parse_line(&line) {
            let merges = b
                .msgs
                .last()
                .is_some_and(|m| m.id == p.key && m.role == p.role);
            if b.msgs.len() >= max_messages.max(1) && !merges && !p.parts.is_empty() {
                stop_at = off;
                break;
            }
            b.push_newer(p);
        }
    }
    Ok(ChatPage {
        messages: b.msgs,
        older: None,
        end: stop_at,
    })
}

/// Context size from the newest main-thread assistant line with usage data. The window
/// size is not in the transcript (the status line feed carries it), except that a model
/// id ending in `[1m]` implies a 1M window.
pub fn context_usage(path: &Path) -> io::Result<Option<ContextUsage>> {
    let mut r = ReverseLines::open(path, None)?;
    while let Some((_, line)) = r.next_line()? {
        if r.bytes_read > SCAN_BUDGET {
            break;
        }
        if !contains(&line, b"\"type\":\"assistant\"") || !contains(&line, b"\"usage\"") {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("assistant")
            || v.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        let model = msg.get("model").and_then(Value::as_str).unwrap_or_default();
        if model == "<synthetic>" {
            continue;
        }
        let Some(usage) = msg.get("usage") else {
            continue;
        };
        let used: u64 = [
            "input_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
        ]
        .iter()
        .filter_map(|k| usage.get(*k).and_then(Value::as_u64))
        .sum();
        if used == 0 {
            continue;
        }
        let requested = v
            .get("requestedModel")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let window =
            (model.ends_with("[1m]") || requested.ends_with("[1m]")).then_some(1_000_000u64);
        return Ok(Some(ContextUsage {
            used_tokens: used,
            window_tokens: window,
            used_percent: window.map(|w| used as f64 * 100.0 / w as f64),
            source: UsageSource::Transcript,
        }));
    }
    Ok(None)
}

/// State of the latest turn. A `system` line with `subtype: "turn_duration"` is written
/// when a turn ends (also after an interrupt); an interrupt leaves a user text block
/// starting with `[Request interrupted by user`.
pub fn turn_state(path: &Path) -> io::Result<TurnState> {
    let mut r = ReverseLines::open(path, None)?;
    let mut saw_end = false;
    while let Some((_, line)) = r.next_line()? {
        if r.bytes_read > SCAN_BUDGET {
            break;
        }
        if contains(&line, b"\"subtype\":\"turn_duration\"") {
            saw_end = true;
            continue;
        }
        if !is_conversation_line(&line) {
            continue;
        }
        let Some(p) = parse_line(&line) else { continue };
        if p.parts.is_empty() {
            continue;
        }
        if p.parts.iter().any(|x| matches!(x, ChatPart::Interrupted)) {
            return Ok(TurnState::Interrupted);
        }
        return Ok(if saw_end {
            TurnState::Ended
        } else {
            TurnState::InProgress
        });
    }
    Ok(if saw_end {
        TurnState::Ended
    } else {
        TurnState::Unknown
    })
}
