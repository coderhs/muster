//! Normalisation, status and transcript tests against payloads captured from Claude Code
//! 2.1.323 (sanitised; see docs/AGENTS.md for how they were recorded).

use std::path::{Path, PathBuf};

use muster_agents::*;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude").join(name)
}

fn envelopes(name: &str) -> Vec<HookEnvelope> {
    std::fs::read_to_string(fixture(name))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).expect("envelope parses"))
        .collect()
}

fn normalize_all(name: &str) -> Vec<NormalizedEvent> {
    let c = ClaudeCode::default();
    envelopes(name).iter().enumerate().flat_map(|(i, e)| c.normalize(e, 1_000 + i as u64)).collect()
}

fn kind_name(k: &AgentEvent) -> String {
    serde_json::to_value(k).unwrap()["type"].as_str().unwrap().to_string()
}

#[test]
fn print_mode_sequence() {
    let evs = normalize_all("events-print.jsonl");
    let kinds: Vec<String> = evs.iter().map(|e| kind_name(&e.kind)).collect();
    assert_eq!(kinds, ["other", "sessionStarted", "turnStarted", "turnFinished", "sessionEnded"]);
    let sid = evs[0].session_id.clone().unwrap();
    assert!(evs.iter().all(|e| e.session_id.as_deref() == Some(sid.as_str())));
    assert!(matches!(&evs[3].kind, AgentEvent::TurnFinished { last_message: Some(m), error: None, .. } if m == "ok"));
    assert!(matches!(&evs[4].kind, AgentEvent::SessionEnded { reason, exited: true } if reason == "other"));

    let mut t = StatusTracker::new(0);
    let statuses: Vec<AgentStatus> = evs
        .iter()
        .flat_map(|e| t.apply(e))
        .filter_map(|c| match c {
            TrackerChange::Status { to, .. } => Some(to),
            _ => None,
        })
        .collect();
    assert_eq!(statuses, [AgentStatus::Running, AgentStatus::Done, AgentStatus::Exited]);
}

#[test]
fn interactive_sequence_and_status() {
    let evs = normalize_all("events-interactive.jsonl");
    let hooks: Vec<String> =
        evs.iter().filter(|e| !matches!(e.kind, AgentEvent::Usage { .. })).map(|e| kind_name(&e.kind)).collect();
    assert_eq!(
        hooks,
        [
            // first turn
            "sessionStarted",
            "other",
            "turnStarted",
            "turnFinished",
            // permission answered through the hook
            "turnStarted",
            "toolStarted",
            "permissionRequested",
            "toolFinished",
            "other",
            "turnFinished",
            // Claude Code's internal next-prompt suggestion agent
            "subagentStopped",
            // permission answered in the terminal with Escape: no Stop follows
            "turnStarted",
            "toolStarted",
            "permissionRequested",
            // /clear then /exit
            "sessionEnded",
            "sessionStarted",
            "sessionEnded",
            // resume
            "sessionStarted",
            "other",
            "turnStarted",
            "turnFinished",
            "subagentStopped",
            // /exit
            "sessionEnded",
        ]
    );

    let perm = evs
        .iter()
        .find_map(|e| match &e.kind {
            AgentEvent::PermissionRequested { request } => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(perm.tool, "Bash");
    assert_eq!(perm.summary, "Bash: touch muster-perm-test.txt");
    assert_eq!(perm.id, None, "ids are assigned by the receiver, not the adapter");
    assert!(perm.suggestions.is_array());

    let starts: Vec<(SessionSource, Option<String>)> = evs
        .iter()
        .filter_map(|e| match &e.kind {
            AgentEvent::SessionStarted { source, .. } => Some((*source, e.session_id.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        starts.iter().map(|s| s.0).collect::<Vec<_>>(),
        [SessionSource::Startup, SessionSource::Clear, SessionSource::Resume]
    );
    assert_ne!(starts[0].1, starts[1].1, "/clear gives a new session id");
    assert_eq!(starts[0].1, starts[2].1, "resume keeps the original id");
    let ends: Vec<bool> = evs
        .iter()
        .filter_map(|e| match &e.kind {
            AgentEvent::SessionEnded { exited, .. } => Some(*exited),
            _ => None,
        })
        .collect();
    assert_eq!(ends, [false, true, true]);

    // Drive the tracker the way the receiver would: give each request an id and follow it
    // with its resolution (the first was answered through the hook, the second in the
    // terminal with Escape).
    let mut t = StatusTracker::new(0);
    let mut log = Vec::new();
    let mut n = 0;
    for mut e in evs.clone() {
        let mut follow = None;
        if let AgentEvent::PermissionRequested { request } = &mut e.kind {
            n += 1;
            let id = format!("r{n}");
            request.id = Some(id.clone());
            let outcome = if n == 1 {
                PermissionOutcome::Answered { decision: PermissionDecision::Allow { remember: false } }
            } else {
                PermissionOutcome::Elsewhere
            };
            let mut r = e.clone();
            r.kind = AgentEvent::PermissionResolved { request_id: id, outcome };
            follow = Some(r);
        }
        log.extend(t.apply(&e));
        if let Some(r) = follow {
            log.extend(t.apply(&r));
        }
    }
    let statuses: Vec<AgentStatus> = log
        .iter()
        .filter_map(|c| match c {
            TrackerChange::Status { to, .. } => Some(*to),
            _ => None,
        })
        .collect();
    assert_eq!(
        statuses,
        [
            AgentStatus::Running,
            AgentStatus::Done, // turn 1
            AgentStatus::Running,
            AgentStatus::NeedsYou,
            AgentStatus::Running,
            AgentStatus::Done, // turn 2
            AgentStatus::Running,
            AgentStatus::NeedsYou, // turn 3: Escape in the terminal, no Stop
            AgentStatus::Idle,     // /clear: SessionEnded(clear) keeps the process, SessionStarted(clear) -> idle
            AgentStatus::Exited,   // /exit
            AgentStatus::Idle,
            AgentStatus::Running,
            AgentStatus::Done, // resume + turn
            AgentStatus::Exited,
        ]
    );
    let sid_changes: Vec<&TrackerChange> =
        log.iter().filter(|c| matches!(c, TrackerChange::SessionId { .. })).collect();
    assert_eq!(sid_changes.len(), 3, "initial, after /clear, back on resume: {sid_changes:?}");
}

#[test]
fn statusline_usage_and_rate_limits() {
    let evs = normalize_all("events-interactive.jsonl");
    let usages: Vec<&UsageSnapshot> = evs
        .iter()
        .filter_map(|e| match &e.kind {
            AgentEvent::Usage { usage } => Some(usage),
            _ => None,
        })
        .collect();
    assert!(usages.len() >= 5);
    // First refresh: before any API response there is no usage and no rate limits.
    assert_eq!(usages[0].context, None);
    assert_eq!(usages[0].rate_limits, None);
    assert_eq!(usages[0].model_id.as_deref(), Some("claude-fable-5-2"));
    let second = usages[1];
    let ctx = second.context.as_ref().unwrap();
    assert_eq!(ctx.used_tokens, 2 + 11856 + 32598);
    assert_eq!(ctx.window_tokens, Some(1_000_000));
    assert_eq!(ctx.used_percent, Some(4.0));
    let rl = second.rate_limits.as_ref().unwrap();
    assert_eq!(rl.five_hour, Some(LimitWindow { used_percent: 25.0, resets_at_ms: Some(1_794_591_600_000) }));
    assert_eq!(rl.seven_day, Some(LimitWindow { used_percent: 12.0, resets_at_ms: Some(1_794_952_800_000) }));
    assert_eq!(rl.spend, None);
    assert_eq!(second.agent_version.as_deref(), Some("2.1.323"));
    assert!(second.cost_usd.unwrap() > 0.0);
}

#[test]
fn transcript_newest_page_and_paging() {
    let c = ClaudeCode::default();
    let path = fixture("transcript-basic.jsonl");
    let all = c.read_chat(&path, None, 1000).unwrap();
    assert_eq!(all.older, None);
    assert_eq!(all.end, std::fs::metadata(&path).unwrap().len());

    let texts: Vec<String> = all
        .messages
        .iter()
        .map(|m| {
            let parts: Vec<String> = m
                .parts
                .iter()
                .map(|p| match p {
                    ChatPart::Text { text } => text.clone(),
                    ChatPart::ToolCall { summary, .. } => format!("call {summary}"),
                    ChatPart::ToolResult { is_error, summary, .. } => format!("result err={is_error} {summary}"),
                    ChatPart::Interrupted => "interrupted".into(),
                    ChatPart::Thinking => "thinking".into(),
                    ChatPart::Image => "image".into(),
                })
                .collect();
            format!("{:?}: {}", m.role, parts.join(" | "))
        })
        .collect();
    assert_eq!(
        texts,
        [
            "User: reply with the single word ok",
            "Assistant: ok",
            "User: Use the Bash tool to run exactly this command: touch muster-perm-test.txt ; then reply with the single word done",
            "Assistant: call Bash: touch muster-perm-test.txt",
            "Tool: result err=false (Bash completed with no output)",
            "Assistant: done",
            "User: Use the Bash tool to run exactly this command: touch muster-perm-test-2.txt ; then reply with the single word done",
            "Assistant: call Bash: touch muster-perm-test-2.txt",
            "Tool: result err=true The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to…",
            "User: interrupted",
            "User: reply with the single word ok",
            "Assistant: ok",
        ]
    );

    // Paging backwards in pages of 3 yields the same messages.
    let mut pages = Vec::new();
    let mut before = None;
    loop {
        let p = c.read_chat(&path, before, 3).unwrap();
        assert!(p.messages.len() <= 3);
        pages.push(p.messages);
        match p.older {
            Some(o) => before = Some(o),
            None => break,
        }
    }
    pages.reverse();
    let flat: Vec<ChatMessage> = pages.into_iter().flatten().collect();
    assert_eq!(flat, all.messages);
}

#[test]
fn transcript_since_offset() {
    let c = ClaudeCode::default();
    let path = fixture("transcript-basic.jsonl");
    let content = std::fs::read(&path).unwrap();
    // Pretend the file ended after the first assistant "ok"; then read the rest as new.
    let mut cut = 0;
    for (i, line) in content.split(|&b| b == b'\n').enumerate() {
        cut += line.len() + 1;
        if i == 33 {
            break;
        }
    }
    let first = c.read_chat_since(&path, 0, 2).unwrap();
    assert_eq!(first.messages.len(), 2);
    // resumes at the line of the first message it did not return
    assert!(first.end >= cut as u64 && first.end < content.len() as u64);
    let rest = c.read_chat_since(&path, cut as u64, 100).unwrap();
    assert_eq!(rest.messages.len(), 10);
    assert_eq!(rest.end, content.len() as u64);
    let none = c.read_chat_since(&path, rest.end, 100).unwrap();
    assert!(none.messages.is_empty());
}

#[test]
fn transcript_context_usage_and_turn_state() {
    let c = ClaudeCode::default();
    let path = fixture("transcript-basic.jsonl");
    let u = c.context_usage(&path).unwrap().unwrap();
    assert_eq!(u.used_tokens, 61810);
    assert_eq!(u.window_tokens, None, "the window size is not in the transcript");
    assert_eq!(u.source, UsageSource::Transcript);
    assert_eq!(c.turn_state(&path).unwrap(), TurnState::Ended);

    // Truncate right after the interrupt marker (+ its turn_duration line).
    let dir = std::env::temp_dir().join(format!("muster-agents-ts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let idx = lines.iter().position(|l| l.contains("[Request interrupted by user")).unwrap();
    let cut = dir.join("interrupted.jsonl");
    std::fs::write(&cut, lines[..=idx + 1].join("\n") + "\n").unwrap();
    assert_eq!(c.turn_state(&cut).unwrap(), TurnState::Interrupted);
    let mut t = StatusTracker::restore(AgentStatus::NeedsYou, 0, None);
    t.pending_permissions.insert("x".into());
    assert_eq!(
        t.apply_turn_state(TurnState::Interrupted, 5),
        [TrackerChange::Status { from: AgentStatus::NeedsYou, to: AgentStatus::Idle }]
    );

    // A turn in progress: cut right after the first prompt.
    let p = lines.iter().position(|l| l.contains("touch muster-perm-test.txt ; then")).unwrap();
    let cut2 = dir.join("inprogress.jsonl");
    std::fs::write(&cut2, lines[..=p].join("\n") + "\n").unwrap();
    assert_eq!(c.turn_state(&cut2).unwrap(), TurnState::InProgress);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn transcript_path_lookup() {
    let dir = std::env::temp_dir().join(format!("muster-agents-tp-{}", std::process::id()));
    let c = ClaudeCode { program: "claude".into(), config_dir: dir.clone() };
    let cwd = Path::new("/work/some.project");
    let pdir = c.project_dir(cwd);
    std::fs::create_dir_all(&pdir).unwrap();
    std::fs::write(pdir.join("abc.jsonl"), "").unwrap();
    assert_eq!(c.transcript_path("abc", cwd), Some(pdir.join("abc.jsonl")));
    // found by scan when the cwd mapping differs
    assert_eq!(c.transcript_path("abc", Path::new("/elsewhere")), Some(pdir.join("abc.jsonl")));
    assert_eq!(c.transcript_path("nope", cwd), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn hook_settings_shape() {
    let h = HookConfig::new("/opt/m/muster-hook", "/tmp/s.sock", "/tmp/state");
    let user_sl: Value = serde_json::json!({"type": "command", "command": "~/x.sh", "padding": 1});
    let s = muster_agents::claude::settings_json(&h, Some(&user_sl));
    for ev in muster_agents::claude::HOOK_EVENTS {
        let entry = &s["hooks"][ev][0]["hooks"][0];
        assert_eq!(entry["type"], "command");
        assert_eq!(entry["command"], "/opt/m/muster-hook");
        assert_eq!(entry["args"][1], *ev);
    }
    assert_eq!(s["hooks"]["PermissionRequest"][0]["hooks"][0]["args"][2], "--wait");
    assert_eq!(s["statusLine"]["padding"], 1);
    assert!(s.get("permissions").is_none(), "never touches permissions");
}

fn kinds_of(evs: &[NormalizedEvent]) -> Vec<String> {
    evs.iter().filter(|e| !matches!(e.kind, AgentEvent::Usage { .. })).map(|e| kind_name(&e.kind)).collect()
}

#[test]
fn question_subagent_and_compaction() {
    let evs = normalize_all("events-question-subagent-compact.jsonl");
    // AskUserQuestion: typed questions on both the PreToolUse and the answerable request
    let q = evs
        .iter()
        .find_map(|e| match &e.kind {
            AgentEvent::QuestionAsked { questions, .. } => Some(questions.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].question, "Which color do you prefer?");
    assert_eq!(q[0].header.as_deref(), Some("Color"));
    assert_eq!(q[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), ["Red", "Blue"]);
    assert!(!q[0].multi_select);
    let req = evs
        .iter()
        .find_map(|e| match &e.kind {
            AgentEvent::PermissionRequested { request } => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(req.tool, "AskUserQuestion");
    assert_eq!(req.summary, "Question: Which color do you prefer?");
    assert!(matches!(&req.prompt, PromptKind::Question { questions } if *questions == q));

    // the answer reply carries the original questions plus answers
    let c = ClaudeCode::default();
    let reply: Value = serde_json::from_str(&c.permission_reply(
        &req,
        &PermissionDecision::Answer {
            answers: [("Which color do you prefer?".to_string(), vec!["Blue".to_string()])].into(),
        },
    ))
    .unwrap();
    let d = &reply["hookSpecificOutput"]["decision"];
    assert_eq!(d["behavior"], "allow");
    assert_eq!(d["updatedInput"]["answers"]["Which color do you prefer?"], "Blue");
    assert_eq!(d["updatedInput"]["questions"][0]["header"], "Color");

    // background subagent: Stop reports it running, its completion starts a new turn
    let k = kinds_of(&evs);
    let i = k.iter().position(|x| x == "subagentStarted").unwrap();
    assert_eq!(k[i - 1], "toolStarted");
    let bg: Vec<u32> = evs
        .iter()
        .filter_map(|e| match &e.kind {
            AgentEvent::TurnFinished { background_tasks, .. } => Some(*background_tasks),
            _ => None,
        })
        .collect();
    assert_eq!(bg, [0, 1, 0]);
    let stopped: Vec<Option<String>> = evs
        .iter()
        .filter_map(|e| match &e.kind {
            AgentEvent::SubagentStopped { agent_type, .. } => Some(agent_type.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(stopped[0], Some("general-purpose".into()));
    assert!(stopped[1..].iter().all(Option::is_none), "the rest are internal helper agents");

    // /compact: PreCompact, SessionStart(compact) with the same id, PostCompact; no status change
    let cs = k.iter().position(|x| x == "compactionStarted").unwrap();
    let cf = k.iter().position(|x| x == "compactionFinished").unwrap();
    assert!(k[cs..cf].iter().any(|x| x == "sessionStarted"));
    let mut t = StatusTracker::new(0);
    let mut after_compaction = None;
    for e in &evs {
        t.apply(e);
        if matches!(e.kind, AgentEvent::CompactionFinished { .. }) {
            after_compaction = Some(t.status);
        }
    }
    assert_eq!(after_compaction, Some(AgentStatus::Done), "compaction after a finished turn leaves it done");
    let sids: std::collections::BTreeSet<_> = evs.iter().filter_map(|e| e.session_id.clone()).collect();
    assert_eq!(sids.len(), 1, "compaction keeps the session id");
}

#[test]
fn plan_approval() {
    let evs = normalize_all("events-plan.jsonl");
    let plans: Vec<PermissionRequest> = evs
        .iter()
        .filter_map(|e| match &e.kind {
            AgentEvent::PermissionRequested { request } if request.tool == "ExitPlanMode" => Some(request.clone()),
            _ => None,
        })
        .collect();
    assert!(!plans.is_empty());
    let PromptKind::Plan { plan, plan_file_path } = &plans[0].prompt else { panic!() };
    assert!(plan.as_deref().unwrap().contains("plan-test.txt"));
    assert!(plan_file_path.as_deref().unwrap().ends_with(".md"));
    let c = ClaudeCode::default();
    let reply: Value =
        serde_json::from_str(&c.permission_reply(&plans[0], &PermissionDecision::ApprovePlan { accept_edits: true }))
            .unwrap();
    let d = &reply["hookSpecificOutput"]["decision"];
    assert_eq!(d["behavior"], "allow");
    assert_eq!(d["updatedInput"]["plan"], plans[0].input["plan"]);
    assert_eq!(d["updatedPermissions"][0]["mode"], "acceptEdits");
}

#[test]
fn stop_failure() {
    let evs = normalize_all("events-stopfailure.jsonl");
    assert!(evs.iter().any(|e| matches!(&e.kind,
        AgentEvent::TurnFinished { error: Some(err), .. } if err == "model_not_found")));
}

#[test]
fn conflicting_permission_hooks_fixture_parses() {
    let evs = normalize_all("events-conflicting-hooks.jsonl");
    assert_eq!(evs.iter().filter(|e| matches!(e.kind, AgentEvent::PermissionRequested { .. })).count(), 2);
}
