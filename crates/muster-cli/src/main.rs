//! `muster`: a small command-line client for the Muster daemon.

mod client;
mod term;

use client::Client;
use muster_proto::*;
use serde_json::Value;

const USAGE: &str = "usage: muster <command>

  status                              daemon and session summary
  project add <path> [--name N] [--color C]
  project list
  board <project>                     columns and cards (project id or name)
  card new <project> [--column C] [--agent A] [--title T] [--prompt P]
                     [--cwd DIR] [--worktree BRANCH] [--start]
  card move <card> <column> [--before CARD] [--after CARD]
  card start <card>
  card kill <card>                    kill the card's session
  card rm <card> [--kill]
  attach <card> [--watch]             interactive terminal; Ctrl-] detaches
  stats [--interval MS] [--count N]   memory/CPU per card while running
  link <card> <card> | unlink <card> <card>
  handoff <card> [--agent A]          new linked card that continues the work

For agents inside a card ($MUSTER_CARD_ID is the caller):
  context list                        cards linked to mine
  context read <card> [--summary | --transcript N | --screen | --notes | --comments]
                                      (a human outside a card adds --any)
  card move --me <column>
  notes set <text> | notes append <text>
  comment add <text> [--card CARD]    comment on my card (or a linked one), signed as me
  comment list [<card>]               comments on my card (or a linked one)
  send <card> <text>                  delivered when that agent is between turns

Cards may be given by id or unique id prefix; columns by id or title.
Uses $MUSTER_HOME (default ~/Library/Application Support/Muster).";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || matches!(args[0].as_str(), "-h" | "--help" | "help") {
        println!("{USAGE}");
        return;
    }
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let code = rt.block_on(async move {
        match run(args).await {
            Ok(code) => code,
            Err(e) => {
                eprintln!("muster: {e}");
                1
            }
        }
    });
    std::process::exit(code);
}

/// Splits `--flag value` options (and bare `--switch`es) from positionals.
struct Args {
    pos: Vec<String>,
    opts: Vec<(String, Option<String>)>,
}

const SWITCHES: &[&str] =
    &["--start", "--kill", "--watch", "--me", "--any", "--screen", "--notes", "--summary", "--comments"];

fn me() -> Option<String> {
    std::env::var("MUSTER_CARD_ID").ok().filter(|s| !s.is_empty())
}

impl Args {
    fn parse(raw: &[String]) -> Result<Args, String> {
        let mut pos = vec![];
        let mut opts = vec![];
        let mut it = raw.iter();
        while let Some(a) = it.next() {
            if a.starts_with("--") {
                if SWITCHES.contains(&a.as_str()) {
                    opts.push((a.clone(), None));
                } else {
                    let v = it.next().ok_or_else(|| format!("{a} needs a value"))?;
                    opts.push((a.clone(), Some(v.clone())));
                }
            } else {
                pos.push(a.clone());
            }
        }
        Ok(Args { pos, opts })
    }
    fn get(&self, k: &str) -> Option<String> {
        self.opts.iter().rev().find(|(n, _)| n == k).and_then(|(_, v)| v.clone())
    }
    fn has(&self, k: &str) -> bool {
        self.opts.iter().any(|(n, _)| n == k)
    }
    fn arg(&self, i: usize, what: &str) -> Result<&str, String> {
        self.pos.get(i).map(String::as_str).ok_or_else(|| format!("missing {what}\n\n{USAGE}"))
    }
}

async fn run(args: Vec<String>) -> Result<i32, String> {
    let cmd = args[0].as_str();
    let sub = args.get(1).map(String::as_str).unwrap_or("");
    let mut c = Client::connect().await?;
    match (cmd, sub) {
        ("status", _) => {
            let projects: ProjectListResult = c.call(Method::ProjectList(Empty {})).await?;
            println!("daemon {} pid {} port {}", c.daemon_version, c.pid, c.port);
            let mut live = 0;
            let mut total = 0;
            for p in &projects.projects {
                let b: BoardResult = c.call(Method::BoardGet(BoardGetParams { project_id: p.id.clone() })).await?;
                total += b.cards.len();
                for card in b.cards.iter().filter(|c| c.has_session) {
                    live += 1;
                    println!("  live  {}  {:<9} {}  ({})", card.id, card.status.as_str(), card.title, p.name);
                }
            }
            println!("{} project(s), {} card(s), {} live session(s)", projects.projects.len(), total, live);
        }
        ("project", "add") => {
            let a = Args::parse(&args[2..])?;
            let path = std::fs::canonicalize(a.arg(0, "path")?).map_err(|e| e.to_string())?;
            let r: ProjectResult = c
                .call(Method::ProjectAdd(ProjectAddParams {
                    path: path.to_string_lossy().into_owned(),
                    name: a.get("--name"),
                    color: a.get("--color"),
                }))
                .await?;
            println!("{}  {}  {}", r.project.id, r.project.name, r.project.path);
        }
        ("project", "list") | ("projects", _) => {
            let r: ProjectListResult = c.call(Method::ProjectList(Empty {})).await?;
            for p in r.projects {
                println!("{}  {:<20} {}", p.id, p.name, p.path);
            }
        }
        ("board", _) => {
            let a = Args::parse(&args[1..])?;
            let project = resolve_project(&mut c, a.arg(0, "project")?).await?;
            let b: BoardResult = c.call(Method::BoardGet(BoardGetParams { project_id: project.id })).await?;
            println!("{}  {}", b.project.name, b.project.path);
            for col in &b.columns {
                println!("\n{} [{}] ({})", col.title, col.category.as_str(), col.id);
                let mut cards: Vec<&Card> = b.cards.iter().filter(|c| c.column_id == col.id).collect();
                cards.sort_by(|a, b| a.rank.cmp(&b.rank));
                for card in cards {
                    println!(
                        "  {}  {:<9} {:<8} {}{}",
                        card.id,
                        card.status.as_str(),
                        card.agent_id,
                        card.title,
                        if card.has_session { "  *session*" } else { "" }
                    );
                }
            }
        }
        ("card", "new") => {
            let a = Args::parse(&args[2..])?;
            let project = resolve_project(&mut c, a.arg(0, "project")?).await?;
            let b: BoardResult = c.call(Method::BoardGet(BoardGetParams { project_id: project.id.clone() })).await?;
            let column = match a.get("--column") {
                Some(col) => resolve_column(&b, &col)?,
                None => b.columns.first().ok_or("project has no columns")?.id.clone(),
            };
            let r: CardResult = c
                .call(Method::CardCreate(CardCreateParams {
                    project_id: project.id,
                    column_id: column,
                    title: a.get("--title"),
                    agent_id: a.get("--agent").unwrap_or_else(|| "shell".into()),
                    cwd: a.get("--cwd"),
                    prompt: a.get("--prompt"),
                    start: a.has("--start"),
                    worktree: a.get("--worktree").map(|branch| WorktreeSpec { branch }),
                    resume_session_id: a.get("--resume"),
                }))
                .await?;
            println!("{}  {}  {}", r.card.id, r.card.status.as_str(), r.card.cwd);
        }
        ("card", "move") => {
            let a = Args::parse(&args[2..])?;
            let (card, col_arg) = if a.has("--me") {
                let id = me().ok_or("--me needs MUSTER_CARD_ID (run inside a card)")?;
                (get_card(&mut c, &id).await?, a.arg(0, "column")?)
            } else {
                (resolve_card(&mut c, a.arg(0, "card")?).await?, a.arg(1, "column")?)
            };
            let b: BoardResult =
                c.call(Method::BoardGet(BoardGetParams { project_id: card.project_id.clone() })).await?;
            let column = resolve_column(&b, col_arg)?;
            let before = match a.get("--before") {
                Some(x) => Some(resolve_card(&mut c, &x).await?.id),
                None => None,
            };
            let after = match a.get("--after") {
                Some(x) => Some(resolve_card(&mut c, &x).await?.id),
                None => None,
            };
            let r: CardResult =
                c.call(Method::CardMove(CardMoveParams { id: card.id, column_id: column, before, after })).await?;
            println!("{}  rank {}", r.card.id, r.card.rank);
        }
        ("card", "start") => {
            let a = Args::parse(&args[2..])?;
            let card = resolve_card(&mut c, a.arg(0, "card")?).await?;
            let r: CardResult = c.call(Method::CardStart(IdParams { id: card.id })).await?;
            println!("{}  {}", r.card.id, r.card.status.as_str());
        }
        ("card", "kill") => {
            let a = Args::parse(&args[2..])?;
            let card = resolve_card(&mut c, a.arg(0, "card")?).await?;
            let _: Value = c.call(Method::SessionKill(CardIdParams { card_id: card.id })).await?;
        }
        ("card", "rm") => {
            let a = Args::parse(&args[2..])?;
            let card = resolve_card(&mut c, a.arg(0, "card")?).await?;
            let _: Value =
                c.call(Method::CardDelete(CardDeleteParams { id: card.id, kill_session: a.has("--kill") })).await?;
        }
        ("context", "list") => {
            let a = Args::parse(&args[2..])?;
            let id = a.get("--card").or_else(me).ok_or("run inside a card, or pass --card <id>")?;
            let r: ContextListResult = c.call(Method::ContextList(ContextListParams { card_id: id })).await?;
            if r.cards.is_empty() {
                println!("(no linked cards)");
            }
            for card in r.cards {
                println!("{}  {:<9} {:<8} {}", card.id, card.status.as_str(), card.agent_id, card.title);
            }
        }
        ("context", "read") => {
            let a = Args::parse(&args[2..])?;
            let target = a.arg(0, "card")?.to_string();
            let from = me();
            if a.has("--any") && from.is_some() {
                return Err("--any is only for a human outside a card".into());
            }
            if from.is_none() && !a.has("--any") {
                return Err("not inside a card: pass --any to read any card".into());
            }
            let target = match &from {
                Some(_) => target,
                None => resolve_card(&mut c, &target).await?.id,
            };
            let (kind, limit) = if a.has("--screen") {
                (ContextKind::Screen, None)
            } else if a.has("--notes") {
                (ContextKind::Notes, None)
            } else if a.has("--comments") {
                (ContextKind::Comments, None)
            } else if let Some(n) = a.get("--transcript") {
                (ContextKind::Transcript, Some(n.parse().map_err(|_| "--transcript needs a number")?))
            } else {
                (ContextKind::Summary, None)
            };
            let r: TextResult = c
                .call(Method::ContextRead(ContextReadParams { card_id: target, from_card_id: from, kind, limit }))
                .await?;
            println!("{}", r.text);
        }
        ("notes", "set") | ("notes", "append") => {
            let a = Args::parse(&args[2..])?;
            let id = a.get("--card").or_else(me).ok_or("run inside a card, or pass --card <id>")?;
            let text = a.pos.join(" ");
            let card = get_card(&mut c, &id).await?;
            let notes = if sub == "append" {
                match card.notes.filter(|n| !n.is_empty()) {
                    Some(n) => format!("{n}\n{text}"),
                    None => text,
                }
            } else {
                text
            };
            let _: CardResult = c
                .call(Method::CardUpdate(CardUpdateParams {
                    id,
                    notes: Some(Some(notes)),
                    from_card_id: me(),
                    ..Default::default()
                }))
                .await?;
        }
        ("comment", "add") => {
            let a = Args::parse(&args[2..])?;
            let text = a.pos.join(" ");
            if text.trim().is_empty() {
                return Err("comment add needs the comment text".into());
            }
            let from = me();
            let card_id = match (a.get("--card"), &from) {
                (Some(t), Some(_)) => t,
                (Some(t), None) => resolve_card(&mut c, &t).await?.id,
                (None, Some(m)) => m.clone(),
                (None, None) => return Err("run inside a card, or pass --card <id>".into()),
            };
            let r: CommentResult =
                c.call(Method::CommentAdd(CommentAddParams { card_id, from_card_id: from, text })).await?;
            println!("commented on card {}", r.comment.card_id);
        }
        ("comment", "list") => {
            let a = Args::parse(&args[2..])?;
            let from = me();
            let card_id = match (a.pos.first().cloned(), &from) {
                (Some(t), Some(_)) => t,
                (Some(t), None) => resolve_card(&mut c, &t).await?.id,
                (None, Some(m)) => m.clone(),
                (None, None) => return Err("run inside a card, or name a card".into()),
            };
            let r: TextResult = c
                .call(Method::ContextRead(ContextReadParams {
                    card_id,
                    from_card_id: from,
                    kind: ContextKind::Comments,
                    limit: None,
                }))
                .await?;
            println!("{}", r.text);
        }
        ("send", _) => {
            let a = Args::parse(&args[1..])?;
            let target = a.arg(0, "card")?.to_string();
            let target = match me() {
                Some(_) => target,
                None => resolve_card(&mut c, &target).await?.id,
            };
            let text = a.pos[1..].join(" ");
            let r: MessageResult =
                c.call(Method::MessageSend(MessageSendParams { card_id: target, from_card_id: me(), text })).await?;
            println!(
                "message {} {}",
                r.message.id,
                if r.message.delivered_at.is_some() { "delivered" } else { "queued" }
            );
        }
        ("stats", _) => {
            let a = Args::parse(&args[1..])?;
            let interval_ms = a.get("--interval").and_then(|v| v.parse().ok());
            let count: u64 = a.get("--count").and_then(|v| v.parse().ok()).unwrap_or(u64::MAX);
            let _: StatsSubscribeResult = c.call(Method::StatsSubscribe(StatsSubscribeParams { interval_ms })).await?;
            let mut seen = 0;
            while seen < count {
                match c.next().await {
                    client::Incoming::Event(e) if e.m == "stats.changed" => {
                        let s: StatsChanged = serde_json::from_value(e.p).map_err(|e| e.to_string())?;
                        let mb = |b: u64| b as f64 / 1_048_576.0;
                        println!(
                            "daemon {:.1} MB {:.1}%  |  {} card(s) {:.1} MB {:.1}% {} procs",
                            mb(s.daemon.mem_bytes),
                            s.daemon.cpu_percent,
                            s.cards.len(),
                            mb(s.total.mem_bytes),
                            s.total.cpu_percent,
                            s.total.processes
                        );
                        for cs in &s.cards {
                            println!(
                                "  {}  {:.1} MB {:.1}% {} procs",
                                cs.card_id,
                                mb(cs.stats.mem_bytes),
                                cs.stats.cpu_percent,
                                cs.stats.processes
                            );
                        }
                        seen += 1;
                    }
                    client::Incoming::Closed => return Err("connection closed".into()),
                    _ => {}
                }
            }
        }
        ("link", _) | ("unlink", _) => {
            let a = Args::parse(&args[1..])?;
            let x = resolve_card(&mut c, a.arg(0, "card")?).await?.id;
            let y = resolve_card(&mut c, a.arg(1, "card")?).await?.id;
            let p = LinkParams { a: x, b: y };
            let m = if cmd == "link" { Method::LinkAdd(p) } else { Method::LinkRemove(p) };
            let _: serde_json::Value = c.call(m).await?;
        }
        ("handoff", _) => {
            let a = Args::parse(&args[1..])?;
            let card = resolve_card(&mut c, a.arg(0, "card")?).await?;
            let r: CardResult =
                c.call(Method::CardHandoff(CardHandoffParams { id: card.id, agent_id: a.get("--agent") })).await?;
            println!("{}  {}  {}", r.card.id, r.card.status.as_str(), r.card.title);
        }
        ("attach", _) => {
            let a = Args::parse(&args[1..])?;
            let card = resolve_card(&mut c, a.arg(0, "card")?).await?;
            return term::attach(c, &card.id, a.has("--watch")).await;
        }
        _ => return Err(format!("unknown command\n\n{USAGE}")),
    }
    Ok(0)
}

async fn resolve_project(c: &mut Client, key: &str) -> Result<Project, String> {
    let r: ProjectListResult = c.call(Method::ProjectList(Empty {})).await?;
    let matches: Vec<Project> = r
        .projects
        .into_iter()
        .filter(|p| p.id == key || p.name.eq_ignore_ascii_case(key) || p.id.starts_with(key))
        .collect();
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => Err(format!("no project matches {key:?}")),
        _ => Err(format!("{key:?} matches several projects")),
    }
}

fn resolve_column(b: &BoardResult, key: &str) -> Result<String, String> {
    b.columns
        .iter()
        .find(|c| c.id == key || c.title.eq_ignore_ascii_case(key))
        .map(|c| c.id.clone())
        .ok_or_else(|| format!("no column matches {key:?}"))
}

async fn get_card(c: &mut Client, id: &str) -> Result<Card, String> {
    let r: CardResult = c.call(Method::CardGet(IdParams { id: id.to_string() })).await?;
    Ok(r.card)
}

async fn resolve_card(c: &mut Client, key: &str) -> Result<Card, String> {
    if let Ok(card) = get_card(c, key).await {
        return Ok(card);
    }
    let r: ProjectListResult = c.call(Method::ProjectList(Empty {})).await?;
    let mut matches = vec![];
    for p in r.projects {
        let b: BoardResult = c.call(Method::BoardGet(BoardGetParams { project_id: p.id })).await?;
        for card in b.cards {
            if card.id == key {
                return Ok(card);
            }
            if card.id.starts_with(key) {
                matches.push(card);
            }
        }
    }
    match matches.len() {
        1 => Ok(matches.pop().unwrap()),
        0 => Err(format!("no card matches {key:?}")),
        _ => Err(format!("{key:?} matches several cards")),
    }
}
