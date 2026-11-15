//! `card.aiRename`: asks the card's agent to name its own session from the
//! conversation, then takes that name as the card's title.
//!
//! Claude Code's `/rename` with no argument generates the name, but its status line is
//! not refreshed afterwards, so the name is read from the line the command appends to
//! the transcript.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use muster_core::store::{title_source, CardPatch};
use muster_core::util::err;
use muster_core::Result;
use muster_proto::*;

use crate::agents::blocking;
use crate::log;
use crate::state::State;

/// How long the agent gets to write the new name.
const WAIT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(400);

/// Cards with a rename under way, so repeated requests type the command once.
fn renaming() -> &'static Mutex<HashSet<String>> {
    static R: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// A session name as a card title. Generated names are kebab-case
/// (`fix-login-redirect`); those read better as words.
pub fn display_title(name: &str) -> String {
    let name = name.trim();
    let kebab = name.contains('-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !kebab {
        return name.chars().take(80).collect();
    }
    let mut out = String::with_capacity(name.len());
    for (i, c) in name.chars().take(80).enumerate() {
        out.push(match c {
            '-' => ' ',
            c if i == 0 => c.to_ascii_uppercase(),
            c => c,
        });
    }
    out
}

/// Hands the title back to Muster, types the agent's rename command into the session
/// and follows up in the background. Only between turns: typed mid-turn the command
/// would be queued as a prompt.
pub async fn start(st: &Arc<State>, id: &str) -> Result<Card> {
    let card = st.store.lock().unwrap().get_card(id)?;
    let Some(command) = st.agents.adapter(&card.agent_id).and_then(|a| a.rename_command()) else {
        return Err(err(ErrorCode::Invalid, "this agent cannot name its own session"));
    };
    if !st.has_session(id) {
        return Err(err(ErrorCode::Conflict, "the card has no running session"));
    }
    if !matches!(card.status, CardStatus::Idle | CardStatus::Done) {
        return Err(err(ErrorCode::Conflict, "the agent is busy; try again when it is between turns"));
    }
    let Some((adapter, path)) = st.agents.transcript(st, &card).filter(|(_, p)| p.is_file()) else {
        return Err(err(ErrorCode::Conflict, "the session has no conversation to name yet"));
    };
    if !renaming().lock().unwrap().insert(id.to_string()) {
        return Ok(st.decorate(card));
    }
    let typed = async {
        let after = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let card = if card.title_auto {
            card
        } else {
            let patch = CardPatch { title_auto: Some(true), ..Default::default() };
            let c = st.store.lock().unwrap().update_card(id, patch)?;
            st.emit_card(c.clone());
            c
        };
        st.hub.write(id, command.as_bytes()).await?;
        // Give the agent's input a moment to take the text before submitting.
        tokio::time::sleep(Duration::from_millis(300)).await;
        st.hub.write(id, b"\r").await?;
        Ok((card, after))
    };
    let (card, after) = match typed.await {
        Ok(v) => v,
        Err(e) => {
            renaming().lock().unwrap().remove(id);
            return Err(e);
        }
    };
    log!("card {id}: asked the agent to name its session");
    let (st2, id2) = (st.clone(), id.to_string());
    tokio::spawn(async move {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            tokio::time::sleep(POLL).await;
            let Ok(Some(name)) = blocking(|| adapter.session_name_since(&path, after)) else { continue };
            let titled = st2.store.lock().unwrap().set_auto_title(&id2, &display_title(&name), title_source::AGENT);
            if let Ok(Some(c)) = titled {
                st2.emit_card(c);
            }
            break;
        }
        renaming().lock().unwrap().remove(&id2);
    });
    Ok(st.decorate(card))
}

#[cfg(test)]
mod tests {
    use super::display_title;

    #[test]
    fn generated_names_become_words() {
        assert_eq!(display_title("reply-with-pong"), "Reply with pong");
        assert_eq!(display_title("fix-http2-retry"), "Fix http2 retry");
        // Anything else is somebody's own wording.
        assert_eq!(display_title("Single word reply"), "Single word reply");
        assert_eq!(display_title("probe two"), "probe two");
        assert_eq!(display_title("Fix-It"), "Fix-It");
        assert_eq!(display_title("-x-"), "-x-");
        assert_eq!(display_title("plain"), "plain");
        assert_eq!(display_title(&"a".repeat(200)).chars().count(), 80);
    }
}
