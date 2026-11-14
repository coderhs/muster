//! Codex, Cursor and opencode adapters against captured (sanitised) data.

use std::path::{Path, PathBuf};

use muster_agents::*;

fn fixture(p: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(p)
}

fn kinds(evs: &[NormalizedEvent]) -> Vec<String> {
    evs.iter()
        .map(|e| {
            serde_json::to_value(&e.kind).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn codex_unauthenticated_rollout_gives_turn_events() {
    let c = Codex::default();
    let path = fixture("codex/rollout-unauthenticated.jsonl");
    let (evs, next) = c.log_events("c1", &path, 0).unwrap();
    assert_eq!(next, std::fs::metadata(&path).unwrap().len());
    assert_eq!(
        kinds(&evs),
        ["sessionStarted", "turnStarted", "other", "turnFinished"]
    );
    assert_eq!(
        evs[0].session_id.as_deref(),
        Some("01a1d62d-87bc-7362-9c96-9e709c4ef64f")
    );
    let mut t = StatusTracker::new(0);
    let statuses: Vec<AgentStatus> = evs
        .iter()
        .flat_map(|e| t.apply(e))
        .filter_map(|c| match c {
            TrackerChange::Status { to, .. } => Some(to),
            _ => None,
        })
        .collect();
    assert_eq!(statuses, [AgentStatus::Running, AgentStatus::Done]);
    // nothing new after the end
    assert!(c.log_events("c1", &path, next).unwrap().0.is_empty());
    assert_eq!(c.turn_state(&path).unwrap(), TurnState::Ended);
    // the only user message is the prompt; developer/context items are skipped
    let page = c.read_chat(&path, None, 50).unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(
        page.messages[0].parts,
        [ChatPart::Text {
            text: "reply ok".into()
        }]
    );
}

#[test]
fn codex_redacted_rollout_chat_usage_and_limits() {
    let c = Codex::default();
    let path = fixture("codex/rollout-redacted.jsonl");
    let page = c.read_chat(&path, None, 500).unwrap();
    let roles: Vec<ChatRole> = page.messages.iter().map(|m| m.role).collect();
    assert!(
        roles.contains(&ChatRole::User)
            && roles.contains(&ChatRole::Assistant)
            && roles.contains(&ChatRole::Tool)
    );
    let calls: Vec<&ChatPart> = page
        .messages
        .iter()
        .flat_map(|m| &m.parts)
        .filter(|p| matches!(p, ChatPart::ToolCall { .. }))
        .collect();
    assert!(!calls.is_empty());
    assert!(calls.iter().any(
        |p| matches!(p, ChatPart::ToolCall { summary, .. } if summary.ends_with(": echo redacted"))
    ));
    // paging in small pages gives the same messages
    let mut all = Vec::new();
    let mut before = None;
    loop {
        let p = c.read_chat(&path, before, 2).unwrap();
        all.splice(0..0, p.messages);
        match p.older {
            Some(o) => before = Some(o),
            None => break,
        }
    }
    assert_eq!(all, page.messages);

    let ctx = c.context_usage(&path).unwrap().unwrap();
    assert!(ctx.used_tokens > 0);
    assert_eq!(ctx.window_tokens, Some(258_400));

    let (evs, _) = c.log_events("c1", &path, 0).unwrap();
    let usage = evs
        .iter()
        .rev()
        .find_map(|e| match &e.kind {
            AgentEvent::Usage { usage } => usage.rate_limits.clone(),
            _ => None,
        })
        .unwrap();
    assert_eq!(usage.windows[0].name, "primary");
    assert!(usage.windows[0].window_minutes.is_some());
    assert!(usage.windows[0].resets_at_ms.unwrap() > 1_700_000_000_000);
    let k = kinds(&evs);
    assert!(k.contains(&"toolStarted".to_string()) && k.contains(&"toolFinished".to_string()));
}

#[test]
fn codex_find_session_by_cwd_and_time() {
    let home = std::env::temp_dir().join(format!("muster-codex-home-{}", std::process::id()));
    let day = home.join("sessions/2026/11/14");
    std::fs::create_dir_all(&day).unwrap();
    let id = "01a1d62d-87bc-7362-9c96-9e709c4ef64f";
    let f = day.join(format!("rollout-2026-11-14T00-01-10-{id}.jsonl"));
    // written fresh (a copy may keep the fixture's timestamps)
    std::fs::write(
        &f,
        std::fs::read(fixture("codex/rollout-unauthenticated.jsonl")).unwrap(),
    )
    .unwrap();
    let c = Codex {
        program: "codex".into(),
        home: home.clone(),
    };
    let cwd = Path::new("/Users/user/sandbox/aiterm/target/muster-test-r2-15155");
    let found = c.find_session(cwd, now_ms() - 60_000).unwrap();
    assert_eq!(found.session_id, id);
    assert_eq!(found.transcript.as_deref(), Some(f.as_path()));
    assert!(c.find_session(Path::new("/elsewhere"), 0).is_none());
    assert!(
        c.find_session(cwd, now_ms() + 3_600_000).is_none(),
        "older than the launch"
    );
    assert_eq!(c.transcript_path(id, cwd).as_deref(), Some(f.as_path()));
    let _ = std::fs::remove_dir_all(home);
}

fn cursor_envelopes() -> Vec<HookEnvelope> {
    std::fs::read_to_string(fixture("cursor/events-interactive.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn cursor_hook_events_and_status() {
    let c = Cursor::default();
    let evs: Vec<NormalizedEvent> = cursor_envelopes()
        .iter()
        .flat_map(|e| c.normalize(e, 1))
        .collect();
    let k: Vec<String> = kinds(&evs).into_iter().filter(|k| k != "usage").collect();
    assert_eq!(
        k,
        [
            "turnStarted",
            "sessionStarted",
            "other",
            "toolStarted",
            "other",
            "other",
            "toolFinished",
            "turnFinished",
            "other",
            "sessionEnded"
        ]
    );
    let sid = evs[0].session_id.clone().unwrap();
    assert!(evs
        .iter()
        .all(|e| e.session_id.as_deref() == Some(sid.as_str())));
    assert!(evs.iter().any(|e| matches!(&e.kind, AgentEvent::ToolStarted { summary, .. } if summary == "Shell: touch cursor-a.txt")));
    let usage = evs
        .iter()
        .find_map(|e| match &e.kind {
            AgentEvent::Usage { usage } => usage.context.clone(),
            _ => None,
        })
        .unwrap();
    assert_eq!(usage.used_tokens, 38532);
    let mut t = StatusTracker::new(0);
    let statuses: Vec<AgentStatus> = evs
        .iter()
        .flat_map(|e| t.apply(e))
        .filter_map(|c| match c {
            TrackerChange::Status { to, .. } => Some(to),
            _ => None,
        })
        .collect();
    // beforeSubmitPrompt arrives before sessionStart on the first turn
    assert_eq!(
        statuses,
        [
            AgentStatus::Running,
            AgentStatus::Idle,
            AgentStatus::Running,
            AgentStatus::Done,
            AgentStatus::Exited
        ]
    );
}

#[test]
fn cursor_transcript_and_plugin() {
    let c = Cursor::default();
    let path = fixture("cursor/transcript-basic.jsonl");
    let page = c.read_chat(&path, None, 10).unwrap();
    assert_eq!(page.messages.len(), 3);
    assert_eq!(page.messages[0].role, ChatRole::User);
    assert_eq!(
        page.messages[0].parts,
        [ChatPart::Text {
            text: "What is the muster codeword? Then run the shell command: touch cursor-a.txt"
                .into()
        }]
    );
    assert_eq!(c.turn_state(&path).unwrap(), TurnState::Ended);

    let dir = std::env::temp_dir().join(format!("muster-cursor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let instr = dir.join("i.md");
    std::fs::write(&instr, "Use the muster CLI.").unwrap();
    let mut spec = LaunchSpec::new("card1", "/w");
    spec.hooks = Some(HookConfig::new(
        "/opt/muster-hook",
        "/tmp/s.sock",
        dir.join("state"),
    ));
    spec.instructions = Some(instr);
    spec.session_id = Some("bf904a17-e6fd-46bd-a446-012784d81731".into());
    spec.prompt = Some("hi".into());
    let cmd = c.launch(&spec).unwrap();
    assert_eq!(cmd.args[0], "--plugin-dir");
    let plugin = PathBuf::from(&cmd.args[1]);
    assert_eq!(
        cmd.args[2..],
        [
            "--resume",
            "bf904a17-e6fd-46bd-a446-012784d81731",
            "--",
            "hi"
        ]
    );
    let hooks: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(plugin.join("hooks/hooks.json")).unwrap())
            .unwrap();
    assert_eq!(hooks["version"], 1);
    assert_eq!(
        hooks["hooks"]["stop"][0]["command"],
        "/opt/muster-hook hook stop"
    );
    assert_eq!(hooks["hooks"]["sessionStart"].as_array().unwrap().len(), 2);
    let ctx: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(plugin.join("session-context.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(ctx["additional_context"], "Use the muster CLI.");
    assert!(plugin.join(".cursor-plugin/plugin.json").is_file());
    assert!(cmd
        .env
        .contains(&("MUSTER_AGENT_ID".into(), "cursor".into())));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn opencode_session_and_messages() {
    let s = muster_agents::opencode::parse_session(
        &std::fs::read_to_string(fixture("opencode/session-failed.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(s.id, "ses_muster_test_0001");
    assert!(!s.busy);
    assert_eq!(s.outcome.as_deref(), Some("failed"));
    let m = muster_agents::opencode::parse_messages(
        &std::fs::read_to_string(fixture("opencode/messages-failed.json")).unwrap(),
    );
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].role, ChatRole::User);
    assert_eq!(
        m[0].parts,
        [ChatPart::Text {
            text: "reply with the single word ok".into()
        }]
    );
    assert!(
        matches!(&m[1].parts[0], ChatPart::Text { text } if text.starts_with("Error: Free usage limit"))
    );
    let o = Opencode::default();
    let mut spec = LaunchSpec::new("c", "/w");
    spec.session_id = o.new_session_id();
    spec.prompt = Some("hi".into());
    let cmd = o.launch(&spec).unwrap();
    assert_eq!(cmd.args[0], "--session");
    assert!(cmd.args[1].starts_with("ses_muster"));
    assert_eq!(cmd.args[2..], ["--prompt", "hi"]);
    assert_eq!(
        o.session_query("ses_x").args,
        ["api", "GET", "/api/session/ses_x"]
    );
}

#[test]
fn registry_has_all_agents() {
    let r = AdapterRegistry::default();
    for id in [
        "claude",
        "codex",
        "cursor",
        "opencode",
        "gemini",
        "antigravity",
    ] {
        assert!(r.get(id).is_some(), "{id}");
    }
    let g = r.get("gemini").unwrap();
    let mut spec = LaunchSpec::new("c", "/w");
    spec.session_id = g.new_session_id();
    spec.prompt = Some("hi".into());
    let cmd = g.launch(&spec).unwrap();
    assert_eq!(cmd.args[0], "--session-id");
    assert_eq!(cmd.args[2..], ["-i", "hi"]);
    assert_eq!(
        r.get("antigravity")
            .unwrap()
            .resume(&LaunchSpec::new("c", "/w"), "x")
            .unwrap()
            .args,
        ["--conversation", "x"]
    );
}
