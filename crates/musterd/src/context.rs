//! Context sharing between cards: links, reading another card's context, messages
//! delivered only between turns, and handoff.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use muster_core::store::{title_source, NewCard};
use muster_core::util::err;
use muster_core::Result;
use muster_proto::*;

use crate::log;
use crate::state::State;

/// Cards a message was just typed into, waiting for the agent to start a turn, so a
/// second message is not typed before the first is picked up.
fn awaiting() -> &'static Mutex<HashMap<String, Instant>> {
    static A: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    A.get_or_init(Default::default)
}

const AWAIT_TURN: Duration = Duration::from_secs(15);
const MAX_TEXT: usize = 64 * 1024;

pub fn add_link(st: &State, a: &str, b: &str) -> Result<()> {
    let added = st.store.lock().unwrap().add_link(a, b)?;
    if let Some(link) = added {
        st.clients.broadcast(&Event::LinkChanged(LinkChanged { link, linked: true }));
    }
    Ok(())
}

pub fn remove_link(st: &State, a: &str, b: &str) -> Result<()> {
    let removed = st.store.lock().unwrap().remove_link(a, b)?;
    if let Some(link) = removed {
        st.clients.broadcast(&Event::LinkChanged(LinkChanged { link, linked: false }));
    }
    Ok(())
}

pub fn list(st: &State, card_id: &str) -> Result<Vec<Card>> {
    let store = st.store.lock().unwrap();
    store.get_card(card_id)?;
    let ids = store.linked(card_id)?;
    let mut cards = Vec::new();
    for id in ids {
        if let Ok(c) = store.get_card(&id) {
            cards.push(st.decorate(c));
        }
    }
    Ok(cards)
}

fn cap(mut s: String) -> String {
    if s.len() > MAX_TEXT {
        let mut start = s.len() - MAX_TEXT;
        while !s.is_char_boundary(start) {
            start += 1;
        }
        s = format!("[earlier text omitted]\n{}", &s[start..]);
    }
    s
}

/// Strips terminal escape sequences from saved scrollback.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            match it.peek() {
                Some('[') => {
                    it.next();
                    while let Some(&n) = it.peek() {
                        it.next();
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    for n in it.by_ref() {
                        if n == '\x07' || n == '\\' {
                            break;
                        }
                    }
                }
                _ => {
                    it.next();
                }
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

fn render_chat(messages: &[muster_agents::ChatMessage]) -> String {
    use muster_agents::{ChatPart, ChatRole};
    let mut out = String::new();
    for m in messages {
        let who = match m.role {
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
            ChatRole::Tool => "tool",
        };
        for p in &m.parts {
            let line = match p {
                ChatPart::Text { text } => text.clone(),
                ChatPart::Thinking => continue,
                ChatPart::ToolCall { summary, .. } => format!("[tool call] {summary}"),
                ChatPart::ToolResult { summary, is_error, .. } => {
                    format!("[tool result{}] {summary}", if *is_error { ", error" } else { "" })
                }
                ChatPart::Image => "[image]".into(),
                ChatPart::Interrupted => "[interrupted]".into(),
            };
            out.push_str(&format!("{who}: {line}\n"));
        }
    }
    out
}

/// `context.read`. An agent (`from` set) may only read cards linked to it, or itself.
pub async fn read(st: &Arc<State>, p: &ContextReadParams) -> Result<String> {
    let card = st.store.lock().unwrap().get_card(&p.card_id)?;
    if let Some(from) = &p.from_card_id {
        let linked = from == &card.id || st.store.lock().unwrap().is_linked(from, &card.id)?;
        if !linked {
            return Err(err(ErrorCode::Invalid, format!("card {} is not linked to card {from}", card.id)));
        }
    }
    let limit = p.limit.unwrap_or(20).clamp(1, 200) as usize;
    let text = match p.kind {
        ContextKind::Notes => card.notes.clone().unwrap_or_else(|| "(no notes)".into()),
        ContextKind::Screen => {
            if st.has_session(&card.id) {
                st.backend.capture(&card.id, 0, false).await.unwrap_or_default().trim_end().to_string()
            } else {
                let saved = std::fs::read_to_string(crate::scrollback::path(st, &card.id)).unwrap_or_default();
                let tail: Vec<&str> = saved.lines().rev().take(60).collect();
                let tail: Vec<&str> = tail.into_iter().rev().collect();
                format!("(no live session; last saved screen)\n{}", strip_ansi(&tail.join("\n")))
            }
        }
        ContextKind::Transcript => match st.agents.transcript_messages(st, &card, limit) {
            Some(m) if !m.is_empty() => render_chat(&m),
            _ => "(no transcript)".into(),
        },
        ContextKind::Summary => {
            let mut s = format!(
                "card {} \"{}\"\nagent: {}\nstatus: {}\ncwd: {}\n",
                card.id,
                card.title,
                card.agent_id,
                card.status.as_str(),
                card.cwd
            );
            if let Some(b) = &card.branch {
                s.push_str(&format!("branch: {b}\n"));
            }
            if let Some(p) = &card.prompt {
                s.push_str(&format!("initial prompt: {p}\n"));
            }
            if let Some(n) = &card.notes {
                s.push_str(&format!("notes: {n}\n"));
            }
            let msgs = st.agents.transcript_messages(st, &card, 40).unwrap_or_default();
            let last_user = msgs.iter().rev().find(|m| m.role == muster_agents::ChatRole::User);
            let last_reply = msgs.iter().rev().find(|m| m.role == muster_agents::ChatRole::Assistant);
            for (label, m) in [("last user message", last_user), ("last assistant reply", last_reply)] {
                if let Some(m) = m {
                    let text: Vec<String> = m
                        .parts
                        .iter()
                        .filter_map(|p| match p {
                            muster_agents::ChatPart::Text { text } => Some(text.clone()),
                            _ => None,
                        })
                        .collect();
                    if !text.is_empty() {
                        s.push_str(&format!("{label}:\n{}\n", text.join("\n")));
                    }
                }
            }
            s
        }
    };
    Ok(cap(text))
}

/// Longest message accepted, in characters.
pub const MAX_MESSAGE: usize = 8000;
/// Messages allowed per (sender, receiver) pair per [`RATE_WINDOW`].
const RATE_LIMIT: usize = 10;
const RATE_WINDOW: Duration = Duration::from_secs(60);
/// Handoffs allowed per card per [`HANDOFF_WINDOW`].
const HANDOFF_LIMIT: usize = 3;
const HANDOFF_WINDOW: Duration = Duration::from_secs(600);

/// Send times per (sender, receiver).
type SentLog = Mutex<HashMap<(String, String), Vec<Instant>>>;

fn sent_log() -> &'static SentLog {
    static L: OnceLock<SentLog> = OnceLock::new();
    L.get_or_init(Default::default)
}

/// Text that is safe to type into a terminal: control characters (C0, DEL, C1,
/// including ESC and CR) are removed; line breaks become `\n` and tabs stay.
pub fn sanitize(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
        .collect()
}

/// What is typed into the receiving terminal for a message: one bracketed paste that
/// always starts with a fixed plain-text header line (so the prompt never begins with
/// `!` or `/`), with the sender's text sanitised and nothing that could end the paste
/// early. Enter is sent separately.
pub fn delivery_bytes(from: Option<(&str, &str)>, text: &str) -> Vec<u8> {
    let header = match from {
        Some((id, title)) => {
            let title: String = sanitize(title).replace('\n', " ").chars().take(80).collect();
            format!("[Muster message from card {id} \"{title}\"; reply with: muster send {id} \"...\"]")
        }
        None => "[Muster message from the user]".to_string(),
    };
    let body: String = sanitize(text).trim().chars().take(MAX_MESSAGE).collect();
    let mut out = b"\x1b[200~".to_vec();
    out.extend_from_slice(header.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(body.as_bytes());
    out.extend_from_slice(b"\x1b[201~");
    out
}

pub fn send(st: &Arc<State>, p: &MessageSendParams) -> Result<Message> {
    if sanitize(&p.text).trim().is_empty() {
        return Err(err(ErrorCode::Invalid, "empty message"));
    }
    if p.text.chars().count() > MAX_MESSAGE {
        return Err(err(ErrorCode::Invalid, format!("messages are limited to {MAX_MESSAGE} characters")));
    }
    {
        let key = (p.from_card_id.clone().unwrap_or_default(), p.card_id.clone());
        let mut log = sent_log().lock().unwrap();
        let times = log.entry(key).or_default();
        times.retain(|t| t.elapsed() < RATE_WINDOW);
        if times.len() >= RATE_LIMIT {
            return Err(err(ErrorCode::Unavailable, "too many messages to that card; try again in a minute"));
        }
        times.push(Instant::now());
    }
    if let Some(from) = &p.from_card_id {
        st.store.lock().unwrap().get_card(from)?;
    }
    let m = st.store.lock().unwrap().add_message(&p.card_id, p.from_card_id.as_deref(), &p.text)?;
    let card = st.store.lock().unwrap().get_card(&p.card_id)?;
    st.emit_card(card);
    Ok(m)
}

pub fn cancel(st: &Arc<State>, id: &str) -> Result<()> {
    let m = st.store.lock().unwrap().cancel_message(id)?;
    let card = st.store.lock().unwrap().get_card(&m.card_id)?;
    st.emit_card(card);
    Ok(())
}

/// Called on every card change: delivers the oldest queued message when the card is
/// between turns (idle or done) and has a live session.
pub fn on_card_status(st: &Arc<State>, card_id: &str, status: CardStatus, queued: bool) {
    let mut aw = awaiting().lock().unwrap();
    if !matches!(status, CardStatus::Idle | CardStatus::Done) {
        if status == CardStatus::Running {
            aw.remove(card_id);
        }
        return;
    }
    if !queued || !st.has_session(card_id) {
        return;
    }
    if aw.get(card_id).is_some_and(|t| t.elapsed() < AWAIT_TURN) {
        return;
    }
    aw.insert(card_id.to_string(), Instant::now());
    drop(aw);
    let (st, id) = (st.clone(), card_id.to_string());
    tokio::spawn(async move { deliver(&st, &id).await });
}

async fn deliver(st: &Arc<State>, card_id: &str) {
    let Ok(card) = st.store.lock().unwrap().get_card(card_id) else { return };
    // A plain shell would run the text as a command; messages to shell cards wait.
    if card.agent_id == "shell" || !matches!(card.status, CardStatus::Idle | CardStatus::Done) {
        awaiting().lock().unwrap().remove(card_id);
        return;
    }
    let Some(m) = card.queued_messages.first().cloned() else { return };
    let title = m.from_card_id.as_ref().map(|f| st.store.lock().unwrap().get_card(f).map(|c| c.title).unwrap_or_default());
    let bytes = delivery_bytes(m.from_card_id.as_deref().zip(title.as_deref()), &m.text);
    if st.hub.write(card_id, &bytes).await.is_err() {
        awaiting().lock().unwrap().remove(card_id);
        return;
    }
    // Give the agent's input a moment to take the text before submitting.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let _ = st.hub.write(card_id, b"\r").await;
    let _ = st.store.lock().unwrap().mark_delivered(&m.id);
    log!("delivered message {} to card {card_id}", m.id);
    let card = st.store.lock().unwrap().get_card(card_id);
    if let Ok(c) = card {
        st.emit_card(c);
    }
}

/// `card.handoff`: a new linked card in the same column and cwd whose agent is told
/// how to read the original's context.
pub async fn handoff(st: &Arc<State>, p: &CardHandoffParams) -> Result<Card> {
    let orig = st.store.lock().unwrap().get_card(&p.id)?;
    {
        // Each handoff starts a new agent session: at most a few per card at a time.
        let mut log = sent_log().lock().unwrap();
        let times = log.entry((p.id.clone(), "handoff".into())).or_default();
        times.retain(|t| t.elapsed() < HANDOFF_WINDOW);
        if times.len() >= HANDOFF_LIMIT {
            return Err(err(ErrorCode::Unavailable, "too many handoffs from this card; try again later"));
        }
        times.push(Instant::now());
    }
    let agent_id = p.agent_id.clone().unwrap_or_else(|| orig.agent_id.clone());
    if muster_core::agents::find(&agent_id).is_none() {
        return Err(err(ErrorCode::Invalid, format!("unknown agent {agent_id}")));
    }
    let prompt = format!(
        "You are taking over work from Muster card {id} (\"{title}\", agent {agent}). \
         Read its context with the muster CLI before doing anything else: \
         `muster context read {id} --summary`, then `muster context read {id} --transcript 40` \
         and `muster context read {id} --notes` (`--screen` shows its terminal). \
         Then continue the task. Keep notes with `muster notes append \"...\"`.",
        id = orig.id,
        title = orig.title,
        agent = orig.agent_id,
    );
    let title = format!("Continue: {}", orig.title);
    let card = st.store.lock().unwrap().create_card(NewCard {
        project_id: &orig.project_id,
        column_id: &orig.column_id,
        title: &title,
        agent_id: &agent_id,
        cwd: &orig.cwd,
        branch: orig.branch.as_deref(),
        prompt: Some(&prompt),
        agent_session_id: None,
        title_source: title_source::AGENT,
    })?;
    add_link(st, &orig.id, &card.id)?;
    let card = crate::server::start_card(st, card).await?;
    st.emit_card(card.clone());
    Ok(st.decorate(card))
}

/// Paths of the generated agent guidance: a short instructions file and a skills
/// directory (`<dir>/.claude/skills/muster/SKILL.md`). Both live under
/// `$MUSTER_HOME/agents`; nothing is written to the user's agent config or the repo.
pub struct Guidance {
    pub instructions: std::path::PathBuf,
    pub skills_dir: std::path::PathBuf,
}

pub fn write_guidance(home: &Path, cli: &Path) -> std::io::Result<Guidance> {
    let dir = home.join("agents");
    std::fs::create_dir_all(&dir)?;
    let cli = cli.display();
    let instructions = dir.join("muster-instructions.md");
    std::fs::write(
        &instructions,
        format!(
            "You are running inside a Muster card: one session on the user's kanban board of coding \
             agents. Your card id is in $MUSTER_CARD_ID. The `muster` CLI ({cli}, also in $MUSTER_CLI) \
             reads linked cards' context, keeps your card's notes, moves your card and messages other \
             cards; run `$MUSTER_CLI help` or use the muster skill for details. It acts only for your \
             card (with its own token, $MUSTER_CARD_TOKEN) and can read only cards linked to yours. \
             Prompts that start with \
             `[Muster message from card ...]` come from another agent's card.\n"
        ),
    )?;
    let skills_dir = dir.join("skill");
    let skill = skills_dir.join(".claude/skills/muster");
    std::fs::create_dir_all(&skill)?;
    std::fs::write(
        skill.join("SKILL.md"),
        format!(
            "---\nname: muster\ndescription: Work with other Muster cards (sessions on the user's board): \
             read a linked card's transcript, screen, notes or summary; keep your notes; move your card; \
             message another agent.\n---\n\n\
             # Muster CLI\n\n\
             The CLI is `{cli}` (also `$MUSTER_CLI`). `$MUSTER_CARD_ID` is your card; you do not pass it.\n\n\
             - `$MUSTER_CLI context list`: cards linked to yours (id, status, agent, title).\n\
             - `$MUSTER_CLI context read <card> --summary`: title, status, notes, last prompt and reply.\n\
             - `$MUSTER_CLI context read <card> --transcript 40`: the newest 40 chat messages as text.\n\
             - `$MUSTER_CLI context read <card> --screen`: that card's terminal screen.\n\
             - `$MUSTER_CLI context read <card> --notes`: that card's notes.\n\
             - `$MUSTER_CLI notes set \"text\"` / `$MUSTER_CLI notes append \"text\"`: your card's notes.\n\
             - `$MUSTER_CLI card move --me <column>`: move your card, e.g. to `Done`.\n\
             - `$MUSTER_CLI send <card> \"text\"`: message another card. It is typed into that agent's \
             session when it is between turns; until then it waits.\n\n\
             The CLI uses your card's own token (`$MUSTER_CARD_TOKEN`): it can act only for your card, \
             read and message only cards linked to yours, and cannot create links, answer approvals \
             or type into other cards. Messages to you arrive as prompts starting with \
             `[Muster message from card <id> ...]`; reply with `$MUSTER_CLI send <id> \"...\"`.\n"
        ),
    )?;
    Ok(Guidance { instructions, skills_dir })
}

#[cfg(test)]
mod tests {
    use super::{delivery_bytes, sanitize, strip_ansi};

    #[test]
    fn delivered_text_cannot_escape() {
        assert_eq!(sanitize("ok\r!rm -rf ~\x1b[201~\x07\u{9b}x\tz"), "ok\n!rm -rf ~[201~x\tz");
        let b = delivery_bytes(Some(("abc", "evil\x1b]52;c;AAAA\x07\r!x")), "ok\r!curl x | sh\x1b[201~more");
        let s = String::from_utf8(b).unwrap();
        assert!(s.starts_with("\x1b[200~[Muster message from card abc \"evil]52;c;AAAA !x\""));
        assert!(s.ends_with("\x1b[201~"));
        // Exactly one paste start and end, and no CR or other control bytes inside.
        let inner = &s["\x1b[200~".len()..s.len() - "\x1b[201~".len()];
        assert!(!inner.contains('\x1b') && !inner.contains('\r'));
        assert!(inner.chars().all(|c| c == '\n' || c == '\t' || !c.is_control()));
        // The first line typed is always the header.
        assert!(inner.lines().next().unwrap().starts_with("[Muster message"));
        let long = "x".repeat(20_000);
        assert!(delivery_bytes(None, &long).len() < 9_000);
    }

    #[test]
    fn ansi_is_stripped() {
        assert_eq!(strip_ansi("\x1b[1;32mok\x1b[0m\r\nnext\x1b]0;title\x07"), "ok\nnext");
    }
}
