//! Prints normalised hook events as JSON lines and answers permission requests by policy.
//!
//! ```text
//! cargo run -p muster-agents --example hook-receiver -- <socket> [--policy allow,none,deny]
//!     [--launch <card> <cwd> <hook-bin> [prompt]]
//! ```
//!
//! `--policy` is applied to successive answerable permission requests: `allow`,
//! `remember`, `deny`, `deny-interrupt`, or `none` (leave it to the terminal). Requests
//! beyond the list are left unanswered.
//!
//! `--launch` prints the launch command the adapter for `--agent <id>` (default
//! `claude`) builds for that card, generating its hook files under `<socket dir>/state`,
//! and exits. `--instructions <file>` sets `LaunchSpec::instructions`; `--raw <file>`
//! logs every raw envelope.

use std::path::PathBuf;
use std::time::Duration;

use muster_agents::*;

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sock = PathBuf::from(
        args.first().expect("usage: hook-receiver <socket> [--policy a,b] [--launch card cwd hook-bin [prompt]]"),
    );
    let mut policy: Vec<String> = Vec::new();
    let mut raw: Option<PathBuf> = None;
    let mut agent = "claude".to_string();
    let mut instructions: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--policy" => {
                policy = args[i + 1].split(',').map(str::to_string).collect();
                i += 2;
            }
            "--raw" => {
                raw = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--agent" => {
                agent = args[i + 1].clone();
                i += 2;
            }
            "--instructions" => {
                instructions = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--launch" => {
                let registry = AdapterRegistry::default();
                let adapter = registry.get(&agent).expect("unknown agent");
                let state = sock.parent().unwrap().join("state");
                let mut hooks = HookConfig::new(&args[i + 3], &sock, state);
                hooks.approval_wait = Duration::from_secs(120);
                let mut spec = LaunchSpec::new(&args[i + 1], &args[i + 2]);
                spec.session_id = adapter.prepare_session_id(std::path::Path::new(&args[i + 2]));
                spec.prompt = args.get(i + 4).cloned();
                spec.hooks = Some(hooks);
                spec.instructions = instructions.clone();
                spec.extra_args = args[(i + 5).min(args.len())..].to_vec();
                let cmd = adapter.launch(&spec)?;
                println!("{}", cmd.shell_line_with_env());
                return Ok(());
            }
            _ => i += 1,
        }
    }

    let (receiver, events) = HookReceiver::bind(&sock, AdapterRegistry::default())?;
    eprintln!("listening on {}", receiver.path().display());
    if let Some(raw) = raw {
        let (tap_tx, tap_rx) = std::sync::mpsc::channel::<HookEnvelope>();
        receiver.set_raw_tap(Some(tap_tx));
        std::thread::spawn(move || {
            use std::io::Write;
            let mut f = std::fs::File::create(raw).unwrap();
            for env in tap_rx {
                let _ = writeln!(f, "{}", serde_json::to_string(&env).unwrap());
            }
        });
    }
    let mut trackers: std::collections::HashMap<String, StatusTracker> = Default::default();
    let mut answered = 0usize;
    for ev in events.iter() {
        println!("{}", serde_json::to_string(&ev).unwrap());
        let t = trackers.entry(ev.card_id.clone()).or_insert_with(|| StatusTracker::new(ev.at_ms));
        for change in t.apply(&ev) {
            println!("{}", serde_json::json!({"card": ev.card_id, "change": change}));
        }
        if let AgentEvent::PermissionRequested { request } = &ev.kind {
            let Some(id) = request.id.clone() else {
                continue;
            };
            let choice = policy.get(answered).cloned().unwrap_or_else(|| "none".into());
            answered += 1;
            let decision = match choice.as_str() {
                "allow" => Some(PermissionDecision::Allow { remember: false }),
                "remember" => Some(PermissionDecision::Allow { remember: true }),
                "deny" => Some(PermissionDecision::Deny {
                    message: Some("Denied by the Muster test receiver".into()),
                    interrupt: false,
                }),
                "deny-interrupt" => Some(PermissionDecision::Deny { message: None, interrupt: true }),
                "plan" => Some(PermissionDecision::ApprovePlan { accept_edits: false }),
                "plan-edits" => Some(PermissionDecision::ApprovePlan { accept_edits: true }),
                c if c.starts_with("answer:") => match &request.prompt {
                    PromptKind::Question { questions } => Some(PermissionDecision::Answer {
                        answers: questions
                            .iter()
                            .map(|q| (q.question.clone(), vec![c["answer:".len()..].to_string()]))
                            .collect(),
                    }),
                    _ => None,
                },
                _ => None,
            };
            if let Some(d) = decision {
                let receiver = receiver.clone();
                let id = id.clone();
                // Answer after a short pause, as a person on the board would.
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(2));
                    if let Err(e) = receiver.answer(&id, &d) {
                        eprintln!("answer {id}: {e}");
                    }
                });
            } else {
                eprintln!("leaving permission request {id} to the terminal");
            }
        }
    }
    Ok(())
}
